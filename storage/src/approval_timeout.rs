//! Section 13.9: Approval Workflow Timeout
//!
//! Implements configurable approval timeouts, reminders, and escalation
//! for governance workflows.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Timelike, Utc};
use tokio::sync::RwLock;
use tokio::time::{interval, sleep};
use tracing::{debug, info, warn};

/// Approval timeout manager.
pub struct ApprovalTimeoutManager {
    timeouts: Arc<RwLock<HashMap<String, ApprovalTimeout>>>,
    notifications: Arc<dyn NotificationService>,
    check_interval: Duration
}

/// Approval timeout configuration.
#[derive(Debug, Clone)]
pub struct ApprovalTimeout {
    pub request_id: String,
    pub governance_level: GovernanceLevel,
    pub timeout_hours: u64,
    pub created_at: DateTime<Utc>,
    pub reminder_50_sent: bool,
    pub reminder_75_sent: bool,
    pub escalated: bool,
    pub status: TimeoutStatus
}

/// Governance level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GovernanceLevel {
    Tenant,
    Organization,
    Team,
    Project
}

/// Timeout status.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeoutStatus {
    Active,
    Reminded,
    Escalated,
    Expired,
    Resolved
}

/// Notification service trait.
#[async_trait::async_trait]
pub trait NotificationService: Send + Sync {
    async fn send_reminder(&self, request_id: &str, percentage: u8);
    async fn send_escalation(&self, request_id: &str, escalated_to: &str);
    async fn send_expiration_notice(&self, request_id: &str);
}

impl ApprovalTimeoutManager {
    /// Create new timeout manager.
    pub fn new(notifications: Arc<dyn NotificationService>, check_interval_mins: u64) -> Self {
        Self {
            timeouts: Arc::new(RwLock::new(HashMap::new())),
            notifications,
            check_interval: Duration::from_secs(check_interval_mins * 60)
        }
    }

    /// Start timeout monitor.
    pub async fn start(&self) {
        let timeouts = self.timeouts.clone();
        let notifications = self.notifications.clone();
        let check_interval = self.check_interval;

        tokio::spawn(async move {
            let mut interval = interval(check_interval);

            loop {
                interval.tick().await;

                Self::check_timeouts(&timeouts, &notifications).await;
            }
        });
    }

    /// Register new approval request with timeout (13.9.1).
    pub async fn register_request(
        &self,
        request_id: &str,
        level: GovernanceLevel,
        timeout_hours: u64
    ) {
        let timeout = ApprovalTimeout {
            request_id: request_id.to_string(),
            governance_level: level,
            timeout_hours,
            created_at: Utc::now(),
            reminder_50_sent: false,
            reminder_75_sent: false,
            escalated: false,
            status: TimeoutStatus::Active
        };

        self.timeouts
            .write()
            .await
            .insert(request_id.to_string(), timeout);

        info!(
            "Registered approval request {} with {} hour timeout ({:?} level)",
            request_id, timeout_hours, level
        );
    }

    /// Check all timeouts and send notifications.
    async fn check_timeouts(
        timeouts: &Arc<RwLock<HashMap<String, ApprovalTimeout>>>,
        notifications: &Arc<dyn NotificationService>
    ) {
        let mut timeouts_guard = timeouts.write().await;
        let now = Utc::now();

        for timeout in timeouts_guard.values_mut() {
            if timeout.status == TimeoutStatus::Resolved || timeout.status == TimeoutStatus::Expired
            {
                continue;
            }

            let elapsed = now - timeout.created_at;
            let elapsed_hours = elapsed.num_hours() as f64;
            let progress = elapsed_hours / timeout.timeout_hours as f64;

            // 13.9.2: Send reminders at 50% and 75%
            if progress >= 0.5 && !timeout.reminder_50_sent {
                info!("Sending 50% reminder for request {}", timeout.request_id);
                notifications.send_reminder(&timeout.request_id, 50).await;
                timeout.reminder_50_sent = true;
                timeout.status = TimeoutStatus::Reminded;
            }

            if progress >= 0.75 && !timeout.reminder_75_sent {
                info!("Sending 75% reminder for request {}", timeout.request_id);
                notifications.send_reminder(&timeout.request_id, 75).await;
                timeout.reminder_75_sent = true;
            }

            // 13.9.3: Escalate to next tier
            if progress >= 0.9 && !timeout.escalated {
                let escalated_to = Self::get_escalation_target(timeout.governance_level);
                info!(
                    "Escalating request {} to {}",
                    timeout.request_id, escalated_to
                );
                notifications
                    .send_escalation(&timeout.request_id, &escalated_to)
                    .await;
                timeout.escalated = true;
                timeout.status = TimeoutStatus::Escalated;
            }

            // 13.9.4: Auto-close expired
            if elapsed_hours >= timeout.timeout_hours as f64 {
                warn!("Request {} has expired", timeout.request_id);
                notifications
                    .send_expiration_notice(&timeout.request_id)
                    .await;
                timeout.status = TimeoutStatus::Expired;

                // 13.9.5: Log timeout event
                Self::log_timeout_event(timeout);
            }
        }
    }

    /// Get escalation target for governance level.
    fn get_escalation_target(level: GovernanceLevel) -> String {
        match level {
            GovernanceLevel::Project => "team_lead".to_string(),
            GovernanceLevel::Team => "org_manager".to_string(),
            GovernanceLevel::Organization => "company_admin".to_string(),
            GovernanceLevel::Tenant => "executive".to_string()
        }
    }

    /// Log timeout event (13.9.5).
    fn log_timeout_event(timeout: &ApprovalTimeout) {
        info!(
            "AUDIT: Approval timeout - request_id={}, level={:?}, timeout_hours={}",
            timeout.request_id, timeout.governance_level, timeout.timeout_hours
        );
    }

    /// Mark request as resolved.
    pub async fn resolve_request(&self, request_id: &str) {
        if let Some(timeout) = self.timeouts.write().await.get_mut(request_id) {
            timeout.status = TimeoutStatus::Resolved;
            info!("Request {} marked as resolved", request_id);
        }
    }

    /// Get timeout for request.
    pub async fn get_timeout(&self, request_id: &str) -> Option<ApprovalTimeout> {
        self.timeouts.read().await.get(request_id).cloned()
    }
}

/// Default notification service.
pub struct DefaultNotificationService;

#[async_trait::async_trait]
impl NotificationService for DefaultNotificationService {
    async fn send_reminder(&self, request_id: &str, percentage: u8) {
        println!(
            "🔔 Reminder: Request {} is {}% toward timeout",
            request_id, percentage
        );
    }

    async fn send_escalation(&self, request_id: &str, escalated_to: &str) {
        println!(
            "⚠️  Escalation: Request {} escalated to {}",
            request_id, escalated_to
        );
    }

    async fn send_expiration_notice(&self, request_id: &str) {
        println!("⏰ Expired: Request {} has expired", request_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_timeout_registration() {
        let notifications = Arc::new(DefaultNotificationService);
        let manager = ApprovalTimeoutManager::new(notifications, 1);

        manager
            .register_request("req-1", GovernanceLevel::Team, 24)
            .await;

        let timeout = manager.get_timeout("req-1").await;
        assert!(timeout.is_some());
        assert_eq!(timeout.unwrap().timeout_hours, 24);
    }
}
