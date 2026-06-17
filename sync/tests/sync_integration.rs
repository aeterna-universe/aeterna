use async_trait::async_trait;
use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use knowledge::repository::GitRepository;
use memory::manager::MemoryManager;
use memory::providers::MockProvider;
use mk_core::traits::{KnowledgeRepository, MemoryProviderAdapter, StorageBackend};
use mk_core::types::{
    KnowledgeEntry, KnowledgeLayer, KnowledgeStatus, KnowledgeType, MemoryLayer, TenantContext,
    TenantId,
};
use std::collections::HashMap;
use std::sync::Arc;
use sync::bridge::SyncManager;
use sync::state::SyncState;
use sync::state_persister::SyncStatePersister;
use tokio::sync::RwLock;

struct MockStorage {
    data: Arc<RwLock<HashMap<String, Vec<u8>>>>,
}

impl MockStorage {
    fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl StorageBackend for MockStorage {
    type Error = std::io::Error;

    async fn store(&self, _ctx: TenantContext, key: &str, value: &[u8]) -> Result<(), Self::Error> {
        self.data
            .write()
            .await
            .insert(key.to_string(), value.to_vec());
        Ok(())
    }

    async fn retrieve(
        &self,
        _ctx: TenantContext,
        key: &str,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(self.data.read().await.get(key).cloned())
    }

    async fn delete(&self, _ctx: TenantContext, key: &str) -> Result<(), Self::Error> {
        self.data.write().await.remove(key);
        Ok(())
    }

    async fn exists(&self, _ctx: TenantContext, key: &str) -> Result<bool, Self::Error> {
        Ok(self.data.read().await.contains_key(key))
    }

    async fn get_ancestors(
        &self,
        _ctx: TenantContext,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(Vec::new())
    }

    async fn get_descendants(
        &self,
        _ctx: TenantContext,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(Vec::new())
    }

    async fn get_unit_policies(
        &self,
        _ctx: TenantContext,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::Policy>, Self::Error> {
        Ok(Vec::new())
    }

    async fn create_unit(
        &self,
        _unit: &mk_core::types::OrganizationalUnit,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn add_unit_policy(
        &self,
        _ctx: &TenantContext,
        _unit_id: &str,
        _policy: &mk_core::types::Policy,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn assign_role(
        &self,
        _user_id: &mk_core::types::UserId,
        _tenant_id: &mk_core::types::TenantId,
        _unit_id: &str,
        _role: mk_core::types::RoleIdentifier,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn remove_role(
        &self,
        _user_id: &mk_core::types::UserId,
        _tenant_id: &mk_core::types::TenantId,
        _unit_id: &str,
        _role: mk_core::types::RoleIdentifier,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn store_drift_result(
        &self,
        _result: mk_core::types::DriftResult,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_latest_drift_result(
        &self,
        _ctx: TenantContext,
        _project_id: &str,
    ) -> Result<Option<mk_core::types::DriftResult>, Self::Error> {
        Ok(None)
    }

    async fn list_all_units(&self) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(Vec::new())
    }

    async fn record_job_status(
        &self,
        _job_name: &str,
        _tenant_id: &str,
        _status: &str,
        _message: Option<&str>,
        _started_at: i64,
        _finished_at: Option<i64>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_governance_events(
        &self,
        _ctx: TenantContext,
        _since_timestamp: i64,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::GovernanceEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn create_suppression(
        &self,
        _suppression: mk_core::types::DriftSuppression,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_suppressions(
        &self,
        _ctx: TenantContext,
        _project_id: &str,
    ) -> Result<Vec<mk_core::types::DriftSuppression>, Self::Error> {
        Ok(Vec::new())
    }

    async fn delete_suppression(
        &self,
        _ctx: TenantContext,
        _suppression_id: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_drift_config(
        &self,
        _ctx: TenantContext,
        _project_id: &str,
    ) -> Result<Option<mk_core::types::DriftConfig>, Self::Error> {
        Ok(None)
    }

    async fn save_drift_config(
        &self,
        _config: mk_core::types::DriftConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_event(
        &self,
        _event: mk_core::types::PersistentEvent,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_pending_events(
        &self,
        _ctx: TenantContext,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn update_event_status(
        &self,
        _event_id: &str,
        _status: mk_core::types::EventStatus,
        _error: Option<String>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_dead_letter_events(
        &self,
        _ctx: TenantContext,
        _limit: usize,
    ) -> Result<Vec<mk_core::types::PersistentEvent>, Self::Error> {
        Ok(Vec::new())
    }

    async fn check_idempotency(
        &self,
        _consumer_group: &str,
        _idempotency_key: &str,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn record_consumer_state(
        &self,
        _state: mk_core::types::ConsumerState,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_event_metrics(
        &self,
        _ctx: TenantContext,
        _period_start: i64,
        _period_end: i64,
    ) -> Result<Vec<mk_core::types::EventDeliveryMetrics>, Self::Error> {
        Ok(Vec::new())
    }

    async fn record_event_metrics(
        &self,
        _metrics: mk_core::types::EventDeliveryMetrics,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_unit_by_id(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Option<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(None)
    }

    async fn update_unit(
        &self,
        _unit: &mk_core::types::OrganizationalUnit,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn delete_unit(&self, _unit_id: &str, _tenant_id: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn list_unit_members(
        &self,
        _unit_id: &str,
        _tenant_id: &str,
    ) -> Result<Vec<(mk_core::types::UserId, mk_core::types::RoleIdentifier)>, Self::Error> {
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
        _user_id: &mk_core::types::UserId,
        _tenant_id: &mk_core::types::TenantId,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::RoleIdentifier>, Self::Error> {
        Ok(Vec::new())
    }
}

pub struct SimplePersister {
    storage: Arc<MockStorage>,
}

#[async_trait]
impl SyncStatePersister for SimplePersister {
    async fn load(
        &self,
        tenant_id: &TenantId,
    ) -> Result<SyncState, Box<dyn std::error::Error + Send + Sync>> {
        let ctx = TenantContext::new(tenant_id.clone(), mk_core::types::UserId::default());
        match self.storage.retrieve(ctx, "sync_state").await? {
            Some(data) => Ok(serde_json::from_slice(&data)?),
            None => Ok(SyncState::default()),
        }
    }

    async fn save(
        &self,
        tenant_id: &TenantId,
        state: &SyncState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ctx = TenantContext::new(tenant_id.clone(), mk_core::types::UserId::default());
        let data = serde_json::to_vec(state)?;
        self.storage.store(ctx, "sync_state", &data).await?;
        Ok(())
    }
}

#[tokio::test]
async fn test_sync_persistence_and_delta() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let repo_dir = tempfile::tempdir()?;
    let knowledge_repo = Arc::new(GitRepository::new(repo_dir.path())?);
    let governance_engine = Arc::new(GovernanceEngine::new());

    let memory_manager = Arc::new(MemoryManager::new());
    let mock_provider = MockProvider::new();
    let provider: Arc<
        dyn MemoryProviderAdapter<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    > = Arc::new(mock_provider);
    memory_manager
        .register_provider(MemoryLayer::Project, provider)
        .await;

    let storage = Arc::new(MockStorage::new());
    let persister = Arc::new(SimplePersister {
        storage: storage.clone(),
    });

    let sync_manager = SyncManager::new(
        memory_manager.clone(),
        Arc::new(KnowledgeManager::new(
            knowledge_repo.clone(),
            governance_engine.clone(),
        )),
        config::DeploymentConfig::default(),
        None,
        persister.clone(),
        None,
    )
    .await?;

    let entry = KnowledgeEntry {
        path: "test.md".to_string(),
        content: "initial content".to_string(),
        layer: KnowledgeLayer::Project,
        kind: KnowledgeType::Spec,
        status: KnowledgeStatus::Accepted,
        metadata: HashMap::new(),
        summaries: HashMap::new(),
        commit_hash: None,
        author: None,
        updated_at: chrono::Utc::now().timestamp(),
    };
    knowledge_repo
        .store(TenantContext::default(), entry.clone(), "first commit")
        .await?;

    sync_manager.sync_all(TenantContext::default()).await?;

    let ctx = TenantContext::default();
    assert!(storage.exists(ctx, "sync_state").await?);
    let tenant_id = TenantId::default();
    let state = persister.load(&tenant_id).await?;
    assert_eq!(state.stats.total_items_synced, 1);
    assert!(state.last_knowledge_commit.is_some());

    let updated_entry = KnowledgeEntry {
        content: "updated content".to_string(),
        ..entry.clone()
    };
    knowledge_repo
        .store(TenantContext::default(), updated_entry, "second commit")
        .await?;

    sync_manager
        .sync_incremental(TenantContext::default())
        .await?;

    let state = persister.load(&tenant_id).await?;
    assert_eq!(state.stats.total_syncs, 2);

    sync_manager
        .run_sync_cycle(TenantContext::default(), 0)
        .await?;

    let _ = sync_manager
        .detect_conflicts(TenantContext::default())
        .await?;

    let memory_id = format!("ptr_{}", entry.path);
    memory_manager
        .delete_from_layer(TenantContext::default(), MemoryLayer::Project, &memory_id)
        .await?;

    let conflicts = sync_manager
        .detect_conflicts(TenantContext::default())
        .await?;
    assert_eq!(conflicts.len(), 1);

    sync_manager
        .resolve_conflicts(TenantContext::default(), conflicts)
        .await?;

    let conflicts = sync_manager
        .detect_conflicts(TenantContext::default())
        .await?;
    if !conflicts.is_empty() {
        println!("Final conflicts: {:?}", conflicts);
    }
    assert!(conflicts.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_background_sync_trigger() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let repo_dir = tempfile::tempdir()?;
    let knowledge_repo = Arc::new(GitRepository::new(repo_dir.path())?);
    let governance_engine = Arc::new(GovernanceEngine::new());
    let memory_manager = Arc::new(MemoryManager::new());
    let mock_provider = MockProvider::new();
    let provider: Arc<
        dyn MemoryProviderAdapter<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    > = Arc::new(mock_provider);
    memory_manager
        .register_provider(MemoryLayer::Project, provider)
        .await;

    let storage = Arc::new(MockStorage::new());
    let persister = Arc::new(SimplePersister {
        storage: storage.clone(),
    });

    let sync_manager = Arc::new(
        SyncManager::new(
            memory_manager.clone(),
            Arc::new(KnowledgeManager::new(
                knowledge_repo.clone(),
                governance_engine.clone(),
            )),
            config::DeploymentConfig::default(),
            None,
            persister.clone(),
            None,
        )
        .await?,
    );

    let entry = KnowledgeEntry {
        path: "bg_test.md".to_string(),
        content: "initial content".to_string(),
        layer: KnowledgeLayer::Project,
        kind: KnowledgeType::Spec,
        status: KnowledgeStatus::Accepted,
        metadata: HashMap::new(),
        summaries: HashMap::new(),
        commit_hash: None,
        author: None,
        updated_at: chrono::Utc::now().timestamp(),
    };
    knowledge_repo
        .store(TenantContext::default(), entry.clone(), "first commit")
        .await?;

    let (_tx, rx) = tokio::sync::watch::channel(false);
    let handle = sync_manager
        .clone()
        .start_background_sync(TenantContext::default(), 1, 0, rx)
        .await;

    let mut synced = false;
    let tenant_id = TenantId::default();
    for _ in 0..10 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let state = persister.load(&tenant_id).await?;
        if state.stats.total_items_synced > 0 {
            synced = true;
            break;
        }
    }

    handle.abort();
    assert!(synced, "Background sync should have picked up the change");

    Ok(())
}

#[tokio::test]
async fn test_governance_blocking_sync() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let repo_dir = tempfile::tempdir()?;
    let knowledge_repo = Arc::new(GitRepository::new(repo_dir.path())?);
    let mut governance_engine = GovernanceEngine::new();

    governance_engine.add_policy(mk_core::types::Policy {
        id: "p1".to_string(),
        name: "No Secrets".to_string(),
        description: None,
        layer: KnowledgeLayer::Tenant,
        mode: mk_core::types::PolicyMode::Mandatory,
        merge_strategy: mk_core::types::RuleMergeStrategy::Override,
        rules: vec![mk_core::types::PolicyRule {
            id: "r1".to_string(),
            rule_type: mk_core::types::RuleType::Allow,
            target: mk_core::types::ConstraintTarget::Code,
            operator: mk_core::types::ConstraintOperator::MustNotMatch,
            value: serde_json::json!("SECRET"),
            severity: mk_core::types::ConstraintSeverity::Block,
            message: "No secrets allowed".to_string(),
        }],
        metadata: HashMap::new(),
    });

    let memory_manager = Arc::new(MemoryManager::new());
    let mock_provider = MockProvider::new();
    let provider: Arc<
        dyn MemoryProviderAdapter<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
    > = Arc::new(mock_provider);
    memory_manager
        .register_provider(MemoryLayer::Project, provider)
        .await;

    let storage = Arc::new(MockStorage::new());
    let persister = Arc::new(SimplePersister {
        storage: storage.clone(),
    });

    let sync_manager = SyncManager::new(
        memory_manager,
        Arc::new(KnowledgeManager::new(
            knowledge_repo.clone(),
            Arc::new(governance_engine),
        )),
        config::DeploymentConfig::default(),
        None,
        persister.clone(),
        None,
    )
    .await?;

    let entry = KnowledgeEntry {
        path: "secret.md".to_string(),
        content: "My SECRET is 12345".to_string(),
        layer: KnowledgeLayer::Project,
        kind: KnowledgeType::Spec,
        status: KnowledgeStatus::Accepted,
        metadata: HashMap::new(),
        summaries: HashMap::new(),
        commit_hash: None,
        author: None,
        updated_at: chrono::Utc::now().timestamp(),
    };
    knowledge_repo
        .store(TenantContext::default(), entry, "secret commit")
        .await?;

    sync_manager.sync_all(TenantContext::default()).await?;

    let tenant_id = TenantId::default();
    let state = persister.load(&tenant_id).await?;
    assert_eq!(state.stats.total_items_synced, 0);
    assert!(state.stats.total_governance_blocks > 0);

    Ok(())
}
