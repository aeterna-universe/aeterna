use mk_core::traits::{EventPublisher, StorageBackend};
use mk_core::types::{EventStatus, GovernanceEvent, PersistentEvent, TenantContext};
use std::sync::Arc;
use tracing::{error, info, warn};

pub struct DurableEventPublisher<S, P>
where
    S: StorageBackend + Send + Sync,
    P: EventPublisher + Send + Sync,
{
    storage: Arc<S>,
    redis_publisher: Arc<P>,
    tenant_ctx: TenantContext,
}

impl<S, P> DurableEventPublisher<S, P>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::error::Error + Send + Sync + 'static,
    P: EventPublisher + Send + Sync,
    P::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn new(storage: Arc<S>, redis_publisher: Arc<P>, tenant_ctx: TenantContext) -> Self {
        Self {
            storage,
            redis_publisher,
            tenant_ctx,
        }
    }

    pub async fn publish_durable(
        &self,
        event: GovernanceEvent,
    ) -> Result<String, DurablePublishError> {
        let persistent_event = PersistentEvent::new(event.clone());
        let event_id = persistent_event.event_id.clone();
        let idempotency_key = persistent_event.idempotency_key.clone();

        self.storage
            .persist_event(persistent_event)
            .await
            .map_err(|e| DurablePublishError::PersistenceError(e.to_string()))?;

        match self.redis_publisher.publish(event).await {
            Ok(()) => {
                self.storage
                    .update_event_status(&event_id, EventStatus::Published, None)
                    .await
                    .map_err(|e| DurablePublishError::StatusUpdateError(e.to_string()))?;

                info!(event_id = %event_id, "Event published successfully");
                Ok(idempotency_key)
            }
            Err(e) => {
                warn!(event_id = %event_id, error = %e, "Failed to publish to Redis, event persisted for retry");
                self.storage
                    .update_event_status(&event_id, EventStatus::Pending, Some(e.to_string()))
                    .await
                    .ok();

                Err(DurablePublishError::PublishError(e.to_string()))
            }
        }
    }

    pub async fn retry_pending_events(
        &self,
        limit: usize,
    ) -> Result<RetryResult, DurablePublishError> {
        let pending = self
            .storage
            .get_pending_events(self.tenant_ctx.clone(), limit)
            .await
            .map_err(|e| DurablePublishError::StorageError(e.to_string()))?;

        let mut result = RetryResult::default();
        result.total = pending.len();

        for event in pending {
            if !event.is_retriable() {
                continue;
            }

            match self.redis_publisher.publish(event.payload.clone()).await {
                Ok(()) => {
                    self.storage
                        .update_event_status(&event.event_id, EventStatus::Published, None)
                        .await
                        .ok();
                    result.succeeded += 1;
                }
                Err(e) => {
                    let mut updated_event = event.clone();
                    let can_retry = updated_event.mark_failed(e.to_string());

                    if can_retry {
                        self.storage
                            .update_event_status(
                                &event.event_id,
                                EventStatus::Pending,
                                Some(e.to_string()),
                            )
                            .await
                            .ok();
                        result.retried += 1;
                    } else {
                        self.storage
                            .update_event_status(
                                &event.event_id,
                                EventStatus::DeadLettered,
                                Some(e.to_string()),
                            )
                            .await
                            .ok();
                        result.dead_lettered += 1;
                        error!(
                            event_id = %event.event_id,
                            "Event moved to dead letter queue after max retries"
                        );
                    }
                }
            }
        }

        Ok(result)
    }

    pub async fn process_dead_letter_queue(
        &self,
        limit: usize,
        handler: impl Fn(&PersistentEvent) -> bool,
    ) -> Result<DlqResult, DurablePublishError> {
        let dead_letters = self
            .storage
            .get_dead_letter_events(self.tenant_ctx.clone(), limit)
            .await
            .map_err(|e| DurablePublishError::StorageError(e.to_string()))?;

        let mut result = DlqResult::default();
        result.total = dead_letters.len();

        for event in dead_letters {
            if handler(&event) {
                match self.redis_publisher.publish(event.payload.clone()).await {
                    Ok(()) => {
                        self.storage
                            .update_event_status(&event.event_id, EventStatus::Published, None)
                            .await
                            .ok();
                        result.reprocessed += 1;
                    }
                    Err(e) => {
                        warn!(
                            event_id = %event.event_id,
                            error = %e,
                            "DLQ reprocessing failed"
                        );
                        result.failed += 1;
                    }
                }
            } else {
                result.skipped += 1;
            }
        }

        Ok(result)
    }
}

#[derive(Debug, Default)]
pub struct RetryResult {
    pub total: usize,
    pub succeeded: usize,
    pub retried: usize,
    pub dead_lettered: usize,
}

#[derive(Debug, Default)]
pub struct DlqResult {
    pub total: usize,
    pub reprocessed: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum DurablePublishError {
    #[error("Failed to persist event: {0}")]
    PersistenceError(String),

    #[error("Failed to publish event: {0}")]
    PublishError(String),

    #[error("Failed to update event status: {0}")]
    StatusUpdateError(String),

    #[error("Storage operation failed: {0}")]
    StorageError(String),
}

pub struct IdempotentConsumer<S>
where
    S: StorageBackend + Send + Sync,
{
    storage: Arc<S>,
    consumer_group: String,
}

impl<S> IdempotentConsumer<S>
where
    S: StorageBackend + Send + Sync,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    pub fn new(storage: Arc<S>, consumer_group: String) -> Self {
        Self {
            storage,
            consumer_group,
        }
    }

    pub async fn process_if_new<F, T, E>(
        &self,
        idempotency_key: &str,
        tenant_id: &mk_core::types::TenantId,
        handler: F,
    ) -> Result<Option<T>, IdempotentConsumerError>
    where
        F: std::future::Future<Output = Result<T, E>>,
        E: std::error::Error,
    {
        let already_processed = self
            .storage
            .check_idempotency(&self.consumer_group, idempotency_key)
            .await
            .map_err(|e| IdempotentConsumerError::StorageError(e.to_string()))?;

        if already_processed {
            return Ok(None);
        }

        let result = handler
            .await
            .map_err(|e| IdempotentConsumerError::ProcessingError(e.to_string()))?;

        let state = mk_core::types::ConsumerState::new(
            self.consumer_group.clone(),
            idempotency_key.to_string(),
            tenant_id.clone(),
        );

        self.storage
            .record_consumer_state(state)
            .await
            .map_err(|e| IdempotentConsumerError::StorageError(e.to_string()))?;

        Ok(Some(result))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IdempotentConsumerError {
    #[error("Storage operation failed: {0}")]
    StorageError(String),

    #[error("Event processing failed: {0}")]
    ProcessingError(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mk_core::types::{
        ConsumerState, DriftConfig, DriftResult, DriftSuppression, EventDeliveryMetrics,
        GovernanceEvent, OrganizationalUnit, PersistentEvent, Policy, RoleIdentifier, TenantId,
        UnitType, UserId,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::RwLock;

    #[derive(Clone)]
    struct MockStorage {
        persist_calls: Arc<AtomicUsize>,
        update_calls: Arc<AtomicUsize>,
        pending_events: Arc<RwLock<Vec<PersistentEvent>>>,
        dead_letter_events: Arc<RwLock<Vec<PersistentEvent>>>,
        idempotency_keys: Arc<RwLock<Vec<String>>>,
        should_fail: Arc<RwLock<bool>>,
    }

    impl MockStorage {
        fn new() -> Self {
            Self {
                persist_calls: Arc::new(AtomicUsize::new(0)),
                update_calls: Arc::new(AtomicUsize::new(0)),
                pending_events: Arc::new(RwLock::new(Vec::new())),
                dead_letter_events: Arc::new(RwLock::new(Vec::new())),
                idempotency_keys: Arc::new(RwLock::new(Vec::new())),
                should_fail: Arc::new(RwLock::new(false)),
            }
        }

        fn with_pending(events: Vec<PersistentEvent>) -> Self {
            let storage = Self::new();
            let pending = Arc::new(RwLock::new(events));
            Self {
                pending_events: pending,
                ..storage
            }
        }

        fn with_dead_letters(events: Vec<PersistentEvent>) -> Self {
            let storage = Self::new();
            let dlq = Arc::new(RwLock::new(events));
            Self {
                dead_letter_events: dlq,
                ..storage
            }
        }
    }

    #[derive(Debug)]
    struct MockStorageError(String);

    impl std::fmt::Display for MockStorageError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for MockStorageError {}

    #[async_trait]
    impl StorageBackend for MockStorage {
        type Error = MockStorageError;

        async fn store(
            &self,
            _ctx: TenantContext,
            _key: &str,
            _value: &[u8],
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn retrieve(
            &self,
            _ctx: TenantContext,
            _key: &str,
        ) -> Result<Option<Vec<u8>>, Self::Error> {
            Ok(None)
        }

        async fn delete(&self, _ctx: TenantContext, _key: &str) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn exists(&self, _ctx: TenantContext, _key: &str) -> Result<bool, Self::Error> {
            Ok(false)
        }

        async fn get_ancestors(
            &self,
            _ctx: TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
            Ok(Vec::new())
        }

        async fn get_descendants(
            &self,
            _ctx: TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<OrganizationalUnit>, Self::Error> {
            Ok(Vec::new())
        }

        async fn get_unit_policies(
            &self,
            _ctx: TenantContext,
            _unit_id: &str,
        ) -> Result<Vec<Policy>, Self::Error> {
            Ok(Vec::new())
        }

        async fn create_unit(&self, _unit: &OrganizationalUnit) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn add_unit_policy(
            &self,
            _ctx: &TenantContext,
            _unit_id: &str,
            _policy: &Policy,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn assign_role(
            &self,
            _user_id: &UserId,
            _tenant_id: &TenantId,
            _unit_id: &str,
            _role: RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn remove_role(
            &self,
            _user_id: &UserId,
            _tenant_id: &TenantId,
            _unit_id: &str,
            _role: RoleIdentifier,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn store_drift_result(&self, _result: DriftResult) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn get_latest_drift_result(
            &self,
            _ctx: TenantContext,
            _project_id: &str,
        ) -> Result<Option<DriftResult>, Self::Error> {
            Ok(None)
        }

        async fn list_all_units(&self) -> Result<Vec<OrganizationalUnit>, Self::Error> {
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
        ) -> Result<Vec<GovernanceEvent>, Self::Error> {
            Ok(Vec::new())
        }

        async fn persist_event(&self, _event: PersistentEvent) -> Result<(), Self::Error> {
            self.persist_calls.fetch_add(1, Ordering::SeqCst);
            if *self.should_fail.read().await {
                return Err(MockStorageError("Persist failed".to_string()));
            }
            Ok(())
        }

        async fn get_pending_events(
            &self,
            _ctx: TenantContext,
            _limit: usize,
        ) -> Result<Vec<PersistentEvent>, Self::Error> {
            Ok(self.pending_events.read().await.clone())
        }

        async fn update_event_status(
            &self,
            _event_id: &str,
            _status: EventStatus,
            _error: Option<String>,
        ) -> Result<(), Self::Error> {
            self.update_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn get_dead_letter_events(
            &self,
            _ctx: TenantContext,
            _limit: usize,
        ) -> Result<Vec<PersistentEvent>, Self::Error> {
            Ok(self.dead_letter_events.read().await.clone())
        }

        async fn check_idempotency(
            &self,
            _consumer_group: &str,
            idempotency_key: &str,
        ) -> Result<bool, Self::Error> {
            let keys = self.idempotency_keys.read().await;
            Ok(keys.contains(&idempotency_key.to_string()))
        }

        async fn record_consumer_state(&self, state: ConsumerState) -> Result<(), Self::Error> {
            self.idempotency_keys
                .write()
                .await
                .push(state.idempotency_key);
            Ok(())
        }

        async fn get_event_metrics(
            &self,
            _ctx: TenantContext,
            _period_start: i64,
            _period_end: i64,
        ) -> Result<Vec<EventDeliveryMetrics>, Self::Error> {
            Ok(Vec::new())
        }

        async fn record_event_metrics(
            &self,
            _metrics: EventDeliveryMetrics,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn create_suppression(
            &self,
            _suppression: DriftSuppression,
        ) -> Result<(), Self::Error> {
            Ok(())
        }

        async fn list_suppressions(
            &self,
            _ctx: TenantContext,
            _project_id: &str,
        ) -> Result<Vec<DriftSuppression>, Self::Error> {
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
        ) -> Result<Option<DriftConfig>, Self::Error> {
            Ok(None)
        }

        async fn save_drift_config(&self, _config: DriftConfig) -> Result<(), Self::Error> {
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

    struct MockPublisher {
        publish_calls: Arc<AtomicUsize>,
        should_fail: Arc<RwLock<bool>>,
    }

    impl MockPublisher {
        fn new() -> Self {
            Self {
                publish_calls: Arc::new(AtomicUsize::new(0)),
                should_fail: Arc::new(RwLock::new(false)),
            }
        }

        fn failing() -> Self {
            Self {
                publish_calls: Arc::new(AtomicUsize::new(0)),
                should_fail: Arc::new(RwLock::new(true)),
            }
        }
    }

    #[derive(Debug)]
    struct MockPublisherError(String);

    impl std::fmt::Display for MockPublisherError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl std::error::Error for MockPublisherError {}

    #[async_trait]
    impl EventPublisher for MockPublisher {
        type Error = MockPublisherError;

        async fn publish(&self, _event: GovernanceEvent) -> Result<(), Self::Error> {
            self.publish_calls.fetch_add(1, Ordering::SeqCst);
            if *self.should_fail.read().await {
                return Err(MockPublisherError("Publish failed".to_string()));
            }
            Ok(())
        }

        async fn subscribe(
            &self,
            _channels: &[&str],
        ) -> Result<tokio::sync::mpsc::Receiver<GovernanceEvent>, Self::Error> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }
    }

    fn create_test_event() -> GovernanceEvent {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        GovernanceEvent::UnitCreated {
            unit_id: "unit-1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id,
            parent_id: None,
            timestamp: chrono::Utc::now().timestamp(),
        }
    }

    fn create_test_context() -> TenantContext {
        let tenant_id = TenantId::new("acme".to_string()).unwrap();
        let user_id = UserId::new("user-1".to_string()).unwrap();
        TenantContext::new(tenant_id, user_id)
    }

    #[test]
    fn test_retry_result_default() {
        let result = RetryResult::default();
        assert_eq!(result.total, 0);
        assert_eq!(result.succeeded, 0);
        assert_eq!(result.retried, 0);
        assert_eq!(result.dead_lettered, 0);
    }

    #[test]
    fn test_dlq_result_default() {
        let result = DlqResult::default();
        assert_eq!(result.total, 0);
        assert_eq!(result.reprocessed, 0);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn test_durable_publish_error_display() {
        let error = DurablePublishError::PersistenceError("DB down".to_string());
        assert!(error.to_string().contains("persist"));

        let error = DurablePublishError::PublishError("Redis timeout".to_string());
        assert!(error.to_string().contains("publish"));

        let error = DurablePublishError::StatusUpdateError("Update failed".to_string());
        assert!(error.to_string().contains("status"));

        let error = DurablePublishError::StorageError("IO error".to_string());
        assert!(error.to_string().contains("Storage"));
    }

    #[test]
    fn test_idempotent_consumer_error_display() {
        let error = IdempotentConsumerError::StorageError("Connection lost".to_string());
        assert!(error.to_string().contains("Storage"));

        let error = IdempotentConsumerError::ProcessingError("Handler failed".to_string());
        assert!(error.to_string().contains("processing"));
    }

    #[tokio::test]
    async fn test_durable_publisher_new() {
        let storage = Arc::new(MockStorage::new());
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage, publisher, ctx);
        assert!(std::ptr::eq(&*durable.storage, &*durable.storage));
    }

    #[tokio::test]
    async fn test_publish_durable_success() {
        let storage = Arc::new(MockStorage::new());
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();
        let event = create_test_event();

        let durable = DurableEventPublisher::new(storage.clone(), publisher.clone(), ctx);
        let result = durable.publish_durable(event).await;

        assert!(result.is_ok());
        assert_eq!(storage.persist_calls.load(Ordering::SeqCst), 1);
        assert_eq!(publisher.publish_calls.load(Ordering::SeqCst), 1);
        assert_eq!(storage.update_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_publish_durable_publish_failure() {
        let storage = Arc::new(MockStorage::new());
        let publisher = Arc::new(MockPublisher::failing());
        let ctx = create_test_context();
        let event = create_test_event();

        let durable = DurableEventPublisher::new(storage.clone(), publisher.clone(), ctx);
        let result = durable.publish_durable(event).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            DurablePublishError::PublishError(_)
        ));
        assert_eq!(storage.persist_calls.load(Ordering::SeqCst), 1);
        assert_eq!(publisher.publish_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_pending_events_empty() {
        let storage = Arc::new(MockStorage::new());
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage, publisher, ctx);
        let result = durable.retry_pending_events(10).await;

        assert!(result.is_ok());
        let retry_result = result.unwrap();
        assert_eq!(retry_result.total, 0);
        assert_eq!(retry_result.succeeded, 0);
    }

    #[tokio::test]
    async fn test_retry_pending_events_success() {
        let event = create_test_event();
        let pending = vec![PersistentEvent::new(event)];
        let storage = Arc::new(MockStorage::with_pending(pending));
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage.clone(), publisher.clone(), ctx);
        let result = durable.retry_pending_events(10).await;

        assert!(result.is_ok());
        let retry_result = result.unwrap();
        assert_eq!(retry_result.total, 1);
        assert_eq!(retry_result.succeeded, 1);
        assert_eq!(publisher.publish_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_retry_pending_events_failure() {
        let event = create_test_event();
        let pending = vec![PersistentEvent::new(event)];
        let storage = Arc::new(MockStorage::with_pending(pending));
        let publisher = Arc::new(MockPublisher::failing());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage.clone(), publisher, ctx);
        let result = durable.retry_pending_events(10).await;

        assert!(result.is_ok());
        let retry_result = result.unwrap();
        assert_eq!(retry_result.total, 1);
        assert_eq!(retry_result.retried, 1);
    }

    #[tokio::test]
    async fn test_process_dead_letter_queue_empty() {
        let storage = Arc::new(MockStorage::new());
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage, publisher, ctx);
        let result = durable.process_dead_letter_queue(10, |_| true).await;

        assert!(result.is_ok());
        let dlq_result = result.unwrap();
        assert_eq!(dlq_result.total, 0);
    }

    #[tokio::test]
    async fn test_process_dead_letter_queue_reprocess() {
        let event = create_test_event();
        let dead_letters = vec![PersistentEvent::new(event)];
        let storage = Arc::new(MockStorage::with_dead_letters(dead_letters));
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage, publisher.clone(), ctx);
        let result = durable.process_dead_letter_queue(10, |_| true).await;

        assert!(result.is_ok());
        let dlq_result = result.unwrap();
        assert_eq!(dlq_result.total, 1);
        assert_eq!(dlq_result.reprocessed, 1);
        assert_eq!(publisher.publish_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_process_dead_letter_queue_skipped() {
        let event = create_test_event();
        let dead_letters = vec![PersistentEvent::new(event)];
        let storage = Arc::new(MockStorage::with_dead_letters(dead_letters));
        let publisher = Arc::new(MockPublisher::new());
        let ctx = create_test_context();

        let durable = DurableEventPublisher::new(storage, publisher.clone(), ctx);
        let result = durable.process_dead_letter_queue(10, |_| false).await;

        assert!(result.is_ok());
        let dlq_result = result.unwrap();
        assert_eq!(dlq_result.total, 1);
        assert_eq!(dlq_result.skipped, 1);
        assert_eq!(publisher.publish_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_idempotent_consumer_new() {
        let storage = Arc::new(MockStorage::new());
        let consumer = IdempotentConsumer::new(storage, "test_group".to_string());

        assert_eq!(consumer.consumer_group, "test_group");
    }

    #[tokio::test]
    async fn test_idempotent_consumer_process_new_event() {
        let storage = Arc::new(MockStorage::new());
        let consumer = IdempotentConsumer::new(storage, "test_group".to_string());
        let tenant_id = TenantId::new("acme".to_string()).unwrap();

        let result = consumer
            .process_if_new("key-1", &tenant_id, async { Ok::<_, std::io::Error>(42) })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(42));
    }

    #[tokio::test]
    async fn test_idempotent_consumer_skip_duplicate() {
        let storage = Arc::new(MockStorage::new());
        storage
            .idempotency_keys
            .write()
            .await
            .push("key-1".to_string());

        let consumer = IdempotentConsumer::new(storage, "test_group".to_string());
        let tenant_id = TenantId::new("acme".to_string()).unwrap();

        let result = consumer
            .process_if_new("key-1", &tenant_id, async { Ok::<_, std::io::Error>(42) })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }
}
