use serde::{Deserialize, Serialize};
use validator::Validate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StalenessPolicy {
    #[default]
    ServeStaleWarn,
    RegenerateBlocking,
    RegenerateAsync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    All,
    Sampled,
    ErrorsOnly,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct CcaConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub context_architect: ContextArchitectConfig,

    #[serde(default)]
    pub note_taking: NoteTakingConfig,

    #[serde(default)]
    pub hindsight: HindsightConfig,

    #[serde(default)]
    pub meta_agent: MetaAgentConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct ContextArchitectConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub default_token_budget: u32,

    #[serde(default)]
    pub layer_priorities: Vec<String>,

    #[serde(default)]
    pub min_relevance_score: f32,

    #[serde(default)]
    pub enable_caching: bool,

    #[serde(default = "default_cache_ttl_secs")]
    pub cache_ttl_secs: u64,

    #[serde(default)]
    pub staleness_policy: StalenessPolicy,

    #[serde(default = "default_assembly_timeout_ms")]
    pub assembly_timeout_ms: u64,

    #[serde(default = "default_enable_parallel_queries")]
    pub enable_parallel_queries: bool,

    #[serde(default = "default_enable_early_termination")]
    pub enable_early_termination: bool,
}

fn default_cache_ttl_secs() -> u64 {
    300
}

fn default_assembly_timeout_ms() -> u64 {
    100
}

fn default_enable_parallel_queries() -> bool {
    true
}

fn default_enable_early_termination() -> bool {
    true
}

impl Default for ContextArchitectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_token_budget: 4000,
            layer_priorities: vec![
                "session".to_string(),
                "project".to_string(),
                "team".to_string(),
                "org".to_string(),
                "tenant".to_string(),
            ],
            min_relevance_score: 0.3,
            enable_caching: true,
            cache_ttl_secs: 300,
            staleness_policy: StalenessPolicy::default(),
            assembly_timeout_ms: default_assembly_timeout_ms(),
            enable_parallel_queries: default_enable_parallel_queries(),
            enable_early_termination: default_enable_early_termination(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct NoteTakingConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub auto_distill_threshold: usize,

    #[serde(default)]
    pub manual_trigger_enabled: bool,

    #[serde(default)]
    pub sensitive_patterns_enabled: bool,

    #[serde(default)]
    pub capture_mode: CaptureMode,

    #[serde(default = "default_sampling_rate")]
    pub sampling_rate: u32,

    #[serde(default = "default_overhead_budget_ms")]
    pub overhead_budget_ms: u64,

    #[serde(default = "default_queue_size")]
    pub queue_size: usize,

    #[serde(default = "default_batch_size")]
    pub batch_size: usize,

    #[serde(default = "default_batch_flush_ms")]
    pub batch_flush_ms: u64,
}

fn default_sampling_rate() -> u32 {
    10
}

fn default_overhead_budget_ms() -> u64 {
    5
}

fn default_queue_size() -> usize {
    1000
}

fn default_batch_size() -> usize {
    10
}

fn default_batch_flush_ms() -> u64 {
    100
}

impl Default for NoteTakingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_distill_threshold: 10,
            manual_trigger_enabled: true,
            sensitive_patterns_enabled: true,
            capture_mode: CaptureMode::default(),
            sampling_rate: default_sampling_rate(),
            overhead_budget_ms: default_overhead_budget_ms(),
            queue_size: default_queue_size(),
            batch_size: default_batch_size(),
            batch_flush_ms: default_batch_flush_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct HindsightConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub semantic_threshold: f32,

    #[serde(default = "default_max_results")]
    pub max_results: usize,

    #[serde(default = "default_promotion_threshold")]
    pub promotion_threshold: f32,

    #[serde(default)]
    pub auto_capture_enabled: bool,
}

fn default_max_results() -> usize {
    5
}

fn default_promotion_threshold() -> f32 {
    0.8
}

impl Default for HindsightConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            semantic_threshold: 0.8,
            max_results: 5,
            promotion_threshold: 0.8,
            auto_capture_enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Validate, PartialEq)]
pub struct MetaAgentConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,

    #[serde(default = "default_iteration_timeout_secs")]
    pub iteration_timeout_secs: u64,

    #[serde(default = "default_build_timeout_secs")]
    pub build_timeout_secs: u64,

    #[serde(default = "default_test_timeout_secs")]
    pub test_timeout_secs: u64,

    #[serde(default)]
    pub auto_escalate_on_failure: bool,
}

fn default_max_iterations() -> u32 {
    3
}

fn default_iteration_timeout_secs() -> u64 {
    300
}

fn default_build_timeout_secs() -> u64 {
    120
}

fn default_test_timeout_secs() -> u64 {
    60
}

impl Default for MetaAgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_iterations: 3,
            iteration_timeout_secs: 300,
            build_timeout_secs: 120,
            test_timeout_secs: 60,
            auto_escalate_on_failure: true,
        }
    }
}

impl Default for CcaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            context_architect: ContextArchitectConfig::default(),
            note_taking: NoteTakingConfig::default(),
            hindsight: HindsightConfig::default(),
            meta_agent: MetaAgentConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cca_config_default() {
        let config = CcaConfig::default();
        assert!(config.enabled);
        assert_eq!(config.context_architect.default_token_budget, 4000);
        assert_eq!(config.hindsight.max_results, 5);
        assert_eq!(config.meta_agent.max_iterations, 3);
    }

    #[test]
    fn test_context_architect_config_default() {
        let config = ContextArchitectConfig::default();
        assert!(config.enabled);
        assert_eq!(config.default_token_budget, 4000);
        assert_eq!(config.min_relevance_score, 0.3);
        assert!(!config.layer_priorities.is_empty());
    }

    #[test]
    fn test_note_taking_config_default() {
        let config = NoteTakingConfig::default();
        assert!(config.enabled);
        assert_eq!(config.auto_distill_threshold, 10);
        assert!(config.manual_trigger_enabled);
    }

    #[test]
    fn test_hindsight_config_default() {
        let config = HindsightConfig::default();
        assert!(config.enabled);
        assert_eq!(config.semantic_threshold, 0.8);
        assert_eq!(config.max_results, 5);
        assert_eq!(config.promotion_threshold, 0.8);
    }

    #[test]
    fn test_meta_agent_config_default() {
        let config = MetaAgentConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_iterations, 3);
        assert_eq!(config.iteration_timeout_secs, 300);
        assert_eq!(config.build_timeout_secs, 120);
        assert_eq!(config.test_timeout_secs, 60);
    }

    #[test]
    fn test_cca_config_validation() {
        let mut config = CcaConfig::default();
        assert!(config.validate().is_ok());

        config.context_architect.min_relevance_score = 2.0;
        assert!(config.validate().is_ok());

        config.context_architect.min_relevance_score = -1.0;
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = CcaConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: CcaConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config, deserialized);
    }
}
