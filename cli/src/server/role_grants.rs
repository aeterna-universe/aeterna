use std::fmt;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use mk_core::types::{Role, RoleIdentifier, UnitType, UserId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;

use super::{AppState, authenticated_tenant_context};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopedRoleGrant {
    pub user_id: String,
    pub role: RoleIdentifier,
    pub resource_type: ResourceType,
    pub resource_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ResourceType {
    Instance,
    Tenant,
    Organization,
    Team,
    Project,
    Session,
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Instance => "Instance",
            Self::Tenant => "Tenant",
            Self::Organization => "Organization",
            Self::Team => "Team",
            Self::Project => "Project",
            Self::Session => "Session",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScopedRoleGrantRequest {
    user_id: String,
    role: RoleIdentifier,
    resource_type: ResourceType,
    resource_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListGrantsQuery {
    user_id: Option<String>,
    resource_type: Option<ResourceType>,
    resource_id: Option<String>,
}

pub fn cedar_role_entity_id(
    role: &Role,
    resource_type: &ResourceType,
    resource_id: &str,
) -> String {
    format!("{role}@{resource_type}::{resource_id}")
}

pub fn validate_scope(role: &Role, resource_type: &ResourceType) -> Result<(), String> {
    match role {
        Role::PlatformAdmin if *resource_type != ResourceType::Instance => {
            Err("PlatformAdmin can only be granted at instance scope".to_string())
        }
        Role::TenantAdmin
            if *resource_type != ResourceType::Tenant
                && *resource_type != ResourceType::Instance =>
        {
            Err("TenantAdmin can only be granted at tenant or instance scope".to_string())
        }
        _ => Ok(()),
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/roles/grant", post(handle_grant_role))
        .route("/roles/revoke", delete(handle_revoke_role))
        .route("/roles/grants", get(handle_list_grants))
        .with_state(state)
}

async fn handle_grant_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ScopedRoleGrantRequest>,
) -> impl IntoResponse {
    let ctx = match authenticated_tenant_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    if req.user_id.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id is required",
        );
    }
    if req.resource_id.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_resource_id",
            "resource_id is required",
        );
    }

    let user_id = match UserId::new(req.user_id.clone()) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };

    let authz_resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(&ctx, "AssignRoles", &authz_resource)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "AssignRoles permission required",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authz_check_failed",
                &e.to_string(),
            );
        }
    }

    let known_role = match &req.role {
        RoleIdentifier::Known(role) => role,
        RoleIdentifier::Custom(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_role_scope",
                "Custom roles are not supported by this endpoint",
            );
        }
    };

    if let Err(message) = validate_scope(known_role, &req.resource_type) {
        return error_response(StatusCode::BAD_REQUEST, "invalid_role_scope", &message);
    }

    match state
        .postgres
        .assign_role_scoped(&ctx, &user_id, &req.resource_id, req.role.clone())
        .await
    {
        Ok(()) => Json(ScopedRoleGrant {
            user_id: req.user_id,
            role: req.role,
            resource_type: req.resource_type,
            resource_id: req.resource_id,
        })
        .into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "role_grant_failed",
            &err.to_string(),
        ),
    }
}

async fn handle_revoke_role(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<ScopedRoleGrantRequest>,
) -> impl IntoResponse {
    let ctx = match authenticated_tenant_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    if req.user_id.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_user_id",
            "user_id is required",
        );
    }
    if req.resource_id.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_resource_id",
            "resource_id is required",
        );
    }

    let user_id = match UserId::new(req.user_id.clone()) {
        Some(user_id) => user_id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_id",
                "Invalid user id",
            );
        }
    };

    let authz_resource = format!("Aeterna::Tenant::\"{}\"", ctx.tenant_id.as_str());
    match state
        .auth_service
        .check_permission(&ctx, "AssignRoles", &authz_resource)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "forbidden",
                "AssignRoles permission required",
            );
        }
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authz_check_failed",
                &e.to_string(),
            );
        }
    }

    let existing = match state
        .postgres
        .get_user_roles_scoped(&ctx, &user_id, &ctx.tenant_id)
        .await
    {
        Ok(roles) => roles
            .into_iter()
            .any(|(resource_id, role)| resource_id == req.resource_id && role == req.role),
        Err(err) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "role_lookup_failed",
                &err.to_string(),
            );
        }
    };

    if !existing {
        return error_response(
            StatusCode::NOT_FOUND,
            "role_grant_not_found",
            "No matching role grant found",
        );
    }

    match state
        .postgres
        .remove_role_scoped(&ctx, &user_id, &req.resource_id, req.role)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({ "success": true }))).into_response(),
        Err(err) => error_response(
            StatusCode::BAD_REQUEST,
            "role_revoke_failed",
            &err.to_string(),
        ),
    }
}

async fn handle_list_grants(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListGrantsQuery>,
) -> impl IntoResponse {
    let ctx = match authenticated_tenant_context(&state, &headers).await {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };

    if query.user_id.is_none() && query.resource_type.is_none() && query.resource_id.is_none() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_filters",
            "At least one filter required",
        );
    }

    let rows = if let Some(user_id_value) = query.user_id.clone() {
        let user_id = match UserId::new(user_id_value.clone()) {
            Some(user_id) => user_id,
            None => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_user_id",
                    "Invalid user id",
                );
            }
        };

        match state
            .postgres
            .get_user_roles_scoped(&ctx, &user_id, &ctx.tenant_id)
            .await
        {
            Ok(entries) => entries
                .into_iter()
                .map(|(resource_id, role)| (user_id_value.clone(), resource_id, role))
                .collect::<Vec<_>>(),
            Err(err) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "role_list_failed",
                    &err.to_string(),
                );
            }
        }
    } else {
        match sqlx::query("SELECT user_id, unit_id, role FROM user_roles WHERE tenant_id = $1")
            .bind(ctx.tenant_id.as_str())
            .fetch_all(state.postgres.pool())
            .await
        {
            Ok(db_rows) => db_rows
                .into_iter()
                .map(|row| {
                    let user_id: String = row.get("user_id");
                    let resource_id: String = row.get("unit_id");
                    let role_str: String = row.get("role");
                    (
                        user_id,
                        resource_id,
                        RoleIdentifier::from_str_flexible(&role_str),
                    )
                })
                .collect::<Vec<_>>(),
            Err(err) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "role_list_failed",
                    &err.to_string(),
                );
            }
        }
    };

    let mut grants = Vec::new();
    for (user_id, resource_id, role) in rows {
        if let Some(resource_id_filter) = &query.resource_id
            && resource_id_filter != &resource_id
        {
            continue;
        }

        let resolved_type = match resolve_resource_type(&state, &ctx.tenant_id, &resource_id).await
        {
            Ok(resource_type) => resource_type,
            Err(err) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "role_list_failed",
                    &err.to_string(),
                );
            }
        };

        if let Some(resource_type_filter) = &query.resource_type
            && resource_type_filter != &resolved_type
        {
            continue;
        }

        grants.push(ScopedRoleGrant {
            user_id,
            role,
            resource_type: resolved_type,
            resource_id,
        });
    }

    Json(grants).into_response()
}

async fn resolve_resource_type(
    state: &Arc<AppState>,
    tenant_id: &mk_core::types::TenantId,
    resource_id: &str,
) -> Result<ResourceType, sqlx::Error> {
    if resource_id == tenant_id.as_str() {
        return Ok(ResourceType::Tenant);
    }

    if let (Ok(resource_uuid), Ok(Some(record))) = (
        uuid::Uuid::parse_str(resource_id),
        state.tenant_store.get_tenant(tenant_id.as_str()).await,
    ) {
        if let Ok(tenant_uuid) = uuid::Uuid::parse_str(record.id.as_str()) {
            let maybe_hierarchy_type: Option<String> = sqlx::query_scalar(
                "SELECT resource_type
                   FROM (
                         SELECT 'organization'::text AS resource_type, o.id AS resource_id
                           FROM organizations o
                          WHERE o.tenant_id = $1
                            AND o.deleted_at IS NULL
                         UNION ALL
                         SELECT 'team'::text AS resource_type, t.id AS resource_id
                           FROM teams t
                           JOIN organizations o ON o.id = t.org_id
                          WHERE o.tenant_id = $1
                            AND o.deleted_at IS NULL
                            AND t.deleted_at IS NULL
                         UNION ALL
                         SELECT 'project'::text AS resource_type, p.id AS resource_id
                           FROM projects p
                           JOIN teams t ON t.id = p.team_id
                           JOIN organizations o ON o.id = t.org_id
                          WHERE o.tenant_id = $1
                            AND o.deleted_at IS NULL
                            AND t.deleted_at IS NULL
                            AND p.deleted_at IS NULL
                        ) hierarchy_resources
                  WHERE resource_id = $2
                  LIMIT 1",
            )
            .bind(tenant_uuid)
            .bind(resource_uuid)
            .fetch_optional(state.postgres.pool())
            .await?;

            if let Some(resource_type) = maybe_hierarchy_type {
                return Ok(match resource_type.as_str() {
                    "organization" => ResourceType::Organization,
                    "team" => ResourceType::Team,
                    "project" => ResourceType::Project,
                    _ => ResourceType::Instance,
                });
            }
        }
    }

    let maybe_unit_type =
        sqlx::query("SELECT type FROM organizational_units WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_id.as_str())
            .bind(resource_id)
            .fetch_optional(state.postgres.pool())
            .await?
            .and_then(|row| {
                let value: String = row.get("type");
                value.parse::<UnitType>().ok()
            });

    Ok(match maybe_unit_type {
        Some(UnitType::Organization) => ResourceType::Organization,
        Some(UnitType::Team) => ResourceType::Team,
        Some(UnitType::Project) => ResourceType::Project,
        None => ResourceType::Instance,
    })
}

fn error_response(status: StatusCode, error: &str, message: &str) -> axum::response::Response {
    (status, Json(json!({ "error": error, "message": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cedar_role_entity_id() {
        let entity_id = cedar_role_entity_id(&Role::Admin, &ResourceType::Tenant, "acme");
        assert_eq!(entity_id, "Admin@Tenant::acme");
    }

    #[test]
    fn test_resource_type_serialization() {
        // Note: ResourceType deliberately uses #[serde(rename_all = "lowercase")] —
        // a separate convention from the workspace-wide PascalCase enum default.
        let serialized = serde_json::to_string(&ResourceType::Organization).unwrap();
        assert_eq!(serialized, "\"organization\"");

        let deserialized: ResourceType = serde_json::from_str("\"session\"").unwrap();
        assert_eq!(deserialized, ResourceType::Session);
    }

    #[test]
    fn test_scope_validation_platform_admin() {
        assert!(validate_scope(&Role::PlatformAdmin, &ResourceType::Instance).is_ok());
        assert!(validate_scope(&Role::PlatformAdmin, &ResourceType::Tenant).is_err());
        assert!(validate_scope(&Role::PlatformAdmin, &ResourceType::Project).is_err());
    }

    #[test]
    fn test_scope_validation_tenant_admin() {
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Instance).is_ok());
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Tenant).is_ok());
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Organization).is_err());
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Team).is_err());
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Project).is_err());
        assert!(validate_scope(&Role::TenantAdmin, &ResourceType::Session).is_err());
    }

    #[test]
    fn test_scope_validation_developer() {
        assert!(validate_scope(&Role::Developer, &ResourceType::Instance).is_ok());
        assert!(validate_scope(&Role::Developer, &ResourceType::Tenant).is_ok());
        assert!(validate_scope(&Role::Developer, &ResourceType::Organization).is_ok());
        assert!(validate_scope(&Role::Developer, &ResourceType::Team).is_ok());
        assert!(validate_scope(&Role::Developer, &ResourceType::Project).is_ok());
        assert!(validate_scope(&Role::Developer, &ResourceType::Session).is_ok());
    }
}
