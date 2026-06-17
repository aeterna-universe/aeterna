use knowledge::context_architect::{
    BatchedSummarizer, BudgetAwareSummaryGenerator, BudgetAwareSummaryRequest, BudgetError,
    BudgetExhaustedAction, BudgetStatus, BudgetTracker, BudgetTrackerConfig, LlmClient, LlmError,
    SummarizationBudget, SummaryGenerator, SummaryGeneratorConfig, TieredModelConfig,
};
use mk_core::types::{MemoryLayer, SummaryDepth};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

struct MockLlmClient {
    call_count: AtomicU32,
    response: String,
}

impl MockLlmClient {
    fn new(response: &str) -> Self {
        Self {
            call_count: AtomicU32::new(0),
            response: response.to_string(),
        }
    }

    fn _calls(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl LlmClient for MockLlmClient {
    async fn complete(&self, _prompt: &str) -> Result<String, LlmError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }

    async fn complete_with_system(&self, _system: &str, _user: &str) -> Result<String, LlmError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        Ok(self.response.clone())
    }
}

#[test]
fn test_budget_tracker_initialization() {
    let tracker = BudgetTracker::new(BudgetTrackerConfig::default());
    let check = tracker.check(None);

    assert_eq!(check.status, BudgetStatus::Available);
    assert_eq!(check.daily_used, 0);
    assert_eq!(check.hourly_used, 0);
    assert!(check.can_proceed());
    assert_eq!(check.tokens_available(), 100_000);
}

#[test]
fn test_budget_tracker_usage_recording() {
    let tracker = BudgetTracker::new(BudgetTrackerConfig::default());

    tracker.record_usage(5000, MemoryLayer::Session);
    tracker.record_usage(3000, MemoryLayer::Project);

    let check = tracker.check(None);
    assert_eq!(check.daily_used, 8000);
    assert_eq!(check.hourly_used, 8000);

    let session_check = tracker.check(Some(MemoryLayer::Session));
    assert_eq!(session_check.layer_used, Some(5000));

    let project_check = tracker.check(Some(MemoryLayer::Project));
    assert_eq!(project_check.layer_used, Some(3000));
}

#[test]
fn test_budget_threshold_transitions() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(10_000)
            .with_hourly_limit(10_000)
            .with_warning_threshold(50)
            .with_critical_threshold(80),
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    let check = tracker.check(None);
    assert_eq!(check.status, BudgetStatus::Available);

    tracker.record_usage(5500, MemoryLayer::Session);
    let check = tracker.check(None);
    assert_eq!(check.status, BudgetStatus::Warning);

    tracker.record_usage(3000, MemoryLayer::Session);
    let check = tracker.check(None);
    assert_eq!(check.status, BudgetStatus::Critical);

    tracker.record_usage(1500, MemoryLayer::Session);
    let check = tracker.check(None);
    assert_eq!(check.status, BudgetStatus::Exhausted);
}

#[test]
fn test_budget_exhaustion_reject_mode() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(1000)
            .with_hourly_limit(1000),
        exhausted_action: BudgetExhaustedAction::Reject,
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(800, MemoryLayer::Session);

    let result = tracker.try_consume(300, MemoryLayer::Session);
    assert!(result.is_err());

    match result {
        Err(BudgetError::RequestTooLarge {
            requested,
            available,
        }) => {
            assert_eq!(requested, 300);
            assert_eq!(available, 200);
        }
        _ => panic!("Expected RequestTooLarge error"),
    }
}

#[test]
fn test_budget_exhaustion_queue_mode() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(1000)
            .with_hourly_limit(1000),
        exhausted_action: BudgetExhaustedAction::Queue,
        enable_alerts: false,
        queue_max_size: 5,
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(1000, MemoryLayer::Session);

    let _ = tracker.try_consume(100, MemoryLayer::Session);
    let _ = tracker.try_consume(200, MemoryLayer::Project);
    let _ = tracker.try_consume(150, MemoryLayer::Team);

    assert_eq!(tracker.queued_count(), 3);

    let drained = tracker.drain_queue(350);
    assert_eq!(drained.len(), 2);
    assert_eq!(tracker.queued_count(), 1);
}

#[test]
fn test_budget_queue_full_error() {
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
fn test_per_layer_budget_limits() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(1_000_000)
            .with_hourly_limit(100_000)
            .with_layer_limit(MemoryLayer::Session, 5000),
        exhausted_action: BudgetExhaustedAction::Reject,
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(4500, MemoryLayer::Session);

    let result = tracker.try_consume(600, MemoryLayer::Session);
    assert!(result.is_err());

    let result = tracker.try_consume(500, MemoryLayer::Session);
    assert!(result.is_ok());
}

#[test]
fn test_budget_metrics() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(100_000)
            .with_hourly_limit(10_000),
        exhausted_action: BudgetExhaustedAction::Queue,
        enable_alerts: false,
        queue_max_size: 10,
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(25_000, MemoryLayer::Session);

    let metrics = tracker.get_metrics();

    assert_eq!(metrics.daily_tokens_used, 25_000);
    assert_eq!(metrics.daily_tokens_remaining, 75_000);
    assert_eq!(metrics.hourly_tokens_used, 25_000);
    assert_eq!(metrics.hourly_tokens_remaining, 0);
    assert!((metrics.percent_used - 25.0).abs() < 0.1);
    assert_eq!(metrics.status, BudgetStatus::Exhausted);
    assert_eq!(metrics.queued_requests, 0);
}

#[test]
fn test_tiered_model_selection() {
    let config = TieredModelConfig::default();

    assert_eq!(config.model_for_layer(MemoryLayer::Agent), "gpt-4");
    assert_eq!(config.model_for_layer(MemoryLayer::User), "gpt-4");
    assert_eq!(config.model_for_layer(MemoryLayer::Session), "gpt-4");

    assert_eq!(
        config.model_for_layer(MemoryLayer::Project),
        "gpt-3.5-turbo"
    );
    assert_eq!(config.model_for_layer(MemoryLayer::Team), "gpt-3.5-turbo");
    assert_eq!(config.model_for_layer(MemoryLayer::Org), "gpt-3.5-turbo");
    assert_eq!(config.model_for_layer(MemoryLayer::Tenant), "gpt-3.5-turbo");
}

#[test]
fn test_tiered_model_custom_configuration() {
    let config = TieredModelConfig::default()
        .with_expensive_model("claude-3-opus".to_string())
        .with_cheap_model("claude-3-haiku".to_string());

    assert_eq!(config.model_for_layer(MemoryLayer::User), "claude-3-opus");
    assert_eq!(
        config.model_for_layer(MemoryLayer::Tenant),
        "claude-3-haiku"
    );
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

fn create_budget_tracker(daily_limit: u64, hourly_limit: u64) -> Arc<BudgetTracker> {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(daily_limit)
            .with_hourly_limit(hourly_limit),
        exhausted_action: BudgetExhaustedAction::Reject,
        enable_alerts: false,
        queue_max_size: 10,
    };
    Arc::new(BudgetTracker::new(config))
}

fn create_budget_aware_generator(
    response: &str,
    budget_tracker: Arc<BudgetTracker>,
) -> BudgetAwareSummaryGenerator<MockLlmClient> {
    let mock = Arc::new(MockLlmClient::new(response));
    let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());
    BudgetAwareSummaryGenerator::new(generator, budget_tracker, TieredModelConfig::default())
}

#[tokio::test]
async fn test_budget_aware_generator_success() {
    let tracker = create_budget_tracker(1_000_000, 100_000);
    let budget_generator = create_budget_aware_generator("Summarized content.", tracker.clone());

    let request = BudgetAwareSummaryRequest {
        content: "Test content to summarize that is long enough for processing.".to_string(),
        depth: SummaryDepth::Sentence,
        layer: MemoryLayer::Session,
        context: None,
        personalization_context: None,
    };

    let result = budget_generator.generate_with_budget(request).await;
    assert!(result.is_ok());

    let result = result.unwrap();
    assert!(!result.summary.content.is_empty());
    assert!(result.tokens_used > 0);
    assert_eq!(result.model_used, "gpt-4");
}

#[tokio::test]
async fn test_budget_aware_generator_exhausted() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(100)
            .with_hourly_limit(100),
        exhausted_action: BudgetExhaustedAction::Reject,
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = Arc::new(BudgetTracker::new(config));
    tracker.record_usage(100, MemoryLayer::Session);

    let budget_generator = create_budget_aware_generator("Summarized content.", tracker.clone());

    let request = BudgetAwareSummaryRequest {
        content: "Test content that is long enough for processing in this scenario.".to_string(),
        depth: SummaryDepth::Sentence,
        layer: MemoryLayer::Session,
        context: None,
        personalization_context: None,
    };

    let result = budget_generator.generate_with_budget(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_batched_summarizer_layer_prioritization() {
    let tracker = create_budget_tracker(1_000_000, 100_000);

    let mock = Arc::new(MockLlmClient::new("Summary."));
    let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());
    let budget_generator =
        BudgetAwareSummaryGenerator::new(generator, tracker.clone(), TieredModelConfig::default());
    let batched = BatchedSummarizer::new(Arc::new(budget_generator));

    let requests = vec![
        BudgetAwareSummaryRequest {
            content: "User content that is long enough to process.".to_string(),
            layer: MemoryLayer::User,
            depth: SummaryDepth::Sentence,
            context: None,
            personalization_context: None,
        },
        BudgetAwareSummaryRequest {
            content: "Tenant content that is long enough to process.".to_string(),
            layer: MemoryLayer::Tenant,
            depth: SummaryDepth::Sentence,
            context: None,
            personalization_context: None,
        },
        BudgetAwareSummaryRequest {
            content: "Team content that is long enough to process.".to_string(),
            layer: MemoryLayer::Team,
            depth: SummaryDepth::Sentence,
            context: None,
            personalization_context: None,
        },
    ];

    let batch_result = batched.process_batch(requests).await;

    let layer_order: Vec<_> = batch_result.layer_results.iter().map(|r| r.layer).collect();

    let tenant_idx = layer_order.iter().position(|&l| l == MemoryLayer::Tenant);
    let team_idx = layer_order.iter().position(|&l| l == MemoryLayer::Team);
    let user_idx = layer_order.iter().position(|&l| l == MemoryLayer::User);

    assert!(tenant_idx < team_idx);
    assert!(team_idx < user_idx);
}

#[tokio::test]
async fn test_batched_summarizer_partial_budget() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(200)
            .with_hourly_limit(200),
        exhausted_action: BudgetExhaustedAction::Reject,
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = Arc::new(BudgetTracker::new(config));

    let mock = Arc::new(MockLlmClient::new("Summary."));
    let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());
    let budget_generator =
        BudgetAwareSummaryGenerator::new(generator, tracker.clone(), TieredModelConfig::default());
    let batched = BatchedSummarizer::new(Arc::new(budget_generator));

    let requests = vec![
        BudgetAwareSummaryRequest {
            content: "First content that is long enough.".to_string(),
            layer: MemoryLayer::Tenant,
            depth: SummaryDepth::Sentence,
            context: None,
            personalization_context: None,
        },
        BudgetAwareSummaryRequest {
            content: "Second content that is long enough.".to_string(),
            layer: MemoryLayer::User,
            depth: SummaryDepth::Sentence,
            context: None,
            personalization_context: None,
        },
    ];

    let batch_result = batched.process_batch(requests).await;
    assert!(batch_result.successful_count >= 1);
    assert!(batch_result.total_tokens > 0);
}

#[test]
fn test_budget_check_can_proceed() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default().with_daily_limit(1000),
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    let check = tracker.check(None);
    assert!(check.can_proceed());

    tracker.record_usage(1000, MemoryLayer::Session);
    let check = tracker.check(None);
    assert!(!check.can_proceed());
}

#[test]
fn test_budget_check_tokens_available_respects_all_limits() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(10_000)
            .with_hourly_limit(5_000)
            .with_layer_limit(MemoryLayer::Session, 1_000),
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    let check = tracker.check(Some(MemoryLayer::Session));
    assert_eq!(check.tokens_available(), 1_000);

    tracker.record_usage(500, MemoryLayer::Session);
    let check = tracker.check(Some(MemoryLayer::Session));
    assert_eq!(check.tokens_available(), 500);
}

#[test]
fn test_allow_with_warning_mode() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default()
            .with_daily_limit(100)
            .with_hourly_limit(100),
        exhausted_action: BudgetExhaustedAction::AllowWithWarning,
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(100, MemoryLayer::Session);

    let result = tracker.try_consume(50, MemoryLayer::Session);
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_budget_generator_model_selection_by_layer() {
    let tracker = create_budget_tracker(1_000_000, 100_000);
    let model_config = TieredModelConfig::default()
        .with_expensive_model("expensive-model".to_string())
        .with_cheap_model("cheap-model".to_string());

    let mock = Arc::new(MockLlmClient::new("Summary."));
    let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());
    let budget_generator =
        BudgetAwareSummaryGenerator::new(generator, tracker.clone(), model_config);

    let user_request = BudgetAwareSummaryRequest {
        content: "User content that is long enough to process.".to_string(),
        layer: MemoryLayer::User,
        depth: SummaryDepth::Sentence,
        context: None,
        personalization_context: None,
    };
    let result = budget_generator
        .generate_with_budget(user_request)
        .await
        .unwrap();
    assert_eq!(result.model_used, "expensive-model");

    let company_request = BudgetAwareSummaryRequest {
        content: "Tenant content that is long enough to process.".to_string(),
        layer: MemoryLayer::Tenant,
        depth: SummaryDepth::Sentence,
        context: None,
        personalization_context: None,
    };
    let result = budget_generator
        .generate_with_budget(company_request)
        .await
        .unwrap();
    assert_eq!(result.model_used, "cheap-model");
}

#[test]
fn test_multi_tenant_budget_isolation() {
    let tracker1 = BudgetTracker::new(BudgetTrackerConfig::default());
    let tracker2 = BudgetTracker::new(BudgetTrackerConfig::default());

    tracker1.record_usage(50_000, MemoryLayer::Session);

    let check1 = tracker1.check(None);
    let check2 = tracker2.check(None);

    assert_eq!(check1.daily_used, 50_000);
    assert_eq!(check2.daily_used, 0);
}

#[test]
fn test_budget_percent_used_calculation() {
    let config = BudgetTrackerConfig {
        budget: SummarizationBudget::default().with_daily_limit(10_000),
        enable_alerts: false,
        ..Default::default()
    };
    let tracker = BudgetTracker::new(config);

    tracker.record_usage(2500, MemoryLayer::Session);
    let check = tracker.check(None);
    assert!((check.percent_used - 25.0).abs() < 0.01);

    tracker.record_usage(2500, MemoryLayer::Session);
    let check = tracker.check(None);
    assert!((check.percent_used - 50.0).abs() < 0.01);
}

#[test]
fn test_budget_tracker_config_defaults() {
    let config = BudgetTrackerConfig::default();

    assert_eq!(config.budget.daily_token_limit, 1_000_000);
    assert_eq!(config.budget.hourly_token_limit, 100_000);
    assert_eq!(config.budget.warning_threshold_percent, 80);
    assert_eq!(config.budget.critical_threshold_percent, 90);
    assert!(matches!(
        config.exhausted_action,
        BudgetExhaustedAction::Reject
    ));
    assert!(config.enable_alerts);
    assert_eq!(config.queue_max_size, 100);
}

#[test]
fn test_layer_default_limits() {
    let budget = SummarizationBudget::default();

    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Agent),
        Some(&10_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::User),
        Some(&20_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Session),
        Some(&50_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Project),
        Some(&100_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Team),
        Some(&200_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Org),
        Some(&500_000)
    );
    assert_eq!(
        budget.per_layer_limits.get(&MemoryLayer::Tenant),
        Some(&1_000_000)
    );
}
