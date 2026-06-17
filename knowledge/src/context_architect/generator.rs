use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use mk_core::types::{LayerSummary, MemoryLayer, SummaryDepth};
use sha2::{Digest, Sha256};
use tracing::{Instrument, info_span, warn};

use super::budget::{BudgetCheck, BudgetError, BudgetTracker, TieredModelConfig};
use super::prompts::PromptTemplates;
use super::{LlmClient, LlmError};

#[derive(Debug, Clone)]
pub struct SummaryGeneratorConfig {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub sentence_max_tokens: u32,
    pub paragraph_max_tokens: u32,
    pub detailed_max_tokens: u32,
}

impl Default for SummaryGeneratorConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            retry_delay_ms: 1000,
            sentence_max_tokens: 50,
            paragraph_max_tokens: 200,
            detailed_max_tokens: 500,
        }
    }
}

pub struct SummaryGenerator<C: LlmClient> {
    client: Arc<C>,
    config: SummaryGeneratorConfig,
    prompts: PromptTemplates,
}

#[derive(Debug, Clone)]
pub struct SummaryRequest {
    pub content: String,
    pub depth: SummaryDepth,
    pub context: Option<String>,
    pub personalization_context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SummaryResult {
    pub summary: LayerSummary,
    pub tokens_used: u32,
}

#[derive(Debug, Clone)]
pub struct BatchSummaryRequest {
    pub requests: Vec<SummaryRequest>,
}

#[derive(Debug, Clone)]
pub struct BatchSummaryResult {
    pub results: Vec<Result<SummaryResult, GenerationError>>,
    pub total_tokens_used: u32,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum GenerationError {
    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("Empty content provided")]
    EmptyContent,

    #[error("Content too short for summarization: {length} chars, minimum {minimum}")]
    ContentTooShort { length: usize, minimum: usize },

    #[error("Token limit exceeded: {actual} > {limit}")]
    TokenLimitExceeded { actual: u32, limit: u32 },

    #[error("Budget exhausted: {0}")]
    BudgetExhausted(#[from] BudgetError),
}

impl<C: LlmClient> SummaryGenerator<C> {
    pub fn new(client: Arc<C>, config: SummaryGeneratorConfig) -> Self {
        Self {
            client,
            config,
            prompts: PromptTemplates::default(),
        }
    }

    pub fn with_prompts(mut self, prompts: PromptTemplates) -> Self {
        self.prompts = prompts;
        self
    }

    pub async fn generate_summary(
        &self,
        content: &str,
        depth: SummaryDepth,
        context: Option<&str>,
    ) -> Result<LayerSummary, GenerationError> {
        self.generate_summary_with_personalization(content, depth, context, None)
            .await
    }

    pub async fn generate_summary_with_personalization(
        &self,
        content: &str,
        depth: SummaryDepth,
        context: Option<&str>,
        personalization_context: Option<&str>,
    ) -> Result<LayerSummary, GenerationError> {
        let span = info_span!(
            "context_architect.generate_summary",
            depth = ?depth,
            content_length = content.len(),
            has_context = context.is_some(),
            has_personalization = personalization_context.is_some()
        );

        async move {
            if content.is_empty() {
                return Err(GenerationError::EmptyContent);
            }

            let min_length = match depth {
                SummaryDepth::Sentence => 20,
                SummaryDepth::Paragraph => 50,
                SummaryDepth::Detailed => 100,
            };

            if content.len() < min_length {
                return Err(GenerationError::ContentTooShort {
                    length: content.len(),
                    minimum: min_length,
                });
            }

            let (system_prompt, user_prompt) = self.prompts.build_prompt(
                content,
                depth,
                context,
                personalization_context,
                self.token_limit_for_depth(depth),
            );

            let response = self
                .client
                .complete_with_system(&system_prompt, &user_prompt)
                .await?;

            let token_count = estimate_tokens(&response);
            let source_hash = compute_content_hash(content);
            let generated_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            let summary_content = response.trim().to_string();
            let content_hash = Some(compute_content_hash(&summary_content));

            Ok(LayerSummary {
                depth,
                content: summary_content,
                token_count,
                generated_at,
                source_hash,
                content_hash,
                personalized: personalization_context.is_some(),
                personalization_context: personalization_context.map(String::from),
            })
        }
        .instrument(span)
        .await
    }

    pub async fn generate_batch(&self, requests: Vec<SummaryRequest>) -> BatchSummaryResult {
        let span = info_span!(
            "context_architect.generate_batch",
            batch_size = requests.len()
        );

        let _enter = span.enter();

        let mut results = Vec::with_capacity(requests.len());
        let mut total_tokens = 0u32;

        for request in requests {
            let result = self
                .generate_summary_with_personalization(
                    &request.content,
                    request.depth,
                    request.context.as_deref(),
                    request.personalization_context.as_deref(),
                )
                .await;

            match &result {
                Ok(summary) => {
                    total_tokens = total_tokens.saturating_add(summary.token_count);
                    results.push(Ok(SummaryResult {
                        summary: summary.clone(),
                        tokens_used: summary.token_count,
                    }));
                }
                Err(e) => {
                    results.push(Err(e.clone()));
                }
            }
        }

        BatchSummaryResult {
            results,
            total_tokens_used: total_tokens,
        }
    }

    pub fn token_limit_for_depth(&self, depth: SummaryDepth) -> u32 {
        match depth {
            SummaryDepth::Sentence => self.config.sentence_max_tokens,
            SummaryDepth::Paragraph => self.config.paragraph_max_tokens,
            SummaryDepth::Detailed => self.config.detailed_max_tokens,
        }
    }
}

pub fn estimate_tokens(text: &str) -> u32 {
    let word_count = text.split_whitespace().count();
    let char_count = text.chars().count();

    // GPT-style estimation: ~4 chars per token, with adjustment for whitespace
    // Using max of word-based and char-based estimates for safety
    let char_based = (char_count as f64 / 4.0).ceil() as u32;
    let word_based = (word_count as f64 * 1.3).ceil() as u32;

    char_based.max(word_based)
}

fn compute_content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8])
}

pub struct BudgetAwareSummaryGenerator<C: LlmClient> {
    generator: SummaryGenerator<C>,
    budget_tracker: Arc<BudgetTracker>,
    model_config: TieredModelConfig,
}

#[derive(Debug, Clone)]
pub struct BudgetAwareSummaryRequest {
    pub content: String,
    pub depth: SummaryDepth,
    pub layer: MemoryLayer,
    pub context: Option<String>,
    pub personalization_context: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BudgetAwareSummaryResult {
    pub summary: LayerSummary,
    pub tokens_used: u32,
    pub budget_check: BudgetCheck,
    pub model_used: String,
}

impl<C: LlmClient> BudgetAwareSummaryGenerator<C> {
    pub fn new(
        generator: SummaryGenerator<C>,
        budget_tracker: Arc<BudgetTracker>,
        model_config: TieredModelConfig,
    ) -> Self {
        Self {
            generator,
            budget_tracker,
            model_config,
        }
    }

    pub fn check_budget(&self, layer: MemoryLayer) -> BudgetCheck {
        self.budget_tracker.check(Some(layer))
    }

    pub fn model_for_layer(&self, layer: MemoryLayer) -> &str {
        self.model_config.model_for_layer(layer)
    }

    pub async fn generate_with_budget(
        &self,
        request: BudgetAwareSummaryRequest,
    ) -> Result<BudgetAwareSummaryResult, GenerationError> {
        let span = info_span!(
            "context_architect.generate_with_budget",
            layer = ?request.layer,
            depth = ?request.depth,
            content_length = request.content.len()
        );

        async move {
            let estimated_tokens = self.estimate_request_tokens(&request);
            let model = self.model_config.model_for_layer(request.layer).to_string();

            let budget_check = self
                .budget_tracker
                .try_consume(estimated_tokens as u64, request.layer)?;

            if !budget_check.can_proceed() {
                warn!(
                    layer = ?request.layer,
                    estimated_tokens = estimated_tokens,
                    "Budget exhausted for summarization request"
                );
                return Err(GenerationError::BudgetExhausted(BudgetError::Exhausted {
                    reason: format!(
                        "Budget exhausted for layer {:?}: {}% used",
                        request.layer, budget_check.percent_used
                    ),
                }));
            }

            let summary = self
                .generator
                .generate_summary_with_personalization(
                    &request.content,
                    request.depth,
                    request.context.as_deref(),
                    request.personalization_context.as_deref(),
                )
                .await?;

            let actual_tokens = summary.token_count;
            let token_diff = actual_tokens as i64 - estimated_tokens as i64;
            if token_diff > 0 {
                self.budget_tracker
                    .record_usage(token_diff as u64, request.layer);
            }

            let final_check = self.budget_tracker.check(Some(request.layer));

            Ok(BudgetAwareSummaryResult {
                summary,
                tokens_used: actual_tokens,
                budget_check: final_check,
                model_used: model,
            })
        }
        .instrument(span)
        .await
    }

    pub async fn generate_batch_with_budget(
        &self,
        requests: Vec<BudgetAwareSummaryRequest>,
    ) -> Vec<Result<BudgetAwareSummaryResult, GenerationError>> {
        let span = info_span!(
            "context_architect.generate_batch_with_budget",
            batch_size = requests.len()
        );
        let _enter = span.enter();

        let mut results = Vec::with_capacity(requests.len());

        for request in requests {
            let budget_check = self.budget_tracker.check(Some(request.layer));
            if !budget_check.can_proceed() {
                results.push(Err(GenerationError::BudgetExhausted(
                    BudgetError::Exhausted {
                        reason: format!(
                            "Budget exhausted before processing: {}% used",
                            budget_check.percent_used
                        ),
                    },
                )));
                continue;
            }

            results.push(self.generate_with_budget(request).await);
        }

        results
    }

    fn estimate_request_tokens(&self, request: &BudgetAwareSummaryRequest) -> u32 {
        let output_limit = self.generator.token_limit_for_depth(request.depth);
        let input_estimate = estimate_tokens(&request.content);
        let context_estimate = request
            .context
            .as_ref()
            .map(|c| estimate_tokens(c))
            .unwrap_or(0);

        input_estimate + context_estimate + output_limit
    }

    pub fn budget_tracker(&self) -> &BudgetTracker {
        &self.budget_tracker
    }
}

pub struct BatchedSummarizer<C: LlmClient> {
    generator: Arc<BudgetAwareSummaryGenerator<C>>,
    max_concurrent: usize,
}

#[derive(Debug, Clone)]
pub struct LayerBatchResult {
    pub layer: MemoryLayer,
    pub results: Vec<Result<BudgetAwareSummaryResult, GenerationError>>,
    pub total_tokens: u64,
    pub model_used: String,
}

#[derive(Debug, Clone)]
pub struct BatchedSummaryResult {
    pub layer_results: Vec<LayerBatchResult>,
    pub total_tokens: u64,
    pub successful_count: usize,
    pub failed_count: usize,
}

impl<C: LlmClient> BatchedSummarizer<C> {
    pub fn new(generator: Arc<BudgetAwareSummaryGenerator<C>>) -> Self {
        Self {
            generator,
            max_concurrent: 4,
        }
    }

    pub fn with_max_concurrent(mut self, max: usize) -> Self {
        self.max_concurrent = max.max(1);
        self
    }

    pub async fn process_batch(
        &self,
        requests: Vec<BudgetAwareSummaryRequest>,
    ) -> BatchedSummaryResult {
        let span = info_span!(
            "context_architect.batched_summarizer.process_batch",
            request_count = requests.len(),
            max_concurrent = self.max_concurrent
        );
        let _enter = span.enter();

        let grouped = self.group_by_layer(requests);
        let layer_order = self.prioritize_layers(&grouped);

        let mut layer_results = Vec::new();
        let mut total_tokens = 0u64;
        let mut successful_count = 0usize;
        let mut failed_count = 0usize;

        for layer in layer_order {
            if let Some(layer_requests) = grouped.get(&layer) {
                let budget_check = self.generator.check_budget(layer);
                if !budget_check.can_proceed() {
                    let failed_results: Vec<_> = layer_requests
                        .iter()
                        .map(|_| {
                            Err(GenerationError::BudgetExhausted(BudgetError::Exhausted {
                                reason: format!("Budget exhausted for layer {:?}", layer),
                            }))
                        })
                        .collect();

                    failed_count += failed_results.len();
                    layer_results.push(LayerBatchResult {
                        layer,
                        results: failed_results,
                        total_tokens: 0,
                        model_used: self.generator.model_for_layer(layer).to_string(),
                    });
                    continue;
                }

                let results = self.process_layer_batch(layer_requests).await;
                let layer_tokens: u64 = results
                    .iter()
                    .filter_map(|r| r.as_ref().ok())
                    .map(|r| r.tokens_used as u64)
                    .sum();

                successful_count += results.iter().filter(|r| r.is_ok()).count();
                failed_count += results.iter().filter(|r| r.is_err()).count();
                total_tokens += layer_tokens;

                layer_results.push(LayerBatchResult {
                    layer,
                    results,
                    total_tokens: layer_tokens,
                    model_used: self.generator.model_for_layer(layer).to_string(),
                });
            }
        }

        BatchedSummaryResult {
            layer_results,
            total_tokens,
            successful_count,
            failed_count,
        }
    }

    fn group_by_layer(
        &self,
        requests: Vec<BudgetAwareSummaryRequest>,
    ) -> std::collections::HashMap<MemoryLayer, Vec<BudgetAwareSummaryRequest>> {
        let mut grouped: std::collections::HashMap<MemoryLayer, Vec<BudgetAwareSummaryRequest>> =
            std::collections::HashMap::new();

        for request in requests {
            grouped.entry(request.layer).or_default().push(request);
        }

        grouped
    }

    fn prioritize_layers(
        &self,
        grouped: &std::collections::HashMap<MemoryLayer, Vec<BudgetAwareSummaryRequest>>,
    ) -> Vec<MemoryLayer> {
        let layer_priority: std::collections::HashMap<MemoryLayer, u8> = [
            (MemoryLayer::Tenant, 0),
            (MemoryLayer::Org, 1),
            (MemoryLayer::Team, 2),
            (MemoryLayer::Project, 3),
            (MemoryLayer::Session, 4),
            (MemoryLayer::User, 5),
            (MemoryLayer::Agent, 6),
        ]
        .into_iter()
        .collect();

        let mut layers: Vec<_> = grouped.keys().copied().collect();
        layers.sort_by_key(|l| layer_priority.get(l).copied().unwrap_or(99));
        layers
    }

    async fn process_layer_batch(
        &self,
        requests: &[BudgetAwareSummaryRequest],
    ) -> Vec<Result<BudgetAwareSummaryResult, GenerationError>> {
        let mut results = Vec::with_capacity(requests.len());

        for request in requests {
            results.push(self.generator.generate_with_budget(request.clone()).await);
        }

        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockLlmClient {
        responses: Mutex<Vec<String>>,
    }

    impl MockLlmClient {
        fn new(responses: Vec<String>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmClient for MockLlmClient {
        async fn complete(&self, _prompt: &str) -> Result<String, LlmError> {
            let mut responses = self.responses.lock().unwrap();
            responses
                .pop()
                .ok_or_else(|| LlmError::InvalidResponse("No mock response".into()))
        }

        async fn complete_with_system(
            &self,
            _system: &str,
            _user: &str,
        ) -> Result<String, LlmError> {
            let mut responses = self.responses.lock().unwrap();
            responses
                .pop()
                .ok_or_else(|| LlmError::InvalidResponse("No mock response".into()))
        }
    }

    #[tokio::test]
    async fn test_generate_sentence_summary() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "This is a concise one-sentence summary.".to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                       tempor incididunt ut labore et dolore magna aliqua.";

        let result = generator
            .generate_summary(content, SummaryDepth::Sentence, None)
            .await
            .unwrap();

        assert_eq!(result.depth, SummaryDepth::Sentence);
        assert!(!result.content.is_empty());
        assert!(!result.source_hash.is_empty());
        assert!(!result.personalized);
    }

    #[tokio::test]
    async fn test_generate_paragraph_summary() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "This is a paragraph summary. It contains multiple sentences that provide more detail \
             about the content."
                .to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                       tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim \
                       veniam, quis nostrud exercitation ullamco laboris.";

        let result = generator
            .generate_summary(content, SummaryDepth::Paragraph, None)
            .await
            .unwrap();

        assert_eq!(result.depth, SummaryDepth::Paragraph);
        assert!(result.token_count > 0);
    }

    #[tokio::test]
    async fn test_generate_detailed_summary() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "This is a detailed summary with comprehensive information. It covers all the key \
             points from the original content. Multiple aspects are addressed including context \
             and implications."
                .to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                       tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim \
                       veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea \
                       commodo consequat. Duis aute irure dolor in reprehenderit in voluptate \
                       velit esse cillum dolore.";

        let result = generator
            .generate_summary(content, SummaryDepth::Detailed, None)
            .await
            .unwrap();

        assert_eq!(result.depth, SummaryDepth::Detailed);
    }

    #[tokio::test]
    async fn test_generate_with_context() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "Summary considering the provided context.".to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit.";
        let context = "This is a technical document about software architecture.";

        let result = generator
            .generate_summary(content, SummaryDepth::Sentence, Some(context))
            .await
            .unwrap();

        assert_eq!(result.depth, SummaryDepth::Sentence);
    }

    #[tokio::test]
    async fn test_generate_with_personalization() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "Personalized summary for developer context.".to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit.";
        let personalization = "developer";

        let result = generator
            .generate_summary_with_personalization(
                content,
                SummaryDepth::Sentence,
                None,
                Some(personalization),
            )
            .await
            .unwrap();

        assert!(result.personalized);
        assert_eq!(
            result.personalization_context,
            Some("developer".to_string())
        );
    }

    #[tokio::test]
    async fn test_empty_content_error() {
        let mock = Arc::new(MockLlmClient::new(vec![]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let result = generator
            .generate_summary("", SummaryDepth::Sentence, None)
            .await;

        assert!(matches!(result, Err(GenerationError::EmptyContent)));
    }

    #[tokio::test]
    async fn test_content_too_short_error() {
        let mock = Arc::new(MockLlmClient::new(vec![]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let result = generator
            .generate_summary("Short", SummaryDepth::Sentence, None)
            .await;

        assert!(matches!(
            result,
            Err(GenerationError::ContentTooShort { .. })
        ));
    }

    #[tokio::test]
    async fn test_batch_summarization() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "Summary 3".to_string(),
            "Summary 2".to_string(),
            "Summary 1".to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                       tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim \
                       veniam, quis nostrud exercitation ullamco laboris.";
        let requests = vec![
            SummaryRequest {
                content: content.to_string(),
                depth: SummaryDepth::Sentence,
                context: None,
                personalization_context: None,
            },
            SummaryRequest {
                content: content.to_string(),
                depth: SummaryDepth::Paragraph,
                context: None,
                personalization_context: None,
            },
            SummaryRequest {
                content: content.to_string(),
                depth: SummaryDepth::Detailed,
                context: None,
                personalization_context: None,
            },
        ];

        let result = generator.generate_batch(requests).await;

        assert_eq!(result.results.len(), 3);
        assert!(result.results.iter().all(|r| r.is_ok()));
        assert!(result.total_tokens_used > 0);
    }

    #[tokio::test]
    async fn test_batch_with_partial_failure() {
        let mock = Arc::new(MockLlmClient::new(vec![
            "Summary for second request".to_string(),
        ]));
        let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());

        let valid_content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                             eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim \
                             ad minim veniam, quis nostrud exercitation ullamco laboris.";
        let requests = vec![
            SummaryRequest {
                content: "Short".to_string(),
                depth: SummaryDepth::Sentence,
                context: None,
                personalization_context: None,
            },
            SummaryRequest {
                content: valid_content.to_string(),
                depth: SummaryDepth::Paragraph,
                context: None,
                personalization_context: None,
            },
        ];

        let result = generator.generate_batch(requests).await;

        assert_eq!(result.results.len(), 2);
        assert!(result.results[0].is_err());
        assert!(result.results[1].is_ok());
    }

    #[test]
    fn test_estimate_tokens() {
        assert!(estimate_tokens("Hello world") >= 2);
        assert!(estimate_tokens("A") >= 1);
        assert_eq!(estimate_tokens(""), 0);

        let long_text = "The quick brown fox jumps over the lazy dog";
        let tokens = estimate_tokens(long_text);
        assert!(tokens >= 9);
        assert!(tokens <= 15);
    }

    #[test]
    fn test_token_limits_by_depth() {
        let mock = Arc::new(MockLlmClient::new(vec![]));
        let config = SummaryGeneratorConfig {
            sentence_max_tokens: 50,
            paragraph_max_tokens: 200,
            detailed_max_tokens: 500,
            ..Default::default()
        };
        let generator = SummaryGenerator::new(mock, config);

        assert_eq!(generator.token_limit_for_depth(SummaryDepth::Sentence), 50);
        assert_eq!(
            generator.token_limit_for_depth(SummaryDepth::Paragraph),
            200
        );
        assert_eq!(generator.token_limit_for_depth(SummaryDepth::Detailed), 500);
    }

    #[test]
    fn test_source_hash_consistency() {
        let content = "Test content for hashing";
        let hash1 = compute_content_hash(content);
        let hash2 = compute_content_hash(content);

        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn test_source_hash_uniqueness() {
        let hash1 = compute_content_hash("Content A");
        let hash2 = compute_content_hash("Content B");

        assert_ne!(hash1, hash2);
    }

    mod budget_aware_tests {
        use super::*;
        use crate::context_architect::budget::{
            BudgetExhaustedAction, BudgetStatus, BudgetTrackerConfig, SummarizationBudget,
        };

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
            responses: Vec<String>,
            budget_tracker: Arc<BudgetTracker>,
        ) -> BudgetAwareSummaryGenerator<MockLlmClient> {
            let mock = Arc::new(MockLlmClient::new(responses));
            let generator = SummaryGenerator::new(mock, SummaryGeneratorConfig::default());
            BudgetAwareSummaryGenerator::new(
                generator,
                budget_tracker,
                TieredModelConfig::default(),
            )
        }

        #[tokio::test]
        async fn test_generate_with_budget_success() {
            let tracker = create_budget_tracker(1_000_000, 100_000);
            let generator =
                create_budget_aware_generator(vec!["Budget-aware summary.".to_string()], tracker);

            let request = BudgetAwareSummaryRequest {
                content: "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                          tempor incididunt ut labore."
                    .to_string(),
                depth: SummaryDepth::Sentence,
                layer: MemoryLayer::Session,
                context: None,
                personalization_context: None,
            };

            let result = generator.generate_with_budget(request).await;
            assert!(result.is_ok());

            let result = result.unwrap();
            assert!(result.tokens_used > 0);
            assert_eq!(result.budget_check.status, BudgetStatus::Available);
            assert_eq!(result.model_used, "gpt-4");
        }

        #[tokio::test]
        async fn test_generate_with_budget_exhausted() {
            let tracker = create_budget_tracker(100, 100);
            tracker.record_usage(100, MemoryLayer::Session);

            let generator = create_budget_aware_generator(vec![], tracker);

            let request = BudgetAwareSummaryRequest {
                content: "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod \
                          tempor incididunt ut labore."
                    .to_string(),
                depth: SummaryDepth::Sentence,
                layer: MemoryLayer::Session,
                context: None,
                personalization_context: None,
            };

            let result = generator.generate_with_budget(request).await;
            assert!(result.is_err());
            assert!(matches!(result, Err(GenerationError::BudgetExhausted(_))));
        }

        #[tokio::test]
        async fn test_tiered_model_selection() {
            let tracker = create_budget_tracker(1_000_000, 100_000);
            let generator = create_budget_aware_generator(
                vec![
                    "Tenant layer summary.".to_string(),
                    "User layer summary.".to_string(),
                ],
                tracker,
            );

            assert_eq!(generator.model_for_layer(MemoryLayer::User), "gpt-4");
            assert_eq!(generator.model_for_layer(MemoryLayer::Session), "gpt-4");
            assert_eq!(generator.model_for_layer(MemoryLayer::Agent), "gpt-4");

            assert_eq!(
                generator.model_for_layer(MemoryLayer::Tenant),
                "gpt-3.5-turbo"
            );
            assert_eq!(generator.model_for_layer(MemoryLayer::Org), "gpt-3.5-turbo");
            assert_eq!(
                generator.model_for_layer(MemoryLayer::Project),
                "gpt-3.5-turbo"
            );
        }

        #[tokio::test]
        async fn test_budget_check() {
            let tracker = create_budget_tracker(1000, 500);
            let generator = create_budget_aware_generator(vec![], tracker.clone());

            tracker.record_usage(300, MemoryLayer::Session);

            let check = generator.check_budget(MemoryLayer::Session);
            assert_eq!(check.daily_used, 300);
            assert_eq!(check.hourly_used, 300);
            assert_eq!(check.layer_used, Some(300));
        }

        #[tokio::test]
        async fn test_batch_with_budget_partial_exhaustion() {
            let tracker = create_budget_tracker(500, 500);

            let generator = create_budget_aware_generator(
                vec!["Summary 2.".to_string(), "Summary 1.".to_string()],
                tracker.clone(),
            );

            let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                           eiusmod tempor incididunt ut labore et dolore magna aliqua."
                .to_string();

            let requests = vec![
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Session,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Project,
                    context: None,
                    personalization_context: None,
                },
            ];

            let results = generator.generate_batch_with_budget(requests).await;
            assert_eq!(results.len(), 2);

            let successful_count = results.iter().filter(|r| r.is_ok()).count();
            assert!(successful_count >= 1);
        }

        #[tokio::test]
        async fn test_budget_tracking_accumulation() {
            let tracker = create_budget_tracker(10_000, 10_000);
            let generator = create_budget_aware_generator(
                vec![
                    "Third summary.".to_string(),
                    "Second summary.".to_string(),
                    "First summary.".to_string(),
                ],
                tracker.clone(),
            );

            let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                           eiusmod tempor incididunt ut labore."
                .to_string();

            for _ in 0..3 {
                let request = BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Session,
                    context: None,
                    personalization_context: None,
                };
                let _ = generator.generate_with_budget(request).await;
            }

            let metrics = tracker.get_metrics();
            assert!(metrics.daily_tokens_used > 0);
            assert!(metrics.percent_used > 0.0);
        }

        #[tokio::test]
        async fn test_batched_summarizer_groups_by_layer() {
            let tracker = create_budget_tracker(100_000, 100_000);
            let budget_aware_generator = Arc::new(create_budget_aware_generator(
                vec![
                    "Tenant summary.".to_string(),
                    "Org summary.".to_string(),
                    "Session summary 2.".to_string(),
                    "Session summary 1.".to_string(),
                ],
                tracker,
            ));
            let batched = BatchedSummarizer::new(budget_aware_generator);

            let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                           eiusmod tempor incididunt ut labore."
                .to_string();

            let requests = vec![
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Session,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Tenant,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Session,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Org,
                    context: None,
                    personalization_context: None,
                },
            ];

            let result = batched.process_batch(requests).await;

            assert_eq!(result.layer_results.len(), 3);
            assert_eq!(result.successful_count, 4);
            assert_eq!(result.failed_count, 0);

            let layers: Vec<_> = result.layer_results.iter().map(|r| r.layer).collect();
            assert_eq!(layers[0], MemoryLayer::Tenant);
            assert_eq!(layers[1], MemoryLayer::Org);
            assert_eq!(layers[2], MemoryLayer::Session);
        }

        #[tokio::test]
        async fn test_batched_summarizer_respects_budget() {
            let tracker = create_budget_tracker(200, 200);
            tracker.record_usage(100, MemoryLayer::Session);

            let budget_aware_generator = Arc::new(create_budget_aware_generator(
                vec!["Tenant summary.".to_string()],
                tracker,
            ));
            let batched = BatchedSummarizer::new(budget_aware_generator);

            let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                           eiusmod tempor incididunt ut labore."
                .to_string();

            let requests = vec![
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Session,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Tenant,
                    context: None,
                    personalization_context: None,
                },
            ];

            let result = batched.process_batch(requests).await;

            assert!(result.failed_count >= 1);
        }

        #[tokio::test]
        async fn test_batched_summarizer_layer_priority() {
            let tracker = create_budget_tracker(100_000, 100_000);
            let budget_aware_generator = Arc::new(create_budget_aware_generator(
                vec![
                    "Agent summary.".to_string(),
                    "User summary.".to_string(),
                    "Tenant summary.".to_string(),
                ],
                tracker,
            ));
            let batched = BatchedSummarizer::new(budget_aware_generator);

            let content = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do \
                           eiusmod tempor incididunt ut labore."
                .to_string();

            let requests = vec![
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Agent,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::Tenant,
                    context: None,
                    personalization_context: None,
                },
                BudgetAwareSummaryRequest {
                    content: content.clone(),
                    depth: SummaryDepth::Sentence,
                    layer: MemoryLayer::User,
                    context: None,
                    personalization_context: None,
                },
            ];

            let result = batched.process_batch(requests).await;

            let layers: Vec<_> = result.layer_results.iter().map(|r| r.layer).collect();
            assert_eq!(layers[0], MemoryLayer::Tenant);
            assert_eq!(layers[1], MemoryLayer::User);
            assert_eq!(layers[2], MemoryLayer::Agent);
        }
    }
}
