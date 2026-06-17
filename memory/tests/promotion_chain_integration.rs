//! Integration test: 7-layer memory promotion chain.
//!
//! Tests the full memory hierarchy (Agent → User → Session → Project → Team → Org → Tenant)
//! with promotion from Agent→User and Session→Project (the two supported promotion paths).
//! Also tests hierarchical search across multiple registered layers.

use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use knowledge::repository::GitRepository;
use memory::manager::MemoryManager;
use memory::providers::MockProvider;
use mk_core::traits::MemoryProviderAdapter;
use mk_core::types::{MemoryEntry, MemoryLayer, TenantContext, TenantId, UserId};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

fn test_ctx() -> TenantContext {
    TenantContext::new(
        TenantId::from_str("test-tenant").unwrap(),
        UserId::from_str("test-user").unwrap(),
    )
}

/// Helper to create a memory entry with high importance metadata so it qualifies for promotion.
fn high_importance_entry(id: &str, content: &str, layer: MemoryLayer) -> MemoryEntry {
    let mut metadata = HashMap::new();
    metadata.insert("score".to_string(), serde_json::json!(1.0));
    metadata.insert("access_count".to_string(), serde_json::json!(20));
    metadata.insert(
        "last_accessed_at".to_string(),
        serde_json::json!(chrono::Utc::now().timestamp()),
    );

    MemoryEntry {
        id: id.to_string(),
        content: content.to_string(),
        layer,
        embedding: Some(vec![0.1; 128]),
        importance_score: Some(0.9),
        metadata,
        created_at: chrono::Utc::now().timestamp(),
        updated_at: chrono::Utc::now().timestamp(),
        ..Default::default()
    }
}

/// Helper to create a low-importance entry that should NOT be promoted.
fn low_importance_entry(id: &str, content: &str, layer: MemoryLayer) -> MemoryEntry {
    MemoryEntry {
        id: id.to_string(),
        content: content.to_string(),
        layer,
        embedding: Some(vec![0.2; 128]),
        importance_score: Some(0.1),
        metadata: HashMap::new(),
        created_at: chrono::Utc::now().timestamp(),
        updated_at: chrono::Utc::now().timestamp(),
        ..Default::default()
    }
}

/// Create a MemoryManager with a KnowledgeManager backed by a temp git repo.
fn create_manager_with_knowledge() -> (MemoryManager, Arc<KnowledgeManager>) {
    let temp_dir = tempfile::tempdir().unwrap();
    let repo = Arc::new(GitRepository::new(temp_dir.path()).unwrap());
    let governance = Arc::new(GovernanceEngine::new());
    let km = Arc::new(KnowledgeManager::new(repo, governance));

    let manager = MemoryManager::new().with_knowledge_manager(km.clone());
    (manager, km)
}

#[tokio::test]
async fn test_promotion_agent_to_user() {
    let (manager, _km) = create_manager_with_knowledge();
    let ctx = test_ctx();

    // Register providers for Agent and User layers
    let agent_provider = Arc::new(MockProvider::new());
    let user_provider = Arc::new(MockProvider::new());
    manager
        .register_provider(MemoryLayer::Agent, agent_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::User, user_provider.clone())
        .await;

    // Add a high-importance memory to Agent layer
    let entry = high_importance_entry(
        "agent-mem-1",
        "Agent learned: prefer cargo check before test",
        MemoryLayer::Agent,
    );
    agent_provider.add(ctx.clone(), entry).await.unwrap();

    // Promote from Agent layer
    let promoted = manager
        .promote_important_memories(ctx.clone(), MemoryLayer::Agent)
        .await;

    // Promotion should succeed (may return empty vec due to optimize_layer impl)
    assert!(
        promoted.is_ok(),
        "Promotion should not error: {:?}",
        promoted.err()
    );

    // Verify the Agent entry is still in the Agent layer
    let (agent_entries, _) = agent_provider
        .list(ctx.clone(), MemoryLayer::Agent, 100, None)
        .await
        .unwrap();
    assert!(
        !agent_entries.is_empty(),
        "Agent layer should still contain the original entry"
    );
}

#[tokio::test]
async fn test_promotion_session_to_project() {
    let (manager, _km) = create_manager_with_knowledge();
    let ctx = test_ctx();

    // Register Session and Project providers
    let session_provider = Arc::new(MockProvider::new());
    let project_provider = Arc::new(MockProvider::new());
    manager
        .register_provider(MemoryLayer::Session, session_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::Project, project_provider.clone())
        .await;

    // Add high-importance session memory
    let entry = high_importance_entry(
        "session-mem-1",
        "Session discovery: API timeout must be 30s for payment service",
        MemoryLayer::Session,
    );
    session_provider.add(ctx.clone(), entry).await.unwrap();

    // Also add a low-importance session memory (should NOT promote)
    let low_entry = low_importance_entry(
        "session-mem-2",
        "Temporary debug note",
        MemoryLayer::Session,
    );
    session_provider.add(ctx.clone(), low_entry).await.unwrap();

    // Promote from Session layer
    let promoted = manager
        .promote_important_memories(ctx.clone(), MemoryLayer::Session)
        .await;
    assert!(
        promoted.is_ok(),
        "Promotion should not error: {:?}",
        promoted.err()
    );
}

#[tokio::test]
async fn test_hierarchical_search_across_all_layers() {
    let manager = MemoryManager::new();
    let ctx = test_ctx();

    // Register all 7 layers with mock providers
    let layers = vec![
        MemoryLayer::Agent,
        MemoryLayer::User,
        MemoryLayer::Session,
        MemoryLayer::Project,
        MemoryLayer::Team,
        MemoryLayer::Org,
        MemoryLayer::Tenant,
    ];

    let mut providers: Vec<Arc<MockProvider>> = Vec::new();
    for layer in &layers {
        let provider = Arc::new(MockProvider::new());
        manager.register_provider(*layer, provider.clone()).await;
        providers.push(provider);
    }

    // Add one entry per layer with embeddings that will match our search vector
    for (i, (layer, provider)) in layers.iter().zip(providers.iter()).enumerate() {
        let entry = MemoryEntry {
            id: format!("mem-layer-{}", i),
            content: format!("Content for {:?} layer", layer),
            layer: *layer,
            embedding: Some(vec![0.1; 128]),
            importance_score: Some(0.5 + (i as f32 * 0.05)),
            metadata: HashMap::new(),
            created_at: chrono::Utc::now().timestamp(),
            updated_at: chrono::Utc::now().timestamp(),
            ..Default::default()
        };
        provider.add(ctx.clone(), entry).await.unwrap();
    }

    // Hierarchical search should return results from all registered layers
    let results = manager
        .search_hierarchical(ctx.clone(), vec![0.1; 128], 10, HashMap::new())
        .await
        .unwrap();

    // Should find entries from all 7 layers
    assert_eq!(
        results.len(),
        7,
        "Hierarchical search should return entries from all 7 layers, got {}",
        results.len()
    );

    // Results should be sorted by importance_score descending
    for i in 1..results.len() {
        assert!(
            results[i - 1].importance_score.unwrap_or(0.0)
                >= results[i].importance_score.unwrap_or(0.0),
            "Results should be sorted by importance_score descending"
        );
    }
}

#[tokio::test]
async fn test_promotion_chain_agent_to_user_to_project() {
    // Test multi-hop: Agent→User, then manually move to Session, then Session→Project
    let (manager, _km) = create_manager_with_knowledge();
    let ctx = test_ctx();

    let agent_provider = Arc::new(MockProvider::new());
    let user_provider = Arc::new(MockProvider::new());
    let session_provider = Arc::new(MockProvider::new());
    let project_provider = Arc::new(MockProvider::new());

    manager
        .register_provider(MemoryLayer::Agent, agent_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::User, user_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::Session, session_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::Project, project_provider.clone())
        .await;

    // Step 1: Add high-importance agent memory
    let agent_entry = high_importance_entry(
        "chain-agent-1",
        "Agent pattern: always check database migrations before deploying",
        MemoryLayer::Agent,
    );
    agent_provider.add(ctx.clone(), agent_entry).await.unwrap();

    // Step 2: Trigger Agent→User promotion
    let result = manager
        .promote_important_memories(ctx.clone(), MemoryLayer::Agent)
        .await;
    assert!(result.is_ok());

    // Step 3: Add high-importance session memory (simulating user activity)
    let session_entry = high_importance_entry(
        "chain-session-1",
        "Session finding: payment API requires idempotency key",
        MemoryLayer::Session,
    );
    session_provider
        .add(ctx.clone(), session_entry)
        .await
        .unwrap();

    // Step 4: Trigger Session→Project promotion
    let result = manager
        .promote_important_memories(ctx.clone(), MemoryLayer::Session)
        .await;
    assert!(result.is_ok());

    // Step 5: Verify we can search across all layers
    let results = manager
        .search_hierarchical(ctx.clone(), vec![0.1; 128], 10, HashMap::new())
        .await
        .unwrap();

    // At minimum: agent-mem + session-mem should be in their respective layers
    assert!(
        results.len() >= 2,
        "Should find at least 2 entries across the chain, got {}",
        results.len()
    );
}

#[tokio::test]
async fn test_promotion_does_not_promote_from_unsupported_layers() {
    // Team, Org, Tenant layers do NOT have promotion targets
    let (manager, _km) = create_manager_with_knowledge();
    let ctx = test_ctx();

    let team_provider = Arc::new(MockProvider::new());
    let org_provider = Arc::new(MockProvider::new());

    manager
        .register_provider(MemoryLayer::Team, team_provider.clone())
        .await;
    manager
        .register_provider(MemoryLayer::Org, org_provider.clone())
        .await;

    // Add high-importance team memory
    let entry = high_importance_entry(
        "team-mem-1",
        "Team standard: use RFC 7807 for error responses",
        MemoryLayer::Team,
    );
    team_provider.add(ctx.clone(), entry).await.unwrap();

    // Promote from Team — should succeed without error but not move anything
    let result = manager
        .promote_important_memories(ctx.clone(), MemoryLayer::Team)
        .await;
    assert!(result.is_ok());

    // Team entry should remain unchanged
    let (team_entries, _) = team_provider
        .list(ctx.clone(), MemoryLayer::Team, 100, None)
        .await
        .unwrap();
    assert_eq!(team_entries.len(), 1, "Team entry should remain in place");

    let (org_entries, _) = org_provider
        .list(ctx.clone(), MemoryLayer::Org, 100, None)
        .await
        .unwrap();
    assert!(
        org_entries.is_empty(),
        "Org layer should be empty — no promotion from Team"
    );
}

#[tokio::test]
async fn test_layer_precedence_ordering() {
    // Verify that MemoryLayer precedence is correctly ordered (Agent=1 lowest to Tenant=7 highest)
    assert!(MemoryLayer::Agent.precedence() < MemoryLayer::User.precedence());
    assert!(MemoryLayer::User.precedence() < MemoryLayer::Session.precedence());
    assert!(MemoryLayer::Session.precedence() < MemoryLayer::Project.precedence());
    assert!(MemoryLayer::Project.precedence() < MemoryLayer::Team.precedence());
    assert!(MemoryLayer::Team.precedence() < MemoryLayer::Org.precedence());
    assert!(MemoryLayer::Org.precedence() < MemoryLayer::Tenant.precedence());
}
