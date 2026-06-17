use mk_core::traits::StorageBackend;
use mk_core::types::{OrganizationalUnit, RecordSource, TenantContext, TenantId, UnitType, UserId};
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};

async fn create_test_backend() -> Option<PostgresBackend> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;
    backend.initialize_schema().await.ok()?;
    Some(backend)
}

#[tokio::test]
async fn test_recursive_hierarchy_queries() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: UserId::new("u1".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let org_id = unique_id("org");
    let team_id = unique_id("team");
    let proj_id = unique_id("proj");

    let org1 = OrganizationalUnit {
        id: org_id.clone(),
        name: "Org 1".to_string(),
        unit_type: UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&org1).await.unwrap();

    let team1 = OrganizationalUnit {
        id: team_id.clone(),
        name: "Team 1".to_string(),
        unit_type: UnitType::Team,
        tenant_id: tenant_id.clone(),
        parent_id: Some(org_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&team1).await.unwrap();

    let project1 = OrganizationalUnit {
        id: proj_id.clone(),
        name: "Project 1".to_string(),
        unit_type: UnitType::Project,
        tenant_id: tenant_id.clone(),
        parent_id: Some(team_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&project1).await.unwrap();

    let descendants = StorageBackend::get_descendants(&storage, ctx.clone(), &org_id)
        .await
        .unwrap();
    assert_eq!(descendants.len(), 2);
    let ids: Vec<String> = descendants.iter().map(|u| u.id.clone()).collect();
    assert!(ids.contains(&team_id));
    assert!(ids.contains(&proj_id));

    let descendants = StorageBackend::get_descendants(&storage, ctx.clone(), &team_id)
        .await
        .unwrap();
    assert_eq!(descendants.len(), 1);

    let ancestors = StorageBackend::get_ancestors(&storage, ctx.clone(), &proj_id)
        .await
        .unwrap();
    assert_eq!(ancestors.len(), 2);
    let ids: Vec<String> = ancestors.iter().map(|u| u.id.clone()).collect();
    assert!(ids.contains(&team_id));
    assert!(ids.contains(&org_id));

    let tenant2_id = TenantId::new(unique_id("t2")).unwrap();
    let ctx2 = TenantContext {
        tenant_id: tenant2_id.clone(),
        user_id: UserId::new("u2".to_string()).unwrap(),
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let ancestors_t2 = storage.get_ancestors(ctx2.clone(), &proj_id).await.unwrap();
    assert_eq!(ancestors_t2.len(), 0);
}
