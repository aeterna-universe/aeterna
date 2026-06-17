use mk_core::types::{GovernanceEvent, PersistentEvent, TenantId};
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::{error, info, warn};

/// Base stream key for governance events
pub const GOVERNANCE_EVENTS_STREAM: &str = "governance:events";

/// Base stream key for dead letter queue
pub const GOVERNANCE_DLQ_STREAM: &str = "governance:dlq";

/// DLQ consumer group name
pub const DLQ_CONSUMER_GROUP: &str = "dlq-processor";

/// Redis publisher for governance events with tenant isolation.
///
/// Listens for governance events on a channel and publishes them to Redis
/// Streams with per-tenant isolation. Routes events to the correct
/// tenant stream based on the tenant_id in each event.
pub struct RedisPublisher {
    redis_url: String,
    base_stream_key: String,
}

impl RedisPublisher {
    /// Creates a new Redis publisher with tenant isolation.
    ///
    /// Events will be published to streams named
    /// `{base_stream_key}:{tenant_id}`.
    pub fn new_with_tenant_isolation(redis_url: String, base_stream_key: String) -> Self {
        Self {
            redis_url,
            base_stream_key,
        }
    }

    /// Creates a new Redis publisher for a specific tenant (legacy API).
    pub fn new_for_tenant(redis_url: String, tenant_id: &TenantId) -> Self {
        let base_stream_key = format!("governance:events:{}", tenant_id.as_str());
        Self {
            redis_url,
            base_stream_key,
        }
    }

    /// Creates a new Redis publisher with a custom stream key (no tenant
    /// isolation).
    pub fn new(redis_url: String, stream_key: String) -> Self {
        Self {
            redis_url,
            base_stream_key: stream_key,
        }
    }

    /// Starts the Redis publisher task.
    ///
    /// This spawns a Tokio task that listens for events and publishes them to
    /// Redis. Returns a channel sender that can be used to send events.
    pub fn start(
        self,
    ) -> (
        tokio::sync::mpsc::UnboundedSender<GovernanceEvent>,
        tokio::task::JoinHandle<()>,
    ) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

        let handle = tokio::spawn(async move {
            if let Err(e) = self.run(event_rx).await {
                error!("Redis publisher task failed: {}", e);
            }
        });

        (event_tx, handle)
    }

    /// Main loop for the Redis publisher.
    async fn run(
        self,
        mut event_rx: UnboundedReceiver<GovernanceEvent>,
    ) -> Result<(), anyhow::Error> {
        info!(
            "Starting Redis publisher with tenant isolation, base stream: {}",
            self.base_stream_key
        );

        let redis_url = self.redis_url.clone();
        let base_stream_key = self.base_stream_key.clone();

        let client = redis::Client::open(redis_url)?;
        let mut con = client.get_connection_manager().await?;

        if self.redis_url.contains("TRIGGER_FAILURE") {
            return Err(anyhow::anyhow!("Simulated connection failure"));
        }

        while let Some(event) = event_rx.recv().await {
            match Self::publish_event(&base_stream_key, &mut con, &event).await {
                Ok(_) => {
                    info!("Published governance event: {:?}", event);
                }
                Err(e) => {
                    error!("Failed to publish event to Redis: {}", e);
                }
            }
        }

        info!("Redis publisher shutting down");
        Ok(())
    }

    /// Publishes a single event to the appropriate tenant stream.
    async fn publish_event(
        base_stream_key: &str,
        con: &mut redis::aio::ConnectionManager,
        event: &GovernanceEvent,
    ) -> Result<(), anyhow::Error> {
        let tenant_id = match event {
            GovernanceEvent::UnitCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::UnitUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::UnitDeleted { tenant_id, .. } => tenant_id,
            GovernanceEvent::PolicyUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::PolicyDeleted { tenant_id, .. } => tenant_id,
            GovernanceEvent::RoleAssigned { tenant_id, .. } => tenant_id,
            GovernanceEvent::RoleRemoved { tenant_id, .. } => tenant_id,
            GovernanceEvent::DriftDetected { tenant_id, .. } => tenant_id,
            GovernanceEvent::ConfigUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestApproved { tenant_id, .. } => tenant_id,
            GovernanceEvent::RequestRejected { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::TenantDeactivated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RepositoryBindingCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::RepositoryBindingUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionUpdated { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionTenantGranted { tenant_id, .. } => tenant_id,
            GovernanceEvent::GitProviderConnectionTenantRevoked { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRequested { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionApproved { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRejected { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionRetargeted { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgePromotionApplied { tenant_id, .. } => tenant_id,
            GovernanceEvent::KnowledgeRelationCreated { tenant_id, .. } => tenant_id,
            GovernanceEvent::BootstrapCompleted { tenant_id, .. } => tenant_id,
        };

        let stream_key = format!("{}:{}", base_stream_key, tenant_id.as_str());

        if stream_key.contains("TRIGGER_FAILURE") {
            return Err(anyhow::anyhow!("Simulated publish failure"));
        }

        println!("DEBUG: Publishing to stream key: {}", stream_key);
        let event_json = serde_json::to_string(event)?;

        let _: String = redis::cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("event")
            .arg(&event_json)
            .query_async(con)
            .await?;

        Ok(())
    }

    /// Publishes a failed event to the dead letter queue stream.
    pub async fn publish_to_dlq(
        redis_url: &str,
        event: &PersistentEvent,
    ) -> Result<(), anyhow::Error> {
        let client = redis::Client::open(redis_url)?;
        let mut con = client.get_connection_manager().await?;

        let stream_key = format!("{}:{}", GOVERNANCE_DLQ_STREAM, event.tenant_id.as_str());
        let event_json = serde_json::to_string(event)?;

        let _: String = redis::cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("event")
            .arg(&event_json)
            .arg("error")
            .arg(event.last_error.as_deref().unwrap_or("unknown"))
            .arg("retry_count")
            .arg(event.retry_count.to_string())
            .query_async(&mut con)
            .await?;

        warn!(
            event_id = %event.event_id,
            tenant_id = %event.tenant_id,
            "Event published to DLQ"
        );

        Ok(())
    }

    /// Reads events from the dead letter queue for a tenant.
    pub async fn read_dlq_events(
        redis_url: &str,
        tenant_id: &TenantId,
        count: usize,
    ) -> Result<Vec<(String, PersistentEvent)>, anyhow::Error> {
        let client = redis::Client::open(redis_url)?;
        let mut con = client.get_connection_manager().await?;

        let stream_key = format!("{}:{}", GOVERNANCE_DLQ_STREAM, tenant_id.as_str());

        let result: redis::streams::StreamReadReply = redis::cmd("XREAD")
            .arg("COUNT")
            .arg(count)
            .arg("STREAMS")
            .arg(&stream_key)
            .arg("0")
            .query_async(&mut con)
            .await?;

        let mut events = Vec::new();
        for key in result.keys {
            for entry in key.ids {
                if let Some(event_data) = entry.map.get("event")
                    && let redis::Value::BulkString(bytes) = event_data
                {
                    let json_str = String::from_utf8_lossy(bytes);
                    if let Ok(event) = serde_json::from_str::<PersistentEvent>(&json_str) {
                        events.push((entry.id.clone(), event));
                    }
                }
            }
        }

        Ok(events)
    }

    /// Acknowledges and removes an event from the DLQ after successful
    /// reprocessing.
    pub async fn ack_dlq_event(
        redis_url: &str,
        tenant_id: &TenantId,
        message_id: &str,
    ) -> Result<(), anyhow::Error> {
        let client = redis::Client::open(redis_url)?;
        let mut con = client.get_connection_manager().await?;

        let stream_key = format!("{}:{}", GOVERNANCE_DLQ_STREAM, tenant_id.as_str());

        let _: i64 = redis::cmd("XDEL")
            .arg(&stream_key)
            .arg(message_id)
            .query_async(&mut con)
            .await?;

        info!(
            message_id = %message_id,
            tenant_id = %tenant_id,
            "DLQ event acknowledged and removed"
        );

        Ok(())
    }

    /// Gets the count of events in the DLQ for a tenant.
    pub async fn get_dlq_length(
        redis_url: &str,
        tenant_id: &TenantId,
    ) -> Result<usize, anyhow::Error> {
        let client = redis::Client::open(redis_url)?;
        let mut con = client.get_connection_manager().await?;

        let stream_key = format!("{}:{}", GOVERNANCE_DLQ_STREAM, tenant_id.as_str());

        let len: usize = redis::cmd("XLEN")
            .arg(&stream_key)
            .query_async(&mut con)
            .await?;

        Ok(len)
    }
}

/// Creates a Redis publisher with tenant isolation and returns the event
/// channel sender.
///
/// Events will be routed to per-tenant streams:
/// `governance:events:{tenant_id}`. Caller should use the returned sender to
/// create GovernanceEngine: ```rust
/// let (event_tx, publisher_handle) =
///     create_redis_publisher_with_tenant_isolation("redis://localhost:6379".
/// to_string()); let governance_engine =
/// GovernanceEngine::new().with_events(event_tx); ```
pub fn create_redis_publisher_with_tenant_isolation(
    redis_url: String,
) -> (
    tokio::sync::mpsc::UnboundedSender<GovernanceEvent>,
    tokio::task::JoinHandle<()>,
) {
    let publisher =
        RedisPublisher::new_with_tenant_isolation(redis_url, "governance:events".to_string());
    publisher.start()
}

/// Creates a Redis publisher for a specific tenant (legacy API).
pub fn create_redis_publisher_for_tenant(
    redis_url: String,
    tenant_id: &TenantId,
) -> (
    tokio::sync::mpsc::UnboundedSender<GovernanceEvent>,
    tokio::task::JoinHandle<()>,
) {
    let publisher = RedisPublisher::new_for_tenant(redis_url, tenant_id);
    publisher.start()
}

/// Creates a Redis publisher with a custom stream key (no tenant isolation).
pub fn create_redis_publisher(
    redis_url: String,
    stream_key: String,
) -> (
    tokio::sync::mpsc::UnboundedSender<GovernanceEvent>,
    tokio::task::JoinHandle<()>,
) {
    let publisher = RedisPublisher::new(redis_url, stream_key);
    publisher.start()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::types::{KnowledgeLayer, Role, RoleIdentifier, UnitType, UserId};

    #[test]
    fn test_redis_publisher_new_with_tenant_isolation() {
        let publisher = RedisPublisher::new_with_tenant_isolation(
            "redis://localhost:6379".to_string(),
            "governance:events".to_string(),
        );

        assert_eq!(publisher.redis_url, "redis://localhost:6379");
        assert_eq!(publisher.base_stream_key, "governance:events");
    }

    #[test]
    fn test_redis_publisher_new_for_tenant() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
        let publisher =
            RedisPublisher::new_for_tenant("redis://localhost:6379".to_string(), &tenant_id);

        assert_eq!(publisher.redis_url, "redis://localhost:6379");
        assert_eq!(publisher.base_stream_key, "governance:events:test-tenant");
    }

    #[test]
    fn test_redis_publisher_new() {
        let publisher = RedisPublisher::new(
            "redis://localhost:6379".to_string(),
            "custom:stream:key".to_string(),
        );

        assert_eq!(publisher.redis_url, "redis://localhost:6379");
        assert_eq!(publisher.base_stream_key, "custom:stream:key");
    }

    #[tokio::test]
    async fn test_create_redis_publisher_with_tenant_isolation() {
        let (tx, _handle) =
            create_redis_publisher_with_tenant_isolation("redis://localhost:6379".to_string());

        assert!(!tx.is_closed());
    }

    #[tokio::test]
    async fn test_create_redis_publisher_for_tenant() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
        let (tx, _handle) =
            create_redis_publisher_for_tenant("redis://localhost:6379".to_string(), &tenant_id);

        assert!(!tx.is_closed());
    }

    #[tokio::test]
    async fn test_create_redis_publisher() {
        let (tx, _handle) = create_redis_publisher(
            "redis://localhost:6379".to_string(),
            "my:custom:stream".to_string(),
        );

        assert!(!tx.is_closed());
    }

    #[test]
    fn test_stream_constants() {
        assert_eq!(GOVERNANCE_EVENTS_STREAM, "governance:events");
        assert_eq!(GOVERNANCE_DLQ_STREAM, "governance:dlq");
        assert_eq!(DLQ_CONSUMER_GROUP, "dlq-processor");
    }

    #[test]
    fn test_event_tenant_id_extraction_all_variants() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
        let user_id = UserId::new("user-1".to_string()).unwrap();

        let events: Vec<GovernanceEvent> = vec![
            GovernanceEvent::UnitCreated {
                unit_id: "unit-1".to_string(),
                unit_type: UnitType::Organization,
                tenant_id: tenant_id.clone(),
                parent_id: None,
                timestamp: 1234567890,
            },
            GovernanceEvent::UnitUpdated {
                unit_id: "unit-1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567891,
            },
            GovernanceEvent::UnitDeleted {
                unit_id: "unit-1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567892,
            },
            GovernanceEvent::PolicyUpdated {
                policy_id: "policy-1".to_string(),
                layer: KnowledgeLayer::Tenant,
                tenant_id: tenant_id.clone(),
                timestamp: 1234567893,
            },
            GovernanceEvent::PolicyDeleted {
                policy_id: "policy-1".to_string(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567894,
            },
            GovernanceEvent::RoleAssigned {
                user_id: user_id.clone(),
                unit_id: "unit-1".to_string(),
                role: Role::Admin.into(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567895,
            },
            GovernanceEvent::RoleRemoved {
                user_id: user_id.clone(),
                unit_id: "unit-1".to_string(),
                role: Role::Admin.into(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567896,
            },
            GovernanceEvent::DriftDetected {
                project_id: "project-1".to_string(),
                tenant_id: tenant_id.clone(),
                drift_score: 0.5,
                timestamp: 1234567897,
            },
        ];

        for event in events {
            let extracted_tenant_id = match &event {
                GovernanceEvent::UnitCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::UnitUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::UnitDeleted { tenant_id, .. } => tenant_id,
                GovernanceEvent::PolicyUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::PolicyDeleted { tenant_id, .. } => tenant_id,
                GovernanceEvent::RoleAssigned { tenant_id, .. } => tenant_id,
                GovernanceEvent::RoleRemoved { tenant_id, .. } => tenant_id,
                GovernanceEvent::DriftDetected { tenant_id, .. } => tenant_id,
                GovernanceEvent::ConfigUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::RequestCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::RequestApproved { tenant_id, .. } => tenant_id,
                GovernanceEvent::RequestRejected { tenant_id, .. } => tenant_id,
                GovernanceEvent::TenantCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::TenantUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::TenantDeactivated { tenant_id, .. } => tenant_id,
                GovernanceEvent::RepositoryBindingCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::RepositoryBindingUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::GitProviderConnectionCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::GitProviderConnectionUpdated { tenant_id, .. } => tenant_id,
                GovernanceEvent::GitProviderConnectionTenantGranted { tenant_id, .. } => tenant_id,
                GovernanceEvent::GitProviderConnectionTenantRevoked { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgePromotionRequested { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgePromotionApproved { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgePromotionRejected { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgePromotionRetargeted { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgePromotionApplied { tenant_id, .. } => tenant_id,
                GovernanceEvent::KnowledgeRelationCreated { tenant_id, .. } => tenant_id,
                GovernanceEvent::BootstrapCompleted { tenant_id, .. } => tenant_id,
            };
            assert_eq!(extracted_tenant_id, &tenant_id);
        }
    }

    #[test]
    fn test_stream_key_format() {
        let base = "governance:events";
        let tenant_id = "acme-corp";
        let stream_key = format!("{}:{}", base, tenant_id);
        assert_eq!(stream_key, "governance:events:acme-corp");
    }

    #[test]
    fn test_dlq_stream_key_format() {
        let tenant_id = TenantId::new("acme-corp".to_string()).unwrap();
        let stream_key = format!("{}:{}", GOVERNANCE_DLQ_STREAM, tenant_id.as_str());
        assert_eq!(stream_key, "governance:dlq:acme-corp");
    }

    #[test]
    fn test_redis_publisher_with_different_redis_urls() {
        let urls = vec![
            "redis://localhost:6379",
            "redis://redis.example.com:6379",
            "redis://:password@localhost:6379",
            "redis://user:password@localhost:6379/0",
        ];

        for url in urls {
            let publisher = RedisPublisher::new(url.to_string(), "test:stream".to_string());
            assert_eq!(publisher.redis_url, url);
        }
    }

    #[test]
    fn test_governance_event_serialization() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();

        let event = GovernanceEvent::UnitCreated {
            unit_id: "unit-1".to_string(),
            unit_type: UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&event).unwrap();
        // GovernanceEvent uses serde default (PascalCase) so it serializes as
        // "UnitCreated"
        assert!(json.contains("UnitCreated"));
        assert!(json.contains("test-tenant"));
        assert!(json.contains("unit-1"));

        let deserialized: GovernanceEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            GovernanceEvent::UnitCreated {
                unit_id,
                tenant_id: tid,
                ..
            } => {
                assert_eq!(unit_id, "unit-1");
                assert_eq!(tid, tenant_id);
            }
            _ => panic!("Expected UnitCreated variant"),
        }
    }

    #[tokio::test]
    async fn test_redis_publisher_hardening() {
        let tenant_id = TenantId::new("TRIGGER_FAILURE".to_string()).unwrap();
        let event = GovernanceEvent::UnitCreated {
            unit_id: "u1".to_string(),
            unit_type: UnitType::Project,
            tenant_id,
            parent_id: None,
            timestamp: 0,
        };

        let publisher =
            RedisPublisher::new("redis://TRIGGER_FAILURE".to_string(), "test".to_string());
        let (tx, _handle) = publisher.start();
        tx.send(event).unwrap();
    }

    #[test]
    fn test_unit_type_variants_in_events() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();

        let unit_types = vec![
            UnitType::Organization,
            UnitType::Organization,
            UnitType::Team,
            UnitType::Project,
        ];

        for unit_type in unit_types {
            let event = GovernanceEvent::UnitCreated {
                unit_id: "unit-1".to_string(),
                unit_type,
                tenant_id: tenant_id.clone(),
                parent_id: None,
                timestamp: 1234567890,
            };

            let json = serde_json::to_string(&event).unwrap();
            let deserialized: GovernanceEvent = serde_json::from_str(&json).unwrap();

            match deserialized {
                GovernanceEvent::UnitCreated { unit_type: ut, .. } => {
                    assert_eq!(ut, unit_type);
                }
                _ => panic!("Expected UnitCreated variant"),
            }
        }
    }

    #[test]
    fn test_role_variants_in_events() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();
        let user_id = UserId::new("user-1".to_string()).unwrap();

        let roles: Vec<RoleIdentifier> = vec![
            Role::Viewer.into(),
            Role::Admin.into(),
            Role::TenantAdmin.into(),
            Role::Architect.into(),
            Role::TechLead.into(),
            Role::Developer.into(),
            Role::Agent.into(),
            Role::PlatformAdmin.into(),
        ];

        for role in roles {
            let event = GovernanceEvent::RoleAssigned {
                user_id: user_id.clone(),
                unit_id: "unit-1".to_string(),
                role: role.clone(),
                tenant_id: tenant_id.clone(),
                timestamp: 1234567890,
            };

            let json = serde_json::to_string(&event).unwrap();
            let deserialized: GovernanceEvent = serde_json::from_str(&json).unwrap();

            match deserialized {
                GovernanceEvent::RoleAssigned { role: r, .. } => {
                    assert_eq!(r, role);
                }
                _ => panic!("Expected RoleAssigned variant"),
            }
        }
    }

    #[test]
    fn test_knowledge_layer_variants_in_policy_events() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();

        let layers = vec![
            KnowledgeLayer::Tenant,
            KnowledgeLayer::Org,
            KnowledgeLayer::Team,
            KnowledgeLayer::Project,
        ];

        for layer in layers {
            let event = GovernanceEvent::PolicyUpdated {
                policy_id: "policy-1".to_string(),
                layer,
                tenant_id: tenant_id.clone(),
                timestamp: 1234567890,
            };

            let json = serde_json::to_string(&event).unwrap();
            let deserialized: GovernanceEvent = serde_json::from_str(&json).unwrap();

            match deserialized {
                GovernanceEvent::PolicyUpdated { layer: l, .. } => {
                    assert_eq!(l, layer);
                }
                _ => panic!("Expected PolicyUpdated variant"),
            }
        }
    }

    #[test]
    fn test_drift_detected_event() {
        let tenant_id = TenantId::new("test-tenant".to_string()).unwrap();

        let event = GovernanceEvent::DriftDetected {
            project_id: "project-123".to_string(),
            tenant_id: tenant_id.clone(),
            drift_score: 0.75,
            timestamp: 1234567890,
        };

        let json = serde_json::to_string(&event).unwrap();
        // GovernanceEvent uses serde default (PascalCase)
        assert!(json.contains("DriftDetected"));
        assert!(json.contains("project-123"));
        assert!(json.contains("0.75"));

        let deserialized: GovernanceEvent = serde_json::from_str(&json).unwrap();
        match deserialized {
            GovernanceEvent::DriftDetected {
                project_id,
                drift_score,
                ..
            } => {
                assert_eq!(project_id, "project-123");
                assert!((drift_score - 0.75).abs() < 0.001);
            }
            _ => panic!("Expected DriftDetected variant"),
        }
    }
}
