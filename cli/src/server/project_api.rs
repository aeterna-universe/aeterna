use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, put};
use axum::{Json, Router};
use mk_core::traits::StorageBackend;
use mk_core::types::{
    GovernanceEvent, OrganizationalUnit, PersistentEvent, Role, RoleIdentifier, TenantContext,
    UnitType,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use storage::PrincipalType;
use uuid::Uuid;

use super::{AppState, tenant_scoped_context};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectListQuery {
    team: Option<String>,
    /// `?tenant=` — see #44.d RFC and `user_api::UserListQuery::tenant`.
    /// Accepted values: `*` (cross-tenant, PlatformAdmin), `all` (deprecated
    /// alias), `<slug>` (deferred to PR #65 cluster). Omitted → tenant-scoped
    /// behavior unchanged.
    tenant: Option<String>,
    /// Retained for backward compatibility with pre-RFC experimental clients.
    #[allow(dead_code)]
    all: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateProjectRequest {
    name: String,
    description: Option<String>,
    team_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemberBody {
    user_id: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RoleBody {
    role: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MemberResponse {
    user_id: String,
    role: RoleIdentifier,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamAssignmentBody {
    team_id: String,
    assignment_type: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TeamAssignmentResponse {
    team_id: String,
    assignment_type: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/project", get(list_projects).post(create_project))
        .route("/project/{project_id}", get(show_project))
        .route(
            "/project/{project_id}/members",
            get(list_members).post(add_member),
        )
        .route(
            "/project/{project_id}/members/{user_id}",
            delete(remove_member),
        )
        .route(
            "/project/{project_id}/members/{user_id}/role",
            put(set_member_role),
        )
        .route(
            "/project/{project_id}/teams",
            get(list_team_assignments).post(assign_team),
        )
        .route(
            "/project/{project_id}/teams/{team_id}",
            delete(remove_team_assignment),
        )
        .with_state(state)
}

async fn list_projects(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ProjectListQuery>,
) -> impl IntoResponse {
    // #44.d §2.3 — cross-tenant listing dispatch via shared helper.
    match super::context::resolve_list_scope(
        &state,
        &headers,
        query.tenant.as_deref(),
        "/project",
        true, // supports ?tenant=<slug>
    )
    .await
    {
        super::context::ListDispatch::TenantScoped => { /* fall through */ }
        super::context::ListDispatch::CrossTenant => {
            return list_projects_cross_tenant(&state, &query, None).await;
        }
        super::context::ListDispatch::CrossTenantSingle(t) => {
            return list_projects_cross_tenant(&state, &query, Some(&t)).await;
        }
        super::context::ListDispatch::Response(r) => return r,
    }

    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let mut units = match state.postgres.list_all_units().await {
        Ok(units) => units,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_list_failed",
                &err.to_string(),
            );
        }
    };
    units.retain(|unit| unit.tenant_id == ctx.tenant_id && unit.unit_type == UnitType::Project);
    if let Some(team_id) = query.team.as_deref() {
        units.retain(|unit| unit.parent_id.as_deref() == Some(team_id));
    }
    units.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Json(units).into_response()
}

/// Cross-tenant project listing — serves both `?tenant=*` (PlatformAdmin,
/// `single_tenant=None`) and `?tenant=<slug>` (PlatformAdmin,
/// `single_tenant=Some(...)`).
///
/// The only difference between the two modes is an additional row-level
/// filter when `single_tenant` is Some, plus the envelope's `scope` field
/// and an echoed `tenant` object so clients can tell which tenant they are
/// looking at. Per-item decoration (`tenantId`/`tenantSlug`/`tenantName`)
/// is identical in both modes — this keeps the items-array contract uniform.
async fn list_projects_cross_tenant(
    state: &AppState,
    query: &ProjectListQuery,
    single_tenant: Option<&super::context::ResolvedTenant>,
) -> axum::response::Response {
    let mut units = match state.postgres.list_all_units().await {
        Ok(units) => units,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_list_failed",
                &err.to_string(),
            );
        }
    };
    units.retain(|unit| unit.unit_type == UnitType::Project);
    // Single-foreign-tenant narrowing. Applied after list_all_units so the
    // cross-tenant SELECT is reused verbatim; the alternative would be a
    // new single-tenant store method which would drift from the all-tenant
    // one (ordering, filters, decoration — all would need to stay in lock
    // step across two call sites).
    if let Some(t) = single_tenant {
        units.retain(|unit| unit.tenant_id.as_str() == t.id.as_str());
    }
    if let Some(team_id) = query.team.as_deref() {
        units.retain(|unit| unit.parent_id.as_deref() == Some(team_id));
    }
    // Stable ordering: tenant first, then project name, then id — makes the
    // output diffable and safe for clients that paginate visually.
    units.sort_by(|a, b| {
        a.tenant_id
            .cmp(&b.tenant_id)
            .then(a.name.cmp(&b.name))
            .then(a.id.cmp(&b.id))
    });

    // Pre-fetch tenant metadata once and decorate each item. Two DB round
    // trips total (units + tenants) regardless of project count.
    let tenants = match state.tenant_store.list_tenants(true).await {
        Ok(ts) => ts,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "tenant_lookup_failed",
                &err.to_string(),
            );
        }
    };
    let tenant_by_id: std::collections::HashMap<String, &mk_core::types::TenantRecord> = tenants
        .iter()
        .map(|t| (t.id.as_str().to_string(), t))
        .collect();

    let items: Vec<serde_json::Value> = units
        .into_iter()
        .map(|unit| {
            let t = tenant_by_id.get(unit.tenant_id.as_str());
            json!({
                "id":         unit.id,
                "name":       unit.name,
                "parentId":   unit.parent_id,
                "unitType":   "project",
                "tenantId":   unit.tenant_id,
                "tenantSlug": t.map(|t| t.slug.clone()),
                "tenantName": t.map(|t| t.name.clone()),
            })
        })
        .collect();

    // Envelope shape is intentionally consistent across ?tenant=* and
    // ?tenant=<slug>: same top-level keys, same items[] contract. The only
    // differentiators are the `scope` discriminant ("all" vs "tenant") and
    // an echoed `tenant` object in single-tenant mode so clients can render
    // the current view without a second lookup. We OMIT the key entirely in
    // "all" mode rather than serializing it as `null` — it would be
    // meaningless noise and would need to be documented away in the §4.1
    // contract.
    let body = match single_tenant {
        None => json!({
            "success": true,
            "scope":   "all",
            "items":   items,
        }),
        Some(t) => json!({
            "success": true,
            "scope":   "tenant",
            "tenant":  { "id": t.id, "slug": t.slug, "name": t.name },
            "items":   items,
        }),
    };
    (StatusCode::OK, Json(body)).into_response()
}

async fn show_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    match get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        Ok(unit) => Json(unit).into_response(),
        Err(response) => response,
    }
}

async fn create_project(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::RawQuery(raw_query): axum::extract::RawQuery,
    Json(req): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    // #44.d §5.8 — block write-with-?tenant=* (see user_api for rationale).
    if let Some(resp) = super::context::reject_cross_tenant_write(raw_query.as_deref(), "/project")
    {
        return resp;
    }
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_create_project_permission(&state, &ctx).await {
        return response;
    }
    if get_unit_of_type(&state, &ctx, &req.team_id, UnitType::Team)
        .await
        .is_err()
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_team",
            "teamId must reference an existing team unit in the target tenant",
        );
    }

    let now = chrono::Utc::now().timestamp();
    let mut metadata = HashMap::new();
    if let Some(description) = req.description.clone() {
        metadata.insert("description".to_string(), json!(description));
    }
    let unit = OrganizationalUnit {
        id: Uuid::new_v4().to_string(),
        name: req.name,
        unit_type: UnitType::Project,
        parent_id: Some(req.team_id),
        tenant_id: ctx.tenant_id.clone(),
        metadata,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: mk_core::types::RecordSource::Admin,
    };

    match state.postgres.create_unit_scoped(&ctx, &unit).await {
        Ok(()) => {
            persist_event(
                &state,
                &ctx,
                &GovernanceEvent::UnitCreated {
                    unit_id: unit.id.clone(),
                    unit_type: unit.unit_type,
                    tenant_id: ctx.tenant_id.clone(),
                    parent_id: unit.parent_id.clone(),
                    timestamp: now,
                },
            )
            .await;
            audit_action(
                &state,
                &ctx,
                "project_create",
                Some(unit.id.as_str()),
                json!({"name": unit.name, "parentId": unit.parent_id}),
            )
            .await;
            (StatusCode::CREATED, Json(unit)).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_create_failed",
            &err.to_string(),
        ),
    }
}

async fn list_members(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }

    match state
        .postgres
        .list_unit_roles_scoped(&ctx, &project_id)
        .await
    {
        Ok(entries) => Json(
            entries
                .into_iter()
                .map(|(user_id, role)| MemberResponse {
                    user_id: user_id.into_inner(),
                    role,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_members_list_failed",
            &err.to_string(),
        ),
    }
}

async fn add_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(req): Json<MemberBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }

    let Some(user_id) = mk_core::types::UserId::new(req.user_id.clone()) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let role = match parse_project_role(&req.role) {
        Ok(role) => role,
        Err(response) => return response,
    };

    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &project_id, role.clone())
        .await
    {
        Ok(()) => {
            persist_event(
                &state,
                &ctx,
                &GovernanceEvent::RoleAssigned {
                    user_id: user_id.clone(),
                    unit_id: project_id.clone(),
                    role: role.clone(),
                    tenant_id: ctx.tenant_id.clone(),
                    timestamp: chrono::Utc::now().timestamp(),
                },
            )
            .await;
            audit_action(
                &state,
                &ctx,
                "project_member_add",
                Some(project_id.as_str()),
                json!({"userId": user_id.as_str(), "role": role}),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({"userId": user_id.as_str(), "role": role})),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_member_add_failed",
            &err.to_string(),
        ),
    }
}

async fn remove_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((project_id, user_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }
    let Some(user_id) = mk_core::types::UserId::new(user_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let existing = match state
        .postgres
        .list_unit_roles_scoped(&ctx, &project_id)
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_member_remove_failed",
                &err.to_string(),
            );
        }
    };
    let current_roles: Vec<RoleIdentifier> = existing
        .into_iter()
        .filter(|(candidate, _)| candidate.as_str() == user_id.as_str())
        .map(|(_, role)| role)
        .collect();
    if current_roles.is_empty() {
        return error_response(
            StatusCode::NOT_FOUND,
            "project_member_not_found",
            "Member has no roles in this project",
        );
    }
    for role in &current_roles {
        if let Err(err) = state
            .postgres
            .remove_role_scoped(&ctx, &user_id, &project_id, role.clone())
            .await
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_member_remove_failed",
                &err.to_string(),
            );
        }
    }
    audit_action(
        &state,
        &ctx,
        "project_member_remove",
        Some(project_id.as_str()),
        json!({"userId": user_id.as_str(), "removedRoles": current_roles}),
    )
    .await;
    Json(json!({"success": true})).into_response()
}

async fn set_member_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((project_id, user_id)): Path<(String, String)>,
    Json(req): Json<RoleBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }

    let Some(user_id) = mk_core::types::UserId::new(user_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let role = match parse_project_role(&req.role) {
        Ok(role) => role,
        Err(response) => return response,
    };
    let existing = match state
        .postgres
        .list_unit_roles_scoped(&ctx, &project_id)
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_member_set_role_failed",
                &err.to_string(),
            );
        }
    };
    for current_role in existing
        .into_iter()
        .filter(|(candidate, _)| candidate.as_str() == user_id.as_str())
        .map(|(_, role)| role)
    {
        if let Err(err) = state
            .postgres
            .remove_role_scoped(&ctx, &user_id, &project_id, current_role)
            .await
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "project_member_set_role_failed",
                &err.to_string(),
            );
        }
    }
    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &project_id, role.clone())
        .await
    {
        Ok(()) => {
            audit_action(
                &state,
                &ctx,
                "project_member_set_role",
                Some(project_id.as_str()),
                json!({"userId": user_id.as_str(), "role": role}),
            )
            .await;
            Json(json!({"userId": user_id.as_str(), "role": role})).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_member_set_role_failed",
            &err.to_string(),
        ),
    }
}

async fn list_team_assignments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }

    match state
        .postgres
        .list_project_team_assignments(&project_id, ctx.tenant_id.as_str())
        .await
    {
        Ok(assignments) => Json(
            assignments
                .into_iter()
                .map(|(team_id, assignment_type)| TeamAssignmentResponse {
                    team_id,
                    assignment_type,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_team_assignments_list_failed",
            &err.to_string(),
        ),
    }
}

async fn assign_team(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(project_id): Path<String>,
    Json(req): Json<TeamAssignmentBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_manage_team_assignments_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &req.team_id, UnitType::Team).await {
        return response;
    }

    if req.assignment_type != "owner" && req.assignment_type != "contributor" {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_assignment_type",
            "assignmentType must be either 'owner' or 'contributor'",
        );
    }

    match state
        .postgres
        .assign_team_to_project(
            &project_id,
            &req.team_id,
            ctx.tenant_id.as_str(),
            &req.assignment_type,
        )
        .await
    {
        Ok(()) => {
            audit_action(
                &state,
                &ctx,
                "project_team_assign",
                Some(project_id.as_str()),
                json!({"teamId": req.team_id, "assignmentType": req.assignment_type}),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(json!({"teamId": req.team_id, "assignmentType": req.assignment_type})),
            )
                .into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_team_assign_failed",
            &err.to_string(),
        ),
    }
}

async fn remove_team_assignment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((project_id, team_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_manage_team_assignments_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &project_id, UnitType::Project).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        return response;
    }

    match state
        .postgres
        .remove_team_from_project(&project_id, &team_id, ctx.tenant_id.as_str())
        .await
    {
        Ok(()) => {
            audit_action(
                &state,
                &ctx,
                "project_team_remove",
                Some(project_id.as_str()),
                json!({"teamId": team_id}),
            )
            .await;
            Json(json!({"success": true})).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "project_team_remove_failed",
            &err.to_string(),
        ),
    }
}

async fn require_admin_context(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantContext, axum::response::Response> {
    let ctx = tenant_scoped_context(state, headers).await?;
    if ctx.has_known_role(&Role::PlatformAdmin) || ctx.has_known_role(&Role::Admin) {
        Ok(ctx)
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Admin or PlatformAdmin role required",
        ))
    }
}

async fn require_assign_roles_permission(
    state: &AppState,
    ctx: &TenantContext,
) -> Result<(), axum::response::Response> {
    let resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(ctx, "AssignRoles", &resource)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "AssignRoles permission required for this tenant",
        )),
        Err(err) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authz_check_failed",
            &err.to_string(),
        )),
    }
}

async fn require_create_project_permission(
    state: &AppState,
    ctx: &TenantContext,
) -> Result<(), axum::response::Response> {
    let resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(ctx, "CreateProject", &resource)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "CreateProject permission required for this tenant",
        )),
        Err(err) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authz_check_failed",
            &err.to_string(),
        )),
    }
}

async fn require_manage_team_assignments_permission(
    state: &AppState,
    ctx: &TenantContext,
) -> Result<(), axum::response::Response> {
    let resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(ctx, "ManageTeamAssignments", &resource)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(error_response(
            StatusCode::FORBIDDEN,
            "forbidden",
            "ManageTeamAssignments permission required for this tenant",
        )),
        Err(err) => Err(error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "authz_check_failed",
            &err.to_string(),
        )),
    }
}

async fn get_unit_of_type(
    state: &AppState,
    ctx: &TenantContext,
    unit_id: &str,
    expected: UnitType,
) -> Result<OrganizationalUnit, axum::response::Response> {
    match state.postgres.get_unit_scoped(ctx, unit_id).await {
        Ok(Some(unit)) if unit.unit_type == expected => Ok(unit),
        Ok(Some(_)) => Err(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_unit_type",
            "Unit exists but has the wrong type for this operation",
        )),
        Ok(None) => Err(error_response(
            StatusCode::NOT_FOUND,
            "unit_not_found",
            "Requested unit was not found",
        )),
        Err(err) => Err(error_response(
            StatusCode::BAD_REQUEST,
            "unit_lookup_failed",
            &err.to_string(),
        )),
    }
}

fn parse_project_role(value: &str) -> Result<RoleIdentifier, axum::response::Response> {
    let role = RoleIdentifier::from_str_flexible(value);
    if matches!(
        role,
        RoleIdentifier::Known(Role::PlatformAdmin | Role::Admin)
    ) {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "Project roles cannot be PlatformAdmin or Admin",
        ));
    }
    Ok(role)
}

async fn persist_event(state: &AppState, ctx: &TenantContext, event: &GovernanceEvent) {
    // Clone so the boxed future owns the event and satisfies the `'static`
    // bound required by `with_tenant_context`'s HRTB. See the contract in
    // `PostgresBackend::log_event`.
    let event_owned = event.clone();
    let _ = state
        .postgres
        .with_tenant_context(ctx, move |tx| {
            Box::pin(async move {
                storage::postgres::PostgresBackend::log_event(tx, &event_owned).await
            })
        })
        .await;
    let _ = state
        .postgres
        .persist_event(PersistentEvent::new(event.clone()))
        .await;
}

async fn audit_action(
    state: &AppState,
    ctx: &TenantContext,
    action: &str,
    target_id: Option<&str>,
    details: serde_json::Value,
) {
    let Some(storage) = &state.governance_storage else {
        return;
    };
    let actor_id = Uuid::parse_str(ctx.user_id.as_str()).ok();
    // #44.d §2.5 — `acting_as_tenant_id` = impersonated tenant if set, else
    // the actor's own tenant.
    let acting_as = Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    let _ = storage
        .log_audit(
            action,
            None,
            Some("project"),
            target_id,
            PrincipalType::User,
            actor_id,
            None,
            json!({"actorTenantId": ctx.tenant_id.as_str(), "details": details}),
            acting_as,
        )
        .await;
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (status, Json(json!({"error": error, "message": message}))).into_response()
}
