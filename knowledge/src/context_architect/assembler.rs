use std::collections::HashMap;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use config::cca::StalenessPolicy;
use dashmap::DashMap;
use mk_core::traits::RlmAssemblyService;
use mk_core::types::{
    ContextVector, KnowledgeRelation, KnowledgeVariantRole, LayerSummary, MemoryLayer,
    SummaryDepth, TenantContext, compute_xxhash64,
};

use crate::context_architect::compressor::ViewMode;

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct CacheKey {
    pub query_hash: u64,
    pub token_budget: u32,
    pub view_mode: ViewMode,
}

impl CacheKey {
    pub fn new(
        query_embedding: Option<&ContextVector>,
        token_budget: u32,
        view_mode: ViewMode,
    ) -> Self {
        let query_hash = match query_embedding {
            Some(vec) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for &val in vec {
                    let bytes = val.to_le_bytes();
                    for byte in bytes {
                        hasher.write_u8(byte);
                    }
                }
                hasher.finish()
            }
            None => 0,
        };

        Self {
            query_hash,
            token_budget,
            view_mode,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub context: AssembledContext,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub struct AssemblyMetrics {
    total_assemblies: Arc<AtomicU64>,
    cache_hits: Arc<AtomicU64>,
    cache_misses: Arc<AtomicU64>,
    timeouts: Arc<AtomicU64>,
    latency_sum_ms: Arc<AtomicU64>,
    partial_returns: Arc<AtomicU64>,
}

impl AssemblyMetrics {
    pub fn new() -> Self {
        Self {
            total_assemblies: Arc::new(AtomicU64::new(0)),
            cache_hits: Arc::new(AtomicU64::new(0)),
            cache_misses: Arc::new(AtomicU64::new(0)),
            timeouts: Arc::new(AtomicU64::new(0)),
            latency_sum_ms: Arc::new(AtomicU64::new(0)),
            partial_returns: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_assembly(
        &self,
        latency_ms: u64,
        hit_cache: bool,
        timed_out: bool,
        partial: bool,
    ) {
        self.total_assemblies.fetch_add(1, Ordering::Relaxed);
        self.latency_sum_ms.fetch_add(latency_ms, Ordering::Relaxed);

        if hit_cache {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }

        if timed_out {
            self.timeouts.fetch_add(1, Ordering::Relaxed);
        }

        if partial {
            self.partial_returns.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn total_assemblies(&self) -> u64 {
        self.total_assemblies.load(Ordering::Relaxed)
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.total_assemblies();
        if total == 0 {
            0.0
        } else {
            let hits = self.cache_hits.load(Ordering::Relaxed);
            hits as f64 / total as f64
        }
    }

    pub fn timeout_rate(&self) -> f64 {
        let total = self.total_assemblies();
        if total == 0 {
            0.0
        } else {
            let timeouts = self.timeouts.load(Ordering::Relaxed);
            timeouts as f64 / total as f64
        }
    }

    pub fn avg_latency_ms(&self) -> f64 {
        let total = self.total_assemblies.load(Ordering::Relaxed);
        if total == 0 {
            0.0
        } else {
            let sum = self.latency_sum_ms.load(Ordering::Relaxed);
            sum as f64 / total as f64
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StalenessStatus {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct AssemblerConfig {
    pub view_mode: ViewMode,
    pub default_token_budget: u32,
    pub layer_priorities: Vec<MemoryLayer>,
    pub min_relevance_score: f32,
    pub enable_caching: bool,
    pub cache_ttl_secs: u64,
    pub staleness_policy: StalenessPolicy,
    pub assembly_timeout_ms: u64,
    pub enable_parallel_queries: bool,
    pub enable_early_termination: bool,
}

impl Default for AssemblerConfig {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::Ax,
            default_token_budget: 4000,
            layer_priorities: vec![
                MemoryLayer::Session,
                MemoryLayer::Project,
                MemoryLayer::Team,
                MemoryLayer::Org,
                MemoryLayer::Tenant,
            ],
            min_relevance_score: 0.3,
            enable_caching: true,
            cache_ttl_secs: 300,
            staleness_policy: StalenessPolicy::default(),
            assembly_timeout_ms: 100,
            enable_parallel_queries: true,
            enable_early_termination: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub entry_id: String,
    pub layer: MemoryLayer,
    pub content: String,
    pub token_count: u32,
    pub depth: SummaryDepth,
    pub relevance_score: f32,
    pub context_vector: Option<ContextVector>,
    pub staleness_status: StalenessStatus,
}

#[derive(Debug, Clone)]
pub struct ContextMetadata {
    pub view_type: String,
    pub includes_trajectory_logs: bool,
    pub includes_metrics: bool,
    pub includes_traces: bool,
}

impl ContextMetadata {
    pub fn minimal() -> Self {
        Self {
            view_type: "agent_experience".to_string(),
            includes_trajectory_logs: false,
            includes_metrics: false,
            includes_traces: false,
        }
    }

    pub fn with_trajectory_logs(mut self) -> Self {
        self.includes_trajectory_logs = true;
        self
    }

    pub fn with_metrics(mut self) -> Self {
        self.includes_metrics = true;
        self
    }

    pub fn with_traces(mut self) -> Self {
        self.includes_traces = true;
        self
    }

    pub fn full_debug() -> Self {
        Self {
            view_type: "developer_experience".to_string(),
            includes_trajectory_logs: true,
            includes_metrics: true,
            includes_traces: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextView {
    pub content: String,
    pub metadata: ContextMetadata,
}

#[derive(Debug, Clone)]
pub struct AssembledContext {
    pub view: ContextView,
    pub entries: Vec<ContextEntry>,
    pub total_tokens: u32,
    pub token_budget: u32,
    pub layers_included: Vec<MemoryLayer>,
    pub query_embedding: Option<ContextVector>,
    pub stale_entries: Vec<String>,
    pub has_stale_content: bool,
    pub timed_out: bool,
    pub partial: bool,
}

impl AssembledContext {
    pub fn is_within_budget(&self) -> bool {
        self.total_tokens <= self.token_budget
    }

    pub fn content(&self) -> String {
        self.view.content.clone()
    }
}

#[derive(Debug, Clone)]
pub struct SummarySource {
    pub entry_id: String,
    pub layer: MemoryLayer,
    pub summaries: HashMap<SummaryDepth, LayerSummary>,
    pub context_vector: Option<ContextVector>,
    pub full_content: Option<String>,
    pub full_content_tokens: Option<u32>,
    pub current_source_content: Option<String>,
    /// The promotion-lifecycle variant role of the underlying knowledge entry.
    /// `None` is treated as `Canonical` (backward-compatible default).
    pub variant_role: Option<KnowledgeVariantRole>,
    /// Semantic relations attached to this knowledge entry.  Used by the
    /// context assembler to group canonical entries with their residuals and
    /// to bias ordering deterministically (task 6.4).
    pub relations: Vec<KnowledgeRelation>,
}

impl SummarySource {
    /// Returns the effective variant-role precedence weight.
    /// Canonical = 5 (highest); Exception = 1 (lowest).
    pub fn variant_precedence(&self) -> u8 {
        match self.variant_role.unwrap_or(KnowledgeVariantRole::Canonical) {
            KnowledgeVariantRole::Canonical => 5,
            KnowledgeVariantRole::Clarification => 4,
            KnowledgeVariantRole::Specialization => 3,
            KnowledgeVariantRole::Applicability => 2,
            KnowledgeVariantRole::Exception => 1,
            KnowledgeVariantRole::Superseded => 0,
        }
    }
}

pub struct ContextAssembler {
    config: AssemblerConfig,
    cache: Arc<DashMap<CacheKey, CacheEntry>>,
    metrics: Arc<AssemblyMetrics>,
    rlm_handler: Option<Arc<dyn RlmAssemblyService>>,
}

impl ContextAssembler {
    pub fn new(config: AssemblerConfig) -> Self {
        Self {
            config,
            cache: Arc::new(DashMap::new()),
            metrics: Arc::new(AssemblyMetrics::new()),
            rlm_handler: None,
        }
    }

    pub fn with_cache(mut self, cache: Arc<DashMap<CacheKey, CacheEntry>>) -> Self {
        self.cache = cache;
        self
    }

    pub fn with_metrics(mut self, metrics: Arc<AssemblyMetrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_rlm_handler(mut self, handler: Arc<dyn RlmAssemblyService>) -> Self {
        self.rlm_handler = Some(handler);
        self
    }

    pub fn metrics(&self) -> &Arc<AssemblyMetrics> {
        &self.metrics
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub fn has_rlm_handler(&self) -> bool {
        self.rlm_handler.is_some()
    }

    pub fn should_use_rlm(&self, query_text: &str) -> bool {
        self.rlm_handler
            .as_ref()
            .is_some_and(|h| h.should_use_rlm(query_text))
    }

    pub fn compute_query_complexity(&self, query_text: &str) -> f32 {
        self.rlm_handler
            .as_ref()
            .map(|h| h.compute_complexity(query_text))
            .unwrap_or(0.0)
    }

    pub fn assemble_context(
        &self,
        query_embedding: Option<&ContextVector>,
        sources: &[SummarySource],
        token_budget: Option<u32>,
    ) -> AssembledContext {
        let start = Instant::now();
        let budget = token_budget.unwrap_or(self.config.default_token_budget);

        if self.config.enable_caching {
            let cache_key = CacheKey::new(query_embedding, budget, self.config.view_mode);

            if let Some(entry) = self.cache.get(&cache_key)
                && entry.created_at.elapsed() < Duration::from_secs(self.config.cache_ttl_secs)
            {
                let mut cached = entry.context.clone();
                cached.timed_out = false;
                cached.partial = false;
                self.metrics.record_assembly(
                    start.elapsed().as_millis() as u64,
                    true,
                    false,
                    false,
                );
                return cached;
            }

            self.cache.retain(|_, entry| {
                entry.created_at.elapsed() < Duration::from_secs(self.config.cache_ttl_secs)
            });
        }

        let mut result = self.assemble_context_internal(query_embedding, sources, Some(budget));

        let latency_ms = start.elapsed().as_millis() as u64;
        result.timed_out = latency_ms >= self.config.assembly_timeout_ms;
        result.partial = self.config.enable_early_termination && result.total_tokens < budget;

        if self.config.enable_caching {
            let cache_key = CacheKey::new(query_embedding, budget, self.config.view_mode);
            self.cache.insert(
                cache_key,
                CacheEntry {
                    context: result.clone(),
                    created_at: Instant::now(),
                },
            );
        }

        self.metrics
            .record_assembly(latency_ms, false, result.timed_out, result.partial);

        result
    }

    pub async fn assemble_with_rlm(
        &self,
        query_text: &str,
        query_embedding: Option<&ContextVector>,
        sources: &[SummarySource],
        token_budget: Option<u32>,
        tenant: &TenantContext,
    ) -> AssembledContext {
        let start = Instant::now();

        if let Some(handler) = &self.rlm_handler
            && handler.should_use_rlm(query_text)
        {
            match handler.execute_assembly(query_text, tenant).await {
                Ok(rlm_result) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let budget = token_budget.unwrap_or(self.config.default_token_budget);

                    let token_count = (rlm_result.content.len() / 4) as u32;

                    let entry = ContextEntry {
                        entry_id: format!("rlm-{}", uuid::Uuid::new_v4()),
                        layer: MemoryLayer::Project,
                        content: rlm_result.content.clone(),
                        token_count,
                        depth: SummaryDepth::Detailed,
                        relevance_score: 0.95,
                        context_vector: query_embedding.cloned(),
                        staleness_status: StalenessStatus::Fresh,
                    };

                    let view = ContextView {
                        content: rlm_result.content,
                        metadata: ContextMetadata {
                            view_type: "rlm_synthesized".to_string(),
                            includes_trajectory_logs: false,
                            includes_metrics: false,
                            includes_traces: false,
                        },
                    };

                    self.metrics
                        .record_assembly(latency_ms, false, false, false);

                    return AssembledContext {
                        view,
                        entries: vec![entry],
                        total_tokens: token_count,
                        token_budget: budget,
                        layers_included: vec![MemoryLayer::Project],
                        query_embedding: query_embedding.cloned(),
                        stale_entries: vec![],
                        has_stale_content: false,
                        timed_out: false,
                        partial: false,
                    };
                }
                Err(e) => {
                    tracing::warn!("RLM assembly failed, falling back to standard: {}", e);
                }
            }
        }

        self.assemble_context(query_embedding, sources, token_budget)
    }

    fn assemble_context_internal(
        &self,
        query_embedding: Option<&ContextVector>,
        sources: &[SummarySource],
        token_budget: Option<u32>,
    ) -> AssembledContext {
        let budget = token_budget.unwrap_or(self.config.default_token_budget);

        let scored_sources: Vec<_> = sources
            .iter()
            .map(|s| {
                let score = self.compute_relevance_score(query_embedding, s);
                (s, score)
            })
            .filter(|(_, score)| *score >= self.config.min_relevance_score)
            .collect();

        let token_allocations = self.distribute_token_budget(&scored_sources, budget);
        let mut entries = self.select_entries(&scored_sources, &token_allocations);

        if self.config.enable_early_termination {
            let mut total_tokens = 0;
            entries.retain(|e| {
                total_tokens += e.token_count;
                total_tokens <= budget
            });
        }

        let total_tokens = entries.iter().map(|e| e.token_count).sum();
        let layers_included: Vec<_> = entries
            .iter()
            .map(|e| e.layer)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let stale_entries: Vec<String> = entries
            .iter()
            .filter(|e| e.staleness_status == StalenessStatus::Stale)
            .map(|e| e.entry_id.clone())
            .collect();
        let has_stale_content = !stale_entries.is_empty();

        let view = self.create_view(&entries, budget);

        AssembledContext {
            view,
            entries,
            total_tokens,
            token_budget: budget,
            layers_included,
            query_embedding: query_embedding.cloned(),
            stale_entries,
            has_stale_content,
            timed_out: false,
            partial: false,
        }
    }

    fn create_view(&self, entries: &[ContextEntry], budget: u32) -> ContextView {
        let preferred_depths = match self.config.view_mode {
            ViewMode::Ax => vec![SummaryDepth::Sentence],
            ViewMode::Ux => vec![SummaryDepth::Paragraph, SummaryDepth::Sentence],
            ViewMode::Dx => vec![
                SummaryDepth::Detailed,
                SummaryDepth::Paragraph,
                SummaryDepth::Sentence,
            ],
        };

        let metadata = match self.config.view_mode {
            ViewMode::Ax => ContextMetadata::minimal(),
            ViewMode::Ux => ContextMetadata {
                view_type: "user_experience".to_string(),
                includes_trajectory_logs: false,
                includes_metrics: false,
                includes_traces: false,
            },
            ViewMode::Dx => ContextMetadata {
                view_type: "developer_experience".to_string(),
                includes_trajectory_logs: true,
                includes_metrics: true,
                includes_traces: true,
            },
        };

        let (content, _total_tokens) = self.build_view_content(entries, &preferred_depths, budget);

        ContextView { content, metadata }
    }

    fn build_view_content(
        &self,
        entries: &[ContextEntry],
        _preferred_depths: &[SummaryDepth],
        budget: u32,
    ) -> (String, u32) {
        let mut output = Vec::new();
        let mut used_tokens = 0;

        for entry in entries {
            let entry_tokens = entry.token_count;

            if used_tokens + entry_tokens > budget {
                break;
            }

            output.push(entry.content.clone());
            used_tokens += entry_tokens;
        }

        (output.join("\n\n"), used_tokens)
    }

    fn compute_relevance_score(
        &self,
        query_embedding: Option<&ContextVector>,
        source: &SummarySource,
    ) -> f32 {
        let base = match (query_embedding, &source.context_vector) {
            (Some(query), Some(source_vec)) => cosine_similarity(query, source_vec),
            _ => self.layer_base_score(source.layer),
        };
        // Task 6.4: add a small variant-role bonus so that Canonical entries
        // float to the top within the same cosine/layer band.
        // Bonus range: 0.00 (Exception) .. 0.04 (Canonical), keeping the
        // total score well below the cosine range and never crossing layer
        // boundaries (each layer step is 0.1).
        let role_bonus = (source.variant_precedence() as f32 - 1.0) * 0.01;
        (base + role_bonus).min(1.0)
    }

    fn layer_base_score(&self, layer: MemoryLayer) -> f32 {
        let position = self
            .config
            .layer_priorities
            .iter()
            .position(|&l| l == layer);

        match position {
            Some(idx) => 1.0 - (idx as f32 * 0.1),
            None => 0.5,
        }
    }

    fn distribute_token_budget(
        &self,
        scored_sources: &[(&SummarySource, f32)],
        budget: u32,
    ) -> HashMap<String, u32> {
        let mut allocations = HashMap::new();

        if scored_sources.is_empty() {
            return allocations;
        }

        let total_score: f32 = scored_sources.iter().map(|(_, s)| s).sum();

        if total_score <= 0.0 {
            let per_source = budget / scored_sources.len() as u32;
            for (source, _) in scored_sources {
                allocations.insert(source.entry_id.clone(), per_source);
            }
            return allocations;
        }

        for (source, score) in scored_sources {
            let proportion = score / total_score;
            let tokens = (budget as f32 * proportion).floor() as u32;
            allocations.insert(source.entry_id.clone(), tokens.max(50));
        }

        allocations
    }

    fn select_entries(
        &self,
        scored_sources: &[(&SummarySource, f32)],
        allocations: &HashMap<String, u32>,
    ) -> Vec<ContextEntry> {
        let mut entries = Vec::new();

        for (source, relevance) in scored_sources {
            let allocation = allocations.get(&source.entry_id).copied().unwrap_or(0);

            if let Some(entry) = self.select_best_summary(source, allocation, *relevance) {
                entries.push(entry);
            }
        }

        // Task 6.4: deterministic ordering
        //   1. Layer priority (Session > Project > Team > Org > Tenant by default)
        //   2. Variant-role precedence: Canonical first within same layer band
        //   3. Relevance score descending within same role+layer
        entries.sort_by(|a, b| {
            let layer_cmp = self.layer_order(a.layer).cmp(&self.layer_order(b.layer));
            if layer_cmp != std::cmp::Ordering::Equal {
                return layer_cmp;
            }
            // Higher relevance_score already encodes the variant-role bonus from
            // compute_relevance_score, so a single score comparison is sufficient.
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        entries
    }

    fn layer_order(&self, layer: MemoryLayer) -> usize {
        self.config
            .layer_priorities
            .iter()
            .position(|&l| l == layer)
            .unwrap_or(usize::MAX)
    }

    fn select_best_summary(
        &self,
        source: &SummarySource,
        allocation: u32,
        relevance: f32,
    ) -> Option<ContextEntry> {
        let depth_order = [
            SummaryDepth::Detailed,
            SummaryDepth::Paragraph,
            SummaryDepth::Sentence,
        ];

        for depth in depth_order {
            if let Some(summary) = source.summaries.get(&depth)
                && summary.token_count <= allocation
            {
                let staleness = self.check_staleness(source, summary);
                return Some(ContextEntry {
                    entry_id: source.entry_id.clone(),
                    layer: source.layer,
                    content: summary.content.clone(),
                    token_count: summary.token_count,
                    depth,
                    relevance_score: relevance,
                    context_vector: source.context_vector.clone(),
                    staleness_status: staleness,
                });
            }
        }

        for depth in depth_order {
            if let Some(summary) = source.summaries.get(&depth) {
                let staleness = self.check_staleness(source, summary);
                return Some(ContextEntry {
                    entry_id: source.entry_id.clone(),
                    layer: source.layer,
                    content: summary.content.clone(),
                    token_count: summary.token_count,
                    depth,
                    relevance_score: relevance,
                    context_vector: source.context_vector.clone(),
                    staleness_status: staleness,
                });
            }
        }

        source.full_content.as_ref().map(|content| ContextEntry {
            entry_id: source.entry_id.clone(),
            layer: source.layer,
            content: content.clone(),
            token_count: source.full_content_tokens.unwrap_or(0),
            depth: SummaryDepth::Detailed,
            relevance_score: relevance,
            context_vector: source.context_vector.clone(),
            staleness_status: StalenessStatus::Fresh,
        })
    }

    fn check_staleness(&self, source: &SummarySource, summary: &LayerSummary) -> StalenessStatus {
        let Some(current_content) = &source.current_source_content else {
            return StalenessStatus::Unknown;
        };

        let current_hash = compute_xxhash64(current_content.as_bytes());

        if summary.source_hash == current_hash {
            StalenessStatus::Fresh
        } else if let Some(content_hash) = &summary.content_hash {
            if *content_hash == current_hash {
                StalenessStatus::Fresh
            } else {
                StalenessStatus::Stale
            }
        } else {
            StalenessStatus::Stale
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let magnitude_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let magnitude_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if magnitude_a == 0.0 || magnitude_b == 0.0 {
        return 0.0;
    }

    dot_product / (magnitude_a * magnitude_b)
}

#[cfg(test)]
fn sample_source(id: &str, layer: MemoryLayer) -> SummarySource {
    let mut summaries = HashMap::new();
    summaries.insert(
        SummaryDepth::Sentence,
        LayerSummary {
            depth: SummaryDepth::Sentence,
            content: format!("Sentence summary for {id}"),
            token_count: 20,
            generated_at: 0,
            source_hash: "hash".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        },
    );
    summaries.insert(
        SummaryDepth::Paragraph,
        LayerSummary {
            depth: SummaryDepth::Paragraph,
            content: format!("Paragraph summary for {id} with more detail"),
            token_count: 100,
            generated_at: 0,
            source_hash: "hash".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        },
    );

    SummarySource {
        entry_id: id.to_string(),
        layer,
        summaries,
        context_vector: None,
        full_content: None,
        full_content_tokens: None,
        current_source_content: None,
        variant_role: None,
        relations: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_empty_sources() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let result = assembler.assemble_context(None, &[], None);

        assert!(result.entries.is_empty());
        assert_eq!(result.total_tokens, 0);
    }

    #[test]
    fn test_assemble_single_source() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![sample_source("entry1", MemoryLayer::Session)];

        let result = assembler.assemble_context(None, &sources, None);

        assert_eq!(result.entries.len(), 1);
        assert!(result.total_tokens > 0);
    }

    #[test]
    fn test_assemble_multiple_sources() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![
            sample_source("entry1", MemoryLayer::Session),
            sample_source("entry2", MemoryLayer::Project),
            sample_source("entry3", MemoryLayer::Team),
        ];

        let result = assembler.assemble_context(None, &sources, None);

        assert_eq!(result.entries.len(), 3);
        assert!(result.layers_included.contains(&MemoryLayer::Session));
        assert!(result.layers_included.contains(&MemoryLayer::Project));
        assert!(result.layers_included.contains(&MemoryLayer::Team));
    }

    #[test]
    fn test_layer_priority_ordering() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![
            sample_source("team1", MemoryLayer::Team),
            sample_source("session1", MemoryLayer::Session),
            sample_source("project1", MemoryLayer::Project),
        ];

        let result = assembler.assemble_context(None, &sources, None);

        assert_eq!(result.entries[0].layer, MemoryLayer::Session);
        assert_eq!(result.entries[1].layer, MemoryLayer::Project);
        assert_eq!(result.entries[2].layer, MemoryLayer::Team);
    }

    #[test]
    fn test_custom_token_budget() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![sample_source("entry1", MemoryLayer::Session)];

        let result = assembler.assemble_context(None, &sources, Some(100));

        assert_eq!(result.token_budget, 100);
    }

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let vec_a = vec![1.0, 2.0, 3.0];
        let vec_b = vec![1.0, 2.0, 3.0];

        let similarity = cosine_similarity(&vec_a, &vec_b);

        assert!((similarity - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let vec_a = vec![1.0, 0.0];
        let vec_b = vec![0.0, 1.0];

        let similarity = cosine_similarity(&vec_a, &vec_b);

        assert!(similarity.abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let vec_a = vec![1.0, 2.0, 3.0];
        let vec_b = vec![-1.0, -2.0, -3.0];

        let similarity = cosine_similarity(&vec_a, &vec_b);

        assert!((similarity + 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_cosine_similarity_empty_vectors() {
        let vec_a: Vec<f32> = vec![];
        let vec_b: Vec<f32> = vec![];

        let similarity = cosine_similarity(&vec_a, &vec_b);

        assert_eq!(similarity, 0.0);
    }

    #[test]
    fn test_cosine_similarity_mismatched_length() {
        let vec_a = vec![1.0, 2.0, 3.0];
        let vec_b = vec![1.0, 2.0];

        let similarity = cosine_similarity(&vec_a, &vec_b);

        assert_eq!(similarity, 0.0);
    }

    #[test]
    fn test_relevance_scoring_with_embeddings() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let query = vec![1.0, 0.0, 0.0];
        let mut source = sample_source("entry1", MemoryLayer::Session);
        source.context_vector = Some(vec![1.0, 0.0, 0.0]);

        let sources = vec![source];
        let result = assembler.assemble_context(Some(&query), &sources, None);

        assert_eq!(result.entries.len(), 1);
        assert!((result.entries[0].relevance_score - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_min_relevance_filtering() {
        let config = AssemblerConfig {
            min_relevance_score: 0.9,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let query = vec![1.0, 0.0, 0.0];
        let mut source = sample_source("entry1", MemoryLayer::Session);
        source.context_vector = Some(vec![0.0, 1.0, 0.0]);

        let sources = vec![source];
        let result = assembler.assemble_context(Some(&query), &sources, None);

        assert!(result.entries.is_empty());
    }

    #[test]
    fn test_select_appropriate_depth() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let mut source = sample_source("entry1", MemoryLayer::Session);
        source.summaries.insert(
            SummaryDepth::Detailed,
            LayerSummary {
                depth: SummaryDepth::Detailed,
                content: "Very detailed summary".to_string(),
                token_count: 300,
                generated_at: 0,
                source_hash: "hash".to_string(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let sources = vec![source];
        let result = assembler.assemble_context(None, &sources, Some(500));

        assert_eq!(result.entries[0].depth, SummaryDepth::Detailed);
    }

    #[test]
    fn test_fallback_to_smaller_summary() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let mut source = sample_source("entry1", MemoryLayer::Session);
        source.summaries.insert(
            SummaryDepth::Detailed,
            LayerSummary {
                depth: SummaryDepth::Detailed,
                content: "Very detailed summary".to_string(),
                token_count: 500,
                generated_at: 0,
                source_hash: "hash".to_string(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let sources = vec![source];
        let result = assembler.assemble_context(None, &sources, Some(150));

        assert!(result.entries[0].depth != SummaryDepth::Detailed);
    }

    #[test]
    fn test_assembled_context_content() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![
            sample_source("entry1", MemoryLayer::Session),
            sample_source("entry2", MemoryLayer::Project),
        ];

        let result = assembler.assemble_context(None, &sources, None);
        let content = result.content();

        assert!(content.contains("entry1"));
        assert!(content.contains("entry2"));
    }

    #[test]
    fn test_is_within_budget() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let sources = vec![sample_source("entry1", MemoryLayer::Session)];

        let result = assembler.assemble_context(None, &sources, Some(1000));

        assert!(result.is_within_budget());
    }

    #[test]
    fn test_fallback_to_full_content() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let source = SummarySource {
            entry_id: "entry1".to_string(),
            layer: MemoryLayer::Session,
            summaries: HashMap::new(),
            context_vector: None,
            full_content: Some("This is the full content".to_string()),
            full_content_tokens: Some(100),
            current_source_content: None,
            variant_role: None,
            relations: vec![],
        };

        let sources = vec![source];
        let result = assembler.assemble_context(None, &sources, None);

        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].content.contains("full content"));
    }

    #[test]
    fn test_staleness_fresh_when_source_hash_matches() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let content = "Test source content for hashing";
        let source_hash = compute_xxhash64(content.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary of test content".to_string(),
                token_count: 10,
                generated_at: 0,
                source_hash: source_hash.clone(),
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "entry1".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(content.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Fresh);
        assert!(!result.has_stale_content);
        assert!(result.stale_entries.is_empty());
    }

    #[test]
    fn test_staleness_stale_when_source_hash_mismatch() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let original_content = "Original source content";
        let modified_content = "Modified source content - changed!";
        let original_hash = compute_xxhash64(original_content.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary of original content".to_string(),
                token_count: 10,
                generated_at: 0,
                source_hash: original_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "entry1".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(modified_content.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Stale);
        assert!(result.has_stale_content);
        assert_eq!(result.stale_entries.len(), 1);
        assert!(result.stale_entries.contains(&"entry1".to_string()));
    }

    #[test]
    fn test_staleness_unknown_when_no_current_content() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let source = sample_source("entry1", MemoryLayer::Session);

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Unknown);
        assert!(!result.has_stale_content);
    }

    #[test]
    fn test_staleness_mixed_entries() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let fresh_content = "Fresh content here";
        let fresh_hash = compute_xxhash64(fresh_content.as_bytes());
        let mut fresh_summaries = HashMap::new();
        fresh_summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Fresh summary".to_string(),
                token_count: 10,
                generated_at: 0,
                source_hash: fresh_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let fresh_source = SummarySource {
            entry_id: "fresh_entry".to_string(),
            layer: MemoryLayer::Session,
            summaries: fresh_summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(fresh_content.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let stale_original = "Stale original";
        let stale_modified = "Stale modified content";
        let stale_hash = compute_xxhash64(stale_original.as_bytes());
        let mut stale_summaries = HashMap::new();
        stale_summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Stale summary".to_string(),
                token_count: 10,
                generated_at: 0,
                source_hash: stale_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let stale_source = SummarySource {
            entry_id: "stale_entry".to_string(),
            layer: MemoryLayer::Project,
            summaries: stale_summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(stale_modified.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[fresh_source, stale_source], None);

        assert_eq!(result.entries.len(), 2);
        assert!(result.has_stale_content);
        assert_eq!(result.stale_entries.len(), 1);
        assert!(result.stale_entries.contains(&"stale_entry".to_string()));

        let fresh_entry = result
            .entries
            .iter()
            .find(|e| e.entry_id == "fresh_entry")
            .unwrap();
        let stale_entry = result
            .entries
            .iter()
            .find(|e| e.entry_id == "stale_entry")
            .unwrap();

        assert_eq!(fresh_entry.staleness_status, StalenessStatus::Fresh);
        assert_eq!(stale_entry.staleness_status, StalenessStatus::Stale);
    }

    #[test]
    fn test_staleness_empty_content_hashes_correctly() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let empty_content = "";
        let empty_hash = compute_xxhash64(empty_content.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary of empty".to_string(),
                token_count: 5,
                generated_at: 0,
                source_hash: empty_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "empty_entry".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(empty_content.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Fresh);
    }

    #[test]
    fn test_staleness_large_content_hashes_correctly() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let large_content = "x".repeat(100_000);
        let large_hash = compute_xxhash64(large_content.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary of large content".to_string(),
                token_count: 10,
                generated_at: 0,
                source_hash: large_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "large_entry".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(large_content),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Fresh);
    }

    #[test]
    fn test_staleness_unicode_content_hashes_correctly() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let unicode_content = "日本語テスト 🚀 émojis and ñ special chars";
        let unicode_hash = compute_xxhash64(unicode_content.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary of unicode".to_string(),
                token_count: 8,
                generated_at: 0,
                source_hash: unicode_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "unicode_entry".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(unicode_content.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Fresh);
    }

    #[test]
    fn test_staleness_whitespace_changes_detected() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());
        let original = "Content with spaces";
        let modified = "Content  with  spaces";
        let original_hash = compute_xxhash64(original.as_bytes());

        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            LayerSummary {
                depth: SummaryDepth::Sentence,
                content: "Summary".to_string(),
                token_count: 5,
                generated_at: 0,
                source_hash: original_hash,
                content_hash: None,
                personalized: false,
                personalization_context: None,
            },
        );

        let source = SummarySource {
            entry_id: "ws_entry".to_string(),
            layer: MemoryLayer::Session,
            summaries,
            context_vector: None,
            full_content: None,
            full_content_tokens: None,
            current_source_content: Some(modified.to_string()),
            variant_role: None,
            relations: vec![],
        };

        let result = assembler.assemble_context(None, &[source], None);

        assert_eq!(result.entries[0].staleness_status, StalenessStatus::Stale);
    }

    #[test]
    fn test_cache_hit_returns_cached_result() {
        let config = AssemblerConfig {
            enable_caching: true,
            cache_ttl_secs: 3600,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let query = vec![1.0, 0.0, 0.0];

        let result1 = assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(result1.entries.len(), 1);

        let result2 = assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(result2.entries.len(), 1);
        assert_eq!(result2.total_tokens, result1.total_tokens);

        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 1);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_cache_miss_after_ttl_expiry() {
        use std::thread;
        use std::time::Duration;

        let config = AssemblerConfig {
            enable_caching: true,
            cache_ttl_secs: 1,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let query = vec![1.0, 0.0, 0.0];

        let _result1 = assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 1);

        thread::sleep(Duration::from_millis(1100));

        let result2 = assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(result2.entries.len(), 1);

        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_assembly_timeout_detection() {
        let config = AssemblerConfig {
            assembly_timeout_ms: 0,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let result = assembler.assemble_context(None, &sources, None);

        assert_eq!(result.entries.len(), 1);
        assert!(result.timed_out);
    }

    #[test]
    fn test_partial_context_return() {
        let config = AssemblerConfig {
            enable_early_termination: true,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let mut sources = vec![];
        for i in 0..10 {
            sources.push(sample_source(&format!("entry{}", i), MemoryLayer::Session));
        }

        let result = assembler.assemble_context(None, &sources, Some(500));

        assert!(result.partial);
        assert!(result.total_tokens < 500);
    }

    #[test]
    fn test_early_termination_disabled() {
        let config = AssemblerConfig {
            enable_early_termination: false,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let mut sources = vec![];
        for i in 0..10 {
            sources.push(sample_source(&format!("entry{}", i), MemoryLayer::Session));
        }

        let result = assembler.assemble_context(None, &sources, Some(500));

        assert!(!result.partial);
    }

    #[test]
    fn test_metrics_avg_latency() {
        let assembler = ContextAssembler::new(AssemblerConfig::default());

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];

        for _ in 0..10 {
            assembler.assemble_context(None, &sources, None);
        }

        let avg = assembler.metrics().avg_latency_ms();
        assert!(avg >= 0.0);
        assert_eq!(
            assembler.metrics().total_assemblies.load(Ordering::Relaxed),
            10
        );

        let avg = assembler.metrics().avg_latency_ms();
        assert!(avg >= 0.0);
        assert_eq!(
            assembler.metrics().total_assemblies.load(Ordering::Relaxed),
            10
        );
    }

    #[test]
    fn test_metrics_cache_hit_rate() {
        let config = AssemblerConfig {
            enable_caching: true,
            cache_ttl_secs: 3600,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let query = vec![1.0, 0.0, 0.0];

        for _ in 0..5 {
            assembler.assemble_context(Some(&query), &sources, None);
        }

        let hit_rate = assembler.metrics().cache_hit_rate();
        assert!(hit_rate > 0.0 && hit_rate < 1.0);
    }

    #[test]
    fn test_clear_cache() {
        let config = AssemblerConfig {
            enable_caching: true,
            cache_ttl_secs: 3600,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let query = vec![1.0, 0.0, 0.0];

        assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 1);

        assembler.clear_cache();

        assembler.assemble_context(Some(&query), &sources, None);
        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_different_token_budgets_cache_separately() {
        let config = AssemblerConfig {
            enable_caching: true,
            cache_ttl_secs: 3600,
            ..Default::default()
        };
        let assembler = ContextAssembler::new(config);

        let sources = vec![sample_source("entry1", MemoryLayer::Session)];
        let query = vec![1.0, 0.0, 0.0];

        assembler.assemble_context(Some(&query), &sources, Some(100));
        assembler.assemble_context(Some(&query), &sources, Some(200));

        assert_eq!(assembler.metrics().cache_hits.load(Ordering::Relaxed), 0);
        assert_eq!(assembler.metrics().cache_misses.load(Ordering::Relaxed), 2);
    }
}
