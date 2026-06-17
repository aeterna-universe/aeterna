//! Bootstrap module for RLM policy pre-training.
//!
//! Provides synthetic task templates and offline training pipeline
//! to initialize the RLM decomposition policy before real user queries.

use crate::rlm::strategy::{AggregationStrategy, DecompositionAction};
use crate::rlm::trainer::{
    DecompositionTrainer, DecompositionTrajectory, RewardConfig, TimestampedAction, TrainingOutcome,
};
use chrono::Utc;
use mk_core::types::{MemoryLayer, TenantContext};
use serde::{Deserialize, Serialize};

/// Template for generating synthetic training tasks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapTaskTemplate {
    pub name: String,
    pub query_pattern: String,
    pub expected_actions: Vec<DecompositionAction>,
    pub expected_outcome: TrainingOutcome,
    pub complexity_level: ComplexityLevel,
}

/// Complexity classification for bootstrap tasks.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComplexityLevel {
    /// Single-layer search with direct aggregation.
    Simple,
    /// Multi-layer search across 2-3 layers.
    Moderate,
    /// Deep traversal with drill-down and recursive calls.
    Complex,
}

impl BootstrapTaskTemplate {
    pub fn simple(name: &str, query: &str) -> Self {
        Self {
            name: name.to_string(),
            query_pattern: query.to_string(),
            expected_actions: vec![
                DecompositionAction::SearchLayer {
                    layer: MemoryLayer::Project,
                    query: query.to_string(),
                },
                DecompositionAction::Aggregate {
                    strategy: AggregationStrategy::Summary,
                    results: vec![],
                },
            ],
            expected_outcome: TrainingOutcome::ResultUsed { quality_score: 0.8 },
            complexity_level: ComplexityLevel::Simple,
        }
    }

    pub fn moderate(name: &str, query: &str, layers: Vec<MemoryLayer>) -> Self {
        let mut actions: Vec<DecompositionAction> = layers
            .into_iter()
            .map(|layer| DecompositionAction::SearchLayer {
                layer,
                query: query.to_string(),
            })
            .collect();

        actions.push(DecompositionAction::Aggregate {
            strategy: AggregationStrategy::Union,
            results: vec![],
        });

        Self {
            name: name.to_string(),
            query_pattern: query.to_string(),
            expected_actions: actions,
            expected_outcome: TrainingOutcome::ResultUsed {
                quality_score: 0.85,
            },
            complexity_level: ComplexityLevel::Moderate,
        }
    }

    pub fn complex(name: &str, query: &str) -> Self {
        Self {
            name: name.to_string(),
            query_pattern: query.to_string(),
            expected_actions: vec![
                DecompositionAction::SearchLayer {
                    layer: MemoryLayer::Team,
                    query: "related patterns".to_string(),
                },
                DecompositionAction::DrillDown {
                    memory_id: "discovered-memory-id".to_string(),
                    query: "specific details".to_string(),
                },
                DecompositionAction::RecursiveCall {
                    sub_query: format!("details about {}", query),
                },
                DecompositionAction::Aggregate {
                    strategy: AggregationStrategy::Summary,
                    results: vec![],
                },
            ],
            expected_outcome: TrainingOutcome::ResultUsed { quality_score: 0.9 },
            complexity_level: ComplexityLevel::Complex,
        }
    }

    pub fn to_trajectory(&self, tenant: &TenantContext) -> DecompositionTrajectory {
        let now = Utc::now();

        let actions: Vec<TimestampedAction> = self
            .expected_actions
            .iter()
            .enumerate()
            .map(|(i, action)| {
                let reward = match i {
                    _ if i == self.expected_actions.len() - 1 => 0.5,
                    _ => 0.1,
                };

                TimestampedAction {
                    action: action.clone(),
                    timestamp: now,
                    intermediate_reward: reward,
                }
            })
            .collect();

        let outcome_reward = match &self.expected_outcome {
            TrainingOutcome::ResultUsed { quality_score } => *quality_score,
            TrainingOutcome::QueryRefined { .. } => 0.3,
            TrainingOutcome::ResultIgnored => -0.5,
            TrainingOutcome::NoSignal => 0.0,
        };

        DecompositionTrajectory {
            id: uuid::Uuid::new_v4().to_string(),
            query: self.query_pattern.clone(),
            tenant_context: tenant.clone(),
            started_at: now,
            completed_at: Some(now),
            actions,
            result_count: 1,
            outcome: Some(self.expected_outcome.clone()),
            tokens_used: match self.complexity_level {
                ComplexityLevel::Simple => 5_000,
                ComplexityLevel::Moderate => 15_000,
                ComplexityLevel::Complex => 30_000,
            },
            max_depth: self.expected_actions.len() as u8,
            reward: Some(outcome_reward),
        }
    }
}

pub fn default_bootstrap_templates() -> Vec<BootstrapTaskTemplate> {
    vec![
        BootstrapTaskTemplate::simple("simple_config_lookup", "show me the database config"),
        BootstrapTaskTemplate::simple("simple_api_lookup", "what is the login endpoint"),
        BootstrapTaskTemplate::simple("simple_file_lookup", "where is the main entry point"),
        BootstrapTaskTemplate::moderate(
            "multi_layer_search",
            "authentication patterns",
            vec![MemoryLayer::Project, MemoryLayer::Team],
        ),
        BootstrapTaskTemplate::moderate(
            "team_wide_patterns",
            "error handling conventions",
            vec![MemoryLayer::Team, MemoryLayer::Org],
        ),
        BootstrapTaskTemplate::moderate(
            "org_standards",
            "coding guidelines",
            vec![MemoryLayer::Org, MemoryLayer::Tenant],
        ),
        BootstrapTaskTemplate::complex(
            "complex_comparison",
            "compare authentication patterns across all teams",
        ),
        BootstrapTaskTemplate::complex(
            "complex_evolution",
            "trace the evolution of our API design",
        ),
        BootstrapTaskTemplate::complex(
            "complex_aggregation",
            "summarize all database decisions since last quarter",
        ),
    ]
}

pub fn generate_bootstrap_tasks(
    templates: &[BootstrapTaskTemplate],
    tenant: &TenantContext,
    multiplier: usize,
) -> Vec<DecompositionTrajectory> {
    let mut tasks = Vec::with_capacity(templates.len() * multiplier);

    for template in templates {
        for _ in 0..multiplier {
            tasks.push(template.to_trajectory(tenant));
        }
    }

    tasks
}

pub struct BootstrapTrainer {
    trainer: DecompositionTrainer,
    templates: Vec<BootstrapTaskTemplate>,
    trained_count: usize,
}

impl BootstrapTrainer {
    pub fn new(reward_config: RewardConfig) -> Self {
        Self {
            trainer: DecompositionTrainer::new(reward_config),
            templates: default_bootstrap_templates(),
            trained_count: 0,
        }
    }

    pub fn with_templates(mut self, templates: Vec<BootstrapTaskTemplate>) -> Self {
        self.templates = templates;
        self
    }

    pub async fn bootstrap(
        &mut self,
        tenant: &TenantContext,
        iterations: usize,
    ) -> anyhow::Result<BootstrapResult> {
        let tasks = generate_bootstrap_tasks(&self.templates, tenant, iterations);
        let total_tasks = tasks.len();
        let mut total_reward = 0.0;

        for task in tasks {
            self.trainer.train(&task).await?;
            total_reward += task.reward.unwrap_or(0.0);
            self.trained_count += 1;
        }

        let avg_reward = if total_tasks > 0 {
            total_reward / total_tasks as f32
        } else {
            0.0
        };

        Ok(BootstrapResult {
            tasks_trained: total_tasks,
            average_reward: avg_reward,
            final_epsilon: self.trainer.epsilon(),
        })
    }

    pub fn trainer(&self) -> &DecompositionTrainer {
        &self.trainer
    }

    pub fn into_trainer(self) -> DecompositionTrainer {
        self.trainer
    }

    pub fn trained_count(&self) -> usize {
        self.trained_count
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapResult {
    pub tasks_trained: usize,
    pub average_reward: f32,
    pub final_epsilon: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn test_tenant() -> TenantContext {
        use mk_core::types::{TenantId, UserId};
        TenantContext::new(
            TenantId::from_str("test-tenant").unwrap(),
            UserId::from_str("test-user").unwrap(),
        )
    }

    #[test]
    fn test_simple_template_creates_valid_trajectory() {
        let template = BootstrapTaskTemplate::simple("test", "simple query");
        let trajectory = template.to_trajectory(&test_tenant());

        assert_eq!(trajectory.query, "simple query");
        assert_eq!(trajectory.actions.len(), 2);
        assert!(trajectory.reward.is_some());
        assert!(trajectory.outcome.is_some());
    }

    #[test]
    fn test_moderate_template_creates_multi_layer_trajectory() {
        let template = BootstrapTaskTemplate::moderate(
            "test",
            "multi query",
            vec![MemoryLayer::Project, MemoryLayer::Team],
        );
        let trajectory = template.to_trajectory(&test_tenant());

        assert_eq!(trajectory.actions.len(), 3);
        assert_eq!(trajectory.complexity_level(), ComplexityLevel::Moderate);
    }

    #[test]
    fn test_complex_template_creates_deep_trajectory() {
        let template = BootstrapTaskTemplate::complex("test", "complex query");
        let trajectory = template.to_trajectory(&test_tenant());

        assert_eq!(trajectory.actions.len(), 4);
        assert!(trajectory.tokens_used > 20_000);
    }

    #[test]
    fn test_default_templates_cover_all_complexity_levels() {
        let templates = default_bootstrap_templates();

        let simple_count = templates
            .iter()
            .filter(|t| t.complexity_level == ComplexityLevel::Simple)
            .count();
        let moderate_count = templates
            .iter()
            .filter(|t| t.complexity_level == ComplexityLevel::Moderate)
            .count();
        let complex_count = templates
            .iter()
            .filter(|t| t.complexity_level == ComplexityLevel::Complex)
            .count();

        assert!(simple_count >= 2, "Should have at least 2 simple templates");
        assert!(
            moderate_count >= 2,
            "Should have at least 2 moderate templates"
        );
        assert!(
            complex_count >= 2,
            "Should have at least 2 complex templates"
        );
    }

    #[test]
    fn test_generate_bootstrap_tasks_multiplies_correctly() {
        let templates = vec![BootstrapTaskTemplate::simple("test", "query")];
        let tasks = generate_bootstrap_tasks(&templates, &test_tenant(), 5);

        assert_eq!(tasks.len(), 5);
    }

    #[tokio::test]
    async fn test_bootstrap_trainer_completes_training() {
        let mut trainer = BootstrapTrainer::new(RewardConfig::default())
            .with_templates(vec![BootstrapTaskTemplate::simple("test", "simple query")]);

        let result = trainer.bootstrap(&test_tenant(), 3).await.unwrap();

        assert_eq!(result.tasks_trained, 3);
        assert!(result.average_reward > 0.0);
        assert_eq!(trainer.trained_count(), 3);
    }

    #[tokio::test]
    async fn test_bootstrap_trainer_improves_policy() {
        let mut trainer = BootstrapTrainer::new(RewardConfig::default());

        let initial_epsilon = trainer.trainer().epsilon();

        trainer.bootstrap(&test_tenant(), 200).await.unwrap();

        assert!(
            trainer.trainer().epsilon() <= initial_epsilon,
            "Epsilon should not increase during training"
        );
    }

    impl DecompositionTrajectory {
        fn complexity_level(&self) -> ComplexityLevel {
            match self.max_depth {
                0..=2 => ComplexityLevel::Simple,
                3 => ComplexityLevel::Moderate,
                _ => ComplexityLevel::Complex,
            }
        }
    }
}
