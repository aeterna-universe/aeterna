use chrono::Utc;
use mk_core::types::{OrganizationalUnit, RecordSource, TenantId, UnitType};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub tenant_id: TenantId,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Membership {
    pub user_id: String,
    pub unit_id: String,
    pub role: String,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub delegated_by: Option<String>,
    pub capabilities: Vec<String>,
    pub tenant_id: TenantId,
    pub created_at: i64,
    pub active: bool,
}

pub struct MockOnboardingStorage {
    units: Arc<RwLock<HashMap<String, OrganizationalUnit>>>,
    users: Arc<RwLock<HashMap<String, User>>>,
    memberships: Arc<RwLock<Vec<Membership>>>,
    agents: Arc<RwLock<HashMap<String, Agent>>>,
}

impl Default for MockOnboardingStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MockOnboardingStorage {
    pub fn new() -> Self {
        Self {
            units: Arc::new(RwLock::new(HashMap::new())),
            users: Arc::new(RwLock::new(HashMap::new())),
            memberships: Arc::new(RwLock::new(Vec::new())),
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn create_unit(&self, unit: OrganizationalUnit) -> Result<String, String> {
        if let Some(ref parent_id) = unit.parent_id {
            let units = self.units.read().await;
            let parent = units
                .get(parent_id)
                .ok_or_else(|| format!("Parent unit not found: {}", parent_id))?;

            match (parent.unit_type, unit.unit_type) {
                (UnitType::Organization, UnitType::Organization) => {}
                (UnitType::Organization, UnitType::Team) => {}
                (UnitType::Team, UnitType::Project) => {}
                _ => {
                    return Err(format!(
                        "Invalid hierarchy: cannot create {:?} under {:?}",
                        unit.unit_type, parent.unit_type
                    ));
                }
            }
        } else if unit.unit_type != UnitType::Organization {
            return Err("Only Organization units can be root units (no parent)".to_string());
        }

        let id = unit.id.clone();
        self.units.write().await.insert(id.clone(), unit);
        Ok(id)
    }

    pub async fn get_unit(&self, id: &str) -> Result<Option<OrganizationalUnit>, String> {
        let units = self.units.read().await;
        Ok(units.get(id).cloned())
    }

    pub async fn list_children(&self, parent_id: &str) -> Result<Vec<OrganizationalUnit>, String> {
        let units = self.units.read().await;
        let children: Vec<_> = units
            .values()
            .filter(|u| u.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect();
        Ok(children)
    }

    pub async fn get_ancestors(&self, id: &str) -> Result<Vec<OrganizationalUnit>, String> {
        let units = self.units.read().await;
        let mut ancestors = Vec::new();
        let mut current_id = units.get(id).and_then(|u| u.parent_id.clone());

        while let Some(parent_id) = current_id {
            if let Some(parent) = units.get(&parent_id) {
                ancestors.push(parent.clone());
                current_id = parent.parent_id.clone();
            } else {
                break;
            }
        }

        Ok(ancestors)
    }

    pub async fn delete_unit(&self, id: &str) -> Result<bool, String> {
        let children = self.list_children(id).await?;
        if !children.is_empty() {
            return Err(format!(
                "Cannot delete unit with children: {} children exist",
                children.len()
            ));
        }

        let mut units = self.units.write().await;
        Ok(units.remove(id).is_some())
    }

    pub async fn register_user(&self, user: User) -> Result<String, String> {
        let users = self.users.read().await;
        if users.values().any(|u| u.email == user.email) {
            return Err(format!("User with email {} already exists", user.email));
        }
        drop(users);

        let id = user.id.clone();
        self.users.write().await.insert(id.clone(), user);
        Ok(id)
    }

    pub async fn get_user(&self, id: &str) -> Result<Option<User>, String> {
        let users = self.users.read().await;
        Ok(users.get(id).cloned())
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, String> {
        let users = self.users.read().await;
        Ok(users.values().find(|u| u.email == email).cloned())
    }

    pub async fn add_membership(&self, membership: Membership) -> Result<(), String> {
        let memberships = self.memberships.read().await;
        if memberships
            .iter()
            .any(|m| m.user_id == membership.user_id && m.unit_id == membership.unit_id)
        {
            return Err("Membership already exists".to_string());
        }
        drop(memberships);

        self.memberships.write().await.push(membership);
        Ok(())
    }

    pub async fn get_user_memberships(&self, user_id: &str) -> Result<Vec<Membership>, String> {
        let memberships = self.memberships.read().await;
        Ok(memberships
            .iter()
            .filter(|m| m.user_id == user_id)
            .cloned()
            .collect())
    }

    pub async fn get_unit_members(&self, unit_id: &str) -> Result<Vec<Membership>, String> {
        let memberships = self.memberships.read().await;
        Ok(memberships
            .iter()
            .filter(|m| m.unit_id == unit_id)
            .cloned()
            .collect())
    }

    pub async fn remove_membership(&self, user_id: &str, unit_id: &str) -> Result<bool, String> {
        let mut memberships = self.memberships.write().await;
        let len_before = memberships.len();
        memberships.retain(|m| !(m.user_id == user_id && m.unit_id == unit_id));
        Ok(memberships.len() < len_before)
    }

    pub async fn register_agent(&self, agent: Agent) -> Result<String, String> {
        if let Some(ref delegated_by) = agent.delegated_by {
            let users = self.users.read().await;
            if !users.contains_key(delegated_by) {
                let agents = self.agents.read().await;
                if !agents.contains_key(delegated_by) {
                    return Err(format!(
                        "Delegator not found: {} (must be existing user or agent)",
                        delegated_by
                    ));
                }
            }
        }

        let id = agent.id.clone();
        self.agents.write().await.insert(id.clone(), agent);
        Ok(id)
    }

    pub async fn get_agent(&self, id: &str) -> Result<Option<Agent>, String> {
        let agents = self.agents.read().await;
        Ok(agents.get(id).cloned())
    }

    pub async fn list_agents_by_owner(&self, owner_id: &str) -> Result<Vec<Agent>, String> {
        let agents = self.agents.read().await;
        Ok(agents
            .values()
            .filter(|a| a.owner_id == owner_id)
            .cloned()
            .collect())
    }

    pub async fn revoke_agent(&self, id: &str) -> Result<bool, String> {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.active = false;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub async fn get_delegation_chain(&self, agent_id: &str) -> Result<Vec<String>, String> {
        let agents = self.agents.read().await;
        let users = self.users.read().await;

        let mut chain = Vec::new();
        let mut current_id = Some(agent_id.to_string());

        while let Some(id) = current_id {
            if let Some(agent) = agents.get(&id) {
                chain.push(id.clone());
                current_id = agent.delegated_by.clone();
            } else if users.contains_key(&id) {
                chain.push(id);
                break;
            } else {
                break;
            }
        }

        Ok(chain)
    }
}

fn create_test_unit(
    id: &str,
    name: &str,
    unit_type: UnitType,
    parent_id: Option<&str>,
) -> OrganizationalUnit {
    let now = Utc::now();
    OrganizationalUnit {
        id: id.to_string(),
        name: name.to_string(),
        unit_type,
        parent_id: parent_id.map(String::from),
        tenant_id: TenantId::new("test-tenant".to_string()).unwrap(),
        metadata: HashMap::new(),
        created_at: now,
        updated_at: now,
        source_owner: RecordSource::Admin,
    }
}

fn create_test_user(id: &str, email: &str, name: &str) -> User {
    User {
        id: id.to_string(),
        email: email.to_string(),
        name: name.to_string(),
        tenant_id: TenantId::new("test-tenant".to_string()).unwrap(),
        created_at: Utc::now().timestamp(),
    }
}

fn create_test_agent(
    id: &str,
    name: &str,
    owner_id: &str,
    delegated_by: Option<&str>,
    capabilities: Vec<&str>,
) -> Agent {
    Agent {
        id: id.to_string(),
        name: name.to_string(),
        owner_id: owner_id.to_string(),
        delegated_by: delegated_by.map(String::from),
        capabilities: capabilities.into_iter().map(String::from).collect(),
        tenant_id: TenantId::new("test-tenant".to_string()).unwrap(),
        created_at: Utc::now().timestamp(),
        active: true,
    }
}

#[cfg(test)]
mod organizational_unit_tests {
    use super::*;

    #[tokio::test]
    async fn test_create_company_as_root() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);

        let result = storage.create_unit(tenant).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "root-org-1");
    }

    #[tokio::test]
    async fn test_cannot_create_non_company_as_root() {
        let storage = MockOnboardingStorage::new();
        let org = create_test_unit("org-1", "Engineering", UnitType::Organization, None);

        let result = storage.create_unit(org).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Only Organization units can be root")
        );
    }

    #[tokio::test]
    async fn test_create_organization_under_company() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        let result = storage.create_unit(org).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_team_under_organization() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        let result = storage.create_unit(team).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_create_project_under_team() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team).await.unwrap();

        let project = create_test_unit(
            "project-1",
            "payments-service",
            UnitType::Project,
            Some("team-1"),
        );
        let result = storage.create_unit(project).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_invalid_hierarchy_team_under_company() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("root-org-1"));
        let result = storage.create_unit(team).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hierarchy"));
    }

    #[tokio::test]
    async fn test_invalid_hierarchy_project_under_organization() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let project = create_test_unit(
            "project-1",
            "payments-service",
            UnitType::Project,
            Some("org-1"),
        );
        let result = storage.create_unit(project).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid hierarchy"));
    }

    #[tokio::test]
    async fn test_invalid_hierarchy_organization_under_project() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team).await.unwrap();

        let project = create_test_unit(
            "project-1",
            "payments-service",
            UnitType::Project,
            Some("team-1"),
        );
        storage.create_unit(project).await.unwrap();

        let nested_org = create_test_unit(
            "org-2",
            "Nested Org",
            UnitType::Organization,
            Some("project-1"),
        );
        let result = storage.create_unit(nested_org).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parent_not_found() {
        let storage = MockOnboardingStorage::new();
        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("nonexistent"),
        );

        let result = storage.create_unit(org).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Parent unit not found"));
    }

    #[tokio::test]
    async fn test_get_unit() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let result = storage.get_unit("root-org-1").await.unwrap();
        assert!(result.is_some());
        let unit = result.unwrap();
        assert_eq!(unit.name, "Acme Root Org");
        assert_eq!(unit.unit_type, UnitType::Organization);
    }

    #[tokio::test]
    async fn test_get_unit_not_found() {
        let storage = MockOnboardingStorage::new();
        let result = storage.get_unit("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_list_children() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org1 = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org1).await.unwrap();

        let org2 = create_test_unit(
            "org-2",
            "Product",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org2).await.unwrap();

        let children = storage.list_children("root-org-1").await.unwrap();
        assert_eq!(children.len(), 2);
    }

    #[tokio::test]
    async fn test_get_ancestors() {
        let storage = MockOnboardingStorage::new();

        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team).await.unwrap();

        let project = create_test_unit(
            "project-1",
            "payments-service",
            UnitType::Project,
            Some("team-1"),
        );
        storage.create_unit(project).await.unwrap();

        let ancestors = storage.get_ancestors("project-1").await.unwrap();
        assert_eq!(ancestors.len(), 3);
        assert_eq!(ancestors[0].unit_type, UnitType::Team);
        assert_eq!(ancestors[1].unit_type, UnitType::Organization);
        assert_eq!(ancestors[2].unit_type, UnitType::Organization);
    }

    #[tokio::test]
    async fn test_delete_unit() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let result = storage.delete_unit("root-org-1").await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        let unit = storage.get_unit("root-org-1").await.unwrap();
        assert!(unit.is_none());
    }

    #[tokio::test]
    async fn test_cannot_delete_unit_with_children() {
        let storage = MockOnboardingStorage::new();
        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let result = storage.delete_unit("root-org-1").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot delete unit with children")
        );
    }
}

#[cfg(test)]
mod user_registration_tests {
    use super::*;

    #[tokio::test]
    async fn test_register_user() {
        let storage = MockOnboardingStorage::new();
        let user = create_test_user("user-1", "alice@acme.com", "Alice Smith");

        let result = storage.register_user(user).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "user-1");
    }

    #[tokio::test]
    async fn test_duplicate_email_rejected() {
        let storage = MockOnboardingStorage::new();
        let user1 = create_test_user("user-1", "alice@acme.com", "Alice Smith");
        storage.register_user(user1).await.unwrap();

        let user2 = create_test_user("user-2", "alice@acme.com", "Alice Jones");
        let result = storage.register_user(user2).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[tokio::test]
    async fn test_get_user() {
        let storage = MockOnboardingStorage::new();
        let user = create_test_user("user-1", "alice@acme.com", "Alice Smith");
        storage.register_user(user).await.unwrap();

        let result = storage.get_user("user-1").await.unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.email, "alice@acme.com");
        assert_eq!(retrieved.name, "Alice Smith");
    }

    #[tokio::test]
    async fn test_get_user_by_email() {
        let storage = MockOnboardingStorage::new();
        let user = create_test_user("user-1", "alice@acme.com", "Alice Smith");
        storage.register_user(user).await.unwrap();

        let result = storage.get_user_by_email("alice@acme.com").await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, "user-1");
    }

    #[tokio::test]
    async fn test_get_user_not_found() {
        let storage = MockOnboardingStorage::new();
        let result = storage.get_user("nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}

#[cfg(test)]
mod membership_tests {
    use super::*;

    #[tokio::test]
    async fn test_add_membership() {
        let storage = MockOnboardingStorage::new();
        let membership = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };

        let result = storage.add_membership(membership).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_duplicate_membership_rejected() {
        let storage = MockOnboardingStorage::new();
        let membership = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(membership.clone()).await.unwrap();

        let result = storage.add_membership(membership).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[tokio::test]
    async fn test_get_user_memberships() {
        let storage = MockOnboardingStorage::new();

        let m1 = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m1).await.unwrap();

        let m2 = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-2".to_string(),
            role: "techlead".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m2).await.unwrap();

        let memberships = storage.get_user_memberships("user-1").await.unwrap();
        assert_eq!(memberships.len(), 2);
    }

    #[tokio::test]
    async fn test_get_unit_members() {
        let storage = MockOnboardingStorage::new();

        let m1 = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m1).await.unwrap();

        let m2 = Membership {
            user_id: "user-2".to_string(),
            unit_id: "team-1".to_string(),
            role: "techlead".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m2).await.unwrap();

        let members = storage.get_unit_members("team-1").await.unwrap();
        assert_eq!(members.len(), 2);
    }

    #[tokio::test]
    async fn test_remove_membership() {
        let storage = MockOnboardingStorage::new();
        let membership = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(membership).await.unwrap();

        let result = storage.remove_membership("user-1", "team-1").await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        let memberships = storage.get_user_memberships("user-1").await.unwrap();
        assert!(memberships.is_empty());
    }

    #[tokio::test]
    async fn test_remove_nonexistent_membership() {
        let storage = MockOnboardingStorage::new();
        let result = storage.remove_membership("user-1", "team-1").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}

#[cfg(test)]
mod agent_registration_tests {
    use super::*;

    #[tokio::test]
    async fn test_register_agent_without_delegation() {
        let storage = MockOnboardingStorage::new();
        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            None,
            vec!["code_review", "testing"],
        );

        let result = storage.register_agent(agent).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "agent-1");
    }

    #[tokio::test]
    async fn test_register_agent_with_user_delegation() {
        let storage = MockOnboardingStorage::new();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            Some("user-1"),
            vec!["code_review"],
        );

        let result = storage.register_agent(agent).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_register_agent_with_agent_delegation() {
        let storage = MockOnboardingStorage::new();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let agent1 = create_test_agent(
            "agent-1",
            "Primary Agent",
            "user-1",
            Some("user-1"),
            vec!["code_review", "testing"],
        );
        storage.register_agent(agent1).await.unwrap();

        let agent2 = create_test_agent(
            "agent-2",
            "Sub Agent",
            "user-1",
            Some("agent-1"),
            vec!["testing"],
        );

        let result = storage.register_agent(agent2).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_agent_delegation_chain_validation() {
        let storage = MockOnboardingStorage::new();

        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            Some("nonexistent"),
            vec!["code_review"],
        );

        let result = storage.register_agent(agent).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Delegator not found"));
    }

    #[tokio::test]
    async fn test_get_agent() {
        let storage = MockOnboardingStorage::new();
        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            None,
            vec!["code_review"],
        );
        storage.register_agent(agent).await.unwrap();

        let result = storage.get_agent("agent-1").await.unwrap();
        assert!(result.is_some());
        let retrieved = result.unwrap();
        assert_eq!(retrieved.name, "Code Assistant");
        assert!(retrieved.active);
    }

    #[tokio::test]
    async fn test_list_agents_by_owner() {
        let storage = MockOnboardingStorage::new();

        let agent1 = create_test_agent("agent-1", "Agent 1", "user-1", None, vec!["code_review"]);
        storage.register_agent(agent1).await.unwrap();

        let agent2 = create_test_agent("agent-2", "Agent 2", "user-1", None, vec!["testing"]);
        storage.register_agent(agent2).await.unwrap();

        let agent3 = create_test_agent("agent-3", "Agent 3", "user-2", None, vec!["docs"]);
        storage.register_agent(agent3).await.unwrap();

        let agents = storage.list_agents_by_owner("user-1").await.unwrap();
        assert_eq!(agents.len(), 2);
    }

    #[tokio::test]
    async fn test_revoke_agent() {
        let storage = MockOnboardingStorage::new();
        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            None,
            vec!["code_review"],
        );
        storage.register_agent(agent).await.unwrap();

        let result = storage.revoke_agent("agent-1").await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        let agent = storage.get_agent("agent-1").await.unwrap().unwrap();
        assert!(!agent.active);
    }

    #[tokio::test]
    async fn test_revoke_nonexistent_agent() {
        let storage = MockOnboardingStorage::new();
        let result = storage.revoke_agent("nonexistent").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_get_delegation_chain() {
        let storage = MockOnboardingStorage::new();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let agent1 = create_test_agent(
            "agent-1",
            "Primary Agent",
            "user-1",
            Some("user-1"),
            vec!["all"],
        );
        storage.register_agent(agent1).await.unwrap();

        let agent2 = create_test_agent(
            "agent-2",
            "Sub Agent",
            "user-1",
            Some("agent-1"),
            vec!["subset"],
        );
        storage.register_agent(agent2).await.unwrap();

        let chain = storage.get_delegation_chain("agent-2").await.unwrap();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0], "agent-2");
        assert_eq!(chain[1], "agent-1");
        assert_eq!(chain[2], "user-1");
    }

    #[tokio::test]
    async fn test_agent_capabilities() {
        let storage = MockOnboardingStorage::new();
        let agent = create_test_agent(
            "agent-1",
            "Code Assistant",
            "user-1",
            None,
            vec!["code_review", "testing", "documentation"],
        );
        storage.register_agent(agent).await.unwrap();

        let retrieved = storage.get_agent("agent-1").await.unwrap().unwrap();
        assert_eq!(retrieved.capabilities.len(), 3);
        assert!(retrieved.capabilities.contains(&"code_review".to_string()));
        assert!(retrieved.capabilities.contains(&"testing".to_string()));
        assert!(
            retrieved
                .capabilities
                .contains(&"documentation".to_string())
        );
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[tokio::test]
    async fn test_full_onboarding_workflow() {
        let storage = MockOnboardingStorage::new();

        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team).await.unwrap();

        let project = create_test_unit(
            "project-1",
            "payments-service",
            UnitType::Project,
            Some("team-1"),
        );
        storage.create_unit(project).await.unwrap();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let membership = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(membership).await.unwrap();

        let agent = create_test_agent(
            "agent-1",
            "Alice's Assistant",
            "user-1",
            Some("user-1"),
            vec!["code_review", "testing"],
        );
        storage.register_agent(agent).await.unwrap();

        let chain = storage.get_delegation_chain("agent-1").await.unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0], "agent-1");
        assert_eq!(chain[1], "user-1");

        let ancestors = storage.get_ancestors("project-1").await.unwrap();
        assert_eq!(ancestors.len(), 3);

        let memberships = storage.get_user_memberships("user-1").await.unwrap();
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].role, "developer");
    }

    #[tokio::test]
    async fn test_multi_team_user() {
        let storage = MockOnboardingStorage::new();

        let tenant = create_test_unit("root-org-1", "Acme Root Org", UnitType::Organization, None);
        storage.create_unit(tenant).await.unwrap();

        let org = create_test_unit(
            "org-1",
            "Engineering",
            UnitType::Organization,
            Some("root-org-1"),
        );
        storage.create_unit(org).await.unwrap();

        let team1 = create_test_unit("team-1", "API Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team1).await.unwrap();

        let team2 = create_test_unit("team-2", "Frontend Team", UnitType::Team, Some("org-1"));
        storage.create_unit(team2).await.unwrap();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let m1 = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-1".to_string(),
            role: "developer".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m1).await.unwrap();

        let m2 = Membership {
            user_id: "user-1".to_string(),
            unit_id: "team-2".to_string(),
            role: "techlead".to_string(),
            created_at: Utc::now().timestamp(),
        };
        storage.add_membership(m2).await.unwrap();

        let memberships = storage.get_user_memberships("user-1").await.unwrap();
        assert_eq!(memberships.len(), 2);

        let has_developer = memberships.iter().any(|m| m.role == "developer");
        let has_techlead = memberships.iter().any(|m| m.role == "techlead");
        assert!(has_developer);
        assert!(has_techlead);
    }

    #[tokio::test]
    async fn test_cascading_delegation() {
        let storage = MockOnboardingStorage::new();

        let user = create_test_user("user-1", "alice@acme.com", "Alice");
        storage.register_user(user).await.unwrap();

        let agent1 = create_test_agent("agent-1", "Primary", "user-1", Some("user-1"), vec!["all"]);
        storage.register_agent(agent1).await.unwrap();

        let agent2 = create_test_agent(
            "agent-2",
            "Secondary",
            "user-1",
            Some("agent-1"),
            vec!["subset"],
        );
        storage.register_agent(agent2).await.unwrap();

        let agent3 = create_test_agent(
            "agent-3",
            "Tertiary",
            "user-1",
            Some("agent-2"),
            vec!["minimal"],
        );
        storage.register_agent(agent3).await.unwrap();

        let chain = storage.get_delegation_chain("agent-3").await.unwrap();
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0], "agent-3");
        assert_eq!(chain[1], "agent-2");
        assert_eq!(chain[2], "agent-1");
        assert_eq!(chain[3], "user-1");
    }
}
