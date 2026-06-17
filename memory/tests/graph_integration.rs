#[cfg(feature = "llm-integration")]
use memory::manager::MemoryManager;
use mk_core::types::{MemoryEntry, MemoryLayer, TenantContext, TenantId, UserId};
use std::collections::HashMap;
use std::sync::Arc;
use storage::graph::GraphStore;
use storage::graph_duckdb::{DuckDbGraphConfig, DuckDbGraphStore};
#[cfg(feature = "postgres-graph")]
use storage::postgres::PostgresBackend;
#[cfg(not(feature = "postgres-graph"))]
use testing::unique_id;
#[cfg(feature = "postgres-graph")]
use testing::{postgres, unique_id};
#[cfg(feature = "postgres-graph")]
use tokio::sync::OnceCell;

#[cfg(feature = "postgres-graph")]
static SCHEMA_INITIALIZED: OnceCell<bool> = OnceCell::const_new();

#[cfg(feature = "postgres-graph")]
async fn create_test_backend() -> Option<Arc<PostgresBackend>> {
    let fixture = postgres().await?;
    let backend = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);

    SCHEMA_INITIALIZED
        .get_or_init(|| async {
            backend.initialize_schema().await.ok();
            true
        })
        .await;

    Some(backend)
}

fn test_tenant_context() -> TenantContext {
    let tenant_id = TenantId::new(unique_id("test-tenant")).unwrap();
    let user_id = UserId::new(unique_id("test-user")).unwrap();
    TenantContext::new(tenant_id, user_id)
}

#[cfg(feature = "postgres-graph")]
#[tokio::test]
async fn test_graph_based_reasoning() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(graph_store) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let mut llm_service = memory::llm::mock::MockLlmService::new();
    llm_service
        .set_response(
            r#"{
        "entities": [
            { "name": "Rust", "label": "Language", "properties": {} },
            { "name": "Borrow Checker", "label": "Feature", "properties": {} }
        ],
        "relations": [
            { "source": "Rust", "target": "Borrow Checker", "relation": "has", "properties": {} }
        ]
    }"#,
        )
        .await;
    let llm = Arc::new(llm_service);

    let manager = MemoryManager::new()
        .with_graph_store(graph_store)
        .with_llm_service(llm);

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem");
    let entry = MemoryEntry {
        id: mem_id.clone(),
        content: "Rust has a borrow checker.".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;
    manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await?;

    let nodes = manager.search_graph(ctx.clone(), "Rust", 1).await?;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].id, "Rust");

    let neighbors = manager.get_graph_neighbors(ctx.clone(), "Rust").await?;
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].1.id, "Borrow Checker");
    assert_eq!(neighbors[0].0.relation_type, "has");

    Ok(())
}

#[tokio::test]
async fn test_duckdb_graph_entity_extraction_and_traversal()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let graph_store = Arc::new(
        DuckDbGraphStore::new(DuckDbGraphConfig::default())
            .expect("Failed to create DuckDB graph store"),
    );

    let ctx = test_tenant_context();
    let tenant_id = ctx.tenant_id.as_str().to_string();

    let node_rust = storage::graph::GraphNode {
        id: "Rust".to_string(),
        label: "Language".to_string(),
        properties: serde_json::json!({"source_memory_id": "mem_1"}),
        tenant_id: tenant_id.clone(),
    };
    let node_borrow = storage::graph::GraphNode {
        id: "BorrowChecker".to_string(),
        label: "Feature".to_string(),
        properties: serde_json::json!({"source_memory_id": "mem_1"}),
        tenant_id: tenant_id.clone(),
    };
    let node_safety = storage::graph::GraphNode {
        id: "MemorySafety".to_string(),
        label: "Concept".to_string(),
        properties: serde_json::json!({"source_memory_id": "mem_2"}),
        tenant_id: tenant_id.clone(),
    };

    graph_store.add_node(ctx.clone(), node_rust).await?;
    graph_store.add_node(ctx.clone(), node_borrow).await?;
    graph_store.add_node(ctx.clone(), node_safety).await?;

    let edge_has = storage::graph::GraphEdge {
        id: "rust_has_borrow".to_string(),
        source_id: "Rust".to_string(),
        target_id: "BorrowChecker".to_string(),
        relation: "HAS_FEATURE".to_string(),
        properties: serde_json::Value::Null,
        tenant_id: tenant_id.clone(),
    };
    let edge_provides = storage::graph::GraphEdge {
        id: "borrow_provides_safety".to_string(),
        source_id: "BorrowChecker".to_string(),
        target_id: "MemorySafety".to_string(),
        relation: "PROVIDES".to_string(),
        properties: serde_json::Value::Null,
        tenant_id: tenant_id.clone(),
    };

    graph_store.add_edge(ctx.clone(), edge_has).await?;
    graph_store.add_edge(ctx.clone(), edge_provides).await?;

    let results = graph_store
        .search_nodes(ctx.clone(), "Language", 10)
        .await?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "Rust");

    let neighbors = graph_store.get_neighbors(ctx.clone(), "Rust").await?;
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].1.id, "BorrowChecker");
    assert_eq!(neighbors[0].0.relation, "HAS_FEATURE");

    let path = graph_store
        .find_path(ctx.clone(), "Rust", "MemorySafety", 3)
        .await?;
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].source_id, "Rust");
    assert_eq!(path[0].target_id, "BorrowChecker");
    assert_eq!(path[1].source_id, "BorrowChecker");
    assert_eq!(path[1].target_id, "MemorySafety");

    Ok(())
}

#[tokio::test]
async fn test_duckdb_graph_multi_hop_reasoning()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let graph_store = Arc::new(
        DuckDbGraphStore::new(DuckDbGraphConfig::default())
            .expect("Failed to create DuckDB graph store"),
    );

    let ctx = test_tenant_context();
    let tenant_id = ctx.tenant_id.as_str().to_string();

    let nodes = vec![
        ("PaymentsService", "Service"),
        ("StripeAPI", "ExternalService"),
        ("PaymentsDB", "Database"),
        ("OrdersService", "Service"),
        ("OrdersDB", "Database"),
    ];

    for (name, label) in &nodes {
        let node = storage::graph::GraphNode {
            id: name.to_string(),
            label: label.to_string(),
            properties: serde_json::json!({"source_memory_id": "architecture_doc"}),
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_node(ctx.clone(), node).await?;
    }

    let edges = [
        ("PaymentsService", "StripeAPI", "CALLS"),
        ("PaymentsService", "PaymentsDB", "READS_FROM"),
        ("OrdersService", "PaymentsService", "DEPENDS_ON"),
        ("OrdersService", "OrdersDB", "READS_FROM"),
    ];

    for (i, (src, tgt, rel)) in edges.iter().enumerate() {
        let edge = storage::graph::GraphEdge {
            id: format!("edge_{}", i),
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            relation: rel.to_string(),
            properties: serde_json::Value::Null,
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_edge(ctx.clone(), edge).await?;
    }

    let related = graph_store.find_related(ctx.clone(), "OrdersService", 2)?;
    let related_ids: Vec<&str> = related.iter().map(|(_, n)| n.id.as_str()).collect();

    assert!(related_ids.contains(&"PaymentsService"));
    assert!(related_ids.contains(&"OrdersDB"));

    let path = graph_store
        .find_path(ctx.clone(), "OrdersService", "StripeAPI", 5)
        .await?;
    assert_eq!(path.len(), 2);
    assert_eq!(path[0].relation.clone(), "DEPENDS_ON");
    assert_eq!(path[1].relation.clone(), "CALLS");

    Ok(())
}

#[tokio::test]
async fn test_duckdb_graph_memory_deletion_cleanup()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let graph_store = Arc::new(
        DuckDbGraphStore::new(DuckDbGraphConfig::default())
            .expect("Failed to create DuckDB graph store"),
    );

    let ctx = test_tenant_context();
    let tenant_id = ctx.tenant_id.as_str().to_string();

    let mem_to_delete = unique_id("mem_to_delete");
    let other_mem = unique_id("other_mem");

    let node1 = storage::graph::GraphNode {
        id: "Entity1".to_string(),
        label: "TestEntity".to_string(),
        properties: serde_json::json!({"source_memory_id": mem_to_delete.clone()}),
        tenant_id: tenant_id.clone(),
    };
    let node2 = storage::graph::GraphNode {
        id: "Entity2".to_string(),
        label: "TestEntity".to_string(),
        properties: serde_json::json!({"source_memory_id": mem_to_delete.clone()}),
        tenant_id: tenant_id.clone(),
    };
    let node3 = storage::graph::GraphNode {
        id: "UnrelatedEntity".to_string(),
        label: "TestEntity".to_string(),
        properties: serde_json::json!({"source_memory_id": other_mem}),
        tenant_id: tenant_id.clone(),
    };

    graph_store.add_node(ctx.clone(), node1).await?;
    graph_store.add_node(ctx.clone(), node2).await?;
    graph_store.add_node(ctx.clone(), node3).await?;

    let edge = storage::graph::GraphEdge {
        id: "edge_1_2".to_string(),
        source_id: "Entity1".to_string(),
        target_id: "Entity2".to_string(),
        relation: "RELATED".to_string(),
        properties: serde_json::Value::Null,
        tenant_id: tenant_id.clone(),
    };
    graph_store.add_edge(ctx.clone(), edge).await?;

    let all_nodes = graph_store
        .search_nodes(ctx.clone(), "TestEntity", 10)
        .await?;
    assert_eq!(all_nodes.len(), 3);

    let deleted = GraphStore::soft_delete_nodes_by_source_memory_id(
        graph_store.as_ref(),
        ctx.clone(),
        &mem_to_delete,
    )
    .await?;
    assert_eq!(deleted, 2);

    let remaining_nodes = graph_store
        .search_nodes(ctx.clone(), "TestEntity", 10)
        .await?;
    assert_eq!(remaining_nodes.len(), 1);
    assert_eq!(remaining_nodes[0].id, "UnrelatedEntity");

    Ok(())
}

#[tokio::test]
async fn test_duckdb_graph_community_detection()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let graph_store = Arc::new(
        DuckDbGraphStore::new(DuckDbGraphConfig::default())
            .expect("Failed to create DuckDB graph store"),
    );

    let ctx = test_tenant_context();
    let tenant_id = ctx.tenant_id.as_str().to_string();

    for i in 1..=4 {
        let node = storage::graph::GraphNode {
            id: format!("cluster_a_{}", i),
            label: "ClusterA".to_string(),
            properties: serde_json::Value::Null,
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_node(ctx.clone(), node).await?;
    }

    for i in 1..=3 {
        let node = storage::graph::GraphNode {
            id: format!("cluster_b_{}", i),
            label: "ClusterB".to_string(),
            properties: serde_json::Value::Null,
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_node(ctx.clone(), node).await?;
    }

    let cluster_a_edges = [
        ("cluster_a_1", "cluster_a_2"),
        ("cluster_a_2", "cluster_a_3"),
        ("cluster_a_3", "cluster_a_4"),
        ("cluster_a_4", "cluster_a_1"),
    ];
    for (i, (src, tgt)) in cluster_a_edges.iter().enumerate() {
        let edge = storage::graph::GraphEdge {
            id: format!("a_edge_{}", i),
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            relation: "CONNECTED".to_string(),
            properties: serde_json::Value::Null,
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_edge(ctx.clone(), edge).await?;
    }

    let cluster_b_edges = [
        ("cluster_b_1", "cluster_b_2"),
        ("cluster_b_2", "cluster_b_3"),
    ];
    for (i, (src, tgt)) in cluster_b_edges.iter().enumerate() {
        let edge = storage::graph::GraphEdge {
            id: format!("b_edge_{}", i),
            source_id: src.to_string(),
            target_id: tgt.to_string(),
            relation: "CONNECTED".to_string(),
            properties: serde_json::Value::Null,
            tenant_id: tenant_id.clone(),
        };
        graph_store.add_edge(ctx.clone(), edge).await?;
    }

    let communities = graph_store.detect_communities(ctx.clone(), 2)?;
    assert_eq!(communities.len(), 2);

    let sizes: Vec<usize> = communities
        .iter()
        .map(|c| c.member_node_ids.len())
        .collect();
    assert!(sizes.contains(&4));
    assert!(sizes.contains(&3));

    Ok(())
}

#[cfg(feature = "postgres-graph")]
#[tokio::test]
async fn test_memory_add_with_graph_extraction_malformed_json()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(graph_store) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let mut llm_service = memory::llm::mock::MockLlmService::new();
    llm_service
        .set_response("This is not valid JSON at all")
        .await;
    let llm = Arc::new(llm_service);

    let manager = MemoryManager::new()
        .with_graph_store(graph_store.clone())
        .with_llm_service(llm);

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem_malformed");
    let entry = MemoryEntry {
        id: mem_id,
        content: "Content that will produce malformed extraction".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;

    let result = manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await;
    assert!(
        result.is_ok(),
        "Memory add should succeed even if entity extraction fails"
    );

    let nodes = graph_store.search_nodes(ctx.clone(), "any", 10).await?;
    assert!(
        nodes.is_empty(),
        "No nodes should be added when extraction fails"
    );

    Ok(())
}

#[cfg(feature = "postgres-graph")]
#[tokio::test]
async fn test_memory_add_with_graph_extraction_no_json_in_response()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(graph_store) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let mut llm_service = memory::llm::mock::MockLlmService::new();
    llm_service
        .set_response("No JSON here, just plain text without any curly braces")
        .await;
    let llm = Arc::new(llm_service);

    let manager = MemoryManager::new()
        .with_graph_store(graph_store.clone())
        .with_llm_service(llm);

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem_no_json");
    let entry = MemoryEntry {
        id: mem_id,
        content: "Content without JSON extraction".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;

    let result = manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await;
    assert!(
        result.is_ok(),
        "Memory add should succeed even when LLM returns no JSON"
    );

    Ok(())
}

#[cfg(feature = "postgres-graph")]
#[tokio::test]
async fn test_memory_add_with_valid_extraction_creates_graph_entities()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(graph_store) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let mut llm_service = memory::llm::mock::MockLlmService::new();
    llm_service
        .set_response(r#"{
            "entities": [
                { "name": "Python", "label": "Language", "properties": {"version": "3.11"} },
                { "name": "FastAPI", "label": "Framework", "properties": {} }
            ],
            "relations": [
                { "source": "FastAPI", "target": "Python", "relation": "BUILT_WITH", "properties": {} }
            ]
        }"#)
        .await;
    let llm = Arc::new(llm_service);

    let manager = MemoryManager::new()
        .with_graph_store(graph_store.clone())
        .with_llm_service(llm);

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem_valid_extraction");
    let entry = MemoryEntry {
        id: mem_id,
        content: "FastAPI is built with Python".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;

    let result = manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await;
    assert!(
        result.is_ok(),
        "Memory add should succeed with valid extraction"
    );

    let python_nodes = graph_store
        .search_nodes(ctx.clone(), "Language", 10)
        .await?;
    assert_eq!(python_nodes.len(), 1);
    assert_eq!(python_nodes[0].id, "Python");

    let fastapi_nodes = graph_store
        .search_nodes(ctx.clone(), "Framework", 10)
        .await?;
    assert_eq!(fastapi_nodes.len(), 1);
    assert_eq!(fastapi_nodes[0].id, "FastAPI");

    let neighbors = manager.get_graph_neighbors(ctx.clone(), "FastAPI").await?;
    assert_eq!(neighbors.len(), 1);
    assert_eq!(neighbors[0].0.relation_type, "BUILT_WITH");
    assert_eq!(neighbors[0].1.id, "Python");

    Ok(())
}

#[tokio::test]
async fn test_memory_add_without_graph_store_skips_extraction()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut llm_service = memory::llm::mock::MockLlmService::new();
    llm_service
        .set_response(r#"{"entities": [], "relations": []}"#)
        .await;
    let llm = Arc::new(llm_service);

    let manager = MemoryManager::new().with_llm_service(llm);

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem_no_graph");
    let entry = MemoryEntry {
        id: mem_id,
        content: "No graph store configured".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;

    let result = manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await;
    assert!(
        result.is_ok(),
        "Memory add should succeed without graph store"
    );

    Ok(())
}

#[cfg(feature = "postgres-graph")]
#[tokio::test]
async fn test_memory_add_without_llm_service_skips_extraction()
-> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let Some(graph_store) = create_test_backend().await else {
        eprintln!("Skipping test: Docker not available");
        return Ok(());
    };

    let manager = MemoryManager::new().with_graph_store(graph_store.clone());

    let ctx = test_tenant_context();

    let mem_id = unique_id("mem_no_llm");
    let entry = MemoryEntry {
        id: mem_id,
        content: "No LLM service configured".to_string(),
        embedding: None,
        layer: MemoryLayer::User,
        summaries: HashMap::new(),
        context_vector: None,
        importance_score: None,
        metadata: HashMap::new(),
        created_at: 0,
        updated_at: 0,
    };

    manager
        .register_provider(
            MemoryLayer::User,
            Arc::new(memory::providers::MockProvider::new()),
        )
        .await;

    let result = manager
        .add_to_layer(ctx.clone(), MemoryLayer::User, entry)
        .await;
    assert!(
        result.is_ok(),
        "Memory add should succeed without LLM service"
    );

    let nodes = graph_store.search_nodes(ctx.clone(), "any", 10).await?;
    assert!(
        nodes.is_empty(),
        "No nodes should be added without LLM service"
    );

    Ok(())
}
