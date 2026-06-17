use mk_core::traits::StorageBackend;
use mk_core::types::{
    INSTANCE_SCOPE_TENANT_ID, OrganizationalUnit, RecordSource, Role, RoleIdentifier,
    TenantContext, TenantId, UnitType, UserId,
};
use storage::postgres::{PostgresBackend, PostgresError};
use testing::{postgres, unique_id};

async fn create_test_backend() -> Option<PostgresBackend> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;
    backend.initialize_schema().await.ok()?;
    Some(backend)
}

fn unique_tenant_id() -> String {
    unique_id("test-tenant")
}

#[tokio::test]
async fn test_postgres_backend_new() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };
    assert!(backend.pool().size() > 0);
}

#[tokio::test]
async fn test_postgres_backend_initialize_schema() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };
    let result = backend.initialize_schema().await;
    assert!(result.is_ok(), "Should initialize schema");
}

#[tokio::test]
async fn test_postgres_backend_store_and_retrieve() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let key = "test_key";
    let value = b"{\"test\": \"data\"}";
    let store_result = backend.store(ctx.clone(), key, value).await;
    assert!(store_result.is_ok(), "Should store data");

    let retrieve_result = backend.retrieve(ctx, key).await;
    assert!(retrieve_result.is_ok(), "Should retrieve data");
    let retrieved = retrieve_result.unwrap();
    assert!(retrieved.is_some(), "Should have retrieved data");
    let retrieved_json: serde_json::Value = serde_json::from_slice(&retrieved.unwrap()).unwrap();
    let expected_json: serde_json::Value = serde_json::from_slice(value).unwrap();
    assert_eq!(retrieved_json, expected_json);
}

#[tokio::test]
async fn test_postgres_backend_store_update() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let key = "update_key";
    let value1 = b"{\"version\": 1}";
    backend.store(ctx.clone(), key, value1).await.unwrap();

    let value2 = b"{\"version\": 2}";
    backend.store(ctx.clone(), key, value2).await.unwrap();

    let retrieved = backend.retrieve(ctx, key).await.unwrap().unwrap();
    let retrieved_json: serde_json::Value = serde_json::from_slice(&retrieved).unwrap();
    let expected_json: serde_json::Value = serde_json::from_slice(value2).unwrap();
    assert_eq!(retrieved_json, expected_json);
}

#[tokio::test]
async fn test_postgres_backend_delete() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let key = "delete_key";
    let value = b"{\"to_delete\": true}";
    backend.store(ctx.clone(), key, value).await.unwrap();

    let exists_before = backend.exists(ctx.clone(), key).await.unwrap();
    assert!(exists_before, "Key should exist before delete");

    let delete_result = backend.delete(ctx.clone(), key).await;
    assert!(delete_result.is_ok(), "Should delete data");

    let exists_after = backend.exists(ctx.clone(), key).await.unwrap();
    assert!(!exists_after, "Key should not exist after delete");

    let retrieved = backend.retrieve(ctx, key).await.unwrap();
    assert!(retrieved.is_none(), "Should return None for deleted key");
}

#[tokio::test]
async fn test_postgres_backend_exists() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let exists = backend.exists(ctx.clone(), "nonexistent").await.unwrap();
    assert!(!exists, "Nonexistent key should not exist");

    let key = "exists_key";
    let value = b"{\"exists\": true}";
    backend.store(ctx.clone(), key, value).await.unwrap();

    let exists = backend.exists(ctx, key).await.unwrap();
    assert!(exists, "Stored key should exist");
}

#[tokio::test]
async fn test_postgres_backend_retrieve_nonexistent() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let result = backend.retrieve(ctx, "nonexistent_key").await;
    assert!(result.is_ok(), "Should handle nonexistent key");
    assert!(
        result.unwrap().is_none(),
        "Should return None for nonexistent key"
    );
}

#[tokio::test]
async fn test_postgres_backend_invalid_json() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx = TenantContext::new(
        TenantId::new(unique_tenant_id()).unwrap(),
        UserId::default(),
    );
    let key = "invalid_json_key";
    let invalid_json = b"not valid json";
    let result = backend.store(ctx.clone(), key, invalid_json).await;
    assert!(result.is_ok(), "Should handle invalid JSON gracefully");

    let retrieved = backend.retrieve(ctx, key).await.unwrap();
    assert!(retrieved.is_some(), "Should retrieve something");
}

#[tokio::test]
async fn test_postgres_backend_tenant_isolation() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let ctx1 = TenantContext::new(
        TenantId::new("tenant-1".to_string()).unwrap(),
        UserId::default(),
    );
    let ctx2 = TenantContext::new(
        TenantId::new("tenant-2".to_string()).unwrap(),
        UserId::default(),
    );
    let key = "shared-key";
    let val1 = b"{\"tenant\": 1}";
    let val2 = b"{\"tenant\": 2}";

    backend.store(ctx1.clone(), key, val1).await.unwrap();

    let res2 = backend.retrieve(ctx2.clone(), key).await.unwrap();
    assert!(res2.is_none(), "Tenant 2 should not see Tenant 1 data");

    backend.store(ctx2.clone(), key, val2).await.unwrap();

    let res1 = backend.retrieve(ctx1.clone(), key).await.unwrap();
    let res1_json: serde_json::Value = serde_json::from_slice(&res1.unwrap()).unwrap();
    let val1_json: serde_json::Value = serde_json::from_slice(val1).unwrap();
    assert_eq!(res1_json, val1_json);

    let res2 = backend.retrieve(ctx2.clone(), key).await.unwrap();
    let res2_json: serde_json::Value = serde_json::from_slice(&res2.unwrap()).unwrap();
    let val2_json: serde_json::Value = serde_json::from_slice(val2).unwrap();
    assert_eq!(res2_json, val2_json);

    backend.delete(ctx1.clone(), key).await.unwrap();
    assert!(!backend.exists(ctx1, key).await.unwrap());
    assert!(backend.exists(ctx2, key).await.unwrap());
}

#[tokio::test]
async fn test_postgres_backend_connection_error() {
    let result = PostgresBackend::new("postgres://invalid:5432/invalid").await;
    assert!(result.is_err(), "Should fail with invalid connection");

    match result {
        Err(PostgresError::Database(_)) => {}
        _ => panic!("Expected PostgresError::Database"),
    }
}

#[tokio::test]
async fn test_postgres_backend_drift_result() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());

    let drift = mk_core::types::DriftResult {
        project_id: "proj-1".to_string(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.5,
        confidence: 1.0,
        violations: vec![],
        suppressed_violations: vec![],
        requires_manual_review: false,
        timestamp: chrono::Utc::now().timestamp(),
    };

    backend.store_drift_result(drift).await.unwrap();

    let result = backend
        .get_latest_drift_result(ctx.clone(), "proj-1")
        .await
        .unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().drift_score, 0.5);

    let none_result = backend
        .get_latest_drift_result(ctx, "nonexistent")
        .await
        .unwrap();
    assert!(none_result.is_none());
}

#[tokio::test]
async fn test_postgres_backend_role_management() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let user_id = UserId::new("user-1".to_string()).unwrap();
    let comp_id = unique_id("org");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Organization 1".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    backend
        .assign_role(
            &user_id,
            &tenant_id,
            &comp_id,
            mk_core::types::Role::Admin.into(),
        )
        .await
        .unwrap();

    let ctx = TenantContext::new(tenant_id.clone(), user_id.clone());
    let roles = backend
        .get_user_roles_scoped(&ctx, &user_id, &tenant_id)
        .await
        .unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].0, comp_id);
    assert_eq!(roles[0].1, mk_core::types::Role::Admin.into());

    backend
        .remove_role(
            &user_id,
            &tenant_id,
            &comp_id,
            mk_core::types::Role::Admin.into(),
        )
        .await
        .unwrap();

    let roles_after = backend
        .get_user_roles_scoped(&ctx, &user_id, &tenant_id)
        .await
        .unwrap();
    assert_eq!(roles_after.len(), 0);
}

#[tokio::test]
async fn test_resolve_user_id_by_idp_subject_returns_matching_user() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let email = format!("{}@example.com", unique_id("user"));
    let idp_subject = unique_id("github-login");

    let user_id: String = sqlx::query_scalar(
        "INSERT INTO users (email, name, idp_provider, idp_subject, status, created_at, updated_at)
         VALUES ($1, $1, 'github', $2, 'active', NOW(), NOW())
         RETURNING id::text",
    )
    .bind(&email)
    .bind(&idp_subject)
    .fetch_one(backend.pool())
    .await
    .unwrap();

    let resolved = backend
        .resolve_user_id_by_idp_subject_bootstrap(&idp_subject)
        .await
        .unwrap();

    assert_eq!(resolved.as_deref(), Some(user_id.as_str()));
    assert_eq!(
        backend
            .resolve_user_id_by_idp_subject_bootstrap("missing-subject")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn test_get_user_roles_for_auth_includes_instance_scope_and_deduplicates() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let tenant_unit_id = unique_id("org");
    let root_unit_id = unique_id("instance");
    let user_id = unique_id("user");
    let now = chrono::Utc::now();

    backend
        .create_unit(&OrganizationalUnit {
            id: tenant_unit_id.clone(),
            name: "Tenant Root Org".to_string(),
            unit_type: UnitType::Organization,
            parent_id: None,
            tenant_id: tenant_id.clone(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: RecordSource::Admin,
        })
        .await
        .unwrap();

    backend
        .create_unit(&OrganizationalUnit {
            id: root_unit_id.clone(),
            name: "Instance Scope".to_string(),
            unit_type: UnitType::Organization,
            parent_id: None,
            tenant_id: INSTANCE_SCOPE_TENANT_ID.parse().unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: RecordSource::Admin,
        })
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&user_id)
    .bind(tenant_id.as_str())
    .bind(&tenant_unit_id)
    .bind("admin")
    .bind(now)
    .execute(backend.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&user_id)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(&root_unit_id)
    .bind("platformadmin")
    .bind(now)
    .execute(backend.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO user_roles (user_id, tenant_id, unit_id, role, created_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&user_id)
    .bind(INSTANCE_SCOPE_TENANT_ID)
    .bind(&root_unit_id)
    .bind("admin")
    .bind(now)
    .execute(backend.pool())
    .await
    .unwrap();

    let roles = backend
        .get_user_roles_for_auth_bootstrap(&user_id, tenant_id.as_str())
        .await
        .unwrap();

    let actual: std::collections::HashSet<RoleIdentifier> = roles.into_iter().collect();
    let expected: std::collections::HashSet<RoleIdentifier> =
        [Role::Admin.into(), Role::PlatformAdmin.into()]
            .into_iter()
            .collect();

    assert_eq!(actual, expected);
}

#[tokio::test]
async fn test_postgres_backend_unit_policy() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());
    let comp_id = unique_id("org");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Organization 1".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    let policy = mk_core::types::Policy {
        id: "policy-1".to_string(),
        name: "Test Policy".to_string(),
        description: None,
        layer: mk_core::types::KnowledgeLayer::Tenant,
        rules: vec![],
        metadata: std::collections::HashMap::new(),
        mode: mk_core::types::PolicyMode::Optional,
        merge_strategy: mk_core::types::RuleMergeStrategy::Merge,
    };

    backend
        .add_unit_policy_scoped(&ctx, &comp_id, &policy)
        .await
        .unwrap();

    let policies = backend
        .get_unit_policies_scoped(&ctx, &comp_id)
        .await
        .unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].id, "policy-1");

    let nonexistent_result = backend
        .add_unit_policy_scoped(&ctx, "nonexistent", &policy)
        .await;
    assert!(matches!(
        nonexistent_result,
        Err(PostgresError::NotFound(_))
    ));
}

#[tokio::test]
async fn test_postgres_backend_governance_events() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());

    let event = mk_core::types::GovernanceEvent::DriftDetected {
        project_id: "proj-1".to_string(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.5,
        timestamp: chrono::Utc::now().timestamp(),
    };

    backend
        .with_tenant_context(&ctx, |tx| {
            Box::pin(async move { PostgresBackend::log_event(tx, &event).await })
        })
        .await
        .unwrap();

    let events = backend.get_governance_events(ctx, 0, 10).await.unwrap();
    assert_eq!(events.len(), 1);

    if let mk_core::types::GovernanceEvent::DriftDetected { drift_score, .. } = &events[0] {
        assert_eq!(*drift_score, 0.5);
    } else {
        panic!("Wrong event type");
    }
}

#[tokio::test]
async fn test_postgres_backend_governance_event_types() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());

    let events = vec![
        mk_core::types::GovernanceEvent::UnitCreated {
            unit_id: "unit-1".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            tenant_id: tenant_id.clone(),
            parent_id: None,
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::UnitUpdated {
            unit_id: "unit-1".to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::UnitDeleted {
            unit_id: "unit-1".to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::RoleAssigned {
            user_id: mk_core::types::UserId::new("user-1".to_string()).unwrap(),
            unit_id: "unit-1".to_string(),
            role: mk_core::types::Role::Admin.into(),
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::RoleRemoved {
            user_id: mk_core::types::UserId::new("user-1".to_string()).unwrap(),
            unit_id: "unit-1".to_string(),
            role: mk_core::types::Role::Admin.into(),
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::PolicyUpdated {
            policy_id: "policy-1".to_string(),
            layer: mk_core::types::KnowledgeLayer::Tenant,
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
        mk_core::types::GovernanceEvent::PolicyDeleted {
            policy_id: "policy-1".to_string(),
            tenant_id: tenant_id.clone(),
            timestamp: chrono::Utc::now().timestamp(),
        },
    ];

    for event in events {
        backend
            .with_tenant_context(&ctx, |tx| {
                Box::pin(async move { PostgresBackend::log_event(tx, &event).await })
            })
            .await
            .unwrap();
    }

    let retrieved_events = backend.get_governance_events(ctx, 0, 10).await.unwrap();
    assert_eq!(retrieved_events.len(), 7);
}

#[tokio::test]
async fn test_postgres_backend_unit_operations() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());
    let comp_id = unique_id("org");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Organization 1".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    let retrieved = backend.get_unit_scoped(&ctx, &comp_id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().name, "Organization 1");

    let mut updated_root_org = root_org.clone();
    updated_root_org.name = "Updated Organization".to_string();
    updated_root_org.updated_at = chrono::Utc::now();
    backend
        .update_unit_scoped(&ctx, &updated_root_org)
        .await
        .unwrap();

    let updated = backend.get_unit_scoped(&ctx, &comp_id).await.unwrap();
    assert_eq!(updated.unwrap().name, "Updated Organization");

    backend.delete_unit_scoped(&ctx, &comp_id).await.unwrap();
    let deleted = backend.get_unit_scoped(&ctx, &comp_id).await.unwrap();
    assert!(deleted.is_none());
}

#[tokio::test]
async fn test_postgres_backend_list_children() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());
    let comp_id = unique_id("org");
    let org1_id = unique_id("team");
    let org2_id = unique_id("team");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Organization 1".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    let org1 = mk_core::types::OrganizationalUnit {
        id: org1_id,
        name: "Team 1".to_string(),
        unit_type: mk_core::types::UnitType::Team,
        tenant_id: tenant_id.clone(),
        parent_id: Some(comp_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&org1).await.unwrap();

    let org2 = mk_core::types::OrganizationalUnit {
        id: org2_id,
        name: "Team 2".to_string(),
        unit_type: mk_core::types::UnitType::Team,
        tenant_id: tenant_id.clone(),
        parent_id: Some(comp_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&org2).await.unwrap();

    let children = backend.list_children_scoped(&ctx, &comp_id).await.unwrap();
    assert_eq!(children.len(), 2);
}

#[tokio::test]
async fn test_postgres_backend_list_all_units() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let comp_id = unique_id("org");
    let org_id = unique_id("team");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Org".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    let org = mk_core::types::OrganizationalUnit {
        id: org_id,
        name: "Team".to_string(),
        unit_type: mk_core::types::UnitType::Team,
        tenant_id: tenant_id.clone(),
        parent_id: Some(comp_id),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&org).await.unwrap();

    let all_units = backend.list_all_units().await.unwrap();
    let our_units: Vec<_> = all_units
        .iter()
        .filter(|u| u.tenant_id == tenant_id)
        .collect();
    assert_eq!(our_units.len(), 2);
}

#[tokio::test]
async fn test_postgres_backend_hierarchy_validation() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let ctx = TenantContext::new(tenant_id.clone(), UserId::default());
    let comp_id = unique_id("org");
    let org_id = unique_id("team");
    let team_id = unique_id("proj");

    let root_org = mk_core::types::OrganizationalUnit {
        id: comp_id.clone(),
        name: "Org".to_string(),
        unit_type: mk_core::types::UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&root_org).await.unwrap();

    let org = mk_core::types::OrganizationalUnit {
        id: org_id.clone(),
        name: "Team".to_string(),
        unit_type: mk_core::types::UnitType::Team,
        tenant_id: tenant_id.clone(),
        parent_id: Some(comp_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&org).await.unwrap();

    let team = mk_core::types::OrganizationalUnit {
        id: team_id,
        name: "Project".to_string(),
        unit_type: mk_core::types::UnitType::Project,
        tenant_id: tenant_id.clone(),
        parent_id: Some(org_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    backend.create_unit(&team).await.unwrap();

    let retrieved_org = backend.get_unit_scoped(&ctx, &org_id).await.unwrap();
    assert!(retrieved_org.is_some());
    assert_eq!(retrieved_org.unwrap().parent_id, Some(comp_id));
}

#[tokio::test]
async fn test_postgres_backend_job_status() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();
    let now = chrono::Utc::now().timestamp();

    backend
        .record_job_status("test-job", tenant_id.as_str(), "running", None, now, None)
        .await
        .unwrap();

    backend
        .record_job_status(
            "test-job",
            tenant_id.as_str(),
            "completed",
            Some("Job finished successfully"),
            now,
            Some(now + 10),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn test_postgres_backend_event_publisher_subscribe() {
    let Some(backend) = create_test_backend().await else {
        eprintln!("Skipping PostgreSQL test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_tenant_id()).unwrap();

    let event = mk_core::types::GovernanceEvent::DriftDetected {
        project_id: "proj-pub".to_string(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.75,
        timestamp: chrono::Utc::now().timestamp(),
    };

    let ctx = TenantContext::new(tenant_id, UserId::default());
    backend
        .with_tenant_context(&ctx, |tx| {
            Box::pin(async move { PostgresBackend::log_event(tx, &event).await })
        })
        .await
        .unwrap();

    let events = backend.get_governance_events(ctx, 0, 10).await.unwrap();
    assert!(!events.is_empty());
}
