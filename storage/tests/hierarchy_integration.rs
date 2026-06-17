//! Integration tests for organizational hierarchy in PostgreSQL storage backend
//!
//! These tests verify the strict hierarchy enforcement and recursive navigation
//! logic.

use mk_core::traits::StorageBackend;
use mk_core::types::{OrganizationalUnit, RecordSource, TenantContext, TenantId, UnitType, UserId};
use std::collections::HashMap;
use storage::postgres::PostgresBackend;
use testing::postgres;

async fn setup_postgres_backend() -> Result<PostgresBackend, anyhow::Error> {
    let fixture = postgres()
        .await
        .ok_or_else(|| anyhow::anyhow!("Docker not available"))?;
    let backend = PostgresBackend::new(fixture.url()).await?;
    backend.initialize_schema().await?;
    Ok(backend)
}

fn create_test_unit(
    id: &str,
    name: &str,
    unit_type: UnitType,
    parent_id: Option<String>,
    tenant_id: &str,
) -> OrganizationalUnit {
    OrganizationalUnit {
        id: id.to_string(),
        name: name.to_string(),
        unit_type,
        parent_id,
        tenant_id: TenantId::new(tenant_id.to_string()).unwrap(),
        metadata: HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    }
}

#[tokio::test]
async fn test_hierarchy_strict_enforcement() {
    let backend = match setup_postgres_backend().await {
        Ok(b) => b,
        Err(_) => {
            eprintln!("Skipping PostgreSQL hierarchy test: Docker not available");
            return;
        }
    };

    let tenant_id = "tenant-1";

    // 1. Root must be an Organization
    let team_root = create_test_unit("team-1", "Team Root", UnitType::Team, None, tenant_id);
    let result = backend.create_unit(&team_root).await;
    assert!(result.is_err(), "Root unit must be an Organization");

    let org = create_test_unit("org-1", "Org", UnitType::Organization, None, tenant_id);
    backend
        .create_unit(&org)
        .await
        .expect("Organization can be root");

    // 2. Team must be under Organization
    let project_under_org = create_test_unit(
        "proj-1",
        "Proj Under Org",
        UnitType::Project,
        Some("org-1".to_string()),
        tenant_id,
    );
    let result = backend.create_unit(&project_under_org).await;
    assert!(
        result.is_err(),
        "Project cannot be directly under Organization"
    );

    let team = create_test_unit(
        "team-1",
        "Team",
        UnitType::Team,
        Some("org-1".to_string()),
        tenant_id,
    );
    backend
        .create_unit(&team)
        .await
        .expect("Team can be under Organization");

    // 4. Project must be under Team
    let project = create_test_unit(
        "proj-1",
        "Project",
        UnitType::Project,
        Some("team-1".to_string()),
        tenant_id,
    );
    backend
        .create_unit(&project)
        .await
        .expect("Project can be under Team");
}

#[tokio::test]
async fn test_recursive_hierarchy_navigation() {
    let backend: PostgresBackend = match setup_postgres_backend().await {
        Ok(b) => b,
        Err(_) => {
            eprintln!("Skipping PostgreSQL hierarchy test: Docker not available");
            return;
        }
    };

    let tenant_id = "tenant-2";
    let ctx = TenantContext::new(
        TenantId::new(tenant_id.to_string()).unwrap(),
        UserId::default(),
    );

    // Build hierarchy: Org -> Team -> Proj
    let org = create_test_unit("org-2", "Org", UnitType::Organization, None, tenant_id);
    let team = create_test_unit(
        "team-2",
        "Team",
        UnitType::Team,
        Some("org-2".to_string()),
        tenant_id,
    );
    let project = create_test_unit(
        "proj-2",
        "Proj",
        UnitType::Project,
        Some("team-2".to_string()),
        tenant_id,
    );

    backend.create_unit(&org).await.unwrap();
    backend.create_unit(&team).await.unwrap();
    backend.create_unit(&project).await.unwrap();

    // Test Ancestors of Project
    let ancestors = backend.get_ancestors_scoped(&ctx, "proj-2").await.unwrap();
    assert_eq!(ancestors.len(), 2);
    assert_eq!(ancestors[0].id, "team-2");
    assert_eq!(ancestors[1].id, "org-2");

    // Test Descendants of Organization
    let descendants_org = backend
        .get_unit_descendants_scoped(&ctx, "org-2")
        .await
        .unwrap();
    assert_eq!(descendants_org.len(), 2);
    assert_eq!(descendants_org[0].id, "team-2");
    assert_eq!(descendants_org[1].id, "proj-2");

    // Test Descendants of Team
    let descendants_team = backend
        .get_unit_descendants_scoped(&ctx, "team-2")
        .await
        .unwrap();
    assert_eq!(descendants_team.len(), 1);
    assert_eq!(descendants_team[0].id, "proj-2");
}
