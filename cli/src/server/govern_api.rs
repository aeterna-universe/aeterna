use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get};
use axum::{Json, Router};
use mk_core::types::{
    KnowledgeLayer, Policy, PolicyMode, Role, RuleMergeStrategy, SYSTEM_USER_ID, TenantContext,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use storage::governance::{
    ApprovalMode, AuditFilters, CreateDecision, CreateGovernanceRole, Decision, PrincipalType,
    RequestFilters, RequestType,
};
use uuid::Uuid;

use super::{AppState, tenant_scoped_context};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingQuery {
    #[serde(rename = "type")]
    request_type: Option<String>,
    layer: Option<String>,
    requestor: Option<String>,
    mine: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopeQuery {
    scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApproveRejectBody {
    comment: Option<String>,
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateGovernConfigRequest {
    approval_mode: Option<String>,
    min_approvers: Option<i32>,
    timeout_hours: Option<i32>,
    auto_approve: Option<bool>,
    escalation_contact: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignGovernRoleRequest {
    principal: String,
    role: String,
    scope: Option<String>,
    principal_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuditQuery {
    action: Option<String>,
    since: Option<String>,
    actor: Option<String>,
    target_type: Option<String>,
    limit: Option<usize>,
    /// `?tenant=` — see #44.d RFC. Accepts `*` / `all` / `<slug>`.
    ///
    /// Per-row tenant attribution is via `governance_audit_log.acting_as_tenant_id`
    /// (migration 023, populated by every tenant-scoped `log_audit` call site
    /// since Bundle D). `?tenant=<slug>` filters on that column; `?tenant=*`
    /// returns the full cross-tenant stream in envelope form with each row
    /// decorated with `tenantId`/`tenantSlug`. Rows written before Bundle D
    /// have `acting_as_tenant_id = NULL` and are therefore excluded by slug
    /// filters (they remain visible under `?tenant=*`).
    tenant: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GovernRoleResponse {
    principal: String,
    principal_type: String,
    role: String,
    scope: String,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/govern/status", get(status))
        .route("/govern/pending", get(list_pending))
        .route(
            "/govern/approve/{request_id}",
            axum::routing::post(approve_request),
        )
        .route(
            "/govern/reject/{request_id}",
            axum::routing::post(reject_request),
        )
        .route("/govern/config", get(show_config).put(update_config))
        .route("/govern/audit", get(list_audit))
        .route("/govern/roles", get(list_roles).post(assign_role))
        .route("/govern/roles/{principal}/{role}", delete(revoke_role))
        .route("/govern/policies", get(list_policies).post(create_policy))
        .with_state(state)
}

async fn status(
    State(state): State<Arc<AppState>>,
    Query(scope_q): Query<ScopeQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };

    let (tenant_id, org_id, team_id, project_id) =
        match current_scope_ids(&state, &ctx, scope_q.scope.as_deref()).await {
            Ok(ids) => ids,
            Err(response) => return response,
        };

    let config = match storage
        .get_effective_config(tenant_id, org_id, team_id, project_id)
        .await
    {
        Ok(config) => config,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_status_failed",
                &err.to_string(),
            );
        }
    };
    let pending_all = match storage
        .list_pending_requests(&RequestFilters {
            tenant_id,
            org_id,
            team_id,
            project_id,
            limit: Some(200),
            ..Default::default()
        })
        .await
    {
        Ok(requests) => requests,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_status_failed",
                &err.to_string(),
            );
        }
    };
    let your_pending = pending_all
        .iter()
        .filter(|request| request.requestor_id != actor_uuid(ctx.user_id.as_str()))
        .count();

    Json(json!({
        "config": config,
        "metrics": {
            "pendingRequests": pending_all.len(),
            "yourPendingApprovals": your_pending,
            "scope": current_scope_string(org_id, team_id, project_id),
        }
    }))
    .into_response()
}

async fn list_pending(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<PendingQuery>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };

    let request_type = match query.request_type.as_deref() {
        Some("all") | None => None,
        Some(value) => match value.parse::<RequestType>() {
            Ok(parsed) => Some(parsed),
            Err(err) => {
                return error_response(StatusCode::BAD_REQUEST, "invalid_request_type", &err);
            }
        },
    };
    let (tenant_id, org_id, team_id, project_id) =
        match scope_ids_for_layer(&state, &ctx, query.layer.as_deref()).await {
            Ok(ids) => ids,
            Err(response) => return response,
        };
    let requestor_id = if query.mine.unwrap_or(false) {
        Some(actor_uuid(ctx.user_id.as_str()))
    } else {
        query
            .requestor
            .as_deref()
            .and_then(|value| Uuid::parse_str(value).ok())
    };

    match storage
        .list_pending_requests(&RequestFilters {
            request_type,
            tenant_id,
            org_id,
            team_id,
            project_id,
            requestor_id,
            limit: Some(100),
        })
        .await
    {
        Ok(requests) => Json(requests).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "govern_pending_failed",
            &err.to_string(),
        ),
    }
}

async fn approve_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(body): Json<ApproveRejectBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let request_id = match parse_request_id(storage, &request_id).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    let decision = match storage
        .add_decision(&CreateDecision {
            request_id,
            approver_type: PrincipalType::User,
            approver_id: actor_uuid(ctx.user_id.as_str()),
            approver_email: None,
            decision: Decision::Approve,
            comment: body.comment.clone(),
        })
        .await
    {
        Ok(decision) => decision,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_approve_failed",
                &err.to_string(),
            );
        }
    };
    let request = match storage.get_request(request_id).await {
        Ok(Some(request)) => request,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "request_not_found",
                "Approval request was not found",
            );
        }
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_approve_failed",
                &err.to_string(),
            );
        }
    };
    // #44.d §2.5 — `acting_as_tenant_id` = impersonated tenant if set, else
    // actor's own tenant. Governance actions on a target tenant MUST be
    // attributable to that tenant.
    let acting_as = Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    let _ = storage
        .log_audit(
            "approve",
            Some(request_id),
            Some(&request.target_type),
            request.target_id.as_deref(),
            PrincipalType::User,
            Some(actor_uuid(ctx.user_id.as_str())),
            None,
            json!({"comment": body.comment}),
            acting_as,
        )
        .await;

    Json(json!({"decision": decision, "request": request})).into_response()
}

async fn reject_request(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(request_id): Path<String>,
    Json(body): Json<ApproveRejectBody>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let Some(reason) = body.reason.as_deref() else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "reason_required",
            "Rejection reason is required",
        );
    };
    let request_id = match parse_request_id(storage, &request_id).await {
        Ok(id) => id,
        Err(response) => return response,
    };

    let _ = storage
        .add_decision(&CreateDecision {
            request_id,
            approver_type: PrincipalType::User,
            approver_id: actor_uuid(ctx.user_id.as_str()),
            approver_email: None,
            decision: Decision::Reject,
            comment: Some(reason.to_string()),
        })
        .await;
    let request = match storage.reject_request(request_id, reason).await {
        Ok(request) => request,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_reject_failed",
                &err.to_string(),
            );
        }
    };
    // #44.d §2.5 — see `approve_request` for rationale.
    let acting_as = Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    let _ = storage
        .log_audit(
            "reject",
            Some(request_id),
            Some(&request.target_type),
            request.target_id.as_deref(),
            PrincipalType::User,
            Some(actor_uuid(ctx.user_id.as_str())),
            None,
            json!({"reason": reason}),
            acting_as,
        )
        .await;

    Json(request).into_response()
}

async fn show_config(
    State(state): State<Arc<AppState>>,
    Query(scope_q): Query<ScopeQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let (tenant_id, org_id, team_id, project_id) =
        match current_scope_ids(&state, &ctx, scope_q.scope.as_deref()).await {
            Ok(ids) => ids,
            Err(response) => return response,
        };

    match storage
        .get_effective_config(tenant_id, org_id, team_id, project_id)
        .await
    {
        Ok(config) => Json(config).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "govern_config_show_failed",
            &err.to_string(),
        ),
    }
}

async fn update_config(
    State(state): State<Arc<AppState>>,
    Query(scope_q): Query<ScopeQuery>,
    headers: HeaderMap,
    Json(req): Json<UpdateGovernConfigRequest>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let (tenant_id, org_id, team_id, project_id) =
        match current_scope_ids(&state, &ctx, scope_q.scope.as_deref()).await {
            Ok(ids) => ids,
            Err(response) => return response,
        };

    let mut config = match storage
        .get_effective_config(tenant_id, org_id, team_id, project_id)
        .await
    {
        Ok(config) => config,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_config_update_failed",
                &err.to_string(),
            );
        }
    };

    if let Some(mode) = req.approval_mode.as_deref() {
        config.approval_mode = match mode.parse::<ApprovalMode>() {
            Ok(mode) => mode,
            Err(err) => {
                return error_response(StatusCode::BAD_REQUEST, "invalid_approval_mode", &err);
            }
        };
    }
    if let Some(min_approvers) = req.min_approvers {
        config.min_approvers = min_approvers;
    }
    if let Some(timeout_hours) = req.timeout_hours {
        config.timeout_hours = timeout_hours;
    }
    if let Some(auto_approve) = req.auto_approve {
        config.auto_approve_low_risk = auto_approve;
    }
    if let Some(escalation_contact) = req.escalation_contact.clone() {
        config.escalation_contact = Some(escalation_contact);
    }
    config.tenant_id = tenant_id;
    config.org_id = org_id;
    config.team_id = team_id;
    config.project_id = project_id;

    // #44.d §2.5 — see `approve_request` for rationale.
    let acting_as = Uuid::parse_str(
        ctx.target_tenant_id
            .as_ref()
            .map_or(ctx.tenant_id.as_str(), mk_core::TenantId::as_str),
    )
    .ok();
    match storage.upsert_config(&config).await {
        Ok(id) => {
            let _ = storage
                .log_audit(
                    "update_config",
                    None,
                    Some("config"),
                    Some(&id.to_string()),
                    PrincipalType::User,
                    Some(actor_uuid(ctx.user_id.as_str())),
                    None,
                    json!({
                        "approvalMode": config.approval_mode,
                        "minApprovers": config.min_approvers,
                        "timeoutHours": config.timeout_hours,
                        "autoApproveLowRisk": config.auto_approve_low_risk,
                        "escalationContact": config.escalation_contact,
                    }),
                    acting_as,
                )
                .await;
            Json(json!({"id": id, "config": config})).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "govern_config_update_failed",
            &err.to_string(),
        ),
    }
}

/// `AuditScope` captures the #44.d-resolved request shape after the
/// `?tenant=` parser has run. It decides both the response envelope shape
/// and the `acting_as_tenant_id` filter applied at the storage layer.
enum AuditScope {
    /// No `?tenant=` parameter — pre-Bundle-D wire shape (bare JSON array),
    /// no per-item tenant decoration, no row filter.
    Legacy,
    /// `?tenant=*` (or deprecated `all`) — envelope with `scope:"all"`,
    /// per-item tenant decoration, no row filter.
    All,
    /// `?tenant=<slug|uuid>` — envelope with `scope:"tenant"`, per-item
    /// tenant decoration, `acting_as_tenant_id` filter applied.
    Single(super::context::ResolvedTenant),
}

async fn list_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuditQuery>,
) -> impl IntoResponse {
    // #44.d §2.5 (Bundle D) — /govern/audit now supports the full ?tenant=
    // grammar. Since Bundle D, `governance_audit_log.acting_as_tenant_id`
    // carries per-row tenant attribution for every write path (migration
    // 023 + the Bundle-D log_audit call-site sweep), so `?tenant=<slug>`
    // can filter deterministically instead of returning 501.
    let scope = match super::context::resolve_list_scope(
        &state,
        &headers,
        query.tenant.as_deref(),
        "/govern/audit",
        /* supports_single = */ true,
    )
    .await
    {
        super::context::ListDispatch::TenantScoped => AuditScope::Legacy,
        super::context::ListDispatch::CrossTenant => AuditScope::All,
        super::context::ListDispatch::CrossTenantSingle(t) => AuditScope::Single(t),
        super::context::ListDispatch::Response(resp) => return resp,
    };

    let _ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let since = match parse_since(query.since.as_deref().unwrap_or("7d")) {
        Ok(since) => since,
        Err(message) => {
            return error_response(StatusCode::BAD_REQUEST, "invalid_since", &message);
        }
    };
    let actor_id = query
        .actor
        .as_deref()
        .and_then(|value| Uuid::parse_str(value).ok());

    // ?actor, ?since, ?tenant all compose via AND at the storage layer.
    // This is the §2.5 "compose" requirement from tasks.md.
    let acting_as_tenant_id = match &scope {
        AuditScope::Single(t) => Uuid::parse_str(t.id.as_str()).ok(),
        AuditScope::All | AuditScope::Legacy => None,
    };
    let entries = match storage
        .list_audit_logs(&AuditFilters {
            action: query.action,
            actor_id,
            target_type: query.target_type,
            since,
            limit: query.limit.map(|value| value as i32),
            acting_as_tenant_id,
            // B2 §11.1 — the new filter dimensions are intentionally NOT
            // surfaced on `/govern/audit` yet: the public query contract
            // change goes through its own OpenAPI review. Leaving them
            // None here preserves byte-identical pre-migration output.
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        })
        .await
    {
        Ok(entries) => entries,
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "govern_audit_failed",
                &err.to_string(),
            );
        }
    };

    // Legacy shape: no envelope, no decoration — byte-identical to pre-#44.d.
    if matches!(scope, AuditScope::Legacy) {
        return Json(entries).into_response();
    }

    // Envelope modes (`all` / `tenant`): decorate rows with
    // `tenantId`/`tenantSlug`/`tenantName` looked up from the tenant store.
    // Rows written before Bundle D have `acting_as_tenant_id = NULL` and
    // therefore surface without tenant decoration under `scope=all`
    // (and are filtered out entirely under `scope=tenant` by the SQL
    // clause above).
    let tenant_records = match state.tenant_store.list_tenants(true).await {
        Ok(ts) => ts,
        Err(err) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "tenant_lookup_failed",
                &err.to_string(),
            );
        }
    };
    let tenant_by_uuid: std::collections::HashMap<Uuid, &mk_core::types::TenantRecord> =
        tenant_records
            .iter()
            .filter_map(|t| Uuid::parse_str(t.id.as_str()).ok().map(|u| (u, t)))
            .collect();

    let items: Vec<serde_json::Value> = entries
        .into_iter()
        .map(|entry| {
            let tenant = entry
                .acting_as_tenant_id
                .and_then(|id| tenant_by_uuid.get(&id).copied());
            json!({
                "id":                entry.id,
                "action":            entry.action,
                "requestId":         entry.request_id,
                "targetType":        entry.target_type,
                "targetId":          entry.target_id,
                "actorType":         entry.actor_type,
                "actorId":           entry.actor_id,
                "actorEmail":        entry.actor_email,
                "details":           entry.details,
                "oldValues":         entry.old_values,
                "newValues":         entry.new_values,
                "createdAt":         entry.created_at,
                "actingAsTenantId":  entry.acting_as_tenant_id,
                "tenantId":          entry.acting_as_tenant_id,
                "tenantSlug":        tenant.map(|t| t.slug.clone()),
                "tenantName":        tenant.map(|t| t.name.clone()),
            })
        })
        .collect();

    let body = match &scope {
        AuditScope::Legacy => unreachable!("handled above"),
        AuditScope::All => json!({
            "success": true,
            "scope":   "all",
            "items":   items,
        }),
        AuditScope::Single(t) => json!({
            "success": true,
            "scope":   "tenant",
            "tenant":  { "id": t.id, "slug": t.slug, "name": t.name },
            "items":   items,
        }),
    };
    (StatusCode::OK, Json(body)).into_response()
}

async fn list_roles(State(state): State<Arc<AppState>>, headers: HeaderMap) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let tenant_id = match resolve_tenant_scope_uuid(&ctx) {
        Ok(tenant_id) => tenant_id,
        Err(response) => return response,
    };
    match storage.list_roles(Some(tenant_id), None, None).await {
        Ok(roles) => Json(
            roles
                .into_iter()
                .map(|entry| {
                    let role = entry.role.clone();
                    let scope = govern_scope_string(&entry);
                    GovernRoleResponse {
                        principal: entry.principal_id.to_string(),
                        principal_type: entry.principal_type.to_string(),
                        role,
                        scope,
                    }
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "govern_roles_list_failed",
            &err.to_string(),
        ),
    }
}

async fn assign_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<AssignGovernRoleRequest>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let principal_id = match Uuid::parse_str(&req.principal) {
        Ok(id) => id,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_principal",
                "Governance principal must be a UUID",
            );
        }
    };
    let principal_type = match req.principal_type.as_deref().unwrap_or("user") {
        "user" => PrincipalType::User,
        "agent" => PrincipalType::Agent,
        SYSTEM_USER_ID => PrincipalType::System,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_principal_type",
                "principalType must be user, agent, or system",
            );
        }
    };
    let (tenant_id, org_id, team_id, project_id, scope) =
        match resolve_govern_scope(&state, &ctx, req.scope.as_deref()).await {
            Ok(scope) => scope,
            Err(response) => return response,
        };
    let granted_by = actor_uuid(ctx.user_id.as_str());
    let create = CreateGovernanceRole {
        principal_type,
        principal_id,
        role: req.role.clone(),
        tenant_id: Some(tenant_id),
        org_id,
        team_id,
        project_id,
        granted_by,
        expires_at: None,
    };
    match storage.assign_role(&create).await {
        Ok(id) => Json(json!({"id": id, "principal": req.principal, "principalType": principal_type.to_string(), "role": req.role, "scope": scope})).into_response(),
        Err(err) => error_response(StatusCode::BAD_REQUEST, "govern_role_assign_failed", &err.to_string()),
    }
}

async fn revoke_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((principal, role)): Path<(String, String)>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let Some(storage) = &state.governance_storage else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "governance_unavailable",
            "Governance storage is not configured",
        );
    };
    let principal_id = match Uuid::parse_str(&principal) {
        Ok(id) => id,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_principal",
                "Governance principal must be a UUID",
            );
        }
    };
    match storage
        .revoke_role(principal_id, &role, actor_uuid(ctx.user_id.as_str()))
        .await
    {
        Ok(()) => {
            Json(json!({"success": true, "principal": principal, "role": role})).into_response()
        }
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "govern_role_revoke_failed",
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

async fn current_scope_ids(
    state: &AppState,
    ctx: &TenantContext,
    scope: Option<&str>,
) -> Result<(Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>), axum::response::Response> {
    scope_ids_for_layer(state, ctx, scope).await
}

async fn scope_ids_for_layer(
    _state: &AppState,
    ctx: &TenantContext,
    layer: Option<&str>,
) -> Result<(Option<Uuid>, Option<Uuid>, Option<Uuid>, Option<Uuid>), axum::response::Response> {
    let tenant_id = resolve_tenant_scope_uuid(ctx)?;
    match layer.unwrap_or("tenant") {
        "tenant" => Ok((Some(tenant_id), None, None, None)),
        value if value.starts_with("org:") => {
            Ok((Some(tenant_id), Some(parse_uuid_scope(value)?), None, None))
        }
        value if value.starts_with("team:") => {
            Ok((Some(tenant_id), None, Some(parse_uuid_scope(value)?), None))
        }
        value if value.starts_with("project:") => {
            Ok((Some(tenant_id), None, None, Some(parse_uuid_scope(value)?)))
        }
        "org" | "team" | "project" => Err(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "explicit_scope_target_required",
            "Use org:<uuid>, team:<uuid>, or project:<uuid>",
        )),
        other => Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            &format!("Unsupported layer filter: {other}"),
        )),
    }
}

fn parse_uuid_scope(value: &str) -> Result<Uuid, axum::response::Response> {
    let (_, raw_id) = value.split_once(':').ok_or_else(|| {
        error_response(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "Scope must include a UUID",
        )
    })?;
    Uuid::parse_str(raw_id).map_err(|_| {
        error_response(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "Scope target must be a UUID",
        )
    })
}

fn current_scope_string(
    org_id: Option<Uuid>,
    team_id: Option<Uuid>,
    project_id: Option<Uuid>,
) -> String {
    if let Some(project_id) = project_id {
        format!("project:{project_id}")
    } else if let Some(team_id) = team_id {
        format!("team:{team_id}")
    } else if let Some(org_id) = org_id {
        format!("org:{org_id}")
    } else {
        "tenant".to_string()
    }
}

async fn parse_request_id(
    storage: &storage::governance::GovernanceStorage,
    raw: &str,
) -> Result<Uuid, axum::response::Response> {
    if let Ok(id) = Uuid::parse_str(raw) {
        return Ok(id);
    }
    storage
        .get_request_by_number(raw)
        .await
        .map_err(|err| {
            error_response(
                StatusCode::BAD_REQUEST,
                "request_lookup_failed",
                &err.to_string(),
            )
        })?
        .map(|request| request.id)
        .ok_or_else(|| {
            error_response(
                StatusCode::NOT_FOUND,
                "request_not_found",
                "Approval request was not found",
            )
        })
}

fn parse_since(value: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
    let now = chrono::Utc::now();
    let duration = match value {
        "1h" => chrono::Duration::hours(1),
        "24h" => chrono::Duration::hours(24),
        "7d" => chrono::Duration::days(7),
        "30d" => chrono::Duration::days(30),
        "90d" => chrono::Duration::days(90),
        other => return Err(format!("Unsupported since filter: {other}")),
    };
    Ok(now - duration)
}

fn resolve_tenant_scope_uuid(ctx: &TenantContext) -> Result<Uuid, axum::response::Response> {
    Uuid::parse_str(ctx.tenant_id.as_str()).map_err(|_| {
        error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_tenant_id",
            "Tenant id must be a UUID for governance operations",
        )
    })
}

async fn resolve_govern_scope(
    _state: &AppState,
    ctx: &TenantContext,
    raw_scope: Option<&str>,
) -> Result<(Uuid, Option<Uuid>, Option<Uuid>, Option<Uuid>, String), axum::response::Response> {
    let tenant_id = resolve_tenant_scope_uuid(ctx)?;
    let Some(scope) = raw_scope else {
        return Ok((tenant_id, None, None, None, "tenant".to_string()));
    };
    let Some((kind, id)) = scope.split_once(':') else {
        return match scope {
            "tenant" => Ok((tenant_id, None, None, None, "tenant".to_string())),
            "org" | "team" | "project" => Err(error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "explicit_scope_target_required",
                "Use org:<uuid>, team:<uuid>, or project:<uuid> for governance role assignments",
            )),
            _ => Err(error_response(
                StatusCode::BAD_REQUEST,
                "invalid_scope",
                "Scope must be tenant, org:<uuid>, team:<uuid>, or project:<uuid>",
            )),
        };
    };
    let parsed = Uuid::parse_str(id).map_err(|_| {
        error_response(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "Scope target must be a UUID",
        )
    })?;
    match kind {
        "tenant" => Ok((parsed, None, None, None, format!("tenant:{id}"))),
        "org" => Ok((tenant_id, Some(parsed), None, None, format!("org:{id}"))),
        "team" => Ok((tenant_id, None, Some(parsed), None, format!("team:{id}"))),
        "project" => Ok((tenant_id, None, None, Some(parsed), format!("project:{id}"))),
        _ => Err(error_response(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "Scope must be tenant, org:<uuid>, team:<uuid>, or project:<uuid>",
        )),
    }
}

fn govern_scope_string(entry: &storage::governance::GovernanceRole) -> String {
    if let Some(project_id) = entry.project_id {
        format!("project:{project_id}")
    } else if let Some(team_id) = entry.team_id {
        format!("team:{team_id}")
    } else if let Some(org_id) = entry.org_id {
        format!("org:{org_id}")
    } else if let Some(tenant_id) = entry.tenant_id {
        format!("tenant:{tenant_id}")
    } else {
        "tenant".to_string()
    }
}

fn actor_uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).unwrap_or_else(|_| Uuid::new_v5(&Uuid::NAMESPACE_URL, value.as_bytes()))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreatePolicyRequest {
    name: String,
    description: Option<String>,
    layer: Option<String>,
    mode: Option<String>,
}

async fn list_policies(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let tenant_id = match resolve_tenant_scope_uuid(&ctx) {
        Ok(id) => id,
        Err(response) => return response,
    };
    match state
        .postgres
        .get_unit_policies_scoped(&ctx, tenant_id.to_string().as_str())
        .await
    {
        Ok(policies) => Json(json!({ "policies": policies })).into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "policy_list_failed",
            &err.to_string(),
        ),
    }
}

async fn create_policy(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CreatePolicyRequest>,
) -> impl IntoResponse {
    let ctx = match require_admin_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let tenant_id = match resolve_tenant_scope_uuid(&ctx) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let layer = match body.layer.as_deref().unwrap_or("tenant") {
        "tenant" => KnowledgeLayer::Tenant,
        "org" | "organization" => KnowledgeLayer::Org,
        "team" => KnowledgeLayer::Team,
        "project" => KnowledgeLayer::Project,
        other => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_layer",
                &format!("Unsupported policy layer: {other}"),
            );
        }
    };
    let mode = match body.mode.as_deref().unwrap_or("optional") {
        "mandatory" => PolicyMode::Mandatory,
        "optional" => PolicyMode::Optional,
        other => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_mode",
                &format!("Unsupported policy mode: {other}"),
            );
        }
    };
    let policy = Policy {
        id: Uuid::new_v4().to_string(),
        name: body.name,
        description: body.description,
        layer,
        mode,
        merge_strategy: RuleMergeStrategy::Merge,
        rules: vec![],
        metadata: std::collections::HashMap::new(),
    };
    let policy_id = policy.id.clone();
    match state
        .postgres
        .add_unit_policy_scoped(&ctx, tenant_id.to_string().as_str(), &policy)
        .await
    {
        Ok(()) => (
            StatusCode::CREATED,
            Json(json!({ "id": policy_id, "name": policy.name })),
        )
            .into_response(),
        Err(err) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "policy_create_failed",
            &err.to_string(),
        ),
    }
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (status, Json(json!({"error": error, "message": message}))).into_response()
}
