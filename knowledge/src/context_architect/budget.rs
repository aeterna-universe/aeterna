use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mk_core::types::MemoryLayer;
use tracing::{info_span, warn};

#[derive(Debug, Clone)]
pub struct SummarizationBudget {
    pub daily_token_limit: u64,
    pub hourly_token_limit: u64,
    pub per_layer_limits: HashMap<MemoryLayer, u64>,
    pub warning_threshold_percent: u8,
    pub critical_threshold_percent: u8,
}

impl Default for SummarizationBudget {
    fn default() -> Self {
        let mut per_layer_limits = HashMap::new();
        per_layer_limits.insert(MemoryLayer::Agent, 10_000);
        per_layer_limits.insert(MemoryLayer::User, 20_000);
        per_layer_limits.insert(MemoryLayer::Session, 50_000);
        per_layer_limits.insert(MemoryLayer::Project, 100_000);
        per_layer_limits.insert(MemoryLayer::Team, 200_000);
        per_layer_limits.insert(MemoryLayer::Org, 500_000);
        per_layer_limits.insert(MemoryLayer::Tenant, 1_000_000);

        Self {
            daily_token_limit: 1_000_000,
            hourly_token_limit: 100_000,
            per_layer_limits,
            warning_threshold_percent: 80,
            critical_threshold_percent: 90,
        }
    }
}

impl SummarizationBudget {
    pub fn with_daily_limit(mut self, limit: u64) -> Self {
        self.daily_token_limit = limit;
        self
    }

    pub fn with_hourly_limit(mut self, limit: u64) -> Self {
        self.hourly_token_limit = limit;
        self
    }

    pub fn with_layer_limit(mut self, layer: MemoryLayer, limit: u64) -> Self {
        self.per_layer_limits.insert(layer, limit);
        self
    }

    pub fn with_warning_threshold(mut self, percent: u8) -> Self {
        self.warning_threshold_percent = percent.min(100);
        self
    }

    pub fn with_critical_threshold(mut self, percent: u8) -> Self {
        self.critical_threshold_percent = percent.min(100);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetStatus {
    Available,
    Warning,
    Critical,
    Exhausted,
}

#[derive(Debug, Clone)]
pub struct BudgetCheck {
    pub status: BudgetStatus,
    pub daily_used: u64,
    pub daily_remaining: u64,
    pub hourly_used: u64,
    pub hourly_remaining: u64,
    pub layer_used: Option<u64>,
    pub layer_remaining: Option<u64>,
    pub percent_used: f32,
}

impl BudgetCheck {
    pub fn can_proceed(&self) -> bool {
        self.status != BudgetStatus::Exhausted
    }

    pub fn tokens_available(&self) -> u64 {
        self.daily_remaining
            .min(self.hourly_remaining)
            .min(self.layer_remaining.unwrap_or(u64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetExhaustedAction {
    Reject,
    Queue,
    AllowWithWarning,
}

#[derive(Debug, Clone)]
pub struct BudgetTrackerConfig {
    pub budget: SummarizationBudget,
    pub exhausted_action: BudgetExhaustedAction,
    pub enable_alerts: bool,
    pub queue_max_size: usize,
}

impl Default for BudgetTrackerConfig {
    fn default() -> Self {
        Self {
            budget: SummarizationBudget::default(),
            exhausted_action: BudgetExhaustedAction::Reject,
            enable_alerts: true,
            queue_max_size: 100,
        }
    }
}

const DAILY_WINDOW_SECS: u64 = 86400;
const HOURLY_WINDOW_SECS: u64 = 3600;
const ALERT_COOLDOWN_SECS: u64 = 300;
const EXHAUSTED_ALERT_COOLDOWN_SECS: u64 = 60;

struct UsageWindow {
    tokens_used: AtomicU64,
    window_start: AtomicU64,
}

impl UsageWindow {
    fn new() -> Self {
        Self {
            tokens_used: AtomicU64::new(0),
            window_start: AtomicU64::new(current_timestamp()),
        }
    }

    fn record(&self, tokens: u64, window_duration_secs: u64) {
        let now = current_timestamp();
        let window_start = self.window_start.load(Ordering::Relaxed);

        if now - window_start >= window_duration_secs {
            self.tokens_used.store(tokens, Ordering::Relaxed);
            self.window_start.store(now, Ordering::Relaxed);
        } else {
            self.tokens_used.fetch_add(tokens, Ordering::Relaxed);
        }
    }

    fn used(&self, window_duration_secs: u64) -> u64 {
        let now = current_timestamp();
        let window_start = self.window_start.load(Ordering::Relaxed);

        if now - window_start >= window_duration_secs {
            0
        } else {
            self.tokens_used.load(Ordering::Relaxed)
        }
    }

    fn reset_if_expired(&self, window_duration_secs: u64) -> bool {
        let now = current_timestamp();
        let window_start = self.window_start.load(Ordering::Relaxed);

        if now - window_start >= window_duration_secs {
            self.tokens_used.store(0, Ordering::Relaxed);
            self.window_start.store(now, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}

struct LayerUsage {
    usage: RwLock<HashMap<MemoryLayer, UsageWindow>>,
}

impl LayerUsage {
    fn new() -> Self {
        Self {
            usage: RwLock::new(HashMap::new()),
        }
    }

    fn record(&self, layer: MemoryLayer, tokens: u64) {
        {
            let usage = self.usage.read().unwrap();
            if let Some(window) = usage.get(&layer) {
                window.record(tokens, DAILY_WINDOW_SECS);
                return;
            }
        }

        let mut usage = self.usage.write().unwrap();
        let window = usage.entry(layer).or_insert_with(UsageWindow::new);
        window.record(tokens, DAILY_WINDOW_SECS);
    }

    fn used(&self, layer: MemoryLayer) -> u64 {
        let usage = self.usage.read().unwrap();
        usage
            .get(&layer)
            .map(|w| w.used(DAILY_WINDOW_SECS))
            .unwrap_or(0)
    }
}

pub struct BudgetTracker {
    config: BudgetTrackerConfig,
    daily_usage: UsageWindow,
    hourly_usage: UsageWindow,
    layer_usage: LayerUsage,
    alert_triggered_warning: AtomicU64,
    alert_triggered_critical: AtomicU64,
    alert_triggered_exhausted: AtomicU64,
    queued_requests: RwLock<Vec<QueuedRequest>>,
}

#[derive(Debug, Clone)]
pub struct QueuedRequest {
    pub layer: MemoryLayer,
    pub estimated_tokens: u64,
    pub queued_at: u64,
    pub request_id: String,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BudgetError {
    #[error("Budget exhausted: {reason}")]
    Exhausted { reason: String },

    #[error("Queue full: max size {max_size} reached")]
    QueueFull { max_size: usize },

    #[error("Request too large: {requested} tokens exceeds available {available}")]
    RequestTooLarge { requested: u64, available: u64 },
}

impl BudgetTracker {
    pub fn new(config: BudgetTrackerConfig) -> Self {
        Self {
            config,
            daily_usage: UsageWindow::new(),
            hourly_usage: UsageWindow::new(),
            layer_usage: LayerUsage::new(),
            alert_triggered_warning: AtomicU64::new(0),
            alert_triggered_critical: AtomicU64::new(0),
            alert_triggered_exhausted: AtomicU64::new(0),
            queued_requests: RwLock::new(Vec::new()),
        }
    }

    pub fn check(&self, layer: Option<MemoryLayer>) -> BudgetCheck {
        self.daily_usage.reset_if_expired(DAILY_WINDOW_SECS);
        self.hourly_usage.reset_if_expired(HOURLY_WINDOW_SECS);

        let daily_used = self.daily_usage.used(DAILY_WINDOW_SECS);
        let hourly_used = self.hourly_usage.used(HOURLY_WINDOW_SECS);
        let daily_limit = self.config.budget.daily_token_limit;
        let hourly_limit = self.config.budget.hourly_token_limit;

        let daily_remaining = daily_limit.saturating_sub(daily_used);
        let hourly_remaining = hourly_limit.saturating_sub(hourly_used);

        let (layer_used, layer_remaining) = if let Some(l) = layer {
            let used = self.layer_usage.used(l);
            let limit = self
                .config
                .budget
                .per_layer_limits
                .get(&l)
                .copied()
                .unwrap_or(u64::MAX);
            (Some(used), Some(limit.saturating_sub(used)))
        } else {
            (None, None)
        };

        let percent_used = (daily_used as f32 / daily_limit as f32) * 100.0;

        let status = if daily_remaining == 0 || hourly_remaining == 0 || layer_remaining == Some(0)
        {
            BudgetStatus::Exhausted
        } else if percent_used >= self.config.budget.critical_threshold_percent as f32 {
            BudgetStatus::Critical
        } else if percent_used >= self.config.budget.warning_threshold_percent as f32 {
            BudgetStatus::Warning
        } else {
            BudgetStatus::Available
        };

        BudgetCheck {
            status,
            daily_used,
            daily_remaining,
            hourly_used,
            hourly_remaining,
            layer_used,
            layer_remaining,
            percent_used,
        }
    }

    pub fn try_consume(&self, tokens: u64, layer: MemoryLayer) -> Result<BudgetCheck, BudgetError> {
        let span = info_span!(
            "budget.try_consume",
            tokens = tokens,
            layer = ?layer
        );
        let _enter = span.enter();

        let check = self.check(Some(layer));

        if check.tokens_available() < tokens {
            match self.config.exhausted_action {
                BudgetExhaustedAction::Reject => {
                    self.trigger_exhausted_alert();
                    return Err(BudgetError::RequestTooLarge {
                        requested: tokens,
                        available: check.tokens_available(),
                    });
                }
                BudgetExhaustedAction::Queue => {
                    return self.queue_request(layer, tokens);
                }
                BudgetExhaustedAction::AllowWithWarning => {
                    warn!(
                        tokens = tokens,
                        available = check.tokens_available(),
                        "Allowing request despite budget constraints"
                    );
                }
            }
        }

        self.record_usage(tokens, layer);

        let new_check = self.check(Some(layer));
        self.check_and_trigger_alerts(&new_check);

        Ok(new_check)
    }

    pub fn record_usage(&self, tokens: u64, layer: MemoryLayer) {
        self.daily_usage.record(tokens, DAILY_WINDOW_SECS);
        self.hourly_usage.record(tokens, HOURLY_WINDOW_SECS);
        self.layer_usage.record(layer, tokens);
    }

    fn queue_request(&self, layer: MemoryLayer, tokens: u64) -> Result<BudgetCheck, BudgetError> {
        let mut queue = self.queued_requests.write().unwrap();

        if queue.len() >= self.config.queue_max_size {
            return Err(BudgetError::QueueFull {
                max_size: self.config.queue_max_size,
            });
        }

        let request = QueuedRequest {
            layer,
            estimated_tokens: tokens,
            queued_at: current_timestamp(),
            request_id: format!("{}-{}", layer.display_name(), queue.len()),
        };

        queue.push(request);
        drop(queue);

        self.trigger_exhausted_alert();

        Err(BudgetError::Exhausted {
            reason: "Request queued due to budget exhaustion".to_string(),
        })
    }

    pub fn drain_queue(&self, max_tokens: u64) -> Vec<QueuedRequest> {
        let mut queue = self.queued_requests.write().unwrap();
        let mut drained = Vec::new();
        let mut remaining_tokens = max_tokens;

        queue.retain(|req| {
            if req.estimated_tokens <= remaining_tokens {
                remaining_tokens -= req.estimated_tokens;
                drained.push(req.clone());
                false
            } else {
                true
            }
        });

        drained
    }

    pub fn queued_count(&self) -> usize {
        self.queued_requests.read().unwrap().len()
    }

    fn check_and_trigger_alerts(&self, check: &BudgetCheck) {
        if !self.config.enable_alerts {
            return;
        }

        let now = current_timestamp();

        match check.status {
            BudgetStatus::Warning => {
                let last = self.alert_triggered_warning.load(Ordering::Relaxed);
                if now - last >= ALERT_COOLDOWN_SECS {
                    self.alert_triggered_warning.store(now, Ordering::Relaxed);
                    warn!(
                        percent_used = check.percent_used,
                        daily_remaining = check.daily_remaining,
                        "Summarization budget at warning threshold ({}%)",
                        self.config.budget.warning_threshold_percent
                    );
                }
            }
            BudgetStatus::Critical => {
                let last = self.alert_triggered_critical.load(Ordering::Relaxed);
                if now - last >= ALERT_COOLDOWN_SECS {
                    self.alert_triggered_critical.store(now, Ordering::Relaxed);
                    warn!(
                        percent_used = check.percent_used,
                        daily_remaining = check.daily_remaining,
                        "Summarization budget at CRITICAL threshold ({}%)",
                        self.config.budget.critical_threshold_percent
                    );
                }
            }
            BudgetStatus::Exhausted => {
                self.trigger_exhausted_alert();
            }
            BudgetStatus::Available => {}
        }
    }

    fn trigger_exhausted_alert(&self) {
        if !self.config.enable_alerts {
            return;
        }

        let now = current_timestamp();
        let last = self.alert_triggered_exhausted.load(Ordering::Relaxed);

        if now - last >= EXHAUSTED_ALERT_COOLDOWN_SECS {
            self.alert_triggered_exhausted.store(now, Ordering::Relaxed);
            warn!("Summarization budget EXHAUSTED - requests being rejected or queued");
        }
    }

    pub fn get_metrics(&self) -> BudgetMetrics {
        let check = self.check(None);
        let queue_size = self.queued_count();

        BudgetMetrics {
            daily_tokens_used: check.daily_used,
            daily_tokens_remaining: check.daily_remaining,
            hourly_tokens_used: check.hourly_used,
            hourly_tokens_remaining: check.hourly_remaining,
            percent_used: check.percent_used,
            status: check.status,
            queued_requests: queue_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BudgetMetrics {
    pub daily_tokens_used: u64,
    pub daily_tokens_remaining: u64,
    pub hourly_tokens_used: u64,
    pub hourly_tokens_remaining: u64,
    pub percent_used: f32,
    pub status: BudgetStatus,
    pub queued_requests: usize,
}

#[derive(Debug, Clone)]
pub struct TieredModelConfig {
    pub expensive_model: String,
    pub cheap_model: String,
    pub expensive_layers: Vec<MemoryLayer>,
    pub cheap_layers: Vec<MemoryLayer>,
}

impl Default for TieredModelConfig {
    fn default() -> Self {
        Self {
            expensive_model: "gpt-4".to_string(),
            cheap_model: "gpt-3.5-turbo".to_string(),
            expensive_layers: vec![MemoryLayer::User, MemoryLayer::Session, MemoryLayer::Agent],
            cheap_layers: vec![
                MemoryLayer::Tenant,
                MemoryLayer::Org,
                MemoryLayer::Team,
                MemoryLayer::Project,
            ],
        }
    }
}

impl TieredModelConfig {
    pub fn model_for_layer(&self, layer: MemoryLayer) -> &str {
        if self.expensive_layers.contains(&layer) {
            &self.expensive_model
        } else {
            &self.cheap_model
        }
    }

    pub fn with_expensive_model(mut self, model: String) -> Self {
        self.expensive_model = model;
        self
    }

    pub fn with_cheap_model(mut self, model: String) -> Self {
        self.cheap_model = model;
        self
    }
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_summarization_budget_defaults() {
        let budget = SummarizationBudget::default();

        assert_eq!(budget.daily_token_limit, 1_000_000);
        assert_eq!(budget.hourly_token_limit, 100_000);
        assert_eq!(budget.warning_threshold_percent, 80);
        assert_eq!(budget.critical_threshold_percent, 90);
        assert!(budget.per_layer_limits.contains_key(&MemoryLayer::Session));
    }

    #[test]
    fn test_summarization_budget_builder() {
        let budget = SummarizationBudget::default()
            .with_daily_limit(500_000)
            .with_hourly_limit(50_000)
            .with_layer_limit(MemoryLayer::Project, 75_000)
            .with_warning_threshold(70)
            .with_critical_threshold(85);

        assert_eq!(budget.daily_token_limit, 500_000);
        assert_eq!(budget.hourly_token_limit, 50_000);
        assert_eq!(
            budget.per_layer_limits.get(&MemoryLayer::Project),
            Some(&75_000)
        );
        assert_eq!(budget.warning_threshold_percent, 70);
        assert_eq!(budget.critical_threshold_percent, 85);
    }

    #[test]
    fn test_budget_tracker_initial_state() {
        let tracker = BudgetTracker::new(BudgetTrackerConfig::default());
        let check = tracker.check(None);

        assert_eq!(check.status, BudgetStatus::Available);
        assert_eq!(check.daily_used, 0);
        assert_eq!(check.hourly_used, 0);
        assert!(check.can_proceed());
    }

    #[test]
    fn test_budget_tracker_record_usage() {
        let tracker = BudgetTracker::new(BudgetTrackerConfig::default());

        tracker.record_usage(1000, MemoryLayer::Session);
        let check = tracker.check(Some(MemoryLayer::Session));

        assert_eq!(check.daily_used, 1000);
        assert_eq!(check.hourly_used, 1000);
        assert_eq!(check.layer_used, Some(1000));
    }

    #[test]
    fn test_budget_tracker_try_consume_success() {
        let tracker = BudgetTracker::new(BudgetTrackerConfig::default());

        let result = tracker.try_consume(500, MemoryLayer::Session);
        assert!(result.is_ok());

        let check = result.unwrap();
        assert_eq!(check.daily_used, 500);
    }

    #[test]
    fn test_budget_tracker_exhaustion_reject() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(1000)
                .with_hourly_limit(1000),
            exhausted_action: BudgetExhaustedAction::Reject,
            enable_alerts: false,
            ..Default::default()
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(900, MemoryLayer::Session);

        let result = tracker.try_consume(200, MemoryLayer::Session);
        assert!(result.is_err());

        match result {
            Err(BudgetError::RequestTooLarge {
                requested,
                available,
            }) => {
                assert_eq!(requested, 200);
                assert_eq!(available, 100);
            }
            _ => panic!("Expected RequestTooLarge error"),
        }
    }

    #[test]
    fn test_budget_tracker_exhaustion_queue() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(1000)
                .with_hourly_limit(1000),
            exhausted_action: BudgetExhaustedAction::Queue,
            enable_alerts: false,
            queue_max_size: 10,
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(900, MemoryLayer::Session);

        let result = tracker.try_consume(200, MemoryLayer::Session);
        assert!(result.is_err());
        assert_eq!(tracker.queued_count(), 1);
    }

    #[test]
    fn test_budget_tracker_queue_full() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(100)
                .with_hourly_limit(100),
            exhausted_action: BudgetExhaustedAction::Queue,
            enable_alerts: false,
            queue_max_size: 2,
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(100, MemoryLayer::Session);

        let _ = tracker.try_consume(50, MemoryLayer::Session);
        let _ = tracker.try_consume(50, MemoryLayer::Session);
        let result = tracker.try_consume(50, MemoryLayer::Session);

        assert!(matches!(
            result,
            Err(BudgetError::QueueFull { max_size: 2 })
        ));
    }

    #[test]
    fn test_budget_tracker_drain_queue() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(100)
                .with_hourly_limit(100),
            exhausted_action: BudgetExhaustedAction::Queue,
            enable_alerts: false,
            queue_max_size: 10,
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(100, MemoryLayer::Session);

        let _ = tracker.try_consume(30, MemoryLayer::Session);
        let _ = tracker.try_consume(40, MemoryLayer::Session);
        let _ = tracker.try_consume(50, MemoryLayer::Session);

        assert_eq!(tracker.queued_count(), 3);

        let drained = tracker.drain_queue(70);
        assert_eq!(drained.len(), 2);
        assert_eq!(tracker.queued_count(), 1);
    }

    #[test]
    fn test_budget_status_warning() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(1000)
                .with_warning_threshold(50),
            enable_alerts: false,
            ..Default::default()
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(600, MemoryLayer::Session);
        let check = tracker.check(None);

        assert_eq!(check.status, BudgetStatus::Warning);
    }

    #[test]
    fn test_budget_status_critical() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(1000)
                .with_warning_threshold(50)
                .with_critical_threshold(80),
            enable_alerts: false,
            ..Default::default()
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(850, MemoryLayer::Session);
        let check = tracker.check(None);

        assert_eq!(check.status, BudgetStatus::Critical);
    }

    #[test]
    fn test_budget_check_tokens_available() {
        let config = BudgetTrackerConfig {
            budget: SummarizationBudget::default()
                .with_daily_limit(1000)
                .with_hourly_limit(500)
                .with_layer_limit(MemoryLayer::Session, 200),
            ..Default::default()
        };
        let tracker = BudgetTracker::new(config);

        tracker.record_usage(100, MemoryLayer::Session);
        let check = tracker.check(Some(MemoryLayer::Session));

        assert_eq!(check.tokens_available(), 100);
    }

    #[test]
    fn test_budget_metrics() {
        let tracker = BudgetTracker::new(BudgetTrackerConfig::default());
        tracker.record_usage(50_000, MemoryLayer::Session);

        let metrics = tracker.get_metrics();

        assert_eq!(metrics.daily_tokens_used, 50_000);
        assert_eq!(metrics.hourly_tokens_used, 50_000);
        assert!(metrics.percent_used > 0.0);
        assert_eq!(metrics.status, BudgetStatus::Available);
    }

    #[test]
    fn test_tiered_model_config_defaults() {
        let config = TieredModelConfig::default();

        assert_eq!(config.expensive_model, "gpt-4");
        assert_eq!(config.cheap_model, "gpt-3.5-turbo");
        assert!(config.expensive_layers.contains(&MemoryLayer::User));
        assert!(config.cheap_layers.contains(&MemoryLayer::Tenant));
    }

    #[test]
    fn test_tiered_model_selection() {
        let config = TieredModelConfig::default();

        assert_eq!(config.model_for_layer(MemoryLayer::User), "gpt-4");
        assert_eq!(config.model_for_layer(MemoryLayer::Session), "gpt-4");
        assert_eq!(config.model_for_layer(MemoryLayer::Tenant), "gpt-3.5-turbo");
        assert_eq!(
            config.model_for_layer(MemoryLayer::Project),
            "gpt-3.5-turbo"
        );
    }

    #[test]
    fn test_tiered_model_config_builder() {
        let config = TieredModelConfig::default()
            .with_expensive_model("claude-3-opus".to_string())
            .with_cheap_model("claude-3-haiku".to_string());

        assert_eq!(config.expensive_model, "claude-3-opus");
        assert_eq!(config.cheap_model, "claude-3-haiku");
    }

    #[test]
    fn test_multiple_layer_usage() {
        let tracker = BudgetTracker::new(BudgetTrackerConfig::default());

        tracker.record_usage(100, MemoryLayer::Session);
        tracker.record_usage(200, MemoryLayer::Project);
        tracker.record_usage(300, MemoryLayer::Team);

        let check = tracker.check(None);
        assert_eq!(check.daily_used, 600);

        let session_check = tracker.check(Some(MemoryLayer::Session));
        assert_eq!(session_check.layer_used, Some(100));

        let project_check = tracker.check(Some(MemoryLayer::Project));
        assert_eq!(project_check.layer_used, Some(200));
    }
}
