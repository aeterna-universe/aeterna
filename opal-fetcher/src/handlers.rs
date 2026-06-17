//! HTTP request handlers for the OPAL Data Fetcher.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::entities::{
    AgentPermissionRow, CedarEntitiesResponse, HierarchyRow, ProjectTeamAssignmentRow,
    UserPermissionRow, augment_projects_with_team_assignments, collect_project_team_assignments,
    transform_agents, transform_hierarchy, transform_roles, transform_users,
};
use crate::error::Result;
use crate::state::AppState;

/// Query parameters carrying the tenant context for a fetcher request.
///
/// Every entity-returning endpoint requires a `tenant` UUID. Returning the
/// globally-merged entity set would leak cross-tenant identifiers into the
/// OPAL → Cedar pipeline and evaluate policies against mismatched
/// principals (see issue #130). OPAL's data-source client is configured
/// per-tenant; the tenant parameter is the wire expression of that scope.
#[derive(Debug, Clone, Deserialize)]
pub struct TenantQuery {
    pub tenant: Uuid,
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub database: String,
}

/// Health check endpoint.
///
/// Returns 200 if the server is healthy and can connect to the database.
pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    // Check database connectivity
    let db_status = match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => "connected",
        Err(e) => {
            tracing::warn!(error = %e, "Database health check failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "unhealthy".to_string(),
                    database: "disconnected".to_string(),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(HealthResponse {
            status: "healthy".to_string(),
            database: db_status.to_string(),
        }),
    )
}

/// Metrics endpoint placeholder.
///
/// Returns Prometheus-format metrics.
pub async fn metrics() -> impl IntoResponse {
    // TODO: Integrate with metrics-exporter-prometheus
    // For now, return basic metrics
    let metrics = r#"# HELP opal_fetcher_requests_total Total number of requests
# TYPE opal_fetcher_requests_total counter
opal_fetcher_requests_total{endpoint="hierarchy"} 0
opal_fetcher_requests_total{endpoint="users"} 0
opal_fetcher_requests_total{endpoint="agents"} 0
# HELP opal_fetcher_up Whether the fetcher is up
# TYPE opal_fetcher_up gauge
opal_fetcher_up 1
"#;

    (StatusCode::OK, metrics)
}

/// GET /v1/hierarchy?tenant=<uuid>
///
/// Returns the organizational hierarchy (Tenant → Organization → Team →
/// Project) as Cedar entities for OPAL consumption, scoped to a single
/// tenant. The `tenant` query parameter is **required** — missing it
/// returns 400 via `serde_urlencoded` deserialization failure, surfaced
/// by axum's `Query` extractor.
///
/// Isolation contract: rows are filtered by `v_hierarchy.tenant_id = $1`
/// (column exposed by migration `028_tenant_scoped_hierarchy.sql`) and
/// `project_team_assignments.tenant_id = $1`. Cross-tenant rows are
/// never returned even if a tenant UUID is guessed correctly for a
/// different tenant's data — the filter is authoritative at the SQL
/// layer, not at a policy layer downstream.
pub async fn get_hierarchy(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TenantQuery>,
) -> Result<Json<CedarEntitiesResponse>> {
    let tenant_id = params.tenant;
    tracing::debug!(%tenant_id, "Fetching organizational hierarchy");

    let rows: Vec<HierarchyRow> = sqlx::query_as(
        r"
        SELECT
            tenant_id,
            org_id,
            org_slug,
            org_name,
            team_id,
            team_slug,
            team_name,
            project_id,
            project_slug,
            project_name,
            git_remote
        FROM v_hierarchy
        WHERE tenant_id = $1
        ",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let count = rows.len();
    tracing::debug!(%tenant_id, row_count = count, "Fetched hierarchy rows");

    let mut entities = transform_hierarchy(rows)?;
    let assignment_rows: Vec<ProjectTeamAssignmentRow> = sqlx::query_as(
        r"
        SELECT project_id, team_id, tenant_id, assignment_type
        FROM project_team_assignments
        WHERE tenant_id = $1::TEXT
        ",
    )
    .bind(tenant_id.to_string())
    .fetch_all(&state.pool)
    .await?;
    let assignments = collect_project_team_assignments(assignment_rows);
    augment_projects_with_team_assignments(&mut entities, assignments);

    let response = CedarEntitiesResponse::new(entities);

    tracing::info!(
        %tenant_id,
        entity_count = response.count,
        "Returning hierarchy entities"
    );

    Ok(Json(response))
}

/// GET /v1/users?tenant=<uuid>
///
/// Returns users with their team memberships and roles as Cedar entities,
/// scoped to a single tenant. The `tenant` query parameter is **required**.
///
/// Isolation contract: rows are filtered by `v_user_permissions.tenant_id
/// = $1` (column exposed by migration `028_tenant_scoped_hierarchy.sql`).
pub async fn get_users(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TenantQuery>,
) -> Result<Json<CedarEntitiesResponse>> {
    let tenant_id = params.tenant;
    tracing::debug!(%tenant_id, "Fetching user permissions");

    let rows: Vec<UserPermissionRow> = sqlx::query_as(
        r"
        SELECT
            tenant_id,
            user_id,
            email,
            user_name,
            user_status,
            team_id,
            role,
            permissions,
            org_id,
            org_slug,
            team_slug
        FROM v_user_permissions
        WHERE tenant_id = $1
        ",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let count = rows.len();
    tracing::debug!(%tenant_id, row_count = count, "Fetched user permission rows");

    let role_entities = transform_roles(&rows);
    let mut entities = transform_users(rows)?;
    entities.extend(role_entities);
    let response = CedarEntitiesResponse::new(entities);

    tracing::info!(
        %tenant_id,
        entity_count = response.count,
        "Returning user entities"
    );

    Ok(Json(response))
}

/// GET /v1/agents?tenant=<uuid>
///
/// Returns agents with their delegation chains and capabilities as Cedar
/// entities, scoped to a single tenant. The `tenant` query parameter is
/// **required**.
///
/// Isolation contract: rows are filtered by `v_agent_permissions.tenant_id
/// = $1` (column exposed by migration `029_agents_tenant_scope.sql`,
/// backed by a backfilled+NOT-NULL-for-active `agents.tenant_id` column).
/// Revoked / soft-deleted agents with `NULL` tenant_id are never
/// returned because the equality filter excludes `NULL`.
pub async fn get_agents(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TenantQuery>,
) -> Result<Json<CedarEntitiesResponse>> {
    let tenant_id = params.tenant;
    tracing::debug!(%tenant_id, "Fetching agent permissions");

    let rows: Vec<AgentPermissionRow> = sqlx::query_as(
        r"
        SELECT
            tenant_id,
            agent_id,
            agent_name,
            agent_type,
            delegated_by_user_id,
            delegated_by_agent_id,
            delegation_depth,
            capabilities,
            allowed_tenant_ids,
            allowed_org_ids,
            allowed_team_ids,
            allowed_project_ids,
            agent_status,
            delegating_user_email,
            delegating_user_name
        FROM v_agent_permissions
        WHERE tenant_id = $1
        ",
    )
    .bind(tenant_id)
    .fetch_all(&state.pool)
    .await?;

    let count = rows.len();
    tracing::debug!(%tenant_id, row_count = count, "Fetched agent permission rows");

    let entities = transform_agents(rows)?;
    let response = CedarEntitiesResponse::new(entities);

    tracing::info!(
        %tenant_id,
        entity_count = response.count,
        "Returning agent entities"
    );

    Ok(Json(response))
}

/// GET /v1/all?tenant=<uuid>
///
/// Returns all entities (hierarchy, users, agents, and Code Search) in a
/// single response, scoped to a single tenant. Useful for initial full
/// sync of one tenant's OPAL data-source. The `tenant` query parameter
/// is **required**.
///
/// Tenant-filtering applied uniformly to every entity source: hierarchy,
/// users, agents, code-search repositories / requests / identities, and
/// project-team assignments. There are no remaining cross-tenant gaps
/// in this endpoint after migration `029_agents_tenant_scope.sql`.
pub async fn get_all_entities(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TenantQuery>,
) -> Result<Json<CedarEntitiesResponse>> {
    let tenant_id = params.tenant;
    let tenant_text = tenant_id.to_string();
    tracing::debug!(%tenant_id, "Fetching all entities (tenant-scoped)");

    // Fetch all entity types in parallel
    let (
        hierarchy_rows,
        user_rows,
        agent_rows,
        assignment_rows,
        codesearch_repo_rows,
        codesearch_req_rows,
        codesearch_id_rows,
    ) = tokio::try_join!(
        sqlx::query_as::<_, HierarchyRow>(
            r"SELECT tenant_id, org_id, org_slug, org_name, team_id, team_slug, team_name, project_id, project_slug, project_name, git_remote FROM v_hierarchy WHERE tenant_id = $1"
        )
        .bind(tenant_id)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, UserPermissionRow>(
            r"SELECT tenant_id, user_id, email, user_name, user_status, team_id, role, permissions, org_id, org_slug, team_slug FROM v_user_permissions WHERE tenant_id = $1"
        )
        .bind(tenant_id)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, AgentPermissionRow>(
            r"SELECT tenant_id, agent_id, agent_name, agent_type, delegated_by_user_id, delegated_by_agent_id, delegation_depth, capabilities, allowed_tenant_ids, allowed_org_ids, allowed_team_ids, allowed_project_ids, agent_status, delegating_user_email, delegating_user_name FROM v_agent_permissions WHERE tenant_id = $1"
        )
        .bind(tenant_id)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, ProjectTeamAssignmentRow>(
            r"SELECT project_id, team_id, tenant_id, assignment_type FROM project_team_assignments WHERE tenant_id = $1"
        )
        .bind(&tenant_text)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, crate::entities::CodeSearchRepositoryRow>(
            r"SELECT id, tenant_id, name, status, sync_strategy, current_branch FROM v_code_search_repositories WHERE tenant_id = $1"
        )
        .bind(&tenant_text)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, crate::entities::CodeSearchRequestRow>(
            r"SELECT id, repository_id, requester_id, status, tenant_id FROM v_code_search_requests WHERE tenant_id = $1"
        )
        .bind(&tenant_text)
        .fetch_all(&state.pool),
        sqlx::query_as::<_, crate::entities::CodeSearchIdentityRow>(
            r"SELECT id, tenant_id, name, provider FROM v_code_search_identities WHERE tenant_id = $1"
        )
        .bind(&tenant_text)
        .fetch_all(&state.pool),
    )?;

    // Transform all entities
    let mut all_entities = transform_hierarchy(hierarchy_rows)?;
    let assignments = collect_project_team_assignments(assignment_rows);
    augment_projects_with_team_assignments(&mut all_entities, assignments);
    let role_entities = transform_roles(&user_rows);
    all_entities.extend(transform_users(user_rows)?);
    all_entities.extend(role_entities);
    all_entities.extend(transform_agents(agent_rows)?);

    // Add Code Search entities
    all_entities.extend(crate::entities::transform_code_search_repositories(
        codesearch_repo_rows,
    ));
    all_entities.extend(crate::entities::transform_code_search_requests(
        codesearch_req_rows,
    ));
    all_entities.extend(crate::entities::transform_code_search_identities(
        codesearch_id_rows,
    ));

    let response = CedarEntitiesResponse::new(all_entities);

    tracing::info!(
        %tenant_id,
        entity_count = response.count,
        "Returning all entities (tenant-scoped)"
    );

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let response = HealthResponse {
            status: "healthy".to_string(),
            database: "connected".to_string(),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("healthy"));
        assert!(json.contains("connected"));
    }
}
