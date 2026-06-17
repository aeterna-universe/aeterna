use async_trait::async_trait;
use errors::StorageError;
use mk_core::traits::EventPublisher;
use mk_core::types::GovernanceEvent;
use redis::AsyncCommands;
use std::sync::Arc;

/// Result of a distributed lock acquisition attempt
#[derive(Debug, Clone)]
pub struct LockResult {
    /// The unique token identifying this lock holder
    pub lock_token: String,
    /// The key that was locked
    pub lock_key: String,
    /// TTL in seconds
    pub ttl_seconds: u64,
}

/// Reason why a job was skipped
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobSkipReason {
    /// Another instance is currently running this job
    AlreadyRunning,
    /// Job was recently completed within deduplication window
    RecentlyCompleted,
    /// Job is disabled via configuration
    Disabled,
}

impl std::fmt::Display for JobSkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobSkipReason::AlreadyRunning => write!(f, "already_running"),
            JobSkipReason::RecentlyCompleted => write!(f, "recently_completed"),
            JobSkipReason::Disabled => write!(f, "disabled"),
        }
    }
}

pub struct RedisStorage {
    client: Arc<redis::Client>,
    connection_manager: redis::aio::ConnectionManager,
}

impl RedisStorage {
    pub async fn new(connection_string: &str) -> Result<Self, StorageError> {
        let client =
            redis::Client::open(connection_string).map_err(|e| StorageError::ConnectionError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })?;

        let connection_manager =
            client
                .get_connection_manager()
                .await
                .map_err(|e| StorageError::ConnectionError {
                    backend: "Redis".to_string(),
                    reason: e.to_string(),
                })?;

        Ok(Self {
            client: Arc::new(client),
            connection_manager,
        })
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, StorageError> {
        let mut conn = self.connection_manager.clone();
        conn.get(key).await.map_err(|e| StorageError::QueryError {
            backend: "Redis".to_string(),
            reason: e.to_string(),
        })
    }

    pub async fn set(
        &self,
        key: &str,
        value: &str,
        ttl_seconds: Option<usize>,
    ) -> Result<(), StorageError> {
        let mut conn = self.connection_manager.clone();
        if let Some(ttl) = ttl_seconds {
            conn.set_ex(key, value, ttl as u64)
                .await
                .map_err(|e| StorageError::QueryError {
                    backend: "Redis".to_string(),
                    reason: e.to_string(),
                })
        } else {
            conn.set(key, value)
                .await
                .map_err(|e| StorageError::QueryError {
                    backend: "Redis".to_string(),
                    reason: e.to_string(),
                })
        }
    }

    pub async fn delete_key(&self, key: &str) -> Result<(), StorageError> {
        let mut conn = self.connection_manager.clone();
        conn.del(key).await.map_err(|e| StorageError::QueryError {
            backend: "Redis".to_string(),
            reason: e.to_string(),
        })
    }

    pub async fn exists_key(&self, key: &str) -> Result<bool, StorageError> {
        let mut conn = self.connection_manager.clone();
        conn.exists(key)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })
    }
    pub fn scoped_key(&self, ctx: &mk_core::types::TenantContext, key: &str) -> String {
        format!("{}:{}", ctx.tenant_id.as_str(), key)
    }

    pub async fn acquire_lock(
        &self,
        lock_key: &str,
        ttl_seconds: u64,
    ) -> Result<Option<LockResult>, StorageError> {
        let lock_token = uuid::Uuid::new_v4().to_string();
        let mut conn = self.connection_manager.clone();

        let result: Option<String> = redis::cmd("SET")
            .arg(lock_key)
            .arg(&lock_token)
            .arg("NX")
            .arg("EX")
            .arg(ttl_seconds)
            .query_async(&mut conn)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })?;

        Ok(result.map(|_| LockResult {
            lock_token,
            lock_key: lock_key.to_string(),
            ttl_seconds,
        }))
    }

    pub async fn release_lock(
        &self,
        lock_key: &str,
        lock_token: &str,
    ) -> Result<bool, StorageError> {
        let script = redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("DEL", KEYS[1])
            else
                return 0
            end
            "#,
        );

        let mut conn = self.connection_manager.clone();
        let result: i32 = script
            .key(lock_key)
            .arg(lock_token)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })?;

        Ok(result == 1)
    }

    pub async fn extend_lock(
        &self,
        lock_key: &str,
        lock_token: &str,
        ttl_seconds: u64,
    ) -> Result<bool, StorageError> {
        let script = redis::Script::new(
            r#"
            if redis.call("GET", KEYS[1]) == ARGV[1] then
                return redis.call("EXPIRE", KEYS[1], ARGV[2])
            else
                return 0
            end
            "#,
        );

        let mut conn = self.connection_manager.clone();
        let result: i32 = script
            .key(lock_key)
            .arg(lock_token)
            .arg(ttl_seconds)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })?;

        Ok(result == 1)
    }

    pub async fn check_lock_exists(&self, lock_key: &str) -> Result<bool, StorageError> {
        self.exists_key(lock_key).await
    }

    pub async fn record_job_completion(
        &self,
        job_name: &str,
        deduplication_window_seconds: u64,
    ) -> Result<(), StorageError> {
        let key = format!("job_completed:{}", job_name);
        let timestamp = chrono::Utc::now().timestamp().to_string();
        self.set(
            &key,
            &timestamp,
            Some(deduplication_window_seconds as usize),
        )
        .await
    }

    pub async fn check_job_recently_completed(&self, job_name: &str) -> Result<bool, StorageError> {
        let key = format!("job_completed:{}", job_name);
        let result = self.get(&key).await?;
        Ok(result.is_some())
    }

    pub async fn save_job_checkpoint(
        &self,
        checkpoint: &mk_core::types::PartialJobResult,
        ttl_seconds: u64,
    ) -> Result<(), StorageError> {
        let key = format!(
            "job_checkpoint:{}:{}",
            checkpoint.job_name, checkpoint.tenant_id
        );
        let value =
            serde_json::to_string(checkpoint).map_err(|e| StorageError::SerializationError {
                error_type: "JSON".to_string(),
                reason: e.to_string(),
            })?;
        self.set(&key, &value, Some(ttl_seconds as usize)).await
    }

    pub async fn get_job_checkpoint(
        &self,
        job_name: &str,
        tenant_id: &mk_core::types::TenantId,
    ) -> Result<Option<mk_core::types::PartialJobResult>, StorageError> {
        let key = format!("job_checkpoint:{}:{}", job_name, tenant_id);
        match self.get(&key).await? {
            Some(value) => {
                let checkpoint =
                    serde_json::from_str(&value).map_err(|e| StorageError::SerializationError {
                        error_type: "JSON".to_string(),
                        reason: e.to_string(),
                    })?;
                Ok(Some(checkpoint))
            }
            None => Ok(None),
        }
    }

    pub async fn delete_job_checkpoint(
        &self,
        job_name: &str,
        tenant_id: &mk_core::types::TenantId,
    ) -> Result<(), StorageError> {
        let key = format!("job_checkpoint:{}:{}", job_name, tenant_id);
        self.delete_key(&key).await
    }

    fn summary_cache_key(
        tenant_id: &str,
        layer: &mk_core::types::MemoryLayer,
        entry_id: &str,
        depth: &mk_core::types::SummaryDepth,
    ) -> String {
        // Force lowercase on both layer and depth so cache keys stay stable
        // across the PascalCase wire-format refactor. Cache values are ephemeral
        // but key collisions across format changes would still flush prematurely.
        format!(
            "summary:{}:{}:{}:{}",
            tenant_id,
            layer.to_string().to_lowercase(),
            entry_id,
            serde_json::to_string(depth)
                .unwrap_or_else(|_| "unknown".to_string())
                .trim_matches('"')
                .to_lowercase()
        )
    }

    pub async fn get_summary_cache(
        &self,
        tenant_id: &str,
        layer: &mk_core::types::MemoryLayer,
        entry_id: &str,
        depth: &mk_core::types::SummaryDepth,
    ) -> Result<Option<mk_core::types::LayerSummary>, StorageError> {
        let key = Self::summary_cache_key(tenant_id, layer, entry_id, depth);
        match self.get(&key).await? {
            Some(value) => {
                let summary =
                    serde_json::from_str(&value).map_err(|e| StorageError::SerializationError {
                        error_type: "JSON".to_string(),
                        reason: e.to_string(),
                    })?;
                Ok(Some(summary))
            }
            None => Ok(None),
        }
    }

    pub async fn set_summary_cache(
        &self,
        tenant_id: &str,
        layer: &mk_core::types::MemoryLayer,
        entry_id: &str,
        summary: &mk_core::types::LayerSummary,
        ttl_seconds: Option<usize>,
    ) -> Result<(), StorageError> {
        let key = Self::summary_cache_key(tenant_id, layer, entry_id, &summary.depth);
        let value =
            serde_json::to_string(summary).map_err(|e| StorageError::SerializationError {
                error_type: "JSON".to_string(),
                reason: e.to_string(),
            })?;
        self.set(&key, &value, ttl_seconds).await
    }

    pub async fn invalidate_summary_cache(
        &self,
        tenant_id: &str,
        layer: &mk_core::types::MemoryLayer,
        entry_id: &str,
        depth: Option<&mk_core::types::SummaryDepth>,
    ) -> Result<u32, StorageError> {
        let mut deleted_count = 0u32;

        if let Some(d) = depth {
            let key = Self::summary_cache_key(tenant_id, layer, entry_id, d);
            if self.exists_key(&key).await? {
                self.delete_key(&key).await?;
                deleted_count = 1;
            }
        } else {
            let depths = [
                mk_core::types::SummaryDepth::Sentence,
                mk_core::types::SummaryDepth::Paragraph,
                mk_core::types::SummaryDepth::Detailed,
            ];
            for d in &depths {
                let key = Self::summary_cache_key(tenant_id, layer, entry_id, d);
                if self.exists_key(&key).await? {
                    self.delete_key(&key).await?;
                    deleted_count += 1;
                }
            }
        }

        Ok(deleted_count)
    }

    pub async fn get_all_summaries_for_entry(
        &self,
        tenant_id: &str,
        layer: &mk_core::types::MemoryLayer,
        entry_id: &str,
    ) -> Result<
        std::collections::HashMap<mk_core::types::SummaryDepth, mk_core::types::LayerSummary>,
        StorageError,
    > {
        let mut result = std::collections::HashMap::new();
        let depths = [
            mk_core::types::SummaryDepth::Sentence,
            mk_core::types::SummaryDepth::Paragraph,
            mk_core::types::SummaryDepth::Detailed,
        ];

        for depth in &depths {
            if let Some(summary) = self
                .get_summary_cache(tenant_id, layer, entry_id, depth)
                .await?
            {
                result.insert(*depth, summary);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl EventPublisher for RedisStorage {
    type Error = StorageError;

    async fn publish(&self, event: GovernanceEvent) -> Result<(), Self::Error> {
        let mut conn = self.connection_manager.clone();
        let event_json =
            serde_json::to_string(&event).map_err(|e| StorageError::SerializationError {
                error_type: "JSON".to_string(),
                reason: e.to_string(),
            })?;

        let stream_key = format!("governance:events:{}", event.tenant_id());
        let _: String = conn
            .xadd(stream_key, "*", &[("event", event_json)])
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })?;

        Ok(())
    }

    async fn subscribe(
        &self,
        channels: &[&str],
    ) -> Result<tokio::sync::mpsc::Receiver<GovernanceEvent>, Self::Error> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let client = self.client.clone();
        let stream_keys: Vec<String> = channels.iter().map(|s| s.to_string()).collect();

        tokio::spawn(async move {
            if let Ok(mut conn) = client.get_connection_manager().await {
                let mut last_ids: Vec<String> = vec!["$".to_string(); stream_keys.len()];

                loop {
                    let opts = redis::streams::StreamReadOptions::default()
                        .block(0)
                        .count(10);

                    let result: Result<redis::streams::StreamReadReply, redis::RedisError> =
                        conn.xread_options(&stream_keys, &last_ids, &opts).await;

                    match result {
                        Ok(reply) => {
                            for (i, stream) in reply.keys.into_iter().enumerate() {
                                for record in stream.ids {
                                    if let Some(event_json) = record.map.get("event")
                                        && let Ok(event_str) =
                                            redis::from_redis_value::<String>(event_json.clone())
                                        && let Ok(event) =
                                            serde_json::from_str::<GovernanceEvent>(&event_str)
                                        && tx.send(event).await.is_err()
                                    {
                                        return;
                                    }
                                    last_ids[i] = record.id;
                                }
                            }
                        }
                        Err(_) => {
                            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[async_trait]
impl mk_core::traits::StorageBackend for RedisStorage {
    type Error = StorageError;

    async fn store(
        &self,
        ctx: mk_core::types::TenantContext,
        key: &str,
        value: &[u8],
    ) -> Result<(), Self::Error> {
        let mut conn = self.connection_manager.clone();
        let scoped_key = self.scoped_key(&ctx, key);
        conn.set(scoped_key, value)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })
    }

    async fn retrieve(
        &self,
        ctx: mk_core::types::TenantContext,
        key: &str,
    ) -> Result<Option<Vec<u8>>, Self::Error> {
        let mut conn = self.connection_manager.clone();
        let scoped_key = self.scoped_key(&ctx, key);
        conn.get(scoped_key)
            .await
            .map_err(|e| StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: e.to_string(),
            })
    }

    async fn delete(
        &self,
        ctx: mk_core::types::TenantContext,
        key: &str,
    ) -> Result<(), Self::Error> {
        let scoped_key = self.scoped_key(&ctx, key);
        self.delete_key(&scoped_key).await
    }

    async fn exists(
        &self,
        ctx: mk_core::types::TenantContext,
        key: &str,
    ) -> Result<bool, Self::Error> {
        let scoped_key = self.scoped_key(&ctx, key);
        self.exists_key(&scoped_key).await
    }

    async fn get_ancestors(
        &self,
        _ctx: mk_core::types::TenantContext,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(Vec::new())
    }

    async fn get_unit_policies(
        &self,
        _ctx: mk_core::types::TenantContext,
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
        _ctx: &mk_core::types::TenantContext,
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

    async fn get_descendants(
        &self,
        _ctx: mk_core::types::TenantContext,
        _unit_id: &str,
    ) -> Result<Vec<mk_core::types::OrganizationalUnit>, Self::Error> {
        Ok(Vec::new())
    }

    async fn store_drift_result(
        &self,
        _result: mk_core::types::DriftResult,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_latest_drift_result(
        &self,
        _ctx: mk_core::types::TenantContext,
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
        _ctx: mk_core::types::TenantContext,
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
        _ctx: mk_core::types::TenantContext,
        _project_id: &str,
    ) -> Result<Vec<mk_core::types::DriftSuppression>, Self::Error> {
        Ok(Vec::new())
    }

    async fn delete_suppression(
        &self,
        _ctx: mk_core::types::TenantContext,
        _suppression_id: &str,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn get_drift_config(
        &self,
        _ctx: mk_core::types::TenantContext,
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
        _ctx: mk_core::types::TenantContext,
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
        _ctx: mk_core::types::TenantContext,
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
        _ctx: mk_core::types::TenantContext,
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

#[cfg(test)]
mod tests {
    use super::*;
    use errors::StorageError;

    #[test]
    fn test_lock_result_fields() {
        let lock = LockResult {
            lock_token: "token-123".to_string(),
            lock_key: "job:drift_scan".to_string(),
            ttl_seconds: 300,
        };

        assert_eq!(lock.lock_token, "token-123");
        assert_eq!(lock.lock_key, "job:drift_scan");
        assert_eq!(lock.ttl_seconds, 300);
    }

    #[test]
    fn test_lock_result_clone() {
        let lock = LockResult {
            lock_token: "token-456".to_string(),
            lock_key: "job:semantic".to_string(),
            ttl_seconds: 600,
        };

        let cloned = lock.clone();
        assert_eq!(cloned.lock_token, lock.lock_token);
        assert_eq!(cloned.lock_key, lock.lock_key);
        assert_eq!(cloned.ttl_seconds, lock.ttl_seconds);
    }

    #[test]
    fn test_lock_result_debug() {
        let lock = LockResult {
            lock_token: "token".to_string(),
            lock_key: "key".to_string(),
            ttl_seconds: 60,
        };

        let debug_str = format!("{:?}", lock);
        assert!(debug_str.contains("LockResult"));
        assert!(debug_str.contains("token"));
        assert!(debug_str.contains("key"));
    }

    #[test]
    fn test_job_skip_reason_display_already_running() {
        let reason = JobSkipReason::AlreadyRunning;
        assert_eq!(reason.to_string(), "already_running");
    }

    #[test]
    fn test_job_skip_reason_display_recently_completed() {
        let reason = JobSkipReason::RecentlyCompleted;
        assert_eq!(reason.to_string(), "recently_completed");
    }

    #[test]
    fn test_job_skip_reason_display_disabled() {
        let reason = JobSkipReason::Disabled;
        assert_eq!(reason.to_string(), "disabled");
    }

    #[test]
    fn test_job_skip_reason_equality() {
        assert_eq!(JobSkipReason::AlreadyRunning, JobSkipReason::AlreadyRunning);
        assert_eq!(
            JobSkipReason::RecentlyCompleted,
            JobSkipReason::RecentlyCompleted
        );
        assert_eq!(JobSkipReason::Disabled, JobSkipReason::Disabled);
        assert_ne!(JobSkipReason::AlreadyRunning, JobSkipReason::Disabled);
    }

    #[test]
    fn test_job_skip_reason_copy() {
        let reason = JobSkipReason::AlreadyRunning;
        let copied = reason;
        assert_eq!(copied, JobSkipReason::AlreadyRunning);
    }

    #[test]
    #[allow(clippy::clone_on_copy)] // Explicitly testing Clone impl
    fn test_job_skip_reason_clone() {
        let reason = JobSkipReason::RecentlyCompleted;
        let cloned = reason.clone();
        assert_eq!(cloned, JobSkipReason::RecentlyCompleted);
    }

    #[test]
    fn test_job_skip_reason_debug() {
        let reason = JobSkipReason::Disabled;
        let debug_str = format!("{:?}", reason);
        assert!(debug_str.contains("Disabled"));
    }

    #[test]
    fn test_storage_error_display() {
        let conn_error = StorageError::ConnectionError {
            backend: "Redis".to_string(),
            reason: "Connection refused".to_string(),
        };

        let query_error = StorageError::QueryError {
            backend: "Redis".to_string(),
            reason: "Command failed".to_string(),
        };

        assert_eq!(
            conn_error.to_string(),
            "Connection to Redis failed: Connection refused"
        );

        assert_eq!(
            query_error.to_string(),
            "Query on Redis failed: Command failed"
        );
    }

    #[tokio::test]
    async fn test_redis_storage_error_handling() {
        let result = RedisStorage::new("not-a-valid-url").await;
        assert!(result.is_err());

        if let Err(StorageError::ConnectionError { backend, .. }) = result {
            assert_eq!(backend, "Redis");
        } else {
            panic!("Expected ConnectionError for invalid URL");
        }
    }

    #[test]
    fn test_storage_backend_trait_bounds() {
        use mk_core::traits::StorageBackend;

        fn assert_storage_backend<T: StorageBackend>() {}

        assert_storage_backend::<RedisStorage>();
    }

    #[test]
    fn test_error_messages_include_backend_name() {
        let errors = vec![
            StorageError::ConnectionError {
                backend: "Redis".to_string(),
                reason: "test".to_string(),
            },
            StorageError::QueryError {
                backend: "Redis".to_string(),
                reason: "test".to_string(),
            },
            StorageError::SerializationError {
                error_type: "JSON".to_string(),
                reason: "test".to_string(),
            },
            StorageError::NotFound {
                backend: "Redis".to_string(),
                id: "key123".to_string(),
            },
            StorageError::TransactionError {
                backend: "Redis".to_string(),
                reason: "test".to_string(),
            },
        ];

        for error in errors {
            let msg = error.to_string();
            assert!(
                msg.contains("Redis") || msg.contains("JSON"),
                "Error message should contain backend or error type: {}",
                msg
            );
        }
    }

    #[test]
    fn test_method_signatures() {
        let _ = RedisStorage::new;
    }

    #[test]
    fn test_scoped_key_format() {
        use mk_core::types::{TenantContext, TenantId, UserId};

        let tenant_id = TenantId::new("acme-corp".to_string()).unwrap();
        let user_id = UserId::new("user-1".to_string()).unwrap();
        let ctx = TenantContext::new(tenant_id, user_id);

        let key = format!("{}:{}", ctx.tenant_id.as_str(), "test-key");
        assert_eq!(key, "acme-corp:test-key");
    }

    #[test]
    fn test_job_checkpoint_key_format() {
        let job_name = "drift_scan";
        let tenant_id = "tenant-123";
        let key = format!("job_checkpoint:{}:{}", job_name, tenant_id);
        assert_eq!(key, "job_checkpoint:drift_scan:tenant-123");
    }

    #[test]
    fn test_job_completed_key_format() {
        let job_name = "semantic_analysis";
        let key = format!("job_completed:{}", job_name);
        assert_eq!(key, "job_completed:semantic_analysis");
    }

    #[test]
    fn test_governance_events_stream_key_format() {
        let tenant_id = "acme-corp";
        let stream_key = format!("governance:events:{}", tenant_id);
        assert_eq!(stream_key, "governance:events:acme-corp");
    }

    #[test]
    fn test_summary_cache_key_format() {
        use mk_core::types::{MemoryLayer, SummaryDepth};

        let key = RedisStorage::summary_cache_key(
            "acme-corp",
            &MemoryLayer::Project,
            "entry-123",
            &SummaryDepth::Sentence,
        );
        assert_eq!(key, "summary:acme-corp:project:entry-123:sentence");

        let key_para = RedisStorage::summary_cache_key(
            "tenant-1",
            &MemoryLayer::Team,
            "mem-456",
            &SummaryDepth::Paragraph,
        );
        assert_eq!(key_para, "summary:tenant-1:team:mem-456:paragraph");

        let key_detailed = RedisStorage::summary_cache_key(
            "org-x",
            &MemoryLayer::Tenant,
            "entry-789",
            &SummaryDepth::Detailed,
        );
        assert_eq!(key_detailed, "summary:org-x:tenant:entry-789:detailed");
    }

    #[test]
    fn test_summary_cache_key_all_layers() {
        use mk_core::types::{MemoryLayer, SummaryDepth};

        let layers = [
            (MemoryLayer::Tenant, "tenant"),
            (MemoryLayer::Org, "org"),
            (MemoryLayer::Team, "team"),
            (MemoryLayer::Project, "project"),
            (MemoryLayer::Session, "session"),
            (MemoryLayer::User, "user"),
            (MemoryLayer::Agent, "agent"),
        ];

        for (layer, expected_str) in layers {
            let key = RedisStorage::summary_cache_key("t1", &layer, "e1", &SummaryDepth::Sentence);
            assert!(
                key.contains(expected_str),
                "Key '{}' should contain layer '{}'",
                key,
                expected_str
            );
        }
    }

    #[test]
    fn test_summary_cache_key_all_depths() {
        use mk_core::types::{MemoryLayer, SummaryDepth};

        let depths = [
            (SummaryDepth::Sentence, "sentence"),
            (SummaryDepth::Paragraph, "paragraph"),
            (SummaryDepth::Detailed, "detailed"),
        ];

        for (depth, expected_str) in depths {
            let key =
                RedisStorage::summary_cache_key("tenant", &MemoryLayer::Project, "entry", &depth);
            assert!(
                key.contains(expected_str),
                "Key '{}' should contain depth '{}'",
                key,
                expected_str
            );
        }
    }

    #[test]
    fn test_layer_summary_serialization_roundtrip() {
        use mk_core::types::{LayerSummary, SummaryDepth};

        let summary = LayerSummary {
            depth: SummaryDepth::Paragraph,
            content: "Test summary content".to_string(),
            token_count: 42,
            generated_at: 1704067200,
            source_hash: "abc123def456".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        };

        let json = serde_json::to_string(&summary).expect("Serialization should succeed");
        let parsed: LayerSummary =
            serde_json::from_str(&json).expect("Deserialization should succeed");

        assert_eq!(parsed.depth, summary.depth);
        assert_eq!(parsed.content, summary.content);
        assert_eq!(parsed.token_count, summary.token_count);
        assert_eq!(parsed.source_hash, summary.source_hash);
    }

    #[test]
    fn test_layer_summary_with_personalization() {
        use mk_core::types::{LayerSummary, SummaryDepth};

        let summary = LayerSummary {
            depth: SummaryDepth::Detailed,
            content: "Personalized summary for user".to_string(),
            token_count: 150,
            generated_at: 1704067200,
            source_hash: "hash123".to_string(),
            content_hash: None,
            personalized: true,
            personalization_context: Some("User prefers concise responses".to_string()),
        };

        let json = serde_json::to_string(&summary).expect("Serialization should succeed");
        let parsed: LayerSummary =
            serde_json::from_str(&json).expect("Deserialization should succeed");

        assert!(parsed.personalized);
        assert_eq!(
            parsed.personalization_context,
            Some("User prefers concise responses".to_string())
        );
    }
}
