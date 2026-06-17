use async_trait::async_trait;
use mk_core::types::{
    DriftResult, GovernanceEvent, KnowledgeEntry, KnowledgeLayer, TenantContext, ValidationResult,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Debug, thiserror::Error)]
pub enum GovernanceClientError {
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Internal error: {0}")]
    Internal(String),
    #[error("Remote unavailable, using cached data")]
    RemoteUnavailable,
    #[error("Sync conflict: {0}")]
    SyncConflict(String),
    #[error("Governance error: {0}")]
    Governance(#[from] crate::governance::GovernanceError),
}

pub type Result<T> = std::result::Result<T, GovernanceClientError>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncState {
    pub last_sync_timestamp: i64,
    pub local_version: u64,
    pub remote_version: u64,
    pub pending_changes: Vec<PendingChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingChange {
    pub id: String,
    pub change_type: ChangeType,
    pub data: serde_json::Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ChangeType {
    PolicyUpdate,
    DriftResult,
    ProposalAction,
}

#[derive(Debug, Clone)]
struct CacheEntry<T> {
    data: T,
    inserted_at: Instant,
    ttl: Duration,
}

impl<T: Clone> CacheEntry<T> {
    fn new(data: T, ttl: Duration) -> Self {
        Self {
            data,
            inserted_at: Instant::now(),
            ttl,
        }
    }

    fn is_expired(&self) -> bool {
        self.inserted_at.elapsed() > self.ttl
    }
}

#[async_trait]
pub trait GovernanceClient: Send + Sync {
    async fn validate(
        &self,
        ctx: &TenantContext,
        layer: KnowledgeLayer,
        context: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ValidationResult>;

    async fn get_drift_status(
        &self,
        ctx: &TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftResult>>;

    async fn list_proposals(
        &self,
        ctx: &TenantContext,
        layer: Option<KnowledgeLayer>,
    ) -> Result<Vec<KnowledgeEntry>>;

    async fn replay_events(
        &self,
        ctx: &TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<GovernanceEvent>>;
}

pub struct RemoteGovernanceClient {
    client: reqwest::Client,
    base_url: String,
}

impl RemoteGovernanceClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url,
        }
    }
}

pub struct HybridGovernanceClient {
    remote_client: RemoteGovernanceClient,
    local_engine: Arc<crate::governance::GovernanceEngine>,
    cache: Arc<RwLock<HybridCache>>,
    sync_state: Arc<RwLock<SyncState>>,
    cache_ttl: Duration,
    sync_interval: Duration,
}

#[derive(Default)]
struct HybridCache {
    drift_results: HashMap<String, CacheEntry<DriftResult>>,
    proposals: Option<CacheEntry<Vec<KnowledgeEntry>>>,
}

impl HybridGovernanceClient {
    pub fn new(remote_url: String, local_engine: Arc<crate::governance::GovernanceEngine>) -> Self {
        Self {
            remote_client: RemoteGovernanceClient::new(remote_url),
            local_engine,
            cache: Arc::new(RwLock::new(HybridCache::default())),
            sync_state: Arc::new(RwLock::new(SyncState::default())),
            cache_ttl: Duration::from_mins(5),
            sync_interval: Duration::from_mins(1),
        }
    }

    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn with_sync_interval(mut self, interval: Duration) -> Self {
        self.sync_interval = interval;
        self
    }

    fn cache_key(ctx: &TenantContext, suffix: &str) -> String {
        format!(
            "{}:{}:{}",
            ctx.tenant_id.as_str(),
            ctx.user_id.as_str(),
            suffix
        )
    }

    pub async fn sync_pending_changes(&self, ctx: &TenantContext) -> Result<usize> {
        let mut state = self.sync_state.write().await;
        let pending = std::mem::take(&mut state.pending_changes);
        let mut synced = 0;

        for change in pending {
            match self.push_change_to_remote(ctx, &change).await {
                Ok(_) => {
                    synced += 1;
                    state.local_version += 1;
                }
                Err(e) => {
                    tracing::error!("Failed to sync change {}: {:?}", change.id, e);
                    state.pending_changes.push(change);
                }
            }
        }

        if synced > 0 {
            state.last_sync_timestamp = chrono::Utc::now().timestamp();
        }

        Ok(synced)
    }

    async fn push_change_to_remote(
        &self,
        ctx: &TenantContext,
        change: &PendingChange,
    ) -> Result<()> {
        let url = match change.change_type {
            ChangeType::PolicyUpdate => {
                format!(
                    "{}/api/v1/governance/policies/sync",
                    self.remote_client.base_url
                )
            }
            ChangeType::DriftResult => {
                format!(
                    "{}/api/v1/governance/drift/sync",
                    self.remote_client.base_url
                )
            }
            ChangeType::ProposalAction => {
                format!(
                    "{}/api/v1/governance/proposals/sync",
                    self.remote_client.base_url
                )
            }
        };

        let response = self
            .remote_client
            .client
            .post(&url)
            .header("X-Tenant-Id", ctx.tenant_id.as_str())
            .header("X-User-Id", ctx.user_id.as_str())
            .json(&change.data)
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            Err(GovernanceClientError::Api(response.text().await?))
        }
    }

    pub async fn queue_local_change(&self, change: PendingChange) {
        let mut state = self.sync_state.write().await;
        state.pending_changes.push(change);
    }

    pub async fn get_sync_state(&self) -> SyncState {
        self.sync_state.read().await.clone()
    }
}

#[async_trait]
impl GovernanceClient for HybridGovernanceClient {
    async fn validate(
        &self,
        ctx: &TenantContext,
        layer: KnowledgeLayer,
        context: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ValidationResult> {
        let local_result = self
            .local_engine
            .validate_with_context(layer, context, Some(ctx))
            .await?;

        self.queue_local_change(PendingChange {
            id: uuid::Uuid::new_v4().to_string(),
            change_type: ChangeType::PolicyUpdate,
            data: serde_json::json!({
                "layer": layer,
                "context": context,
                "result": local_result
            }),
            created_at: chrono::Utc::now().timestamp(),
        })
        .await;

        Ok(local_result)
    }

    async fn get_drift_status(
        &self,
        ctx: &TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftResult>> {
        let cache_key = Self::cache_key(ctx, &format!("drift:{}", project_id));

        {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.drift_results.get(&cache_key)
                && !entry.is_expired()
            {
                return Ok(Some(entry.data.clone()));
            }
        }

        match self.remote_client.get_drift_status(ctx, project_id).await {
            Ok(result) => {
                if let Some(ref drift) = result {
                    let mut cache = self.cache.write().await;
                    cache
                        .drift_results
                        .insert(cache_key, CacheEntry::new(drift.clone(), self.cache_ttl));
                }
                Ok(result)
            }
            Err(_) => {
                if let Some(storage) = self.local_engine.storage() {
                    match storage
                        .get_latest_drift_result(ctx.clone(), project_id)
                        .await
                    {
                        Ok(result) => Ok(result),
                        Err(e) => Err(GovernanceClientError::Internal(format!("{:?}", e))),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }

    async fn list_proposals(
        &self,
        ctx: &TenantContext,
        layer: Option<KnowledgeLayer>,
    ) -> Result<Vec<KnowledgeEntry>> {
        {
            let cache = self.cache.read().await;
            if let Some(ref entry) = cache.proposals
                && !entry.is_expired()
            {
                let filtered: Vec<_> = entry
                    .data
                    .iter()
                    .filter(|e| layer.is_none() || Some(e.layer) == layer)
                    .cloned()
                    .collect();
                return Ok(filtered);
            }
        }

        match self.remote_client.list_proposals(ctx, layer).await {
            Ok(proposals) => {
                let mut cache = self.cache.write().await;
                cache.proposals = Some(CacheEntry::new(proposals.clone(), self.cache_ttl));
                Ok(proposals)
            }
            Err(_) => {
                if let Some(repo) = self.local_engine.repository() {
                    let target_layer = layer.unwrap_or(KnowledgeLayer::Project);
                    match repo.list(ctx.clone(), target_layer, "proposals/").await {
                        Ok(entries) => Ok(entries),
                        Err(e) => Err(GovernanceClientError::Internal(format!("{:?}", e))),
                    }
                } else {
                    Ok(vec![])
                }
            }
        }
    }

    async fn replay_events(
        &self,
        ctx: &TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<GovernanceEvent>> {
        self.remote_client
            .replay_events(ctx, since_timestamp, limit)
            .await
    }
}

pub enum GovernanceClientKind {
    Local(LocalGovernanceClient),
    Hybrid(HybridGovernanceClient),
    Remote(RemoteGovernanceClient),
}

impl std::fmt::Debug for GovernanceClientKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GovernanceClientKind::Local(_) => f.debug_tuple("Local").finish(),
            GovernanceClientKind::Hybrid(_) => f.debug_tuple("Hybrid").finish(),
            GovernanceClientKind::Remote(_) => f.debug_tuple("Remote").finish(),
        }
    }
}

impl GovernanceClientKind {
    pub fn as_client(&self) -> &dyn GovernanceClient {
        match self {
            GovernanceClientKind::Local(c) => c,
            GovernanceClientKind::Hybrid(c) => c,
            GovernanceClientKind::Remote(c) => c,
        }
    }
}

pub fn create_governance_client(
    config: &config::DeploymentConfig,
    engine: Option<Arc<crate::governance::GovernanceEngine>>,
) -> Result<GovernanceClientKind> {
    match config.mode.as_str() {
        "local" => {
            let engine = engine.ok_or_else(|| {
                GovernanceClientError::Internal(
                    "Local mode requires a GovernanceEngine instance".to_string(),
                )
            })?;
            Ok(GovernanceClientKind::Local(LocalGovernanceClient::new(
                engine,
            )))
        }
        "hybrid" => {
            let engine = engine.ok_or_else(|| {
                GovernanceClientError::Internal(
                    "Hybrid mode requires a GovernanceEngine instance".to_string(),
                )
            })?;
            let remote_url = config.remote_url.clone().ok_or_else(|| {
                GovernanceClientError::Internal(
                    "Hybrid mode requires a remote_url configuration".to_string(),
                )
            })?;
            Ok(GovernanceClientKind::Hybrid(HybridGovernanceClient::new(
                remote_url, engine,
            )))
        }
        "remote" => {
            let remote_url = config.remote_url.clone().ok_or_else(|| {
                GovernanceClientError::Internal(
                    "Remote mode requires a remote_url configuration".to_string(),
                )
            })?;
            Ok(GovernanceClientKind::Remote(RemoteGovernanceClient::new(
                remote_url,
            )))
        }
        other => Err(GovernanceClientError::Internal(format!(
            "Invalid deployment mode: {}",
            other
        ))),
    }
}

/// Local governance client that wraps the `GovernanceEngine` directly.
///
/// Used in "local" deployment mode where all governance operations are
/// performed locally without any remote communication.
pub struct LocalGovernanceClient {
    engine: Arc<crate::governance::GovernanceEngine>,
}

impl LocalGovernanceClient {
    pub fn new(engine: Arc<crate::governance::GovernanceEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl GovernanceClient for LocalGovernanceClient {
    async fn validate(
        &self,
        _ctx: &TenantContext,
        layer: KnowledgeLayer,
        context: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ValidationResult> {
        Ok(self
            .engine
            .validate_with_context(layer, context, None)
            .await?)
    }

    async fn get_drift_status(
        &self,
        ctx: &TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftResult>> {
        if let Some(storage) = self.engine.storage() {
            match storage
                .get_latest_drift_result(ctx.clone(), project_id)
                .await
            {
                Ok(result) => Ok(result),
                Err(e) => Err(GovernanceClientError::Internal(format!("{:?}", e))),
            }
        } else {
            Ok(None)
        }
    }

    async fn list_proposals(
        &self,
        ctx: &TenantContext,
        layer: Option<KnowledgeLayer>,
    ) -> Result<Vec<KnowledgeEntry>> {
        if let Some(repo) = self.engine.repository() {
            let target_layer = layer.unwrap_or(KnowledgeLayer::Project);
            match repo.list(ctx.clone(), target_layer, "proposals/").await {
                Ok(entries) => Ok(entries),
                Err(e) => Err(GovernanceClientError::Internal(format!("{:?}", e))),
            }
        } else {
            Ok(vec![])
        }
    }

    async fn replay_events(
        &self,
        ctx: &TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<GovernanceEvent>> {
        if let Some(storage) = self.engine.storage() {
            match storage
                .get_governance_events(ctx.clone(), since_timestamp, limit)
                .await
            {
                Ok(events) => Ok(events),
                Err(e) => Err(GovernanceClientError::Internal(format!("{:?}", e))),
            }
        } else {
            Ok(vec![])
        }
    }
}

#[async_trait]
impl GovernanceClient for RemoteGovernanceClient {
    async fn validate(
        &self,
        ctx: &TenantContext,
        layer: KnowledgeLayer,
        context: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ValidationResult> {
        let url = format!("{}/api/v1/governance/validate", self.base_url);
        let response = self
            .client
            .post(&url)
            .header("X-Tenant-Id", ctx.tenant_id.as_str())
            .header("X-User-Id", ctx.user_id.as_str())
            .json(&serde_json::json!({
                "layer": layer,
                "context": context
            }))
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Err(GovernanceClientError::Api(response.text().await?))
        }
    }

    async fn get_drift_status(
        &self,
        ctx: &TenantContext,
        project_id: &str,
    ) -> Result<Option<DriftResult>> {
        let url = format!("{}/api/v1/governance/drift/{}", self.base_url, project_id);
        let response = self
            .client
            .get(&url)
            .header("X-Tenant-Id", ctx.tenant_id.as_str())
            .header("X-User-Id", ctx.user_id.as_str())
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Err(GovernanceClientError::Api(response.text().await?))
        }
    }

    async fn list_proposals(
        &self,
        ctx: &TenantContext,
        layer: Option<KnowledgeLayer>,
    ) -> Result<Vec<KnowledgeEntry>> {
        let mut url = format!("{}/api/v1/governance/proposals", self.base_url);
        if let Some(l) = layer {
            url.push_str(&format!("?layer={:?}", l));
        }

        let response = self
            .client
            .get(&url)
            .header("X-Tenant-Id", ctx.tenant_id.as_str())
            .header("X-User-Id", ctx.user_id.as_str())
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Err(GovernanceClientError::Api(response.text().await?))
        }
    }

    async fn replay_events(
        &self,
        ctx: &TenantContext,
        since_timestamp: i64,
        limit: usize,
    ) -> Result<Vec<GovernanceEvent>> {
        let url = format!(
            "{}/api/v1/governance/events/replay?since_timestamp={}&limit={}",
            self.base_url, since_timestamp, limit
        );

        let response = self
            .client
            .get(&url)
            .header("X-Tenant-Id", ctx.tenant_id.as_str())
            .header("X-User-Id", ctx.user_id.as_str())
            .send()
            .await?;

        if response.status().is_success() {
            Ok(response.json().await?)
        } else {
            Err(GovernanceClientError::Api(response.text().await?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{TenantId, UserId};

    fn test_tenant_context() -> TenantContext {
        TenantContext::new(TenantId::default(), UserId::default())
    }

    #[test]
    fn test_sync_state_default() {
        let state = SyncState::default();
        assert_eq!(state.last_sync_timestamp, 0);
        assert_eq!(state.local_version, 0);
        assert_eq!(state.remote_version, 0);
        assert!(state.pending_changes.is_empty());
    }

    #[test]
    fn test_pending_change_serialization() {
        let change = PendingChange {
            id: "change-1".to_string(),
            change_type: ChangeType::PolicyUpdate,
            data: serde_json::json!({"key": "value"}),
            created_at: 1234567890,
        };

        let json = serde_json::to_string(&change).unwrap();
        let deserialized: PendingChange = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "change-1");
        assert_eq!(deserialized.change_type, ChangeType::PolicyUpdate);
        assert_eq!(deserialized.created_at, 1234567890);
    }

    #[test]
    fn test_change_type_variants() {
        assert_eq!(ChangeType::PolicyUpdate, ChangeType::PolicyUpdate);
        assert_eq!(ChangeType::DriftResult, ChangeType::DriftResult);
        assert_eq!(ChangeType::ProposalAction, ChangeType::ProposalAction);
        assert_ne!(ChangeType::PolicyUpdate, ChangeType::DriftResult);
    }

    #[test]
    fn test_cache_entry_expiration() {
        let entry = CacheEntry::new("test data".to_string(), Duration::from_millis(10));
        assert!(!entry.is_expired());

        std::thread::sleep(Duration::from_millis(15));
        assert!(entry.is_expired());
    }

    #[test]
    fn test_cache_entry_not_expired() {
        let entry = CacheEntry::new(42i32, Duration::from_mins(1));
        assert!(!entry.is_expired());
    }

    #[test]
    fn test_hybrid_cache_default() {
        let cache = HybridCache::default();
        assert!(cache.drift_results.is_empty());
        assert!(cache.proposals.is_none());
    }

    #[test]
    fn test_cache_key_generation() {
        let ctx = test_tenant_context();
        let key = HybridGovernanceClient::cache_key(&ctx, "drift:proj-1");
        assert_eq!(key, "default:default:drift:proj-1");
    }

    #[test]
    fn test_remote_client_construction() {
        let client = RemoteGovernanceClient::new("http://localhost:8080".to_string());
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_hybrid_client_construction() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client =
            HybridGovernanceClient::new("http://localhost:8080".to_string(), engine.clone());

        assert_eq!(client.cache_ttl, Duration::from_mins(5));
        assert_eq!(client.sync_interval, Duration::from_mins(1));
    }

    #[tokio::test]
    async fn test_hybrid_client_with_custom_ttl() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine)
            .with_cache_ttl(Duration::from_mins(2));

        assert_eq!(client.cache_ttl, Duration::from_mins(2));
    }

    #[tokio::test]
    async fn test_hybrid_client_with_custom_sync_interval() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine)
            .with_sync_interval(Duration::from_secs(30));

        assert_eq!(client.sync_interval, Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_queue_local_change() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        let change = PendingChange {
            id: "test-change".to_string(),
            change_type: ChangeType::PolicyUpdate,
            data: serde_json::json!({"test": true}),
            created_at: chrono::Utc::now().timestamp(),
        };

        client.queue_local_change(change).await;

        let state = client.get_sync_state().await;
        assert_eq!(state.pending_changes.len(), 1);
        assert_eq!(state.pending_changes[0].id, "test-change");
    }

    #[tokio::test]
    async fn test_get_sync_state_initial() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        let state = client.get_sync_state().await;
        assert_eq!(state.last_sync_timestamp, 0);
        assert_eq!(state.local_version, 0);
        assert_eq!(state.remote_version, 0);
        assert!(state.pending_changes.is_empty());
    }

    #[tokio::test]
    async fn test_hybrid_validate_queues_change() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        let ctx = test_tenant_context();
        let context = HashMap::new();

        let result = client
            .validate(&ctx, KnowledgeLayer::Project, &context)
            .await;
        assert!(result.is_ok());

        let state = client.get_sync_state().await;
        assert_eq!(state.pending_changes.len(), 1);
        assert_eq!(
            state.pending_changes[0].change_type,
            ChangeType::PolicyUpdate
        );
    }

    #[tokio::test]
    async fn test_hybrid_get_drift_status_no_storage() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://invalid-host:9999".to_string(), engine);

        let ctx = test_tenant_context();
        let result = client.get_drift_status(&ctx, "proj-1").await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_hybrid_list_proposals_no_repo() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://invalid-host:9999".to_string(), engine);

        let ctx = test_tenant_context();
        let result = client.list_proposals(&ctx, None).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_governance_client_error_display() {
        let err = GovernanceClientError::Api("Not found".to_string());
        assert_eq!(err.to_string(), "API error: Not found");

        let err = GovernanceClientError::Internal("Something went wrong".to_string());
        assert_eq!(err.to_string(), "Internal error: Something went wrong");

        let err = GovernanceClientError::RemoteUnavailable;
        assert_eq!(err.to_string(), "Remote unavailable, using cached data");

        let err = GovernanceClientError::SyncConflict("Version mismatch".to_string());
        assert_eq!(err.to_string(), "Sync conflict: Version mismatch");
    }

    #[test]
    fn test_sync_state_serialization() {
        let state = SyncState {
            last_sync_timestamp: 1234567890,
            local_version: 5,
            remote_version: 3,
            pending_changes: vec![PendingChange {
                id: "change-1".to_string(),
                change_type: ChangeType::DriftResult,
                data: serde_json::json!({}),
                created_at: 1234567890,
            }],
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SyncState = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.last_sync_timestamp, 1234567890);
        assert_eq!(deserialized.local_version, 5);
        assert_eq!(deserialized.remote_version, 3);
        assert_eq!(deserialized.pending_changes.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_queued_changes() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        for i in 0..5 {
            client
                .queue_local_change(PendingChange {
                    id: format!("change-{}", i),
                    change_type: ChangeType::PolicyUpdate,
                    data: serde_json::json!({"index": i}),
                    created_at: chrono::Utc::now().timestamp(),
                })
                .await;
        }

        let state = client.get_sync_state().await;
        assert_eq!(state.pending_changes.len(), 5);
    }

    #[tokio::test]
    async fn test_sync_pending_changes_empty() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        let ctx = test_tenant_context();
        let synced = client.sync_pending_changes(&ctx).await.unwrap();

        assert_eq!(synced, 0);
    }

    #[tokio::test]
    async fn test_cache_key_with_custom_context() {
        let tenant_id = TenantId::new("acme-corp".to_string()).unwrap();
        let user_id = UserId::new("john-doe".to_string()).unwrap();
        let ctx = TenantContext::new(tenant_id, user_id);

        let key = HybridGovernanceClient::cache_key(&ctx, "proposals");
        assert_eq!(key, "acme-corp:john-doe:proposals");
    }

    #[test]
    fn test_change_type_serialization() {
        let policy = ChangeType::PolicyUpdate;
        let drift = ChangeType::DriftResult;
        let proposal = ChangeType::ProposalAction;

        let policy_json = serde_json::to_string(&policy).unwrap();
        let drift_json = serde_json::to_string(&drift).unwrap();
        let proposal_json = serde_json::to_string(&proposal).unwrap();

        assert_eq!(policy_json, "\"PolicyUpdate\"");
        assert_eq!(drift_json, "\"DriftResult\"");
        assert_eq!(proposal_json, "\"ProposalAction\"");

        let deserialized: ChangeType = serde_json::from_str(&policy_json).unwrap();
        assert_eq!(deserialized, ChangeType::PolicyUpdate);
    }

    #[tokio::test]
    async fn test_hybrid_client_builder_chain() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine)
            .with_cache_ttl(Duration::from_mins(10))
            .with_sync_interval(Duration::from_mins(2));

        assert_eq!(client.cache_ttl, Duration::from_mins(10));
        assert_eq!(client.sync_interval, Duration::from_mins(2));
    }

    #[tokio::test]
    async fn test_hybrid_validate_returns_valid_result() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://localhost:8080".to_string(), engine);

        let ctx = test_tenant_context();
        let context = HashMap::new();

        let result = client
            .validate(&ctx, KnowledgeLayer::Project, &context)
            .await
            .unwrap();

        assert!(result.is_valid);
        assert!(result.violations.is_empty());
    }

    #[tokio::test]
    async fn test_hybrid_list_proposals_with_layer_filter() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = HybridGovernanceClient::new("http://invalid-host:9999".to_string(), engine);

        let ctx = test_tenant_context();
        let result = client
            .list_proposals(&ctx, Some(KnowledgeLayer::Tenant))
            .await;

        assert!(result.is_ok());
    }

    #[test]
    fn test_local_client_construction() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let _client = LocalGovernanceClient::new(engine);
    }

    #[tokio::test]
    async fn test_local_client_validate() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = LocalGovernanceClient::new(engine);

        let ctx = test_tenant_context();
        let context = HashMap::new();

        let result = client
            .validate(&ctx, KnowledgeLayer::Project, &context)
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_valid);
    }

    #[tokio::test]
    async fn test_local_client_get_drift_status_no_storage() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = LocalGovernanceClient::new(engine);

        let ctx = test_tenant_context();
        let result = client.get_drift_status(&ctx, "proj-1").await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_local_client_list_proposals_no_repo() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = LocalGovernanceClient::new(engine);

        let ctx = test_tenant_context();
        let result = client.list_proposals(&ctx, None).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_local_client_replay_events_no_storage() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client = LocalGovernanceClient::new(engine);

        let ctx = test_tenant_context();
        let result = client.replay_events(&ctx, 0, 100).await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_create_governance_client_local_mode() {
        let config = config::DeploymentConfig {
            mode: "local".to_string(),
            remote_url: None,
            sync_enabled: true,
        };
        let engine = Arc::new(crate::governance::GovernanceEngine::new());

        let result = create_governance_client(&config, Some(engine));
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), GovernanceClientKind::Local(_)));
    }

    #[test]
    fn test_create_governance_client_local_mode_requires_engine() {
        let config = config::DeploymentConfig {
            mode: "local".to_string(),
            remote_url: None,
            sync_enabled: true,
        };

        let result = create_governance_client(&config, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Local mode requires")
        );
    }

    #[test]
    fn test_create_governance_client_hybrid_mode() {
        let config = config::DeploymentConfig {
            mode: "hybrid".to_string(),
            remote_url: Some("http://localhost:8080".to_string()),
            sync_enabled: true,
        };
        let engine = Arc::new(crate::governance::GovernanceEngine::new());

        let result = create_governance_client(&config, Some(engine));
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), GovernanceClientKind::Hybrid(_)));
    }

    #[test]
    fn test_create_governance_client_hybrid_mode_requires_engine() {
        let config = config::DeploymentConfig {
            mode: "hybrid".to_string(),
            remote_url: Some("http://localhost:8080".to_string()),
            sync_enabled: true,
        };

        let result = create_governance_client(&config, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Hybrid mode requires a GovernanceEngine")
        );
    }

    #[test]
    fn test_create_governance_client_hybrid_mode_requires_url() {
        let config = config::DeploymentConfig {
            mode: "hybrid".to_string(),
            remote_url: None,
            sync_enabled: true,
        };
        let engine = Arc::new(crate::governance::GovernanceEngine::new());

        let result = create_governance_client(&config, Some(engine));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Hybrid mode requires a remote_url")
        );
    }

    #[test]
    fn test_create_governance_client_remote_mode() {
        let config = config::DeploymentConfig {
            mode: "remote".to_string(),
            remote_url: Some("http://localhost:8080".to_string()),
            sync_enabled: false,
        };

        let result = create_governance_client(&config, None);
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), GovernanceClientKind::Remote(_)));
    }

    #[test]
    fn test_create_governance_client_remote_mode_requires_url() {
        let config = config::DeploymentConfig {
            mode: "remote".to_string(),
            remote_url: None,
            sync_enabled: false,
        };

        let result = create_governance_client(&config, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Remote mode requires a remote_url")
        );
    }

    #[test]
    fn test_create_governance_client_invalid_mode() {
        let config = config::DeploymentConfig {
            mode: "invalid".to_string(),
            remote_url: None,
            sync_enabled: true,
        };

        let result = create_governance_client(&config, None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid deployment mode")
        );
    }

    #[test]
    fn test_governance_client_kind_as_client_local() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client_kind = GovernanceClientKind::Local(LocalGovernanceClient::new(engine));
        let _client: &dyn GovernanceClient = client_kind.as_client();
    }

    #[test]
    fn test_governance_client_kind_as_client_hybrid() {
        let engine = Arc::new(crate::governance::GovernanceEngine::new());
        let client_kind = GovernanceClientKind::Hybrid(HybridGovernanceClient::new(
            "http://localhost:8080".to_string(),
            engine,
        ));
        let _client: &dyn GovernanceClient = client_kind.as_client();
    }

    #[test]
    fn test_governance_client_kind_as_client_remote() {
        let client_kind = GovernanceClientKind::Remote(RemoteGovernanceClient::new(
            "http://localhost:8080".to_string(),
        ));
        let _client: &dyn GovernanceClient = client_kind.as_client();
    }
}
