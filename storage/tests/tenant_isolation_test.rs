use mk_core::types::{TenantContext, TenantId, UserId};
use storage::graph::{GraphEdge, GraphNode, GraphStore};
use storage::graph_duckdb::{DuckDbGraphConfig, DuckDbGraphStore, GraphError};

fn test_tenant_context(tenant_id: &str) -> TenantContext {
    let tenant = TenantId::new(tenant_id.to_string()).unwrap();
    let user = UserId::new("user-1".to_string()).unwrap();
    TenantContext::new(tenant, user)
}

#[tokio::test]
async fn test_tenant_isolation_cross_tenant_node_access() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx_a = test_tenant_context("tenant-a");
    let ctx_b = test_tenant_context("tenant-b");

    let node = GraphNode {
        id: "node-1".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };

    store.add_node(ctx_a.clone(), node).await.unwrap();

    let neighbors = store.get_neighbors(ctx_b, "node-1").await.unwrap();
    assert!(
        neighbors.is_empty(),
        "Tenant B should not see Tenant A's nodes"
    );
}

#[tokio::test]
async fn test_tenant_isolation_cross_tenant_edge_creation_blocked() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx_a = test_tenant_context("tenant-a");
    let ctx_b = test_tenant_context("tenant-b");

    let node_a = GraphNode {
        id: "node-a".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    store.add_node(ctx_a.clone(), node_a).await.unwrap();

    let node_b = GraphNode {
        id: "node-b".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-b".to_string(),
    };
    store.add_node(ctx_b.clone(), node_b).await.unwrap();

    let edge = GraphEdge {
        id: "edge-cross".to_string(),
        source_id: "node-a".to_string(),
        target_id: "node-b".to_string(),
        relation: "relates_to".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };

    let result = store.add_edge(ctx_a, edge).await;
    assert!(
        result.is_err(),
        "Should not allow edge to cross-tenant node"
    );
}

#[tokio::test]
async fn test_tenant_id_validation_empty() {
    let tenant = TenantId::new("".to_string());
    assert!(
        tenant.is_none(),
        "Empty tenant ID should be rejected at TenantId creation"
    );
}

#[tokio::test]
async fn test_tenant_id_validation_sql_injection_single_quote() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let malicious_tenant_id = "tenant-drop-table";
    if let Some(t) = TenantId::new(malicious_tenant_id.to_string()) {
        let ctx = TenantContext::new(t, UserId::new("user".to_string()).unwrap());
        let node = GraphNode {
            id: "node-1".to_string(),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: "tenant'; DROP TABLE memory_nodes; --".to_string(),
        };

        let result = store.add_node(ctx, node).await;
        assert!(
            result.is_err(),
            "Node with SQL injection in tenant_id should be rejected: {:?}",
            result
        );
        let err = result.unwrap_err();
        if let Some(graph_err) = err.downcast_ref::<GraphError>() {
            assert!(
                matches!(
                    graph_err,
                    GraphError::TenantViolation(_) | GraphError::InvalidTenantIdFormat(_)
                ),
                "Expected TenantViolation or InvalidTenantIdFormat, got {:?}",
                graph_err
            );
        } else {
            panic!("Expected GraphError, got {:?}", err);
        }
    }
}

#[tokio::test]
async fn test_tenant_id_validation_sql_injection_double_dash() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let valid_tenant_id = "tenant-valid";
    if let Some(t) = TenantId::new(valid_tenant_id.to_string()) {
        let ctx = TenantContext::new(t, UserId::new("user".to_string()).unwrap());
        let node = GraphNode {
            id: "node-1".to_string(),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: "tenant--comment".to_string(),
        };

        let result = store.add_node(ctx, node).await;
        assert!(
            result.is_err(),
            "Node with SQL comment in tenant_id should be rejected: {:?}",
            result
        );
        let err = result.unwrap_err();
        if let Some(graph_err) = err.downcast_ref::<GraphError>() {
            assert!(
                matches!(
                    graph_err,
                    GraphError::TenantViolation(_) | GraphError::InvalidTenantIdFormat(_)
                ),
                "Expected TenantViolation or InvalidTenantIdFormat, got {:?}",
                graph_err
            );
        } else {
            panic!("Expected GraphError, got {:?}", err);
        }
    }
}

#[tokio::test]
async fn test_tenant_id_validation_sql_injection_union() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let valid_tenant_id = "tenant-valid";
    if let Some(t) = TenantId::new(valid_tenant_id.to_string()) {
        let ctx = TenantContext::new(t, UserId::new("user".to_string()).unwrap());
        let node = GraphNode {
            id: "node-1".to_string(),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: "tenantUNIONSELECT".to_string(),
        };

        let result = store.add_node(ctx, node).await;
        assert!(
            result.is_err(),
            "Node with UNION SELECT in tenant_id should be rejected: {:?}",
            result
        );
        let err = result.unwrap_err();
        if let Some(graph_err) = err.downcast_ref::<GraphError>() {
            assert!(
                matches!(
                    graph_err,
                    GraphError::TenantViolation(_) | GraphError::InvalidTenantIdFormat(_)
                ),
                "Expected TenantViolation or InvalidTenantIdFormat, got {:?}",
                graph_err
            );
        } else {
            panic!("Expected GraphError, got {:?}", err);
        }
    }
}

#[tokio::test]
async fn test_tenant_id_validation_sql_injection_semicolon() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let valid_tenant_id = "tenant-valid";
    if let Some(t) = TenantId::new(valid_tenant_id.to_string()) {
        let ctx = TenantContext::new(t, UserId::new("user".to_string()).unwrap());
        let node = GraphNode {
            id: "node-1".to_string(),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: "tenant;DELETE".to_string(),
        };

        let result = store.add_node(ctx, node).await;
        assert!(
            result.is_err(),
            "Node with semicolon in tenant_id should be rejected: {:?}",
            result
        );
        let err = result.unwrap_err();
        if let Some(graph_err) = err.downcast_ref::<GraphError>() {
            assert!(
                matches!(
                    graph_err,
                    GraphError::TenantViolation(_) | GraphError::InvalidTenantIdFormat(_)
                ),
                "Expected TenantViolation or InvalidTenantIdFormat, got {:?}",
                graph_err
            );
        } else {
            panic!("Expected GraphError, got {:?}", err);
        }
    }
}

#[tokio::test]
async fn test_tenant_id_validation_too_long() {
    let long_tenant_id = "a".repeat(200);
    let tenant = TenantId::new(long_tenant_id);
    assert!(
        tenant.is_none(),
        "Too-long tenant ID should be rejected at TenantId creation"
    );
}

#[tokio::test]
async fn test_tenant_id_validation_valid_patterns() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let valid_tenant_ids = vec![
        "tenant-1",
        "tenant_one",
        "TenantOne",
        "tenant123",
        "my-tenant-id",
        "UPPERCASE_TENANT",
    ];

    for tenant_id in valid_tenant_ids {
        let ctx = test_tenant_context(tenant_id);
        let node = GraphNode {
            id: format!("node-{}", tenant_id),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: tenant_id.to_string(),
        };

        let result = store.add_node(ctx, node).await;
        assert!(
            result.is_ok(),
            "Valid tenant ID '{}' should be accepted: {:?}",
            tenant_id,
            result
        );
    }
}

#[tokio::test]
async fn test_tenant_isolation_node_mismatch_rejected() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx = test_tenant_context("tenant-a");
    let node = GraphNode {
        id: "node-1".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-b".to_string(),
    };

    let result = store.add_node(ctx, node).await;
    assert!(
        result.is_err(),
        "Node with mismatched tenant_id should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err();
    if let Some(graph_err) = err.downcast_ref::<GraphError>() {
        assert!(
            matches!(graph_err, GraphError::TenantViolation(_)),
            "Expected TenantViolation, got {:?}",
            graph_err
        );
    } else {
        panic!("Expected GraphError, got {:?}", err);
    }
}

#[tokio::test]
async fn test_tenant_isolation_edge_mismatch_rejected() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx = test_tenant_context("tenant-a");

    let node1 = GraphNode {
        id: "node-1".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    let node2 = GraphNode {
        id: "node-2".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    store.add_node(ctx.clone(), node1).await.unwrap();
    store.add_node(ctx.clone(), node2).await.unwrap();

    let edge = GraphEdge {
        id: "edge-1".to_string(),
        source_id: "node-1".to_string(),
        target_id: "node-2".to_string(),
        relation: "relates_to".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-b".to_string(),
    };

    let result = store.add_edge(ctx, edge).await;
    assert!(
        result.is_err(),
        "Edge with mismatched tenant_id should be rejected: {:?}",
        result
    );
    let err = result.unwrap_err();
    if let Some(graph_err) = err.downcast_ref::<GraphError>() {
        assert!(
            matches!(graph_err, GraphError::TenantViolation(_)),
            "Expected TenantViolation, got {:?}",
            graph_err
        );
    } else {
        panic!("Expected GraphError, got {:?}", err);
    }
}

#[tokio::test]
async fn test_tenant_isolation_find_related_cross_tenant() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx_a = test_tenant_context("tenant-a");
    let ctx_b = test_tenant_context("tenant-b");

    let node_a1 = GraphNode {
        id: "node-a1".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    let node_a2 = GraphNode {
        id: "node-a2".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    store.add_node(ctx_a.clone(), node_a1).await.unwrap();
    store.add_node(ctx_a.clone(), node_a2).await.unwrap();

    let edge_a = GraphEdge {
        id: "edge-a".to_string(),
        source_id: "node-a1".to_string(),
        target_id: "node-a2".to_string(),
        relation: "relates_to".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-a".to_string(),
    };
    store.add_edge(ctx_a.clone(), edge_a).await.unwrap();

    let related = store.find_related(ctx_b, "node-a1", 2).unwrap();
    assert!(
        related.is_empty(),
        "Tenant B should not see Tenant A's related nodes"
    );

    let related_a = store.find_related(ctx_a, "node-a1", 2).unwrap();
    assert!(
        !related_a.is_empty(),
        "Tenant A should see its own related nodes"
    );
}

#[tokio::test]
async fn test_tenant_isolation_stats_per_tenant() {
    let store = DuckDbGraphStore::new(DuckDbGraphConfig::default()).unwrap();

    let ctx_a = test_tenant_context("tenant-a");
    let ctx_b = test_tenant_context("tenant-b");

    for i in 0..3 {
        let node = GraphNode {
            id: format!("node-a-{}", i),
            label: "test".to_string(),
            properties: serde_json::json!({}),
            tenant_id: "tenant-a".to_string(),
        };
        store.add_node(ctx_a.clone(), node).await.unwrap();
    }

    let node_b = GraphNode {
        id: "node-b-0".to_string(),
        label: "test".to_string(),
        properties: serde_json::json!({}),
        tenant_id: "tenant-b".to_string(),
    };
    store.add_node(ctx_b.clone(), node_b).await.unwrap();

    let stats_a = store.get_stats(ctx_a).unwrap();
    let stats_b = store.get_stats(ctx_b).unwrap();

    assert_eq!(stats_a.node_count, 3, "Tenant A should see 3 nodes");
    assert_eq!(stats_b.node_count, 1, "Tenant B should see 1 node");
}
