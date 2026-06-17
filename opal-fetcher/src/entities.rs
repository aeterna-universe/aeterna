//! Cedar entity transformation from `PostgreSQL` rows.
//!
//! This module transforms organizational data from `PostgreSQL` into Cedar
//! entity format that can be consumed by OPAL and Cedar Agent for authorization
//! decisions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

use crate::error::Result;

// ============================================================================
// Database Row Types
// ============================================================================

/// Row from the `v_hierarchy` view.
///
/// `tenant_id` is surfaced by migration `028_tenant_scoped_hierarchy.sql`
/// as the first column of the view. Handlers MUST filter by this column
/// before returning rows to OPAL — see `handlers::get_hierarchy`.
#[derive(Debug, Clone, FromRow)]
pub struct HierarchyRow {
    pub tenant_id: Uuid,
    pub org_id: Option<Uuid>,
    pub org_slug: Option<String>,
    pub org_name: Option<String>,
    pub team_id: Option<Uuid>,
    pub team_slug: Option<String>,
    pub team_name: Option<String>,
    pub project_id: Option<Uuid>,
    pub project_slug: Option<String>,
    pub project_name: Option<String>,
    pub git_remote: Option<String>,
}

/// Row from the `v_user_permissions` view.
///
/// `tenant_id` is surfaced by migration `028_tenant_scoped_hierarchy.sql`
/// as the first column of the view. Handlers MUST filter by this column
/// before returning rows to OPAL — see `handlers::get_users`.
#[derive(Debug, Clone, FromRow)]
pub struct UserPermissionRow {
    pub tenant_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub user_name: Option<String>,
    pub user_status: String,
    pub team_id: Uuid,
    pub role: String,
    pub permissions: serde_json::Value,
    pub org_id: Uuid,
    pub org_slug: String,
    pub team_slug: String,
}

/// Row from the `v_agent_permissions` view.
/// Row from the `v_agent_permissions` view.
///
/// `tenant_id` is surfaced by migration `029_agents_tenant_scope.sql`
/// as the first column of the view, mirroring the pattern established
/// by migration 028 for `v_hierarchy` and `v_user_permissions`. The
/// column is nullable on the view because soft-deleted / revoked
/// agents retain `NULL` tenant_id (they are historical rows kept for
/// audit). Active agents always have a tenant — the migration's CHECK
/// constraint enforces this.
///
/// Handlers MUST filter by this column before returning rows to OPAL.
/// See `handlers::get_agents`.
#[derive(Debug, Clone, FromRow)]
pub struct AgentPermissionRow {
    pub tenant_id: Option<Uuid>,
    pub agent_id: Uuid,
    pub agent_name: String,
    pub agent_type: String,
    pub delegated_by_user_id: Option<Uuid>,
    pub delegated_by_agent_id: Option<Uuid>,
    pub delegation_depth: i32,
    pub capabilities: serde_json::Value,
    pub allowed_tenant_ids: Option<Vec<Uuid>>,
    pub allowed_org_ids: Option<Vec<Uuid>>,
    pub allowed_team_ids: Option<Vec<Uuid>>,
    pub allowed_project_ids: Option<Vec<Uuid>>,
    pub agent_status: String,
    pub delegating_user_email: Option<String>,
    pub delegating_user_name: Option<String>,
}

/// Row from the `codesearch_repositories` table.
#[derive(Debug, Clone, FromRow)]
pub struct CodeSearchRepositoryRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub status: String,
    pub sync_strategy: String,
    pub current_branch: String,
}

/// Row from the `codesearch_requests` table.
#[derive(Debug, Clone, FromRow)]
pub struct CodeSearchRequestRow {
    pub id: Uuid,
    pub repository_id: Uuid,
    pub requester_id: String,
    pub status: String,
    pub tenant_id: String,
}

/// Row from the `codesearch_identities` table.
#[derive(Debug, Clone, FromRow)]
pub struct CodeSearchIdentityRow {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub provider: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectTeamAssignmentRow {
    pub project_id: String,
    pub team_id: String,
    pub tenant_id: String,
    pub assignment_type: String,
}

// ============================================================================
// Cedar Entity Types
// ============================================================================

/// A Cedar entity with type, ID, attributes, and parent relationships.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CedarEntity {
    /// Entity UID in format "`Type::`\"id\"".
    pub uid: CedarEntityUid,
    /// Entity attributes.
    pub attrs: serde_json::Value,
    /// Parent entity UIDs (for hierarchy).
    pub parents: Vec<CedarEntityUid>,
}

/// Cedar entity UID structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CedarEntityUid {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub id: String,
}

impl CedarEntityUid {
    /// Creates a new entity UID.
    #[must_use]
    pub fn new(entity_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            entity_type: entity_type.into(),
            id: id.into(),
        }
    }
}

/// Response containing Cedar entities for OPAL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CedarEntitiesResponse {
    /// List of Cedar entities.
    pub entities: Vec<CedarEntity>,
    /// Timestamp of the response.
    pub timestamp: DateTime<Utc>,
    /// Number of entities.
    pub count: usize,
}

impl CedarEntitiesResponse {
    /// Creates a new response with the given entities.
    #[must_use]
    pub fn new(entities: Vec<CedarEntity>) -> Self {
        let count = entities.len();
        Self {
            entities,
            timestamp: Utc::now(),
            count,
        }
    }
}

// ============================================================================
// Entity Transformation Functions
// ============================================================================

/// Transforms hierarchy rows into Cedar entities.
///
/// Creates entities for: Tenant, Organization, Team, Project.
pub fn transform_hierarchy(rows: Vec<HierarchyRow>) -> Result<Vec<CedarEntity>> {
    let mut entities: Vec<CedarEntity> = Vec::new();
    let mut seen_tenants = std::collections::HashSet::new();
    let mut seen_orgs = std::collections::HashSet::new();
    let mut seen_teams = std::collections::HashSet::new();
    let mut seen_projects = std::collections::HashSet::new();

    for row in rows {
        if seen_tenants.insert(row.tenant_id) {
            entities.push(CedarEntity {
                uid: CedarEntityUid::new("Aeterna::Tenant", row.tenant_id.to_string()),
                attrs: serde_json::json!({}),
                parents: vec![],
            });
        }

        // Organization entity
        if let (Some(id), Some(slug), Some(name)) = (&row.org_id, &row.org_slug, &row.org_name)
            && seen_orgs.insert(*id)
        {
            entities.push(CedarEntity {
                uid: CedarEntityUid::new("Aeterna::Organization", id.to_string()),
                attrs: serde_json::json!({
                    "slug": slug,
                    "name": name,
                }),
                parents: vec![CedarEntityUid::new(
                    "Aeterna::Tenant",
                    row.tenant_id.to_string(),
                )],
            });
        }

        // Team entity
        if let (Some(id), Some(slug), Some(name), Some(org_id)) =
            (&row.team_id, &row.team_slug, &row.team_name, &row.org_id)
            && seen_teams.insert(*id)
        {
            entities.push(CedarEntity {
                uid: CedarEntityUid::new("Aeterna::Team", id.to_string()),
                attrs: serde_json::json!({
                    "slug": slug,
                    "name": name,
                }),
                parents: vec![CedarEntityUid::new(
                    "Aeterna::Organization",
                    org_id.to_string(),
                )],
            });
        }

        // Project entity
        if let (Some(id), Some(slug), Some(name), Some(team_id)) = (
            &row.project_id,
            &row.project_slug,
            &row.project_name,
            &row.team_id,
        ) && seen_projects.insert(*id)
        {
            let mut attrs = serde_json::json!({
                "slug": slug,
                "name": name,
            });
            if let Some(git_remote) = &row.git_remote {
                attrs["git_remote"] = serde_json::json!(git_remote);
            }
            entities.push(CedarEntity {
                uid: CedarEntityUid::new("Aeterna::Project", id.to_string()),
                attrs,
                parents: vec![CedarEntityUid::new("Aeterna::Team", team_id.to_string())],
            });
        }
    }

    Ok(entities)
}

pub fn collect_project_team_assignments(
    rows: Vec<ProjectTeamAssignmentRow>,
) -> std::collections::HashMap<String, Vec<(String, String)>> {
    let mut map: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    for row in rows {
        let ProjectTeamAssignmentRow {
            project_id,
            team_id,
            assignment_type,
            ..
        } = row;
        map.entry(project_id)
            .or_default()
            .push((team_id, assignment_type));
    }
    map
}

pub fn augment_projects_with_team_assignments(
    entities: &mut Vec<CedarEntity>,
    assignments: std::collections::HashMap<String, Vec<(String, String)>>,
) {
    for entity in entities.iter_mut() {
        if entity.uid.entity_type == "Aeterna::Project"
            && let Some(team_assignments) = assignments.get(&entity.uid.id)
        {
            for (team_id, _) in team_assignments {
                let team_uid = CedarEntityUid::new("Aeterna::Team", team_id.clone());
                if !entity.parents.contains(&team_uid) {
                    entity.parents.push(team_uid);
                }
            }
            let assignments_json: Vec<serde_json::Value> = team_assignments
                .iter()
                .map(|(tid, atype)| serde_json::json!({"team_id": tid, "assignment_type": atype}))
                .collect();
            entity.attrs["team_assignments"] = serde_json::json!(assignments_json);
        }
    }
}

/// Transforms user permission rows into Cedar entities.
///
/// Creates User entities with role and membership information.
#[must_use]
pub fn normalize_role_to_entity_id(role: &str) -> String {
    match role {
        "platformadmin" => "PlatformAdmin".to_string(),
        "tenantadmin" => "TenantAdmin".to_string(),
        "admin" => "Admin".to_string(),
        "architect" => "Architect".to_string(),
        "techlead" => "TechLead".to_string(),
        "developer" => "Developer".to_string(),
        "viewer" => "Viewer".to_string(),
        _ => role.to_string(),
    }
}

#[must_use]
pub fn transform_roles(rows: &[UserPermissionRow]) -> Vec<CedarEntity> {
    let unique_roles: BTreeSet<String> = rows
        .iter()
        .map(|row| normalize_role_to_entity_id(&row.role))
        .collect();

    unique_roles
        .into_iter()
        .map(|normalized_role| CedarEntity {
            uid: CedarEntityUid::new("Aeterna::Role", normalized_role.clone()),
            attrs: serde_json::json!({
                "name": normalized_role,
            }),
            parents: vec![],
        })
        .collect()
}

/// Transforms user permission rows into Cedar entities.
///
/// Creates User entities with role and membership information.
pub fn transform_users(rows: Vec<UserPermissionRow>) -> Result<Vec<CedarEntity>> {
    let mut entities: Vec<CedarEntity> = Vec::new();
    let mut user_teams: HashMap<Uuid, Vec<CedarEntityUid>> = HashMap::new();
    let mut user_roles: HashMap<Uuid, BTreeSet<String>> = HashMap::new();
    let mut user_info: HashMap<Uuid, (String, Option<String>, String)> = HashMap::new();

    // Collect all team memberships per user
    for row in &rows {
        let team_uid = CedarEntityUid::new("Aeterna::Team", row.team_id.to_string());
        user_teams.entry(row.user_id).or_default().push(team_uid);
        let normalized_role = normalize_role_to_entity_id(&row.role);
        user_roles
            .entry(row.user_id)
            .or_default()
            .insert(normalized_role);
        user_info.entry(row.user_id).or_insert((
            row.email.clone(),
            row.user_name.clone(),
            row.user_status.clone(),
        ));
    }

    // Create User entities with all their team memberships as parents
    for (user_id, mut parents) in user_teams {
        let (email, name, status) = user_info.get(&user_id).cloned().unwrap_or_default();

        // Collect roles for this user
        let roles: Vec<String> = rows
            .iter()
            .filter(|r| r.user_id == user_id)
            .map(|r| r.role.clone())
            .collect();

        if let Some(role_ids) = user_roles.get(&user_id) {
            for role_id in role_ids {
                parents.push(CedarEntityUid::new("Aeterna::Role", role_id.clone()));
            }
        }

        let mut attrs = serde_json::json!({
            "email": email,
            "status": status,
            "roles": roles,
        });
        if let Some(n) = name {
            attrs["name"] = serde_json::json!(n);
        }

        entities.push(CedarEntity {
            uid: CedarEntityUid::new("Aeterna::User", user_id.to_string()),
            attrs,
            parents,
        });
    }

    Ok(entities)
}

/// Transforms agent permission rows into Cedar entities.
///
/// Creates Agent entities with delegation chain and capability information.
pub fn transform_agents(rows: Vec<AgentPermissionRow>) -> Result<Vec<CedarEntity>> {
    let mut entities: Vec<CedarEntity> = Vec::new();

    for row in rows {
        let mut parents = Vec::new();

        // Add allowed teams as parents for resource access
        if let Some(team_ids) = &row.allowed_team_ids {
            for team_id in team_ids {
                parents.push(CedarEntityUid::new("Aeterna::Team", team_id.to_string()));
            }
        }

        // Parse capabilities from JSON
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();

        let mut attrs = serde_json::json!({
            "name": row.agent_name,
            "agent_type": row.agent_type,
            "delegation_depth": row.delegation_depth,
            "capabilities": capabilities,
            "status": row.agent_status,
        });

        // Add delegation info
        if let Some(user_id) = row.delegated_by_user_id {
            attrs["delegated_by"] = serde_json::json!({
                "type": "User",
                "id": user_id.to_string(),
            });
            if let Some(email) = &row.delegating_user_email {
                attrs["delegating_user_email"] = serde_json::json!(email);
            }
        } else if let Some(agent_id) = row.delegated_by_agent_id {
            attrs["delegated_by"] = serde_json::json!({
                "type": "Agent",
                "id": agent_id.to_string(),
            });
        }

        // Add scope arrays
        if let Some(ids) = &row.allowed_tenant_ids
            && !ids.is_empty()
        {
            attrs["allowed_tenant_ids"] = serde_json::json!(ids);
        }
        if let Some(ids) = &row.allowed_org_ids
            && !ids.is_empty()
        {
            attrs["allowed_org_ids"] = serde_json::json!(ids);
        }
        if let Some(ids) = &row.allowed_team_ids
            && !ids.is_empty()
        {
            attrs["allowed_team_ids"] = serde_json::json!(ids);
        }
        if let Some(ids) = &row.allowed_project_ids
            && !ids.is_empty()
        {
            attrs["allowed_project_ids"] = serde_json::json!(ids);
        }

        entities.push(CedarEntity {
            uid: CedarEntityUid::new("Aeterna::Agent", row.agent_id.to_string()),
            attrs,
            parents,
        });
    }

    Ok(entities)
}

/// Transforms code search repositories into Cedar entities.
pub fn transform_code_search_repositories(rows: Vec<CodeSearchRepositoryRow>) -> Vec<CedarEntity> {
    rows.into_iter()
        .map(|row| CedarEntity {
            uid: CedarEntityUid::new("CodeSearch::Repository", row.id.to_string()),
            attrs: serde_json::json!({
                "name": row.name,
                "tenant_id": row.tenant_id,
                "status": row.status,
                "sync_strategy": row.sync_strategy,
                "current_branch": row.current_branch,
            }),
            parents: vec![],
        })
        .collect()
}

/// Transforms code search requests into Cedar entities.
pub fn transform_code_search_requests(rows: Vec<CodeSearchRequestRow>) -> Vec<CedarEntity> {
    rows.into_iter()
        .map(|row| CedarEntity {
            uid: CedarEntityUid::new("CodeSearch::Request", row.id.to_string()),
            attrs: serde_json::json!({
                "repository_id": row.repository_id.to_string(),
                "requester_id": row.requester_id,
                "status": row.status,
                "tenant_id": row.tenant_id,
            }),
            parents: vec![],
        })
        .collect()
}

/// Transforms code search identities into Cedar entities.
pub fn transform_code_search_identities(rows: Vec<CodeSearchIdentityRow>) -> Vec<CedarEntity> {
    rows.into_iter()
        .map(|row| CedarEntity {
            uid: CedarEntityUid::new("CodeSearch::Identity", row.id.to_string()),
            attrs: serde_json::json!({
                "name": row.name,
                "tenant_id": row.tenant_id,
                "provider": row.provider,
            }),
            parents: vec![],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_user_row() -> UserPermissionRow {
        UserPermissionRow {
            tenant_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            email: "alice@acme.com".to_string(),
            user_name: Some("Alice".to_string()),
            user_status: "active".to_string(),
            team_id: Uuid::new_v4(),
            role: "developer".to_string(),
            permissions: serde_json::json!([]),
            org_id: Uuid::new_v4(),
            org_slug: "platform-engineering".to_string(),
            team_slug: "api-team".to_string(),
        }
    }

    fn sample_hierarchy_row() -> HierarchyRow {
        HierarchyRow {
            tenant_id: Uuid::new_v4(),
            org_id: Some(Uuid::new_v4()),
            org_slug: Some("platform-engineering".to_string()),
            org_name: Some("Platform Engineering".to_string()),
            team_id: Some(Uuid::new_v4()),
            team_slug: Some("api-team".to_string()),
            team_name: Some("API Team".to_string()),
            project_id: Some(Uuid::new_v4()),
            project_slug: Some("payments-service".to_string()),
            project_name: Some("Payments Service".to_string()),
            git_remote: Some("git@github.com:acme/payments.git".to_string()),
        }
    }

    fn sample_project_team_assignment(
        project_id: &str,
        team_id: &str,
        assignment_type: &str,
    ) -> ProjectTeamAssignmentRow {
        ProjectTeamAssignmentRow {
            project_id: project_id.to_string(),
            team_id: team_id.to_string(),
            tenant_id: "acme-corp".to_string(),
            assignment_type: assignment_type.to_string(),
        }
    }

    #[test]
    fn test_cedar_entity_uid() {
        let uid = CedarEntityUid::new("Aeterna::User", "123");
        assert_eq!(uid.entity_type, "Aeterna::User");
        assert_eq!(uid.id, "123");
    }

    #[test]
    fn test_transform_hierarchy_single_row() {
        let row = sample_hierarchy_row();
        let rows = vec![row.clone()];

        let entities = transform_hierarchy(rows).unwrap();

        // Should create 4 entities: Tenant, Org, Team, Project
        assert_eq!(entities.len(), 4);

        // Verify Tenant
        let tenant = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Tenant")
            .unwrap();
        assert!(tenant.parents.is_empty());

        // Verify Organization
        let org = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Organization")
            .unwrap();
        assert_eq!(org.parents.len(), 1);
        assert_eq!(org.parents[0].entity_type, "Aeterna::Tenant");

        // Verify Team
        let team = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Team")
            .unwrap();
        assert_eq!(team.parents.len(), 1);
        assert_eq!(team.parents[0].entity_type, "Aeterna::Organization");

        // Verify Project
        let project = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project")
            .unwrap();
        assert_eq!(project.parents.len(), 1);
        assert_eq!(project.parents[0].entity_type, "Aeterna::Team");
        assert!(project.attrs.get("git_remote").is_some());
    }

    #[test]
    fn test_transform_hierarchy_deduplication() {
        let row = sample_hierarchy_row();
        let rows = vec![row.clone(), row.clone()];

        let entities = transform_hierarchy(rows).unwrap();

        // Should still be 4 entities (deduplicated)
        assert_eq!(entities.len(), 4);
    }

    #[test]
    fn test_transform_users() {
        let row = sample_user_row();
        let user_id = row.user_id;
        let team_id = row.team_id;

        let entities = transform_users(vec![row]).unwrap();

        assert_eq!(entities.len(), 1);
        let user = &entities[0];
        assert_eq!(user.uid.entity_type, "Aeterna::User");
        assert_eq!(user.uid.id, user_id.to_string());
        assert_eq!(user.attrs["email"], "alice@acme.com");
        assert_eq!(user.attrs["status"], "active");
        assert_eq!(user.parents.len(), 2);
        assert!(user.parents.iter().any(
            |parent| parent.entity_type == "Aeterna::Team" && parent.id == team_id.to_string()
        ));
        assert!(
            user.parents
                .iter()
                .any(|parent| parent.entity_type == "Aeterna::Role" && parent.id == "Developer")
        );
    }

    #[test]
    fn test_transform_users_multiple_teams() {
        let row1 = sample_user_row();
        let team1_id = row1.team_id;
        let team2_id = Uuid::new_v4();

        let row2 = UserPermissionRow {
            team_id: team2_id,
            role: "tech_lead".to_string(),
            team_slug: "data-team".to_string(),
            ..row1.clone()
        };

        let entities = transform_users(vec![row1, row2]).unwrap();

        assert_eq!(entities.len(), 1);
        let user = &entities[0];
        assert_eq!(user.parents.len(), 4);
        assert!(user.parents.iter().any(
            |parent| parent.entity_type == "Aeterna::Team" && parent.id == team1_id.to_string()
        ));
        assert!(user.parents.iter().any(
            |parent| parent.entity_type == "Aeterna::Team" && parent.id == team2_id.to_string()
        ));
        assert!(
            user.parents
                .iter()
                .any(|parent| parent.entity_type == "Aeterna::Role" && parent.id == "Developer")
        );
        assert!(
            user.parents
                .iter()
                .any(|parent| parent.entity_type == "Aeterna::Role" && parent.id == "tech_lead")
        );
        // Should have both roles
        let roles: Vec<String> = serde_json::from_value(user.attrs["roles"].clone()).unwrap();
        assert!(roles.contains(&"developer".to_string()));
        assert!(roles.contains(&"tech_lead".to_string()));
    }

    #[test]
    fn test_normalize_role_to_entity_id_known_role_mappings() {
        let known_mappings = vec![
            ("platformadmin", "PlatformAdmin"),
            ("tenantadmin", "TenantAdmin"),
            ("admin", "Admin"),
            ("architect", "Architect"),
            ("techlead", "TechLead"),
            ("developer", "Developer"),
            ("viewer", "Viewer"),
        ];

        for (input, expected) in known_mappings {
            assert_eq!(normalize_role_to_entity_id(input), expected);
        }
    }

    #[test]
    fn test_transform_roles_emits_role_entities_with_deduplication() {
        let user_one = sample_user_row();
        let user_two = UserPermissionRow {
            user_id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            role: "developer".to_string(),
            ..sample_user_row()
        };
        let user_three = UserPermissionRow {
            user_id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            role: "viewer".to_string(),
            ..sample_user_row()
        };

        let role_entities = transform_roles(&[user_one, user_two, user_three]);

        assert_eq!(role_entities.len(), 2);
        assert!(role_entities.iter().any(|entity| {
            entity.uid == CedarEntityUid::new("Aeterna::Role", "Developer")
                && entity.attrs["name"] == "Developer"
                && entity.parents.is_empty()
        }));
        assert!(role_entities.iter().any(|entity| {
            entity.uid == CedarEntityUid::new("Aeterna::Role", "Viewer")
                && entity.attrs["name"] == "Viewer"
                && entity.parents.is_empty()
        }));
    }

    #[test]
    fn test_transform_users_adds_role_parent_memberships() {
        let row = UserPermissionRow {
            role: "platformadmin".to_string(),
            ..sample_user_row()
        };
        let user_id = row.user_id.to_string();

        let users = transform_users(vec![row]).unwrap();

        assert_eq!(users.len(), 1);
        let user = users.first().unwrap();
        assert_eq!(user.uid, CedarEntityUid::new("Aeterna::User", user_id));
        assert!(
            user.parents
                .iter()
                .any(|parent| parent == &CedarEntityUid::new("Aeterna::Role", "PlatformAdmin"))
        );
        assert!(
            user.parents
                .iter()
                .any(|parent| parent.entity_type == "Aeterna::Team")
        );
    }

    #[test]
    fn test_transform_roles_custom_roles_pass_through() {
        let custom_role = "super-operator".to_string();
        let row = UserPermissionRow {
            role: custom_role.clone(),
            ..sample_user_row()
        };

        let roles = transform_roles(&[row]);

        assert_eq!(roles.len(), 1);
        assert_eq!(
            roles[0].uid,
            CedarEntityUid::new("Aeterna::Role", custom_role)
        );
        assert_eq!(roles[0].attrs["name"], "super-operator");
    }

    #[test]
    fn test_transform_agents() {
        let agent_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let team_id = Uuid::new_v4();

        let row = AgentPermissionRow {
            tenant_id: Some(Uuid::new_v4()),
            agent_id,
            agent_name: "OpenCode Assistant".to_string(),
            agent_type: "opencode".to_string(),
            delegated_by_user_id: Some(user_id),
            delegated_by_agent_id: None,
            delegation_depth: 1,
            capabilities: serde_json::json!(["memory:read", "memory:write"]),
            allowed_tenant_ids: None,
            allowed_org_ids: None,
            allowed_team_ids: Some(vec![team_id]),
            allowed_project_ids: None,
            agent_status: "active".to_string(),
            delegating_user_email: Some("alice@acme.com".to_string()),
            delegating_user_name: Some("Alice".to_string()),
        };

        let entities = transform_agents(vec![row]).unwrap();

        assert_eq!(entities.len(), 1);
        let agent = &entities[0];
        assert_eq!(agent.uid.entity_type, "Aeterna::Agent");
        assert_eq!(agent.uid.id, agent_id.to_string());
        assert_eq!(agent.attrs["name"], "OpenCode Assistant");
        assert_eq!(agent.attrs["agent_type"], "opencode");
        assert_eq!(agent.attrs["delegation_depth"], 1);
        assert_eq!(agent.attrs["status"], "active");

        // Should have team as parent
        assert_eq!(agent.parents.len(), 1);
        assert_eq!(agent.parents[0].entity_type, "Aeterna::Team");

        // Should have delegated_by info
        let delegated_by = &agent.attrs["delegated_by"];
        assert_eq!(delegated_by["type"], "User");
        assert_eq!(delegated_by["id"], user_id.to_string());
    }

    #[test]
    fn test_transform_agents_delegated_by_agent() {
        let agent_id = Uuid::new_v4();
        let delegating_agent_id = Uuid::new_v4();

        let row = AgentPermissionRow {
            tenant_id: Some(Uuid::new_v4()),
            agent_id,
            agent_name: "Sub-Agent".to_string(),
            agent_type: "custom".to_string(),
            delegated_by_user_id: None,
            delegated_by_agent_id: Some(delegating_agent_id),
            delegation_depth: 2,
            capabilities: serde_json::json!(["knowledge:read"]),
            allowed_tenant_ids: None,
            allowed_org_ids: None,
            allowed_team_ids: None,
            allowed_project_ids: None,
            agent_status: "active".to_string(),
            delegating_user_email: None,
            delegating_user_name: None,
        };

        let entities = transform_agents(vec![row]).unwrap();

        assert_eq!(entities.len(), 1);
        let agent = &entities[0];
        let delegated_by = &agent.attrs["delegated_by"];
        assert_eq!(delegated_by["type"], "Agent");
        assert_eq!(delegated_by["id"], delegating_agent_id.to_string());
    }

    #[test]
    fn test_cedar_entities_response() {
        let entities = vec![CedarEntity {
            uid: CedarEntityUid::new("Aeterna::User", "123"),
            attrs: serde_json::json!({"email": "test@test.com"}),
            parents: vec![],
        }];

        let response = CedarEntitiesResponse::new(entities);

        assert_eq!(response.count, 1);
        assert_eq!(response.entities.len(), 1);
    }

    #[test]
    fn test_collect_project_team_assignments_single() {
        let row = sample_project_team_assignment("proj-1", "team-1", "owner");

        let assignments = collect_project_team_assignments(vec![row]);

        assert_eq!(assignments.len(), 1);
        let project_assignments = assignments.get("proj-1").unwrap();
        assert_eq!(project_assignments.len(), 1);
        assert_eq!(
            project_assignments[0],
            ("team-1".to_string(), "owner".to_string())
        );
    }

    #[test]
    fn test_collect_project_team_assignments_multiple_teams() {
        let row1 = sample_project_team_assignment("proj-1", "team-1", "owner");
        let row2 = sample_project_team_assignment("proj-1", "team-2", "contributor");

        let assignments = collect_project_team_assignments(vec![row1, row2]);

        assert_eq!(assignments.len(), 1);
        let project_assignments = assignments.get("proj-1").unwrap();
        assert_eq!(project_assignments.len(), 2);
        assert!(project_assignments.contains(&("team-1".to_string(), "owner".to_string())));
        assert!(project_assignments.contains(&("team-2".to_string(), "contributor".to_string())));
    }

    #[test]
    fn test_collect_project_team_assignments_multiple_projects() {
        let row1 = sample_project_team_assignment("proj-1", "team-1", "owner");
        let row2 = sample_project_team_assignment("proj-2", "team-2", "contributor");

        let assignments = collect_project_team_assignments(vec![row1, row2]);

        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments.get("proj-1").unwrap().len(), 1);
        assert_eq!(assignments.get("proj-2").unwrap().len(), 1);
    }

    #[test]
    fn test_collect_project_team_assignments_empty() {
        let assignments = collect_project_team_assignments(vec![]);

        assert!(assignments.is_empty());
    }

    #[test]
    fn test_augment_projects_adds_team_parents() {
        let row = sample_hierarchy_row();
        let mut entities = transform_hierarchy(vec![row]).unwrap();

        let project_id = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project")
            .unwrap()
            .uid
            .id
            .clone();
        let original_parent_count = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project")
            .unwrap()
            .parents
            .len();

        let assignments = std::collections::HashMap::from([(
            project_id.clone(),
            vec![("extra-team-99".to_string(), "owner".to_string())],
        )]);

        augment_projects_with_team_assignments(&mut entities, assignments);

        let project = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project" && e.uid.id == project_id)
            .unwrap();
        assert_eq!(project.parents.len(), original_parent_count + 1);
        assert!(
            project
                .parents
                .iter()
                .any(|p| p.id == "extra-team-99" && p.entity_type == "Aeterna::Team")
        );

        let team_assignments = project
            .attrs
            .get("team_assignments")
            .and_then(serde_json::Value::as_array)
            .unwrap();
        assert_eq!(team_assignments.len(), 1);
    }

    #[test]
    fn test_augment_projects_no_duplicate_parents() {
        let row = sample_hierarchy_row();
        let mut entities = transform_hierarchy(vec![row]).unwrap();

        let project = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project")
            .unwrap();
        let project_id = project.uid.id.clone();
        let existing_team_id = project.parents[0].id.clone();
        let parent_count_before = project.parents.len();

        let assignments = std::collections::HashMap::from([(
            project_id.clone(),
            vec![(existing_team_id.clone(), "owner".to_string())],
        )]);

        augment_projects_with_team_assignments(&mut entities, assignments);

        let updated_project = entities
            .iter()
            .find(|e| e.uid.entity_type == "Aeterna::Project" && e.uid.id == project_id)
            .unwrap();

        assert_eq!(updated_project.parents.len(), parent_count_before);
        assert_eq!(
            updated_project
                .parents
                .iter()
                .filter(|p| p.entity_type == "Aeterna::Team" && p.id == existing_team_id)
                .count(),
            1
        );
    }

    #[test]
    fn test_augment_projects_unrelated_project_unaffected() {
        let row = sample_hierarchy_row();
        let mut entities = transform_hierarchy(vec![row]).unwrap();
        let original_entities = entities.clone();

        let assignments = std::collections::HashMap::from([(
            "non-existent-project".to_string(),
            vec![("extra-team-99".to_string(), "owner".to_string())],
        )]);

        augment_projects_with_team_assignments(&mut entities, assignments);

        assert_eq!(entities, original_entities);
    }
}
