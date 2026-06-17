//! RLM Executor for decomposition-based memory search.

use crate::rlm::strategy::{ActionExecutor, DecompositionAction};
use metrics::counter;
use mk_core::traits::LlmService;
use mk_core::types::{SearchQuery, SearchResult, TenantContext};
use std::sync::Arc;
use std::time::Instant;

/// Single step in an RLM execution trajectory.
pub struct TrajectoryStep {
    pub action: DecompositionAction,
    pub observation: String,
    pub reward: f32,
    pub involved_memory_ids: Vec<String>,
}

/// Complete trajectory from RLM execution.
pub struct RlmTrajectory {
    pub query: SearchQuery,
    pub steps: Vec<TrajectoryStep>,
    pub total_reward: f32,
}

/// Executes RLM decomposition strategies using an LLM.
pub struct RlmExecutor {
    llm: Arc<dyn LlmService<Error = anyhow::Error>>,
    strategy_executor: Arc<dyn ActionExecutor>,
    config: config::RlmConfig,
}

impl RlmExecutor {
    pub fn new(
        llm: Arc<dyn LlmService<Error = anyhow::Error>>,
        strategy_executor: Arc<dyn ActionExecutor>,
        config: config::RlmConfig,
    ) -> Self {
        Self {
            llm,
            strategy_executor,
            config,
        }
    }

    #[tracing::instrument(skip(self, tenant), fields(query_text = %query.text, tenant_id = %tenant.tenant_id))]
    pub async fn execute(
        &self,
        query: SearchQuery,
        tenant: &TenantContext,
    ) -> anyhow::Result<(Vec<SearchResult>, RlmTrajectory)> {
        let start = Instant::now();
        counter!("rlm_search_requests_total").increment(1);

        let mut trajectory = RlmTrajectory {
            query: query.clone(),
            steps: Vec::new(),
            total_reward: 0.0,
        };

        let mut current_observation = format!("Initial query: {}", query.text);
        let mut current_depth: u8 = 0;

        for _ in 0..self.config.max_steps {
            let prompt = format!(
                r#"You are an expert search decomposition engine.
Your goal is to answer the query: '{}'
Current observations:
{}

Available actions (as JSON):
1. {{"SearchLayer": {{"layer": "Session|Project|Team|Org|Tenant", "query": "..."}}}} - Search a specific memory layer.
2. {{"DrillDown": {{"memory_id": "...", "query": "..."}}}} - Explore a specific memory in detail.
3. {{"Filter": {{"criteria": "regex", "results": ["...", "..."]}}}} - Filter current observations using a regex criteria.
4. {{"RecursiveCall": {{"sub_query": "..."}}}} - Execute a sub-query.
5. {{"Aggregate": {{"strategy": "Union|Intersection|Difference|Summary", "results": ["...", "..."]}}}} - TERMINAL ACTION: Combine findings into a final answer.

Rules:
- You have a maximum of {} steps.
- Start by searching relevant layers.
- Once you have enough information, use 'Aggregate' with 'Summary' strategy to provide the final synthesized result.
- ALWAYS output valid JSON for the action.

Next decomposition action:"#,
                query.text, current_observation, self.config.max_steps
            );

            let action_json = self.llm.generate(&prompt).await?;
            let action: DecompositionAction = serde_json::from_str(&action_json)?;

            let (observation, involved_ids) = self
                .strategy_executor
                .execute(action.clone(), tenant)
                .await?;

            let reward = if observation.is_empty() { -0.1 } else { 0.1 };

            current_depth += 1;

            trajectory.steps.push(TrajectoryStep {
                action: action.clone(),
                observation: observation.clone(),
                reward,
                involved_memory_ids: involved_ids,
            });

            current_observation = observation;

            if matches!(action, DecompositionAction::Aggregate { .. }) {
                break;
            }
        }

        trajectory.total_reward = trajectory.steps.iter().map(|s| s.reward).sum();

        let duration = start.elapsed();
        metrics::histogram!("rlm_execution_duration_seconds").record(duration.as_secs_f64());
        metrics::histogram!("rlm_execution_depth").record(current_depth as f64);

        tracing::debug!(
            duration_ms = duration.as_millis() as u64,
            depth = current_depth,
            total_reward = trajectory.total_reward,
            "RLM execution completed"
        );

        if trajectory.total_reward > 0.0 {
            tracing::info!(
                "RLM query '{}' completed with positive reward: {}",
                query.text,
                trajectory.total_reward
            );
        }

        Ok((
            vec![SearchResult {
                content: current_observation.clone(),
                score: 0.95,
                layer: query
                    .target_layers
                    .first()
                    .cloned()
                    .unwrap_or(mk_core::types::MemoryLayer::Project),
                metadata: serde_json::json!({
                    "rlm_synthesized": true,
                    "steps": trajectory.steps.len(),
                    "total_reward": trajectory.total_reward
                }),
                memory_id: format!("rlm-{}", uuid::Uuid::new_v4()),
            }],
            trajectory,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rlm::strategy::{AggregationStrategy, DecompositionAction, StrategyExecutor};
    use mk_core::traits::LlmService;
    use mk_core::types::{SearchQuery, TenantContext, ValidationResult};
    use std::sync::Arc;

    struct MockLlm {
        responses: std::sync::Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl LlmService for MockLlm {
        type Error = anyhow::Error;

        async fn generate(&self, _prompt: &str) -> Result<String, Self::Error> {
            let mut resps = self.responses.lock().unwrap();
            if resps.is_empty() {
                return Err(anyhow::anyhow!("No more responses"));
            }
            Ok(resps.remove(0))
        }

        async fn analyze_drift(
            &self,
            _content: &str,
            _policies: &[mk_core::types::Policy],
        ) -> Result<ValidationResult, Self::Error> {
            Ok(ValidationResult {
                is_valid: true,
                violations: vec![],
            })
        }
    }

    #[tokio::test]
    async fn test_rlm_executor_multi_hop() {
        let action1 = DecompositionAction::SearchLayer {
            layer: mk_core::types::MemoryLayer::Project,
            query: "query 1".to_string(),
        };
        let action2 = DecompositionAction::Aggregate {
            strategy: AggregationStrategy::Summary,
            results: vec!["Step 1 done".to_string(), "Final answer".to_string()],
        };

        let llm = Arc::new(MockLlm {
            responses: std::sync::Mutex::new(vec![
                serde_json::to_string(&action1).unwrap(),
                serde_json::to_string(&action2).unwrap(),
            ]),
        });
        let strategy_executor = Arc::new(StrategyExecutor::new(Arc::new(
            knowledge::manager::KnowledgeManager::new(
                Arc::new(knowledge::repository::GitRepository::new_mock()),
                Arc::new(knowledge::governance::GovernanceEngine::new()),
            ),
        )));
        let executor = RlmExecutor::new(llm, strategy_executor, config::RlmConfig::default());

        let query = SearchQuery {
            text: "test query".to_string(),
            ..Default::default()
        };
        let tenant = {
            use std::str::FromStr;
            TenantContext::new(
                mk_core::types::TenantId::from_str("test-tenant").unwrap(),
                mk_core::types::UserId::from_str("test-user").unwrap(),
            )
        };

        let (results, _traj) = executor.execute(query, &tenant).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            results[0]
                .content
                .contains("Summary of results: Step 1 done Final answer")
        );
    }
}
