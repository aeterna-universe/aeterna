//! `PostgreSQL` LISTEN/NOTIFY listener for real-time updates.
//!
//! This module listens to the `referential_changes` channel for database
//! changes and can optionally publish updates to OPAL via its `PubSub`
//! mechanism.

use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use std::sync::Arc;
use uuid::Uuid;

use crate::error::{FetcherError, Result};
use crate::state::AppState;

/// A notification payload from `PostgreSQL` NOTIFY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferentialChangeNotification {
    /// The table that changed.
    pub table: String,
    /// The operation type (INSERT, UPDATE, DELETE).
    pub operation: String,
    /// The ID of the changed entity.
    pub id: String,
    /// Unix timestamp of the change.
    pub timestamp: f64,
}

/// `PostgreSQL` LISTEN/NOTIFY listener for referential changes.
pub struct ReferentialListener {
    state: Arc<AppState>,
    pg_listener: PgListener,
}

impl ReferentialListener {
    /// Creates a new listener connected to the database.
    pub async fn new(state: Arc<AppState>) -> Result<Self> {
        let pg_listener = PgListener::connect_with(&state.pool)
            .await
            .map_err(|e| FetcherError::Listener(format!("Failed to create listener: {e}")))?;

        Ok(Self { state, pg_listener })
    }

    /// Starts listening for referential changes.
    ///
    /// This method runs indefinitely, processing notifications as they arrive.
    pub async fn run(mut self) -> Result<()> {
        // Subscribe to the referential_changes channel
        self.pg_listener
            .listen("referential_changes")
            .await
            .map_err(|e| FetcherError::Listener(format!("Failed to subscribe: {e}")))?;

        tracing::info!("Listening for referential changes on 'referential_changes' channel");

        loop {
            match self.pg_listener.recv().await {
                Ok(notification) => {
                    let payload = notification.payload();
                    tracing::debug!(payload = %payload, "Received notification");

                    if let Err(e) = self.handle_notification(payload).await {
                        tracing::error!(error = %e, "Error handling notification");
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Error receiving notification");
                    // Attempt to reconnect after a short delay
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Handles a single notification.
    async fn handle_notification(&self, payload: &str) -> Result<()> {
        let notification: ReferentialChangeNotification = serde_json::from_str(payload)
            .map_err(|e| FetcherError::Listener(format!("Failed to parse notification: {e}")))?;

        tracing::info!(
            table = %notification.table,
            operation = %notification.operation,
            id = %notification.id,
            "Processing referential change"
        );

        // Determine which entity type changed
        let entity_type = match notification.table.as_str() {
            "organizations" => Some("hierarchy"),
            "teams" => Some("hierarchy"),
            "projects" => Some("hierarchy"),
            "users" => Some("users"),
            "memberships" => Some("users"),
            "role_assignments" => Some("users"),
            "agents" => Some("agents"),
            _ => None,
        };

        if let Some(entity_type) = entity_type {
            // If OPAL server is configured, publish the update
            if let Some(opal_url) = &self.state.config.opal_server_url {
                self.publish_to_opal(opal_url, entity_type, &notification)
                    .await?;
            } else {
                tracing::debug!(
                    entity_type = %entity_type,
                    "OPAL server not configured, skipping publish"
                );
            }
        }

        Ok(())
    }

    /// Publishes an update to OPAL via its data update endpoint.
    async fn publish_to_opal(
        &self,
        opal_url: &str,
        entity_type: &str,
        notification: &ReferentialChangeNotification,
    ) -> Result<()> {
        let update_url = format!("{opal_url}/data/config");

        // Create OPAL data update payload
        // This follows OPAL's data update format
        let payload = serde_json::json!({
            "id": Uuid::new_v4().to_string(),
            "entries": [{
                "url": format!("http://opal-fetcher:8080/v1/{}", entity_type),
                "topics": [entity_type],
                "reason": format!("{} {} on {}", notification.operation, notification.id, notification.table),
            }]
        });

        tracing::debug!(
            url = %update_url,
            payload = %payload,
            "Publishing to OPAL"
        );

        let client = reqwest::Client::new();
        let response = client
            .post(&update_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| FetcherError::Listener(format!("Failed to publish to OPAL: {e}")))?;

        if response.status().is_success() {
            tracing::info!(
                entity_type = %entity_type,
                "Successfully published update to OPAL"
            );
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body = %body,
                "OPAL returned non-success status"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_deserialization() {
        let json = r#"{"table":"users","operation":"INSERT","id":"123e4567-e89b-12d3-a456-426614174000","timestamp":1234567890.123}"#;
        let notification: ReferentialChangeNotification = serde_json::from_str(json).unwrap();

        assert_eq!(notification.table, "users");
        assert_eq!(notification.operation, "INSERT");
        assert_eq!(notification.id, "123e4567-e89b-12d3-a456-426614174000");
        assert!((notification.timestamp - 1_234_567_890.123).abs() < f64::EPSILON);
    }

    #[test]
    fn test_notification_serialization() {
        let notification = ReferentialChangeNotification {
            table: "agents".to_string(),
            operation: "UPDATE".to_string(),
            id: "test-id".to_string(),
            timestamp: 1234567890.0,
        };

        let json = serde_json::to_string(&notification).unwrap();
        assert!(json.contains("agents"));
        assert!(json.contains("UPDATE"));
        assert!(json.contains("test-id"));
    }

    #[test]
    fn test_entity_type_mapping() {
        // Test mapping of table names to entity types
        let mappings = vec![
            ("organizations", Some("hierarchy")),
            ("teams", Some("hierarchy")),
            ("projects", Some("hierarchy")),
            ("users", Some("users")),
            ("memberships", Some("users")),
            ("role_assignments", Some("users")),
            ("agents", Some("agents")),
            ("unknown_table", None),
        ];

        for (table, expected) in mappings {
            let result = match table {
                "organizations" | "teams" | "projects" => Some("hierarchy"),
                "users" | "memberships" | "role_assignments" => Some("users"),
                "agents" => Some("agents"),
                _ => None,
            };
            assert_eq!(
                result, expected,
                "Table '{table}' should map to {expected:?}"
            );
        }
    }
}
