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
struct TeamListQuery {
    org: Option<String>,
    #[allow(dead_code)]
    all: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTeamRequest {
    name: String,
    description: Option<String>,
    org_id: String,
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

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/team", get(list_teams).post(create_team))
        .route("/team/{team_id}", get(show_team))
        .route(
            "/team/{team_id}/members",
            get(list_members).post(add_member),
        )
        .route("/team/{team_id}/members/{user_id}", delete(remove_member))
        .route(
            "/team/{team_id}/members/{user_id}/role",
            put(set_member_role),
        )
        .with_state(state)
}

async fn list_teams(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<TeamListQuery>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let mut units = match state.postgres.list_all_units().await {
        Ok(units) => units,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "team_list_failed",
                &err.to_string(),
            );
        }
    };
    units.retain(|unit| unit.tenant_id == ctx.tenant_id && unit.unit_type == UnitType::Team);
    if let Some(org_id) = query.org.as_deref() {
        units.retain(|unit| unit.parent_id.as_deref() == Some(org_id));
    }
    units.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Json(units).into_response()
}

async fn show_team(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    match get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        Ok(unit) => Json(unit).into_response(),
        Err(response) => response,
    }
}

async fn create_team(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<CreateTeamRequest>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if get_unit_of_type(&state, &ctx, &req.org_id, UnitType::Organization)
        .await
        .is_err()
    {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_org",
            "orgId must reference an existing organization unit in the target tenant",
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
        unit_type: UnitType::Team,
        parent_id: Some(req.org_id),
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
                "team_create",
                Some(unit.id.as_str()),
                json!({"name": unit.name, "parentId": unit.parent_id}),
            )
            .await;
            (StatusCode::CREATED, Json(unit)).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "team_create_failed",
            &err.to_string(),
        ),
    }
}

async fn list_members(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        return response;
    }

    match state.postgres.list_unit_roles_scoped(&ctx, &team_id).await {
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
            "team_members_list_failed",
            &err.to_string(),
        ),
    }
}

async fn add_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(team_id): Path<String>,
    Json(req): Json<MemberBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        return response;
    }

    let Some(user_id) = mk_core::types::UserId::new(req.user_id.clone()) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let role = match parse_team_role(&req.role) {
        Ok(role) => role,
        Err(response) => return response,
    };

    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &team_id, role.clone())
        .await
    {
        Ok(()) => {
            persist_event(
                &state,
                &ctx,
                &GovernanceEvent::RoleAssigned {
                    user_id: user_id.clone(),
                    unit_id: team_id.clone(),
                    role: role.clone(),
                    tenant_id: ctx.tenant_id.clone(),
                    timestamp: chrono::Utc::now().timestamp(),
                },
            )
            .await;
            audit_action(
                &state,
                &ctx,
                "team_member_add",
                Some(team_id.as_str()),
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
            "team_member_add_failed",
            &err.to_string(),
        ),
    }
}

async fn remove_member(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((team_id, user_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        return response;
    }
    let Some(user_id) = mk_core::types::UserId::new(user_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let existing = match state.postgres.list_unit_roles_scoped(&ctx, &team_id).await {
        Ok(entries) => entries,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "team_member_remove_failed",
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
            "team_member_not_found",
            "Member has no roles in this team",
        );
    }
    for role in &current_roles {
        if let Err(err) = state
            .postgres
            .remove_role_scoped(&ctx, &user_id, &team_id, role.clone())
            .await
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "team_member_remove_failed",
                &err.to_string(),
            );
        }
    }
    audit_action(
        &state,
        &ctx,
        "team_member_remove",
        Some(team_id.as_str()),
        json!({"userId": user_id.as_str(), "removedRoles": current_roles}),
    )
    .await;
    Json(json!({"success": true})).into_response()
}

async fn set_member_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((team_id, user_id)): Path<(String, String)>,
    Json(req): Json<RoleBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = require_assign_roles_permission(&state, &ctx).await {
        return response;
    }
    if let Err(response) = get_unit_of_type(&state, &ctx, &team_id, UnitType::Team).await {
        return response;
    }

    let Some(user_id) = mk_core::types::UserId::new(user_id) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "Invalid user id",
        );
    };
    let role = match parse_team_role(&req.role) {
        Ok(role) => role,
        Err(response) => return response,
    };
    let existing = match state.postgres.list_unit_roles_scoped(&ctx, &team_id).await {
        Ok(entries) => entries,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "team_member_set_role_failed",
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
            .remove_role_scoped(&ctx, &user_id, &team_id, current_role)
            .await
        {
            return error_response(
                StatusCode::BAD_REQUEST,
                "team_member_set_role_failed",
                &err.to_string(),
            );
        }
    }
    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &team_id, role.clone())
        .await
    {
        Ok(()) => {
            audit_action(
                &state,
                &ctx,
                "team_member_set_role",
                Some(team_id.as_str()),
                json!({"userId": user_id.as_str(), "role": role}),
            )
            .await;
            Json(json!({"userId": user_id.as_str(), "role": role})).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "team_member_set_role_failed",
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

fn parse_team_role(value: &str) -> Result<RoleIdentifier, axum::response::Response> {
    let role = RoleIdentifier::from_str_flexible(value);
    if matches!(
        role,
        RoleIdentifier::Known(Role::PlatformAdmin | Role::Admin)
    ) {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_role",
            "Team roles cannot be PlatformAdmin or Admin",
        ));
    }
    Ok(role)
}

async fn persist_event(state: &AppState, ctx: &TenantContext, event: &GovernanceEvent) {
    // Write the governance event inside a per-tenant transaction so the
    // `governance_events` RLS policy (keyed on `app.tenant_id`) admits the
    // INSERT. See `PostgresBackend::log_event` rustdoc for the contract.
    //
    // We clone `event` into the closure so the boxed future owns its
    // payload and satisfies the `'static` bound required by
    // `with_tenant_context`'s HRTB.
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
    // the actor's own tenant. Drives `/govern/audit?tenant=<slug>`.
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
            Some("team"),
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
