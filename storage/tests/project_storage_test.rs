use mk_core::traits::StorageBackend;
use mk_core::types::{
    OrganizationalUnit, RecordSource, Role, TenantContext, TenantId, UnitType, UserId,
};
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};

async fn create_test_backend() -> Option<PostgresBackend> {
    let fixture = postgres().await?;
    let backend = PostgresBackend::new(fixture.url()).await.ok()?;
    backend.initialize_schema().await.ok()?;
    Some(backend)
}

fn make_unit(
    id: &str,
    name: &str,
    unit_type: UnitType,
    parent_id: Option<String>,
    tenant_id: &TenantId,
) -> OrganizationalUnit {
    OrganizationalUnit {
        id: id.to_string(),
        name: name.to_string(),
        unit_type,
        parent_id,
        tenant_id: tenant_id.clone(),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    }
}

async fn create_hierarchy(
    storage: &PostgresBackend,
    tenant_id: &TenantId,
) -> (String, String, String) {
    let org_id = unique_id("org");
    let team_id = unique_id("team");
    let project_id = unique_id("proj");

    let org = make_unit(
        &org_id,
        "Organization",
        UnitType::Organization,
        None,
        tenant_id,
    );
    storage.create_unit(&org).await.unwrap();

    let team = make_unit(
        &team_id,
        "Team",
        UnitType::Team,
        Some(org_id.clone()),
        tenant_id,
    );
    storage.create_unit(&team).await.unwrap();

    let project = make_unit(
        &project_id,
        "Project",
        UnitType::Project,
        Some(team_id.clone()),
        tenant_id,
    );
    storage.create_unit(&project).await.unwrap();

    (org_id, team_id, project_id)
}

#[tokio::test]
async fn test_assign_team_to_project_and_list() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (_, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;

    storage
        .assign_team_to_project(&proj_id, &team_id, tenant_id.as_str(), "owner")
        .await
        .unwrap();

    let assignments = storage
        .list_project_team_assignments(&proj_id, tenant_id.as_str())
        .await
        .unwrap();

    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0], (team_id, "owner".to_string()));
}

#[tokio::test]
async fn test_assign_multiple_teams_to_project() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (org_id, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;

    let team2_id = unique_id("team");
    let team2 = make_unit(
        &team2_id,
        "Team 2",
        UnitType::Team,
        Some(org_id.clone()),
        &tenant_id,
    );
    storage.create_unit(&team2).await.unwrap();

    storage
        .assign_team_to_project(&proj_id, &team_id, tenant_id.as_str(), "owner")
        .await
        .unwrap();
    storage
        .assign_team_to_project(&proj_id, &team2_id, tenant_id.as_str(), "contributor")
        .await
        .unwrap();

    let mut assignments = storage
        .list_project_team_assignments(&proj_id, tenant_id.as_str())
        .await
        .unwrap();
    assignments.sort();

    assert_eq!(assignments.len(), 2);
    assert_eq!(
        assignments[0],
        (team2_id.clone(), "contributor".to_string())
    );
    assert_eq!(assignments[1], (team_id.clone(), "owner".to_string()));
}

#[tokio::test]
async fn test_remove_team_from_project() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (_, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;

    storage
        .assign_team_to_project(&proj_id, &team_id, tenant_id.as_str(), "owner")
        .await
        .unwrap();

    storage
        .remove_team_from_project(&proj_id, &team_id, tenant_id.as_str())
        .await
        .unwrap();

    let assignments = storage
        .list_project_team_assignments(&proj_id, tenant_id.as_str())
        .await
        .unwrap();

    assert!(assignments.is_empty());
}

#[tokio::test]
async fn test_assign_team_to_project_upsert_updates_assignment_type() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (_, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;

    storage
        .assign_team_to_project(&proj_id, &team_id, tenant_id.as_str(), "owner")
        .await
        .unwrap();
    storage
        .assign_team_to_project(&proj_id, &team_id, tenant_id.as_str(), "contributor")
        .await
        .unwrap();

    let assignments = storage
        .list_project_team_assignments(&proj_id, tenant_id.as_str())
        .await
        .unwrap();

    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0], (team_id, "contributor".to_string()));
}

#[tokio::test]
async fn test_get_effective_roles_at_scope_inherited_from_ancestor() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (_, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;
    let user_id = UserId::new(unique_id("user")).unwrap();

    storage
        .assign_role(&user_id, &tenant_id, &team_id, Role::TechLead.into())
        .await
        .unwrap();

    let roles = storage
        .get_effective_roles_at_scope(&user_id, &tenant_id, &proj_id)
        .await
        .unwrap();

    assert!(roles.contains(&Role::TechLead.into()));
}

#[tokio::test]
async fn test_get_effective_roles_at_scope_multiple_roles_different_scopes() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (org_id, team_id, proj_id) = create_hierarchy(&storage, &tenant_id).await;
    let user_id = UserId::new(unique_id("user")).unwrap();

    storage
        .assign_role(&user_id, &tenant_id, &team_id, Role::Developer.into())
        .await
        .unwrap();
    storage
        .assign_role(&user_id, &tenant_id, &org_id, Role::Viewer.into())
        .await
        .unwrap();

    let roles = storage
        .get_effective_roles_at_scope(&user_id, &tenant_id, &proj_id)
        .await
        .unwrap();

    assert_eq!(roles.len(), 2);
    assert!(roles.contains(&Role::Developer.into()));
    assert!(roles.contains(&Role::Viewer.into()));
}

#[tokio::test]
async fn test_get_effective_roles_at_scope_tenant_isolation() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_a = TenantId::new(unique_id("tenant-a")).unwrap();
    let tenant_b = TenantId::new(unique_id("tenant-b")).unwrap();
    let (_, team_id, proj_id) = create_hierarchy(&storage, &tenant_a).await;
    let user_id = UserId::new(unique_id("user")).unwrap();

    storage
        .assign_role(&user_id, &tenant_a, &team_id, Role::TechLead.into())
        .await
        .unwrap();

    let roles = storage
        .get_effective_roles_at_scope(&user_id, &tenant_b, &proj_id)
        .await
        .unwrap();

    assert!(roles.is_empty());
}

#[tokio::test]
async fn test_get_effective_roles_at_scope_no_roles_returns_empty() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let (_, _, proj_id) = create_hierarchy(&storage, &tenant_id).await;
    let user_id = UserId::new(unique_id("user")).unwrap();

    let roles = storage
        .get_effective_roles_at_scope(&user_id, &tenant_id, &proj_id)
        .await
        .unwrap();

    assert!(roles.is_empty());
}

#[tokio::test]
async fn test_tenant_context_smoke_for_pattern_parity() {
    let Some(storage) = create_test_backend().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let user_id = UserId::new(unique_id("user")).unwrap();
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id,
        agent_id: None,
        roles: Vec::new(),
        target_tenant_id: None,
    };

    let (_, _, proj_id) = create_hierarchy(&storage, &ctx.tenant_id).await;
    let assignments = storage
        .list_project_team_assignments(&proj_id, ctx.tenant_id.as_str())
        .await
        .unwrap();
    assert!(assignments.is_empty());
}
