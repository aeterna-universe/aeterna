//! In-memory `StorageBackend` implementation for unit tests.
//!
//! Provides `MockStorageBackend` — a thread-safe, `Arc`-friendly implementation
//! of `mk_core::traits::StorageBackend` that stores everything in `DashMap`s.
//! No database, no network, no Docker required.
//!
//! The associated `Error` type is `storage::postgres::PostgresError` so that
//! this mock can be dropped directly into governance tools and other code that
//! is hardcoded to `dyn StorageBackend<Error = PostgresError>`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use testing::mock_storage::MockStorageBackend;
//!
//! let backend = Arc::new(MockStorageBackend::new());
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use mk_core::traits::StorageBackend;
use mk_core::types::{
    ConsumerState, DriftConfig, DriftResult, DriftSuppression, EventDeliveryMetrics, EventStatus,
    GovernanceEvent, OrganizationalUnit, PersistentEvent, Policy, RoleIdentifier, TenantContext,
    TenantId, UserId,
};
use storage::postgres::PostgresError;

/// Thread-safe in-memory storage backend for unit tests.
///
/// All state lives in `DashMap` collections, which are safe to share across
/// async tasks via `Arc<MockStorageBackend>`.
pub struct MockStorageBackend {
    pub kv: DashMap<String, Vec<u8>>,
    pub units: DashMap<String, OrganizationalUnit>,
    pub policies: DashMap<String, Vec<Policy>>, // key = unit_id
    pub roles: DashMap<String, Vec<RoleIdentifier>>, // key = "{user_id}:{tenant_id}:{unit_id}"
    pub drift_results: DashMap<String, DriftResult>, // key = project_id
    pub drift_configs: DashMap<String, DriftConfig>, // key = project_id
    pub suppressions: DashMap<String, Vec<DriftSuppression>>, // key = project_id
    pub events: DashMap<String, PersistentEvent>, // key = event_id
    pub governance_events: DashMap<String, Vec<GovernanceEvent>>, // key = tenant_id
    pub consumer_states: DashMap<String, ConsumerState>,
    pub metrics: DashMap<String, EventDeliveryMetrics>,
    pub idempotency_keys: DashMap<String, bool>, // key = "{consumer_group}:{key}"
    pub job_statuses: DashMap<String, String>,
}

impl Default for MockStorageBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MockStorageBackend {
    /// Create a new, empty `MockStorageBackend`.
    pub fn new() -> Self {
        Self {
            kv: DashMap::new(),
            units: DashMap::new(),
            policies: DashMap::new(),
            roles: DashMap::new(),
            drift_results: DashMap::new(),
            drift_configs: DashMap::new(),
            suppressions: DashMap::new(),
            events: DashMap::new(),
            governance_events: DashMap::new(),
            consumer_states: DashMap::new(),
            metrics: DashMap::new(),
            idempotency_keys: DashMap::new(),
            job_statuses: DashMap::new(),
        }
    }

    /// Wrap in an `Arc` for ergonomic use in tests.
    pub fn arc() -> Arc<Self> {
        Arc::new(Self::new())
    }
}

#[async_trait]
impl StorageBackend for MockStorageBackend {
    type Error = PostgresError;

    async fn store(&self, ctx: TenantContext, key: &str, value: &[u8]) -> Result<(), Self::Error> {
        let full_key = format!("{}:{}", ctx.tenant_id, key);
        self.kv.insert(full_key, value.to_vec());
        Ok(())
    }

    async fn retrieve(
        &self,
        ctx: TenantContext,
        key: &str,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        let full_key = format!("{}:{}", ctx.tenant_id, key);
        Ok(self.kv.get(&full_key).map(|v| v.clone()))
    }

    async fn delete(&self, ctx: TenantContext, key: &str) -> Result<(), Self::Error> {
        let full_key = format!("{}:{}", ctx.tenant_id, key);
        self.kv.remove(&full_key);
        Ok(())
    }

    async fn exists(&self, ctx: TenantContext, key: &str) -> Result<bool, Self::Error> {
        let full_key = format!("{}:{}", ctx.tenant_id, key);
        Ok(self.kv.contains_key(&full_key))
    }

    async fn get_ancestors(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        // Walk parent chain until no parent.
        let mut result = Vec::new();
        let mut current_id = unit_id.to_string();
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current_id.clone()) {
                break; // Cycle guard
            }
            let unit = self.units.get(&current_id).map(|u| u.clone());
            match unit {
                Some(u) => {
                    if let Some(ref parent_id) = u.parent_id {
                        let pid = parent_id.clone();
                        result.push(u);
                        current_id = pid;
                    } else {
                        result.push(u);
                        break;
                    }
                }
                None => break,
            }
        }
        Ok(result)
    }

    async fn get_descendants(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        // BFS over parent_id references.
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(unit_id.to_string());
        let mut visited = std::collections::HashSet::new();

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            for entry in self.units.iter() {
                if entry.parent_id.as_deref() == Some(&current) {
                    queue.push_back(entry.id.clone());
                    result.push(entry.clone());
                }
            }
        }
        Ok(result)
    }

    async fn get_unit_policies(
        &self,
        _ctx: TenantContext,
        unit_id: &str,
    ) -> Result<Vec<Policy>, Self::Error> {
        Ok(self
            .policies
            .get(unit_id)
            .map(|p| p.clone())
            .unwrap_or_default())
    }

    async fn create_unit(&self, unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        self.units.insert(unit.id.clone(), unit.clone());
        Ok(())
    }

    async fn add_unit_policy(
        &self,
        _ctx: &TenantContext,
        unit_id: &str,
        policy: &Policy,
    ) -> Result<(), Self::Error> {
        self.policies
            .entry(unit_id.to_string())
            .or_default()
            .push(policy.clone());
        Ok(())
    }

    async fn assign_role(
        &self,
        user_id: &UserId,
        tenant_id: &TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        let key = format!("{}:{}:{}", user_id.as_str(), tenant_id.as_str(), unit_id);
        self.roles.entry(key).or_default().push(role);
        Ok(())
    }

    async fn remove_role(
        &self,
        user_id: &UserId,
        tenant_id: &TenantId,
        unit_id: &str,
        role: RoleIdentifier,
    ) -> Result<(), Self::Error> {
        let key = format!("{}:{}:{}", user_id.as_str(), tenant_id.as_str(), unit_id);
        if let Some(mut roles) = self.roles.get_mut(&key) {
            roles.retain(|r| r != &role);
        }
        Ok(())
    }

    async fn store_drift_result(&self, result: DriftResult) -> Result<(), Self::Error> {
        self.drift_results.insert(result.project_id.clone(), result);
        Ok(())
    }

    async fn get_latest_drift_result(
        &self,
        _ctx: TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftResult>, Self::Error> {
        Ok(self.drift_results.get(project_id).map(|r| r.clone()))
    }

    async fn list_all_units(&self) -> Result<Vec<OrganizationalUnit>, Self::Error> {
        Ok(self.units.iter().map(|e| e.clone()).collect())
    }

    async fn record_job_status(
        &self,
        job_name: &str,
        tenant_id: &str,
        status: &str,
        _message: Option<&str>,
        _started_at: i64,
        _finished_at: Option<i64>,
    ) -> Result<(), Self::Error> {
        let key = format!("{}:{}", tenant_id, job_name);
        self.job_statuses.insert(key, status.to_string());
        Ok(())
    }

    async fn get_governance_events(
        &self,
        ctx: TenantContext,
        _since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<GovernanceEvent>, Self::Error> {
        Ok(self
            .governance_events
            .get(ctx.tenant_id.as_str())
            .map(|e| e.iter().take(limit).cloned().collect())
            .unwrap_or_default())
    }

    async fn create_suppression(&self, suppression: DriftSuppression) -> Result<(), Self::Error> {
        self.suppressions
            .entry(suppression.project_id.clone())
            .or_default()
            .push(suppression);
        Ok(())
    }

    async fn list_suppressions(
        &self,
        _ctx: TenantContext,
        project_id: &str,
    ) -> Result<Vec<DriftSuppression>, Self::Error> {
        Ok(self
            .suppressions
            .get(project_id)
            .map(|s| s.clone())
            .unwrap_or_default())
    }

    async fn delete_suppression(
        &self,
        _ctx: TenantContext,
        suppression_id: &str,
    ) -> Result<(), Self::Error> {
        for mut entry in self.suppressions.iter_mut() {
            entry.retain(|s| s.id != suppression_id);
        }
        Ok(())
    }

    async fn get_drift_config(
        &self,
        _ctx: TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftConfig>, Self::Error> {
        Ok(self.drift_configs.get(project_id).map(|c| c.clone()))
    }

    async fn save_drift_config(&self, config: DriftConfig) -> Result<(), Self::Error> {
        self.drift_configs.insert(config.project_id.clone(), config);
        Ok(())
    }

    async fn persist_event(&self, event: PersistentEvent) -> Result<(), Self::Error> {
        self.events.insert(event.id.clone(), event);
        Ok(())
    }

    async fn get_pending_events(
        &self,
        _ctx: TenantContext,
        limit: usize,
    ) -> Result<Vec<PersistentEvent>, Self::Error> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.status == EventStatus::Pending)
            .take(limit)
            .map(|e| e.clone())
            .collect())
    }

    async fn update_event_status(
        &self,
        event_id: &str,
        status: EventStatus,
        _error: Option<String>,
    ) -> Result<(), Self::Error> {
        if let Some(mut event) = self.events.get_mut(event_id) {
            event.status = status;
        }
        Ok(())
    }

    async fn get_dead_letter_events(
        &self,
        _ctx: TenantContext,
        limit: usize,
    ) -> Result<Vec<PersistentEvent>, Self::Error> {
        Ok(self
            .events
            .iter()
            .filter(|e| e.status == EventStatus::DeadLettered)
            .take(limit)
            .map(|e| e.clone())
            .collect())
    }

    async fn check_idempotency(
        &self,
        consumer_group: &str,
        idempotency_key: &str,
    ) -> Result<bool, Self::Error> {
        let key = format!("{}:{}", consumer_group, idempotency_key);
        if self.idempotency_keys.contains_key(&key) {
            return Ok(true); // Already processed
        }
        self.idempotency_keys.insert(key, true);
        Ok(false)
    }

    async fn record_consumer_state(&self, state: ConsumerState) -> Result<(), Self::Error> {
        self.consumer_states
            .insert(state.consumer_group.clone(), state);
        Ok(())
    }

    async fn get_event_metrics(
        &self,
        _ctx: TenantContext,
        _period_start: i64,
        _period_end: i64,
    ) -> Result<Vec<EventDeliveryMetrics>, Self::Error> {
        Ok(self.metrics.iter().map(|e| e.clone()).collect())
    }

    async fn record_event_metrics(&self, metrics: EventDeliveryMetrics) -> Result<(), Self::Error> {
        self.metrics.insert(metrics.event_type.clone(), metrics);
        Ok(())
    }

    async fn get_unit_by_id(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Option<OrganizationalUnit>, Self::Error> {
        Ok(None)
    }

    async fn update_unit(&self, _unit: &OrganizationalUnit) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn delete_unit(&self, _unit_id: &str, _tenant_id: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_unit_members(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Vec<(UserId, RoleIdentifier)>, Self::Error> {
        Ok(Vec::new())
    }

    async fn assign_team_to_project(
        &self,
        _project_id: &str,
        _team_id: &str,
        _tenant_id: &str,
        _assignment_type: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remove_team_from_project(
        &self,
        _project_id: &str,
        _team_id: &str,
        _tenant_id: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_project_team_assignments(
        &self,
        _project_id: &str,
        _tenant_id: &str,
    ) -> Result<Vec<(String, String)>, Self::Error> {
        Ok(Vec::new())
    }

    async fn get_effective_roles_at_scope(
        &self,
        _user_id: &UserId,
        _tenant_id: &TenantId,
        _unit_id: &str,
    ) -> Result<Vec<RoleIdentifier>, Self::Error> {
        Ok(Vec::new())
    }
}

// =============================================================================
// Self-tests for MockStorageBackend
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{RecordSource, Role, TenantContext, TenantId, UnitType, UserId};

    fn tenant_ctx(id: &str) -> TenantContext {
        TenantContext {
            tenant_id: TenantId::new(id.to_string()).unwrap(),
            user_id: UserId::new("user-1".to_string()).unwrap(),
            agent_id: None,
            roles: Vec::new(),
            target_tenant_id: None,
        }
    }

    fn make_unit(id: &str, parent: Option<&str>, tenant: &str) -> OrganizationalUnit {
        OrganizationalUnit {
            id: id.to_string(),
            name: id.to_string(),
            unit_type: UnitType::Team,
            parent_id: parent.map(|s| s.to_string()),
            tenant_id: TenantId::new(tenant.to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        }
    }

    // -----------
    // KV store
    // -----------

    #[tokio::test]
    async fn test_store_and_retrieve() {
        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("acme");
        backend.store(ctx.clone(), "key1", b"hello").await.unwrap();
        let val = backend.retrieve(ctx, "key1").await.unwrap();
        assert_eq!(val, Some(b"hello".to_vec()));
    }

    #[tokio::test]
    async fn test_retrieve_missing_returns_none() {
        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("acme");
        let val = backend.retrieve(ctx, "missing-key").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn test_delete_removes_entry() {
        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("acme");
        backend.store(ctx.clone(), "key", b"val").await.unwrap();
        backend.delete(ctx.clone(), "key").await.unwrap();
        assert_eq!(backend.retrieve(ctx, "key").await.unwrap(), None);
    }

    #[tokio::test]
    async fn test_exists_true_and_false() {
        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("acme");
        assert!(!backend.exists(ctx.clone(), "k").await.unwrap());
        backend.store(ctx.clone(), "k", b"v").await.unwrap();
        assert!(backend.exists(ctx, "k").await.unwrap());
    }

    #[tokio::test]
    async fn test_tenant_isolation() {
        let backend = MockStorageBackend::new();
        let ctx_a = tenant_ctx("tenant-a");
        let ctx_b = tenant_ctx("tenant-b");
        backend
            .store(ctx_a.clone(), "key", b"from-a")
            .await
            .unwrap();
        let val_b = backend.retrieve(ctx_b, "key").await.unwrap();
        assert!(val_b.is_none(), "Tenant B should not see Tenant A's data");
    }

    // -----------
    // Units
    // -----------

    #[tokio::test]
    async fn test_create_unit_and_list_all() {
        let backend = MockStorageBackend::new();
        let unit = make_unit("unit-1", None, "t1");
        backend.create_unit(&unit).await.unwrap();
        let all = backend.list_all_units().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "unit-1");
    }

    #[tokio::test]
    async fn test_get_ancestors_chain() {
        let backend = MockStorageBackend::new();
        let grandparent = make_unit("gp", None, "t");
        let parent = make_unit("p", Some("gp"), "t");
        let child = make_unit("c", Some("p"), "t");
        backend.create_unit(&grandparent).await.unwrap();
        backend.create_unit(&parent).await.unwrap();
        backend.create_unit(&child).await.unwrap();

        let ctx = tenant_ctx("t");
        let ancestors = backend.get_ancestors(ctx, "c").await.unwrap();
        // Should include child itself, then parent, then grandparent
        let ids: Vec<&str> = ancestors.iter().map(|u| u.id.as_str()).collect();
        assert!(ids.contains(&"c"), "Should include starting unit");
        assert!(ids.contains(&"p"), "Should include parent");
        assert!(ids.contains(&"gp"), "Should include grandparent");
    }

    #[tokio::test]
    async fn test_get_descendants() {
        let backend = MockStorageBackend::new();
        let root = make_unit("root", None, "t");
        let child1 = make_unit("c1", Some("root"), "t");
        let child2 = make_unit("c2", Some("root"), "t");
        let grandchild = make_unit("gc", Some("c1"), "t");
        backend.create_unit(&root).await.unwrap();
        backend.create_unit(&child1).await.unwrap();
        backend.create_unit(&child2).await.unwrap();
        backend.create_unit(&grandchild).await.unwrap();

        let ctx = tenant_ctx("t");
        let descendants = backend.get_descendants(ctx, "root").await.unwrap();
        let ids: Vec<&str> = descendants.iter().map(|u| u.id.as_str()).collect();
        assert!(ids.contains(&"c1"));
        assert!(ids.contains(&"c2"));
        assert!(ids.contains(&"gc"));
        assert!(
            !ids.contains(&"root"),
            "Root should not be in its own descendants"
        );
    }

    // -----------
    // Roles
    // -----------

    #[tokio::test]
    async fn test_assign_and_remove_role() {
        let backend = MockStorageBackend::new();
        let user = UserId::new("user-alice".to_string()).unwrap();
        let tenant = TenantId::new("t1".to_string()).unwrap();

        backend
            .assign_role(&user, &tenant, "unit-1", Role::Developer.into())
            .await
            .unwrap();

        let key = "user-alice:t1:unit-1";
        assert!(
            backend
                .roles
                .get(key)
                .unwrap()
                .contains(&RoleIdentifier::from(Role::Developer))
        );

        backend
            .remove_role(&user, &tenant, "unit-1", Role::Developer.into())
            .await
            .unwrap();
        assert!(backend.roles.get(key).unwrap().is_empty());
    }

    // -----------
    // Policies
    // -----------

    #[tokio::test]
    async fn test_add_and_get_unit_policies() {
        use mk_core::types::{KnowledgeLayer, Policy, PolicyMode, RuleMergeStrategy};

        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("t");
        let policy = Policy {
            id: "p1".to_string(),
            name: "Test Policy".to_string(),
            description: None,
            layer: KnowledgeLayer::Team,
            mode: PolicyMode::Mandatory,
            merge_strategy: RuleMergeStrategy::Merge,
            rules: vec![],
            metadata: std::collections::HashMap::new(),
        };

        backend
            .add_unit_policy(&ctx, "unit-x", &policy)
            .await
            .unwrap();
        let policies = backend.get_unit_policies(ctx, "unit-x").await.unwrap();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].id, "p1");
    }

    // -----------
    // Idempotency
    // -----------

    #[tokio::test]
    async fn test_idempotency_first_call_returns_false_second_returns_true() {
        let backend = MockStorageBackend::new();
        let first = backend
            .check_idempotency("group-a", "event-xyz")
            .await
            .unwrap();
        let second = backend
            .check_idempotency("group-a", "event-xyz")
            .await
            .unwrap();
        assert!(!first, "First call should return false (not yet seen)");
        assert!(second, "Second call should return true (already processed)");
    }

    // -----------
    // Events
    // -----------

    #[tokio::test]
    async fn test_persist_and_get_pending_events() {
        use mk_core::types::GovernanceEvent;

        let backend = MockStorageBackend::new();
        let ctx = tenant_ctx("t");

        let event = PersistentEvent {
            id: "evt-1".to_string(),
            event_id: "evt-1".to_string(),
            event_type: "ConfigUpdated".to_string(),
            tenant_id: TenantId::new("t".to_string()).unwrap(),
            payload: GovernanceEvent::ConfigUpdated {
                config_id: "cfg-1".to_string(),
                scope: "tenant".to_string(),
                tenant_id: TenantId::new("t".to_string()).unwrap(),
                timestamp: 0,
            },
            status: EventStatus::Pending,
            created_at: 0,
            retry_count: 0,
            max_retries: 3,
            last_error: None,
            published_at: None,
            acknowledged_at: None,
            dead_lettered_at: None,
            idempotency_key: "key-1".to_string(),
        };

        backend.persist_event(event).await.unwrap();
        let pending = backend.get_pending_events(ctx, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "evt-1");
    }

    #[tokio::test]
    async fn test_update_event_status() {
        use mk_core::types::GovernanceEvent;

        let backend = MockStorageBackend::new();

        let event = PersistentEvent {
            id: "evt-2".to_string(),
            event_id: "evt-2".to_string(),
            event_type: "ConfigUpdated".to_string(),
            tenant_id: TenantId::new("t".to_string()).unwrap(),
            payload: GovernanceEvent::ConfigUpdated {
                config_id: "cfg-2".to_string(),
                scope: "org".to_string(),
                tenant_id: TenantId::new("t".to_string()).unwrap(),
                timestamp: 0,
            },
            status: EventStatus::Pending,
            created_at: 0,
            retry_count: 0,
            max_retries: 3,
            last_error: None,
            published_at: None,
            acknowledged_at: None,
            dead_lettered_at: None,
            idempotency_key: "key-2".to_string(),
        };

        backend.persist_event(event).await.unwrap();
        backend
            .update_event_status("evt-2", EventStatus::Acknowledged, None)
            .await
            .unwrap();

        let updated = backend.events.get("evt-2").unwrap();
        assert_eq!(updated.status, EventStatus::Acknowledged);
    }
}
