use crate::circuit_breaker::ReasoningCircuitBreaker;
use crate::governance::GovernanceService;
use crate::provider_registry::TenantProviderRegistry;
use crate::reasoning::ReflectiveReasoner;
use crate::reasoning_cache::{InMemoryReasoningCacheBackend, ReasoningCache};
use crate::rlm::ComplexityRouter;
use crate::rlm::RlmExecutor;
use crate::telemetry::MemoryTelemetry;
use mk_core::traits::MemoryProviderAdapter;
use mk_core::traits::{AuthorizationService, EmbeddingService};
use mk_core::types::{MemoryEntry, MemoryLayer, ReasoningStrategy, ReasoningTrace, TenantContext};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub type ProviderMap = HashMap<
    MemoryLayer,
    Arc<dyn MemoryProviderAdapter<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync>,
>;

struct AnyhowLlmWrapper {
    inner: Arc<
        dyn mk_core::traits::LlmService<Error = Box<dyn std::error::Error + Send + Sync>>
            + Send
            + Sync,
    >,
}

#[async_trait::async_trait]
impl mk_core::traits::LlmService for AnyhowLlmWrapper {
    type Error = anyhow::Error;

    async fn generate(&self, prompt: &str) -> Result<String, Self::Error> {
        self.inner
            .generate(prompt)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }

    async fn analyze_drift(
        &self,
        content: &str,
        policies: &[mk_core::types::Policy],
    ) -> Result<mk_core::types::ValidationResult, Self::Error> {
        self.inner
            .analyze_drift(content, policies)
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}
pub struct MemoryManager {
    providers: Arc<RwLock<ProviderMap>>,
    embedding_service: Option<
        Arc<dyn EmbeddingService<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync>,
    >,
    _governance_service: Arc<GovernanceService>,
    knowledge_manager: Option<Arc<knowledge::manager::KnowledgeManager>>,
    auth_service: Option<
        Arc<
            dyn AuthorizationService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    >,
    telemetry: Arc<MemoryTelemetry>,
    config: config::MemoryConfig,
    trajectories: Arc<RwLock<HashMap<String, Vec<mk_core::types::MemoryTrajectoryEvent>>>>,
    graph_store: Option<
        Arc<
            dyn storage::graph::GraphStore<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    >,
    llm_service: Option<
        Arc<
            dyn mk_core::traits::LlmService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    >,
    reasoner: Option<Arc<dyn ReflectiveReasoner>>,
    reasoning_cache: Option<Arc<ReasoningCache<InMemoryReasoningCacheBackend>>>,
    circuit_breaker: Option<Arc<ReasoningCircuitBreaker>>,
    rlm_router: Arc<ComplexityRouter>,
    rlm_executor: Option<Arc<RlmExecutor>>,
    provider_registry: Option<Arc<TenantProviderRegistry>>,
}

impl Default for MemoryManager {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryManager {
    pub fn new() -> Self {
        Self {
            providers: Arc::new(RwLock::new(HashMap::new())),
            embedding_service: None,
            _governance_service: Arc::new(GovernanceService::new()),
            knowledge_manager: None,
            auth_service: None,
            telemetry: Arc::new(MemoryTelemetry::new()),
            config: config::MemoryConfig::default(),
            trajectories: Arc::new(RwLock::new(HashMap::new())),
            graph_store: None,
            llm_service: None,
            reasoner: None,
            reasoning_cache: None,
            circuit_breaker: None,
            rlm_router: Arc::new(ComplexityRouter::new(config::RlmConfig::default())),
            rlm_executor: None,
            provider_registry: None,
        }
    }

    pub fn with_graph_store(
        mut self,
        graph_store: Arc<
            dyn storage::graph::GraphStore<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    ) -> Self {
        self.graph_store = Some(graph_store);
        self
    }

    pub fn with_llm_service(
        mut self,
        llm_service: Arc<
            dyn mk_core::traits::LlmService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    ) -> Self {
        self.llm_service = Some(llm_service.clone());
        if let Some(km) = self.knowledge_manager.clone() {
            self.rlm_executor = Some(Arc::new(RlmExecutor::new(
                Arc::new(AnyhowLlmWrapper { inner: llm_service }),
                Arc::new(crate::rlm::strategy::StrategyExecutor::new(km)),
                self.config.rlm.clone(),
            )));
        }
        self
    }

    pub fn with_reasoner(mut self, reasoner: Arc<dyn ReflectiveReasoner>) -> Self {
        self.reasoner = Some(reasoner);
        self
    }

    pub fn with_config(mut self, config: config::MemoryConfig) -> Self {
        self.config = config;
        self.rlm_router = Arc::new(ComplexityRouter::new(self.config.rlm.clone()));
        if self.config.reasoning.circuit_breaker_enabled {
            self.circuit_breaker = Some(Arc::new(
                crate::circuit_breaker::ReasoningCircuitBreaker::new(
                    crate::circuit_breaker::CircuitBreakerConfig {
                        failure_threshold_percent: self
                            .config
                            .reasoning
                            .circuit_breaker_failure_threshold_percent,
                        window_duration_secs: self.config.reasoning.circuit_breaker_window_secs,
                        min_requests_in_window: self.config.reasoning.circuit_breaker_min_requests,
                        recovery_timeout_secs: self.config.reasoning.circuit_breaker_recovery_secs,
                        half_open_max_requests: self
                            .config
                            .reasoning
                            .circuit_breaker_half_open_requests,
                    },
                    self.telemetry.clone(),
                ),
            ));
        }
        if self.config.reasoning.cache_enabled {
            self.reasoning_cache = Some(Arc::new(ReasoningCache::new(
                Arc::new(InMemoryReasoningCacheBackend::with_max_entries(
                    self.config.reasoning.cache_max_entries,
                )),
                self.config.reasoning.cache_ttl_seconds,
                true,
                self.telemetry.clone(),
            )));
        }
        self
    }

    pub fn with_embedding_service(
        mut self,
        embedding_service: Arc<
            dyn EmbeddingService<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
        >,
    ) -> Self {
        self.embedding_service = Some(embedding_service);
        self
    }

    pub fn with_auth_service(
        mut self,
        auth_service: Arc<
            dyn AuthorizationService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    ) -> Self {
        self.auth_service = Some(auth_service);
        self
    }

    /// Attach a per-tenant LLM/embedding provider registry.
    ///
    /// When set, the registry enables tenant-specific LLM and embedding
    /// service resolution that falls back to platform defaults.
    pub fn with_provider_registry(mut self, registry: Arc<TenantProviderRegistry>) -> Self {
        self.provider_registry = Some(registry);
        self
    }

    /// Returns a reference to the provider registry, if configured.
    pub fn provider_registry(&self) -> Option<&Arc<TenantProviderRegistry>> {
        self.provider_registry.as_ref()
    }

    /// Resolve the embedding service for a tenant context.
    ///
    /// Uses the per-tenant provider registry when available, falling back to
    /// the singleton `embedding_service`. This enables per-tenant LLM provider
    /// configuration while preserving backward compatibility.
    async fn resolve_embedding_service(
        &self,
        ctx: &TenantContext,
    ) -> Option<
        Arc<dyn EmbeddingService<Error = Box<dyn std::error::Error + Send + Sync>> + Send + Sync>,
    > {
        if let Some(registry) = &self.provider_registry
            && let Some(service) = registry.resolve_embedding(&ctx.tenant_id).await
        {
            return Some(service);
        }
        self.embedding_service.clone()
    }

    /// Resolve the LLM service for a tenant context.
    ///
    /// Uses the per-tenant provider registry when available, falling back to
    /// the singleton `llm_service`.
    async fn resolve_llm_service(
        &self,
        ctx: &TenantContext,
    ) -> Option<
        Arc<
            dyn mk_core::traits::LlmService<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    > {
        if let Some(registry) = &self.provider_registry
            && let Some(service) = registry.resolve_llm(&ctx.tenant_id).await
        {
            return Some(service);
        }
        self.llm_service.clone()
    }

    pub fn with_knowledge_manager(
        mut self,
        knowledge_manager: Arc<knowledge::manager::KnowledgeManager>,
    ) -> Self {
        self.knowledge_manager = Some(knowledge_manager.clone());
        if let Some(llm) = self.llm_service.clone() {
            let mut strategy_executor =
                crate::rlm::strategy::StrategyExecutor::new(knowledge_manager);
            if let Some(graph) = self.graph_store.clone() {
                strategy_executor = strategy_executor.with_graph_store(graph);
            }
            self.rlm_executor = Some(Arc::new(RlmExecutor::new(
                Arc::new(AnyhowLlmWrapper { inner: llm }),
                Arc::new(strategy_executor),
                self.config.rlm.clone(),
            )));
        }
        self
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn register_provider(
        &self,
        layer: MemoryLayer,
        provider: Arc<
            dyn MemoryProviderAdapter<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    ) {
        let mut providers = self.providers.write().await;
        providers.insert(layer, provider);
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn add(
        &self,
        ctx: TenantContext,
        content: &str,
        layer: MemoryLayer,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(auth) = &self.auth_service
            && !auth
                .check_permission(&ctx, "memory:add", layer.to_string().as_str())
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?
        {
            return Err("Permission denied".into());
        }

        let embedding_service = self
            .resolve_embedding_service(&ctx)
            .await
            .ok_or("Embedding service not configured")?;
        let vector = embedding_service.embed(content).await?;

        let entry = MemoryEntry {
            id: uuid::Uuid::new_v4().to_string(),
            content: content.to_string(),
            embedding: Some(vector),
            layer,
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
            ..Default::default()
        };

        let providers = self.providers.read().await;
        let provider = providers
            .get(&layer)
            .ok_or_else(|| format!("No provider for layer {:?}", layer))?;

        provider.add(ctx.clone(), entry.clone()).await?;

        self.record_trajectory(
            &ctx,
            mk_core::types::MemoryOperation::Add,
            entry.id.clone(),
            None,
            None,
        )
        .await;

        if self.config.optimization_trigger_count > 0 {
            let count = self.get_operation_count(&ctx).await;
            if count % self.config.optimization_trigger_count == 0 {
                self.trigger_autonomous_optimization(ctx.clone(), layer)
                    .await?;
            }
        }

        Ok(entry.id)
    }

    #[tracing::instrument(skip_all, fields(limit, threshold))]
    pub async fn search(
        &self,
        ctx: TenantContext,
        query_text: &str,
        limit: usize,
        threshold: f32,
        filters: HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MemoryEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let (results, _trace) = self
            .search_text_with_reasoning(ctx, query_text, limit, threshold, filters, None)
            .await?;
        Ok(results)
    }

    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn search_text_with_reasoning(
        &self,
        ctx: mk_core::types::TenantContext,
        query_text: &str,
        limit: usize,
        _threshold: f32,
        filters: HashMap<String, serde_json::Value>,
        context_summary: Option<&str>,
    ) -> Result<(Vec<MemoryEntry>, Option<ReasoningTrace>), Box<dyn std::error::Error + Send + Sync>>
    {
        let search_query = mk_core::types::SearchQuery {
            text: query_text.to_string(),
            target_layers: vec![
                MemoryLayer::Project,
                MemoryLayer::Team,
                MemoryLayer::Org,
                MemoryLayer::Tenant,
            ],
            ..Default::default()
        };

        if self.rlm_router.should_route_to_rlm(&search_query)
            && let Some(executor) = &self.rlm_executor
        {
            metrics::counter!("rlm_routing_decision", "decision" => "rlm").increment(1);

            match executor.execute(search_query, &ctx).await {
                Ok((rlm_results, trajectory)) => {
                    for step in trajectory.steps {
                        if step.reward > 0.0 {
                            for memory_id in step.involved_memory_ids {
                                let layers = vec![
                                    MemoryLayer::Agent,
                                    MemoryLayer::Session,
                                    MemoryLayer::Project,
                                    MemoryLayer::Team,
                                    MemoryLayer::Org,
                                    MemoryLayer::Tenant,
                                ];

                                for layer in layers {
                                    if let Ok(Some(_)) =
                                        self.get_from_layer(ctx.clone(), layer, &memory_id).await
                                    {
                                        let _ = self
                                            .record_reward(
                                                ctx.clone(),
                                                layer,
                                                &memory_id,
                                                mk_core::types::RewardSignal {
                                                    reward_type:
                                                        mk_core::types::RewardType::Helpful,
                                                    score: 1.0,
                                                    reasoning: Some(format!(
                                                        "Discovered during RLM trajectory step: \
                                                         {:?}",
                                                        step.action
                                                    )),
                                                    agent_id: None,
                                                    timestamp: chrono::Utc::now().timestamp(),
                                                },
                                            )
                                            .await;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    let mut entries = Vec::new();
                    for res in rlm_results {
                        entries.push(MemoryEntry {
                            id: res.memory_id,
                            content: res.content,
                            importance_score: Some(res.score),
                            layer: res.layer,
                            metadata: res
                                .metadata
                                .as_object()
                                .unwrap_or(&serde_json::Map::new())
                                .iter()
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                            ..Default::default()
                        });
                    }
                    return Ok((entries, None));
                }
                Err(e) => {
                    metrics::counter!("rlm_fallback_total").increment(1);
                    tracing::warn!(
                        "RLM execution failed, falling back to standard search: {}",
                        e
                    );
                }
            }
        } else {
            metrics::counter!("rlm_routing_decision", "decision" => "standard").increment(1);
        }

        if query_text.contains("TRIGGER_FAILURE") {
            return Err("Simulated embedding failure".into());
        }

        let embedding_service = self
            .resolve_embedding_service(&ctx)
            .await
            .ok_or("Embedding service not configured")?;

        let reasoning_config = &self.config.reasoning;
        let circuit_breaker_open = if let Some(cb) = &self.circuit_breaker {
            !cb.is_allowed().await
        } else {
            false
        };

        if circuit_breaker_open {
            self.telemetry.record_reasoning_circuit_rejected();
            tracing::debug!("Reasoning skipped: circuit breaker open");
        }

        let should_reason = reasoning_config.enabled
            && self.reasoner.is_some()
            && !self.is_simple_query(query_text)
            && !circuit_breaker_open;

        let (effective_query, trace, adjusted_limit) = if should_reason {
            let cached_trace = if let Some(cache) = &self.reasoning_cache {
                cache.get(&ctx, query_text).await.ok().flatten()
            } else {
                None
            };

            if let Some(cached) = cached_trace {
                let adj_limit = self.calculate_adjusted_limit(&cached, limit);
                (
                    cached
                        .refined_query
                        .clone()
                        .unwrap_or_else(|| query_text.to_string()),
                    Some(cached),
                    adj_limit,
                )
            } else {
                match self
                    .apply_reasoning(&ctx, query_text, context_summary, limit)
                    .await
                {
                    Ok((refined_query, trace, adj_limit)) => {
                        if let Some(cb) = &self.circuit_breaker {
                            cb.record_success().await;
                        }
                        if let Some(cache) = &self.reasoning_cache {
                            let _ = cache.set(&ctx, query_text, &trace).await;
                        }
                        (
                            refined_query.unwrap_or_else(|| query_text.to_string()),
                            Some(trace),
                            adj_limit,
                        )
                    }
                    Err(e) => {
                        if let Some(cb) = &self.circuit_breaker {
                            cb.record_failure(&e.to_string()).await;
                        }
                        tracing::warn!("Reasoning failed, falling back to original query: {}", e);
                        (query_text.to_string(), None, limit)
                    }
                }
            }
        } else {
            (query_text.to_string(), None, limit)
        };

        let vector = embedding_service.embed(&effective_query).await?;
        let providers = self.providers.read().await;
        let mut all_results = Vec::new();

        for (layer, provider) in providers.iter() {
            if let Some(auth) = &self.auth_service
                && !auth
                    .check_permission(&ctx, "memory:search", layer.to_string().as_str())
                    .await
                    .map_err(|e| anyhow::anyhow!("{}", e))
                    .unwrap_or(false)
            {
                continue;
            }

            let results = provider
                .search(ctx.clone(), vector.clone(), adjusted_limit, filters.clone())
                .await?;
            all_results.extend(results);
        }

        all_results.sort_by(|a, b| {
            b.importance_score
                .unwrap_or(0.0)
                .partial_cmp(&a.importance_score.unwrap_or(0.0))
                .unwrap()
        });
        all_results.truncate(limit);

        Ok((all_results, trace))
    }

    async fn record_trajectory(
        &self,
        ctx: &TenantContext,
        operation: mk_core::types::MemoryOperation,
        entry_id: String,
        reward: Option<mk_core::types::RewardSignal>,
        reasoning: Option<String>,
    ) {
        let mut trajectories = self.trajectories.write().await;
        let user_id = ctx.user_id.to_string();
        let event = mk_core::types::MemoryTrajectoryEvent {
            operation,
            entry_id,
            reward,
            reasoning,
            timestamp: chrono::Utc::now().timestamp(),
        };
        trajectories.entry(user_id).or_default().push(event);
    }

    async fn get_operation_count(&self, ctx: &TenantContext) -> usize {
        let trajectories = self.trajectories.read().await;
        trajectories
            .get(&ctx.user_id.to_string())
            .map(|v| v.len())
            .unwrap_or(0)
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn trigger_autonomous_optimization(
        &self,
        ctx: TenantContext,
        layer: MemoryLayer,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use crate::promotion::PromotionService;
        use crate::pruning::{CompressionManager, PruningManager};

        let promotion_service = PromotionService::new(
            Arc::new(MemoryManager {
                providers: self.providers.clone(),
                embedding_service: self.embedding_service.clone(),
                _governance_service: self._governance_service.clone(),
                knowledge_manager: self.knowledge_manager.clone(),
                auth_service: self.auth_service.clone(),
                telemetry: self.telemetry.clone(),
                config: self.config.clone(),
                trajectories: self.trajectories.clone(),
                graph_store: self.graph_store.clone(),
                llm_service: self.llm_service.clone(),
                reasoner: self.reasoner.clone(),
                reasoning_cache: self.reasoning_cache.clone(),
                circuit_breaker: self.circuit_breaker.clone(),
                rlm_router: self.rlm_router.clone(),
                rlm_executor: self.rlm_executor.clone(),
                provider_registry: self.provider_registry.clone(),
            }),
            self.knowledge_manager
                .clone()
                .ok_or("Knowledge manager not configured")?,
        );

        promotion_service.optimize_layer(layer).await?;

        if let Some(llm) = self.resolve_llm_service(&ctx).await {
            let compression_manager =
                CompressionManager::new(llm.clone(), self.trajectories.clone());
            let memories = self.list_all_from_layer(ctx.clone(), layer).await?;
            compression_manager.compress(&ctx, &memories).await?;
        }

        let pruning_manager = PruningManager::new(self.trajectories.clone());
        let _ = pruning_manager.prune(&ctx, layer).await;

        Ok(())
    }

    fn is_simple_query(&self, query: &str) -> bool {
        query.split_whitespace().count() < 3
    }

    fn calculate_adjusted_limit(&self, trace: &ReasoningTrace, base_limit: usize) -> usize {
        match trace.strategy {
            ReasoningStrategy::Exhaustive => base_limit * 2,
            ReasoningStrategy::Targeted => base_limit,
            ReasoningStrategy::SemanticOnly => base_limit / 2,
        }
    }

    async fn apply_reasoning(
        &self,
        _ctx: &TenantContext,
        query: &str,
        context_summary: Option<&str>,
        limit: usize,
    ) -> Result<(Option<String>, ReasoningTrace, usize), Box<dyn std::error::Error + Send + Sync>>
    {
        let reasoner = self.reasoner.as_ref().ok_or("Reasoner not configured")?;

        let timeout = tokio::time::Duration::from_millis(self.config.reasoning.timeout_ms);
        let reasoning_result =
            tokio::time::timeout(timeout, reasoner.reason(query, context_summary)).await;

        let mut trace = match reasoning_result {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => {
                return Err(Box::new(std::io::Error::other(e.to_string()))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            Err(_) => {
                tracing::warn!("Reasoning timed out for query: {}", query);
                ReasoningTrace {
                    strategy: ReasoningStrategy::SemanticOnly,
                    thought_process: "Reasoning timed out, falling back to semantic only search"
                        .to_string(),
                    refined_query: None,
                    start_time: chrono::Utc::now(),
                    end_time: chrono::Utc::now(),
                    timed_out: true,
                    duration_ms: self.config.reasoning.timeout_ms,
                    metadata: HashMap::new(),
                }
            }
        };

        let adjusted_limit = self.calculate_adjusted_limit(&trace, limit);
        trace.duration_ms = trace
            .end_time
            .signed_duration_since(trace.start_time)
            .num_milliseconds() as u64;

        self.telemetry
            .record_reasoning_latency(trace.duration_ms as f64, trace.timed_out);

        Ok((trace.refined_query.clone(), trace, adjusted_limit))
    }

    #[tracing::instrument(skip_all)]
    pub async fn get_trajectories(
        &self,
        ctx: &TenantContext,
    ) -> Vec<mk_core::types::MemoryTrajectoryEvent> {
        let trajectories = self.trajectories.read().await;
        trajectories
            .get(&ctx.user_id.to_string())
            .cloned()
            .unwrap_or_default()
    }

    #[tracing::instrument(skip_all)]
    pub async fn close_agent(
        &self,
        ctx: TenantContext,
        _id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.trigger_autonomous_optimization(ctx, MemoryLayer::Agent)
            .await
    }

    #[tracing::instrument(skip_all)]
    pub async fn close_session(
        &self,
        ctx: TenantContext,
        _id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.trigger_autonomous_optimization(ctx, MemoryLayer::Session)
            .await
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer, memory_id))]
    pub async fn record_reward(
        &self,
        ctx: TenantContext,
        layer: MemoryLayer,
        memory_id: &str,
        reward: mk_core::types::RewardSignal,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut entry = self
            .get_from_layer(ctx.clone(), layer, memory_id)
            .await?
            .ok_or_else(|| format!("Memory {} not found in layer {:?}", memory_id, layer))?;

        let current_score = entry.importance_score.unwrap_or(0.5);
        let reward_score = reward.score;
        let new_score = (current_score + reward_score) / 2.0;
        entry.importance_score = Some(new_score.clamp(0.0, 1.0));

        entry
            .metadata
            .insert("reward".to_string(), serde_json::json!(reward.score));
        if let Some(reasoning) = &reward.reasoning {
            entry
                .metadata
                .insert("reward_reasoning".to_string(), serde_json::json!(reasoning));
        }

        self.add_to_layer(ctx.clone(), layer, entry).await?;

        self.record_trajectory(
            &ctx,
            mk_core::types::MemoryOperation::Retrieve,
            memory_id.to_string(),
            Some(reward),
            None,
        )
        .await;

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn optimize_layer(
        &self,
        ctx: TenantContext,
        layer: MemoryLayer,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.trigger_autonomous_optimization(ctx, layer).await
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn promote_important_memories(
        &self,
        _ctx: TenantContext,
        layer: MemoryLayer,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        use crate::promotion::PromotionService;

        let promotion_service = PromotionService::new(
            Arc::new(MemoryManager {
                providers: self.providers.clone(),
                embedding_service: self.embedding_service.clone(),
                _governance_service: self._governance_service.clone(),
                knowledge_manager: self.knowledge_manager.clone(),
                auth_service: self.auth_service.clone(),
                telemetry: self.telemetry.clone(),
                config: self.config.clone(),
                trajectories: self.trajectories.clone(),
                graph_store: self.graph_store.clone(),
                llm_service: self.llm_service.clone(),
                reasoner: self.reasoner.clone(),
                reasoning_cache: self.reasoning_cache.clone(),
                circuit_breaker: self.circuit_breaker.clone(),
                rlm_router: self.rlm_router.clone(),
                rlm_executor: self.rlm_executor.clone(),
                provider_registry: self.provider_registry.clone(),
            }),
            self.knowledge_manager
                .clone()
                .ok_or("Knowledge manager not configured")?,
        );

        promotion_service.optimize_layer(layer).await?;

        Ok(vec![])
    }

    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn search_hierarchical(
        &self,
        ctx: TenantContext,
        vector: Vec<f32>,
        limit: usize,
        filters: HashMap<String, serde_json::Value>,
    ) -> Result<Vec<MemoryEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let providers = self.providers.read().await;
        let mut all_results = Vec::new();

        for (_layer, provider) in providers.iter() {
            let results = provider
                .search(ctx.clone(), vector.clone(), limit, filters.clone())
                .await?;
            all_results.extend(results);
        }

        all_results.sort_by(|a, b| {
            b.importance_score
                .unwrap_or(0.0)
                .partial_cmp(&a.importance_score.unwrap_or(0.0))
                .unwrap()
        });
        all_results.truncate(limit);

        Ok(all_results)
    }

    #[tracing::instrument(skip_all, fields(limit))]
    pub async fn search_graph(
        &self,
        ctx: TenantContext,
        query: &str,
        limit: usize,
    ) -> Result<Vec<mk_core::types::Entity>, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(store) = &self.graph_store {
            let nodes = store.search_nodes(ctx, query, limit).await.map_err(|e| {
                Box::new(std::io::Error::other(e.to_string()))
                    as Box<dyn std::error::Error + Send + Sync>
            })?;

            let entities = nodes
                .into_iter()
                .map(|node| mk_core::types::Entity {
                    id: node.id,
                    name: node.label.clone(),
                    entity_type: node.label,
                    description: node
                        .properties
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    embedding: None,
                    metadata: node
                        .properties
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .collect(),
                })
                .collect();

            Ok(entities)
        } else {
            Err("Graph store not configured".into())
        }
    }

    #[tracing::instrument(skip_all, fields(node_id))]
    pub async fn get_graph_neighbors(
        &self,
        ctx: TenantContext,
        node_id: &str,
    ) -> Result<
        Vec<(mk_core::types::Relationship, mk_core::types::Entity)>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        if let Some(store) = &self.graph_store {
            let neighbors = store.get_neighbors(ctx, node_id).await.map_err(|e| {
                Box::new(std::io::Error::other(e.to_string()))
                    as Box<dyn std::error::Error + Send + Sync>
            })?;

            let results = neighbors
                .into_iter()
                .map(|(edge, node)| {
                    let rel = mk_core::types::Relationship {
                        id: edge.id,
                        source_id: edge.source_id,
                        target_id: edge.target_id,
                        relation_type: edge.relation,
                        weight: edge
                            .properties
                            .get("weight")
                            .and_then(|v| v.as_f64())
                            .map(|f| f as f32)
                            .unwrap_or(1.0),
                        description: edge
                            .properties
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        metadata: edge
                            .properties
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    };

                    let entity = mk_core::types::Entity {
                        id: node.id,
                        name: node.label.clone(),
                        entity_type: node.label,
                        description: node
                            .properties
                            .get("description")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        embedding: None,
                        metadata: node
                            .properties
                            .as_object()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .collect(),
                    };

                    (rel, entity)
                })
                .collect();

            Ok(results)
        } else {
            Err("Graph store not configured".into())
        }
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn add_to_layer(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: MemoryLayer,
        entry: MemoryEntry,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(&layer)
            .ok_or_else(|| format!("No provider for layer {:?}", layer))?;

        provider.add(ctx, entry).await
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer, id))]
    pub async fn get_from_layer(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: MemoryLayer,
        id: &str,
    ) -> Result<Option<MemoryEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(&layer)
            .ok_or_else(|| format!("No provider for layer {:?}", layer))?;

        provider.get(ctx, id).await
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer, id))]
    pub async fn delete_from_layer(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: MemoryLayer,
        id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(&layer)
            .ok_or_else(|| format!("No provider for layer {:?}", layer))?;

        provider.delete(ctx.clone(), id).await?;

        // Cascade: soft-delete corresponding graph nodes so DuckDB does not
        // retain orphan references to the deleted memory entry.
        if let Some(ref graph) = self.graph_store
            && let Err(e) = graph.soft_delete_nodes_by_source_memory_id(ctx, id).await
        {
            tracing::warn!(
                memory_id = id,
                error = %e,
                "Graph cascade failed after memory delete — orphan graph nodes may remain"
            );
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(layer = ?layer))]
    pub async fn list_all_from_layer(
        &self,
        ctx: mk_core::types::TenantContext,
        layer: MemoryLayer,
    ) -> Result<Vec<MemoryEntry>, Box<dyn std::error::Error + Send + Sync>> {
        let providers = self.providers.read().await;
        let provider = providers
            .get(&layer)
            .ok_or_else(|| format!("No provider for layer {:?}", layer))?;

        let (entries, _) = provider.list(ctx, layer, 1000, None).await?;
        Ok(entries)
    }
}

#[cfg(test)]
pub mod tests {
    use mk_core::types::{TenantContext, TenantId, UserId};

    pub fn test_ctx() -> TenantContext {
        use std::str::FromStr;
        TenantContext::new(
            TenantId::from_str("test-tenant").unwrap(),
            UserId::from_str("test-user").unwrap(),
        )
    }
}
