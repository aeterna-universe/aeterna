use std::collections::HashMap;

use mk_core::types::{LayerSummary, MemoryLayer, SummaryDepth};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewMode {
    Ax,
    Ux,
    Dx,
}

impl ViewMode {
    pub fn token_budget_multiplier(&self) -> f32 {
        match self {
            ViewMode::Ax => 0.3,
            ViewMode::Ux => 0.6,
            ViewMode::Dx => 1.0,
        }
    }

    pub fn preferred_depths(&self) -> Vec<SummaryDepth> {
        match self {
            ViewMode::Ax => vec![SummaryDepth::Sentence],
            ViewMode::Ux => vec![SummaryDepth::Paragraph, SummaryDepth::Sentence],
            ViewMode::Dx => vec![
                SummaryDepth::Detailed,
                SummaryDepth::Paragraph,
                SummaryDepth::Sentence,
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompressorConfig {
    pub base_token_budget: u32,
    pub layer_order: Vec<MemoryLayer>,
    pub enable_inheritance: bool,
    pub inheritance_compression_ratio: f32,
    pub min_tokens_per_layer: u32,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        Self {
            base_token_budget: 4000,
            layer_order: vec![
                MemoryLayer::Tenant,
                MemoryLayer::Org,
                MemoryLayer::Team,
                MemoryLayer::Project,
                MemoryLayer::Session,
            ],
            enable_inheritance: true,
            inheritance_compression_ratio: 0.5,
            min_tokens_per_layer: 50,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayerContent {
    pub layer: MemoryLayer,
    pub entries: Vec<LayerEntry>,
}

#[derive(Debug, Clone)]
pub struct LayerEntry {
    pub entry_id: String,
    pub summaries: HashMap<SummaryDepth, LayerSummary>,
    pub full_content: Option<String>,
    pub full_content_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct CompressedLayer {
    pub layer: MemoryLayer,
    pub entries: Vec<CompressedEntry>,
    pub inherited_context: Option<String>,
    pub inherited_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct CompressedEntry {
    pub entry_id: String,
    pub content: String,
    pub depth: SummaryDepth,
    pub token_count: u32,
    pub is_fallback: bool,
}

#[derive(Debug, Clone)]
pub struct CompressionResult {
    pub layers: Vec<CompressedLayer>,
    pub total_tokens: u32,
    pub token_budget: u32,
    pub view_mode: ViewMode,
}

impl CompressionResult {
    pub fn combined_content(&self) -> String {
        self.layers
            .iter()
            .flat_map(|layer| {
                let mut parts = Vec::new();
                if let Some(inherited) = &layer.inherited_context {
                    parts.push(inherited.clone());
                }
                parts.extend(layer.entries.iter().map(|e| e.content.clone()));
                parts
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn is_within_budget(&self) -> bool {
        self.total_tokens <= self.token_budget
    }
}

pub struct HierarchicalCompressor {
    config: CompressorConfig,
}

impl HierarchicalCompressor {
    pub fn new(config: CompressorConfig) -> Self {
        Self { config }
    }

    pub fn compress(
        &self,
        layers: &[LayerContent],
        view_mode: ViewMode,
        token_budget: Option<u32>,
    ) -> CompressionResult {
        let base_budget = token_budget.unwrap_or(self.config.base_token_budget);
        let adjusted_budget = (base_budget as f32 * view_mode.token_budget_multiplier()) as u32;

        let layer_budgets = self.distribute_budget_to_layers(layers, adjusted_budget);
        let preferred_depths = view_mode.preferred_depths();

        let mut compressed_layers = Vec::new();
        let mut inherited_context: Option<String> = None;
        let mut inherited_tokens: u32 = 0;

        for layer_content in self.order_layers(layers) {
            let layer_budget = layer_budgets
                .get(&layer_content.layer)
                .copied()
                .unwrap_or(self.config.min_tokens_per_layer);

            let available_budget = if self.config.enable_inheritance && inherited_tokens > 0 {
                layer_budget.saturating_sub(inherited_tokens)
            } else {
                layer_budget
            };

            let compressed = self.compress_layer(
                layer_content,
                available_budget,
                &preferred_depths,
                inherited_context.clone(),
                inherited_tokens,
            );

            if self.config.enable_inheritance && !compressed.entries.is_empty() {
                inherited_context =
                    Some(self.create_inherited_context(&compressed, &preferred_depths));
                inherited_tokens = self.estimate_tokens(inherited_context.as_deref().unwrap_or(""));
                inherited_tokens =
                    (inherited_tokens as f32 * self.config.inheritance_compression_ratio) as u32;
            }

            compressed_layers.push(compressed);
        }

        let total_tokens = compressed_layers.iter().map(|l| l.total_tokens).sum();

        CompressionResult {
            layers: compressed_layers,
            total_tokens,
            token_budget: adjusted_budget,
            view_mode,
        }
    }

    fn distribute_budget_to_layers(
        &self,
        layers: &[LayerContent],
        total_budget: u32,
    ) -> HashMap<MemoryLayer, u32> {
        let mut budgets = HashMap::new();

        if layers.is_empty() {
            return budgets;
        }

        let layer_count = layers.len() as u32;
        let min_total = self.config.min_tokens_per_layer * layer_count;

        if total_budget <= min_total {
            for layer in layers {
                budgets.insert(layer.layer, self.config.min_tokens_per_layer);
            }
            return budgets;
        }

        let weights: Vec<(MemoryLayer, f32)> = layers
            .iter()
            .map(|l| {
                let position = self
                    .config
                    .layer_order
                    .iter()
                    .position(|&x| x == l.layer)
                    .unwrap_or(self.config.layer_order.len());
                let weight = 1.0 + (position as f32 * 0.2);
                (l.layer, weight)
            })
            .collect();

        let total_weight: f32 = weights.iter().map(|(_, w)| w).sum();

        for (layer, weight) in weights {
            let proportion = weight / total_weight;
            let tokens =
                ((total_budget as f32 * proportion) as u32).max(self.config.min_tokens_per_layer);
            budgets.insert(layer, tokens);
        }

        budgets
    }

    fn order_layers<'a>(&self, layers: &'a [LayerContent]) -> Vec<&'a LayerContent> {
        let mut ordered: Vec<_> = layers.iter().collect();
        ordered.sort_by_key(|l| {
            self.config
                .layer_order
                .iter()
                .position(|&x| x == l.layer)
                .unwrap_or(usize::MAX)
        });
        ordered
    }

    fn compress_layer(
        &self,
        layer: &LayerContent,
        budget: u32,
        preferred_depths: &[SummaryDepth],
        inherited_context: Option<String>,
        inherited_tokens: u32,
    ) -> CompressedLayer {
        let mut entries = Vec::new();
        let mut remaining_budget = budget;

        let entry_budget = if !layer.entries.is_empty() {
            budget / layer.entries.len() as u32
        } else {
            budget
        };

        for entry in &layer.entries {
            if remaining_budget < self.config.min_tokens_per_layer {
                break;
            }

            let allocation = entry_budget.min(remaining_budget);
            if let Some(compressed) = self.select_best_content(entry, allocation, preferred_depths)
            {
                remaining_budget = remaining_budget.saturating_sub(compressed.token_count);
                entries.push(compressed);
            }
        }

        let entry_tokens: u32 = entries.iter().map(|e| e.token_count).sum();
        let total_tokens = entry_tokens + inherited_tokens;

        CompressedLayer {
            layer: layer.layer,
            entries,
            inherited_context,
            inherited_tokens,
            total_tokens,
        }
    }

    fn select_best_content(
        &self,
        entry: &LayerEntry,
        budget: u32,
        preferred_depths: &[SummaryDepth],
    ) -> Option<CompressedEntry> {
        for &depth in preferred_depths {
            if let Some(summary) = entry.summaries.get(&depth)
                && summary.token_count <= budget
            {
                return Some(CompressedEntry {
                    entry_id: entry.entry_id.clone(),
                    content: summary.content.clone(),
                    depth,
                    token_count: summary.token_count,
                    is_fallback: false,
                });
            }
        }

        let all_depths = [
            SummaryDepth::Sentence,
            SummaryDepth::Paragraph,
            SummaryDepth::Detailed,
        ];
        for depth in all_depths {
            if let Some(summary) = entry.summaries.get(&depth)
                && summary.token_count <= budget
            {
                return Some(CompressedEntry {
                    entry_id: entry.entry_id.clone(),
                    content: summary.content.clone(),
                    depth,
                    token_count: summary.token_count,
                    is_fallback: false,
                });
            }
        }

        for depth in all_depths {
            if let Some(summary) = entry.summaries.get(&depth) {
                return Some(CompressedEntry {
                    entry_id: entry.entry_id.clone(),
                    content: summary.content.clone(),
                    depth,
                    token_count: summary.token_count,
                    is_fallback: false,
                });
            }
        }

        entry.full_content.as_ref().map(|content| CompressedEntry {
            entry_id: entry.entry_id.clone(),
            content: content.clone(),
            depth: SummaryDepth::Detailed,
            token_count: entry
                .full_content_tokens
                .unwrap_or_else(|| self.estimate_tokens(content)),
            is_fallback: true,
        })
    }

    fn create_inherited_context(
        &self,
        layer: &CompressedLayer,
        preferred_depths: &[SummaryDepth],
    ) -> String {
        let shortest_depth = preferred_depths
            .last()
            .copied()
            .unwrap_or(SummaryDepth::Sentence);

        layer
            .entries
            .iter()
            .filter(|e| e.depth == shortest_depth || e.depth == SummaryDepth::Sentence)
            .map(|e| e.content.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    }

    fn estimate_tokens(&self, content: &str) -> u32 {
        let char_count = content.chars().count();
        (char_count / 4).max(1) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_summary(depth: SummaryDepth, content: &str, tokens: u32) -> LayerSummary {
        LayerSummary {
            depth,
            content: content.to_string(),
            token_count: tokens,
            generated_at: 0,
            source_hash: "test".to_string(),
            content_hash: None,
            personalized: false,
            personalization_context: None,
        }
    }

    fn sample_entry(id: &str) -> LayerEntry {
        let mut summaries = HashMap::new();
        summaries.insert(
            SummaryDepth::Sentence,
            sample_summary(SummaryDepth::Sentence, &format!("Sentence: {id}"), 20),
        );
        summaries.insert(
            SummaryDepth::Paragraph,
            sample_summary(
                SummaryDepth::Paragraph,
                &format!("Paragraph about {id}"),
                80,
            ),
        );
        summaries.insert(
            SummaryDepth::Detailed,
            sample_summary(
                SummaryDepth::Detailed,
                &format!("Detailed content for {id}"),
                200,
            ),
        );

        LayerEntry {
            entry_id: id.to_string(),
            summaries,
            full_content: None,
            full_content_tokens: None,
        }
    }

    fn sample_layer(layer: MemoryLayer, entry_ids: &[&str]) -> LayerContent {
        LayerContent {
            layer,
            entries: entry_ids.iter().map(|id| sample_entry(id)).collect(),
        }
    }

    #[test]
    fn test_compress_empty_layers() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let result = compressor.compress(&[], ViewMode::Dx, None);

        assert!(result.layers.is_empty());
        assert_eq!(result.total_tokens, 0);
    }

    #[test]
    fn test_compress_single_layer() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![sample_layer(MemoryLayer::Session, &["entry1"])];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert_eq!(result.layers.len(), 1);
        assert!(!result.layers[0].entries.is_empty());
    }

    #[test]
    fn test_compress_multiple_layers() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![
            sample_layer(MemoryLayer::Tenant, &["company1"]),
            sample_layer(MemoryLayer::Team, &["team1"]),
            sample_layer(MemoryLayer::Session, &["session1"]),
        ];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert_eq!(result.layers.len(), 3);
    }

    #[test]
    fn test_layer_ordering() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![
            sample_layer(MemoryLayer::Session, &["session1"]),
            sample_layer(MemoryLayer::Tenant, &["company1"]),
            sample_layer(MemoryLayer::Team, &["team1"]),
        ];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert_eq!(result.layers[0].layer, MemoryLayer::Tenant);
        assert_eq!(result.layers[1].layer, MemoryLayer::Team);
        assert_eq!(result.layers[2].layer, MemoryLayer::Session);
    }

    #[test]
    fn test_view_mode_ax_prefers_sentence() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![sample_layer(MemoryLayer::Session, &["entry1"])];

        let result = compressor.compress(&layers, ViewMode::Ax, Some(1000));

        assert_eq!(result.layers[0].entries[0].depth, SummaryDepth::Sentence);
    }

    #[test]
    fn test_view_mode_dx_prefers_detailed() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![sample_layer(MemoryLayer::Session, &["entry1"])];

        let result = compressor.compress(&layers, ViewMode::Dx, Some(1000));

        assert_eq!(result.layers[0].entries[0].depth, SummaryDepth::Detailed);
    }

    #[test]
    fn test_view_mode_budget_multiplier() {
        assert!((ViewMode::Ax.token_budget_multiplier() - 0.3).abs() < 0.001);
        assert!((ViewMode::Ux.token_budget_multiplier() - 0.6).abs() < 0.001);
        assert!((ViewMode::Dx.token_budget_multiplier() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_inheritance_creates_context() {
        let config = CompressorConfig {
            enable_inheritance: true,
            ..Default::default()
        };
        let compressor = HierarchicalCompressor::new(config);
        let layers = vec![
            sample_layer(MemoryLayer::Tenant, &["company1"]),
            sample_layer(MemoryLayer::Team, &["team1"]),
        ];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert!(result.layers[1].inherited_context.is_some());
    }

    #[test]
    fn test_inheritance_disabled() {
        let config = CompressorConfig {
            enable_inheritance: false,
            ..Default::default()
        };
        let compressor = HierarchicalCompressor::new(config);
        let layers = vec![
            sample_layer(MemoryLayer::Tenant, &["company1"]),
            sample_layer(MemoryLayer::Team, &["team1"]),
        ];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert!(result.layers[1].inherited_context.is_none());
    }

    #[test]
    fn test_fallback_to_full_content() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());

        let entry = LayerEntry {
            entry_id: "fallback".to_string(),
            summaries: HashMap::new(),
            full_content: Some("Full content here".to_string()),
            full_content_tokens: Some(50),
        };

        let layers = vec![LayerContent {
            layer: MemoryLayer::Session,
            entries: vec![entry],
        }];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert!(result.layers[0].entries[0].is_fallback);
        assert!(result.layers[0].entries[0].content.contains("Full content"));
    }

    #[test]
    fn test_empty_layer_entries() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![LayerContent {
            layer: MemoryLayer::Session,
            entries: vec![],
        }];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert_eq!(result.layers.len(), 1);
        assert!(result.layers[0].entries.is_empty());
    }

    #[test]
    fn test_combined_content() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![
            sample_layer(MemoryLayer::Tenant, &["company1"]),
            sample_layer(MemoryLayer::Session, &["session1"]),
        ];

        let result = compressor.compress(&layers, ViewMode::Dx, None);
        let content = result.combined_content();

        assert!(content.contains("company1"));
        assert!(content.contains("session1"));
    }

    #[test]
    fn test_is_within_budget() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![sample_layer(MemoryLayer::Session, &["entry1"])];

        let result = compressor.compress(&layers, ViewMode::Dx, Some(10000));

        assert!(result.is_within_budget());
    }

    #[test]
    fn test_progressive_compression_tight_budget() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());
        let layers = vec![sample_layer(MemoryLayer::Session, &["e1", "e2", "e3"])];

        let result = compressor.compress(&layers, ViewMode::Ax, Some(100));

        for entry in &result.layers[0].entries {
            assert_eq!(entry.depth, SummaryDepth::Sentence);
        }
    }

    #[test]
    fn test_missing_summaries_skips_entry() {
        let compressor = HierarchicalCompressor::new(CompressorConfig::default());

        let entry = LayerEntry {
            entry_id: "empty".to_string(),
            summaries: HashMap::new(),
            full_content: None,
            full_content_tokens: None,
        };

        let layers = vec![LayerContent {
            layer: MemoryLayer::Session,
            entries: vec![entry],
        }];

        let result = compressor.compress(&layers, ViewMode::Dx, None);

        assert!(result.layers[0].entries.is_empty());
    }
}
