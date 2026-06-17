//! Decomposition strategies and action execution.

use knowledge::manager::KnowledgeManager;
use mk_core::types::{MemoryLayer, TenantContext};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Trait for executing decomposition actions.
#[async_trait::async_trait]
pub trait ActionExecutor: Send + Sync {
    async fn execute(
        &self,
        action: DecompositionAction,
        tenant: &TenantContext,
    ) -> anyhow::Result<(String, Vec<String>)>;
}

/// Actions available for query decomposition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DecompositionAction {
    SearchLayer {
        layer: MemoryLayer,
        query: String,
    },
    DrillDown {
        memory_id: String,
        query: String,
    },
    Filter {
        criteria: String,
        results: Vec<String>,
    },
    RecursiveCall {
        sub_query: String,
    },
    Aggregate {
        strategy: AggregationStrategy,
        results: Vec<String>,
    },
}

/// Strategies for aggregating search results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AggregationStrategy {
    Union,
    Intersection,
    Difference,
    Summary,
}

/// Executes decomposition actions against knowledge and graph stores.
pub struct StrategyExecutor {
    knowledge_manager: Arc<KnowledgeManager>,
    graph_store: Option<
        Arc<
            dyn storage::graph::GraphStore<Error = Box<dyn std::error::Error + Send + Sync>>
                + Send
                + Sync,
        >,
    >,
}

impl StrategyExecutor {
    pub fn new(knowledge_manager: Arc<KnowledgeManager>) -> Self {
        Self {
            knowledge_manager,
            graph_store: None,
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
}

#[async_trait::async_trait]
impl ActionExecutor for StrategyExecutor {
    async fn execute(
        &self,
        action: DecompositionAction,
        tenant: &TenantContext,
    ) -> anyhow::Result<(String, Vec<String>)> {
        match action {
            DecompositionAction::SearchLayer { layer, query } => {
                let knowledge_layer: Option<mk_core::types::KnowledgeLayer> = layer.into();
                let layer_vec = if let Some(kl) = knowledge_layer {
                    vec![kl]
                } else {
                    vec![]
                };

                let results = self
                    .knowledge_manager
                    .query(tenant.clone(), &query, layer_vec, 10)
                    .await?;

                let memory_ids: Vec<String> = results.iter().map(|e| e.path.clone()).collect();

                let output = results
                    .into_iter()
                    .map(|e: mk_core::types::KnowledgeEntry| e.content)
                    .collect::<Vec<_>>()
                    .join("\n---\n");

                Ok((output, memory_ids))
            }
            DecompositionAction::DrillDown { memory_id, query } => {
                let mut output = Vec::new();
                let mut discovered_ids = vec![memory_id.clone()];

                let layers = vec![
                    mk_core::types::KnowledgeLayer::Project,
                    mk_core::types::KnowledgeLayer::Team,
                    mk_core::types::KnowledgeLayer::Org,
                    mk_core::types::KnowledgeLayer::Tenant,
                ];

                for layer in layers {
                    if let Ok(Some(entry)) = self
                        .knowledge_manager
                        .get(tenant.clone(), layer, &memory_id)
                        .await
                    {
                        output.push(format!("Primary Entry:\n{}", entry.content));
                        break;
                    }
                }

                if let Some(graph) = &self.graph_store
                    && let Ok(neighbors) = graph.get_neighbors(tenant.clone(), &memory_id).await
                    && !neighbors.is_empty()
                {
                    output.push("\nRelated Entries (Graph):".to_string());
                    for (edge, node) in neighbors {
                        if query.is_empty()
                            || node.label.to_lowercase().contains(&query.to_lowercase())
                            || node.id.to_lowercase().contains(&query.to_lowercase())
                        {
                            output.push(format!(
                                "- [{}] {} (Properties: {})",
                                edge.relation, node.id, node.properties
                            ));
                            discovered_ids.push(node.id.clone());
                        }
                    }
                }

                tracing::debug!("Discovered IDs during DrillDown: {:?}", discovered_ids);

                if output.is_empty() {
                    Err(anyhow::anyhow!(
                        "Entry {} not found and no relationships discovered",
                        memory_id
                    ))
                } else {
                    Ok((output.join("\n"), discovered_ids))
                }
            }
            DecompositionAction::Filter { criteria, results } => {
                let regex = regex::Regex::new(&criteria)
                    .map_err(|e| anyhow::anyhow!("Invalid filter regex: {}", e))?;

                let mut filtered = Vec::new();
                let mut discovered_ids = Vec::new();

                for res in results {
                    if regex.is_match(&res) {
                        filtered.push(res.clone());
                        if let Some(caps) =
                            regex::Regex::new(r"\[([^\]]+)\]").unwrap().captures(&res)
                        {
                            discovered_ids.push(caps.get(1).unwrap().as_str().to_string());
                        }
                    }
                }

                if filtered.is_empty() {
                    Ok((
                        "No results matched the filter criteria.".to_string(),
                        vec![],
                    ))
                } else {
                    Ok((filtered.join("\n---\n"), discovered_ids))
                }
            }
            DecompositionAction::RecursiveCall { sub_query } => {
                Ok((format!("Recursively searching for {}", sub_query), vec![]))
            }
            DecompositionAction::Aggregate { strategy, results } => {
                let mut discovered_ids = Vec::new();
                for res in &results {
                    if let Some(caps) = regex::Regex::new(r"\[([^\]]+)\]").unwrap().captures(res) {
                        discovered_ids.push(caps.get(1).unwrap().as_str().to_string());
                    }
                }
                Ok((self.aggregate(strategy, results).await?, discovered_ids))
            }
        }
    }
}

impl StrategyExecutor {
    async fn aggregate(
        &self,
        strategy: AggregationStrategy,
        results: Vec<String>,
    ) -> anyhow::Result<String> {
        match strategy {
            AggregationStrategy::Union => {
                let combined = results.join("\n---\n");
                Ok(format!("Union of results:\n{}", combined))
            }
            AggregationStrategy::Intersection => {
                let combined = results.join("\nAND\n");
                Ok(format!("Intersection of results:\n{}", combined))
            }
            AggregationStrategy::Difference => {
                let first = results.first().cloned().unwrap_or_default();
                let others = results
                    .iter()
                    .skip(1)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                Ok(format!(
                    "Results in first but not in [{}]:\n{}",
                    others, first
                ))
            }
            AggregationStrategy::Summary => {
                let combined = results.join(" ");
                Ok(format!("Summary of results: {}", combined))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knowledge::governance::GovernanceEngine;
    use knowledge::manager::KnowledgeManager;
    use knowledge::repository::GitRepository;
    use mk_core::types::TenantContext;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_rlm_filter_action() {
        let repo = Arc::new(GitRepository::new_mock());
        let governance = Arc::new(GovernanceEngine::new());
        let km = Arc::new(KnowledgeManager::new(repo, governance));
        let executor = StrategyExecutor::new(km);
        let tenant = TenantContext::default();

        let action = DecompositionAction::Filter {
            criteria: ".*error.*".to_string(),
            results: vec![
                "this is a success message".to_string(),
                "this is an error message".to_string(),
                "another error occurred".to_string(),
                "some warning".to_string(),
            ],
        };

        let result = executor.execute(action, &tenant).await.unwrap();
        assert!(result.0.contains("this is an error message"));
        assert!(result.0.contains("another error occurred"));
        assert!(!result.0.contains("success"));
        assert!(!result.0.contains("warning"));
    }

    #[tokio::test]
    async fn test_rlm_filter_action_no_matches() {
        let repo = Arc::new(GitRepository::new_mock());
        let governance = Arc::new(GovernanceEngine::new());
        let km = Arc::new(KnowledgeManager::new(repo, governance));
        let executor = StrategyExecutor::new(km);
        let tenant = TenantContext::default();

        let action = DecompositionAction::Filter {
            criteria: "missing".to_string(),
            results: vec!["alpha".to_string(), "beta".to_string()],
        };

        let result = executor.execute(action, &tenant).await.unwrap();
        assert_eq!(result.0, "No results matched the filter criteria.");
    }
}
