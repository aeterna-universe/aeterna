use knowledge::api::{
    GovernanceDashboardApi, approve_proposal, get_drift_status, get_job_status, get_org_report,
    reject_proposal, replay_events,
};
use knowledge::governance::GovernanceEngine;
use mk_core::traits::{EventPublisher, KnowledgeRepository, StorageBackend};
use mk_core::types::{
    GovernanceEvent, KnowledgeEntry, KnowledgeLayer, KnowledgeStatus, KnowledgeType,
    OrganizationalUnit, RecordSource, TenantContext, TenantId, UnitType,
};
use std::sync::Arc;
use storage::postgres::PostgresBackend;
use testing::{postgres, unique_id};
use tokio::sync::OnceCell;

static SCHEMA_INITIALIZED: OnceCell<bool> = OnceCell::const_new();

/// Creates a storage backend, initializing schema only once across all tests
async fn create_test_storage() -> Option<Arc<PostgresBackend>> {
    let fixture = postgres().await?;
    let storage = Arc::new(PostgresBackend::new(fixture.url()).await.ok()?);

    // Initialize schema only once
    SCHEMA_INITIALIZED
        .get_or_init(|| async {
            storage.initialize_schema().await.ok();
            true
        })
        .await;

    Some(storage)
}

#[tokio::test]
async fn test_governance_dashboard_api_get_drift_status() {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let engine = Arc::new(GovernanceEngine::new());
    let deployment_config = config::config::DeploymentConfig {
        mode: "local".to_string(),
        remote_url: None,
        ..Default::default()
    };

    let api = Arc::new(GovernanceDashboardApi::new(
        engine.clone(),
        storage.clone(),
        deployment_config,
    ));

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let project_id = unique_id("p");
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: mk_core::types::UserId::new(unique_id("u")).unwrap(),
        agent_id: Some(unique_id("a")),
        roles: vec![],
        target_tenant_id: None,
    };

    let res = get_drift_status(api.clone(), &ctx, &project_id)
        .await
        .unwrap();
    assert!(res.is_none());

    let drift = mk_core::types::DriftResult {
        project_id: project_id.clone(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.75,
        violations: vec![],
        timestamp: 123456789,
        confidence: 0.9,
        suppressed_violations: vec![],
        requires_manual_review: false,
    };
    storage.store_drift_result(drift).await.unwrap();

    let res = get_drift_status(api.clone(), &ctx, &project_id)
        .await
        .unwrap();
    assert!(res.is_some());
    assert_eq!(res.unwrap().drift_score, 0.75);
}

#[tokio::test]
async fn test_governance_dashboard_api_get_org_report() {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let engine = Arc::new(GovernanceEngine::new());
    let deployment_config = config::config::DeploymentConfig {
        mode: "local".to_string(),
        remote_url: None,
        ..Default::default()
    };

    let api = Arc::new(GovernanceDashboardApi::new(
        engine.clone(),
        storage.clone(),
        deployment_config,
    ));

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let org_id = unique_id("org");
    let team_id = unique_id("team");
    let project_id = unique_id("p");

    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: mk_core::types::UserId::new(unique_id("u")).unwrap(),
        agent_id: Some(unique_id("a")),
        roles: vec![],
        target_tenant_id: None,
    };

    let company_unit = OrganizationalUnit {
        id: tenant_id.as_str().to_string(),
        name: "Tenant Root".to_string(),
        unit_type: UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&company_unit).await.unwrap();

    let org_unit = OrganizationalUnit {
        id: org_id.clone(),
        name: "Org 1".to_string(),
        unit_type: UnitType::Organization,
        tenant_id: tenant_id.clone(),
        parent_id: Some(tenant_id.as_str().to_string()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&org_unit).await.unwrap();

    let team_unit = OrganizationalUnit {
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
    storage.create_unit(&team_unit).await.unwrap();

    let project_unit = OrganizationalUnit {
        id: project_id.clone(),
        name: "Project 1".to_string(),
        unit_type: UnitType::Project,
        tenant_id: tenant_id.clone(),
        parent_id: Some(team_id.clone()),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        updated_at: chrono::DateTime::from_timestamp(1000, 0).unwrap(),
        source_owner: RecordSource::Admin,
    };
    storage.create_unit(&project_unit).await.unwrap();

    let drift = mk_core::types::DriftResult {
        project_id: project_id.clone(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.5,
        violations: vec![],
        timestamp: 123456789,
        confidence: 0.9,
        suppressed_violations: vec![],
        requires_manual_review: false,
    };
    storage.store_drift_result(drift).await.unwrap();

    let report = get_org_report(api.clone(), &ctx, tenant_id.as_str())
        .await
        .unwrap();
    assert_eq!(report["orgId"], tenant_id.as_str());
    assert_eq!(report["averageDrift"], 0.5);
    assert_eq!(report["projectCount"], 1);
}

#[tokio::test]
async fn test_governance_dashboard_api_proposals() {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let temp_dir = tempfile::tempdir().unwrap();
    let repo = Arc::new(
        knowledge::repository::GitRepository::new(temp_dir.path().to_str().unwrap()).unwrap(),
    );

    let engine = Arc::new(GovernanceEngine::new().with_repository(repo.clone()));
    let deployment_config = config::config::DeploymentConfig {
        mode: "local".to_string(),
        remote_url: None,
        ..Default::default()
    };

    let api = Arc::new(GovernanceDashboardApi::new(
        engine.clone(),
        storage.clone(),
        deployment_config,
    ));

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let user_id = unique_id("u");
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: mk_core::types::UserId::new(user_id.clone()).unwrap(),
        agent_id: Some(unique_id("a")),
        roles: vec![],
        target_tenant_id: None,
    };

    let prop1_path = unique_id("prop");
    let entry = KnowledgeEntry {
        path: prop1_path.clone(),
        content: "Content".to_string(),
        layer: KnowledgeLayer::Project,
        kind: KnowledgeType::Adr,
        status: KnowledgeStatus::Proposed,
        metadata: std::collections::HashMap::new(),
        commit_hash: None,
        author: Some(user_id.clone()),
        updated_at: 1000,
        summaries: std::collections::HashMap::new(),
    };
    repo.store(ctx.clone(), entry, "Initial proposal")
        .await
        .unwrap();

    let proposals = api.list_proposals(&ctx, None).await.unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].path, prop1_path);

    approve_proposal(api.clone(), &ctx, &prop1_path)
        .await
        .unwrap();
    let approved = repo
        .get(ctx.clone(), KnowledgeLayer::Project, &prop1_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved.status, KnowledgeStatus::Accepted);

    let prop2_path = unique_id("prop");
    let entry2 = KnowledgeEntry {
        path: prop2_path.clone(),
        content: "Content".to_string(),
        layer: KnowledgeLayer::Project,
        kind: KnowledgeType::Adr,
        status: KnowledgeStatus::Proposed,
        metadata: std::collections::HashMap::new(),
        commit_hash: None,
        author: Some(user_id.clone()),
        updated_at: 1000,
        summaries: std::collections::HashMap::new(),
    };
    repo.store(ctx.clone(), entry2, "Initial proposal")
        .await
        .unwrap();

    reject_proposal(api.clone(), &ctx, &prop2_path, "Too complex")
        .await
        .unwrap();
    let rejected = repo
        .get(ctx.clone(), KnowledgeLayer::Project, &prop2_path)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected.status, KnowledgeStatus::Draft);
    assert_eq!(
        rejected.metadata.get("rejection_reason").unwrap(),
        &serde_json::json!("Too complex")
    );
}

#[tokio::test]
async fn test_governance_dashboard_api_get_job_status() {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let engine = Arc::new(GovernanceEngine::new());
    let api = Arc::new(GovernanceDashboardApi::new(
        engine,
        storage.clone(),
        config::config::DeploymentConfig::default(),
    ));

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: mk_core::types::UserId::new(unique_id("u")).unwrap(),
        agent_id: Some(unique_id("a")),
        roles: vec![],
        target_tenant_id: None,
    };

    let job1 = unique_id("test-job");
    let job2 = unique_id("other-job");
    let job3 = unique_id("global-job");

    storage
        .record_job_status(
            &job1,
            tenant_id.as_str(),
            "completed",
            Some("Done"),
            1000,
            Some(1001),
        )
        .await
        .unwrap();
    storage
        .record_job_status(&job2, tenant_id.as_str(), "running", None, 1002, None)
        .await
        .unwrap();
    storage
        .record_job_status(&job3, "all", "completed", None, 1003, Some(1004))
        .await
        .unwrap();

    let all_jobs = get_job_status(api.clone(), &ctx, None).await.unwrap();
    assert_eq!(all_jobs.as_array().unwrap().len(), 3);

    let filtered_jobs = get_job_status(api.clone(), &ctx, Some(&job1))
        .await
        .unwrap();
    assert_eq!(filtered_jobs.as_array().unwrap().len(), 1);
    assert_eq!(filtered_jobs[0]["jobName"], job1);
    assert_eq!(filtered_jobs[0]["status"], "completed");
}

#[tokio::test]
async fn test_governance_dashboard_api_replay_events() {
    let Some(storage) = create_test_storage().await else {
        eprintln!("Skipping Postgres test: Docker not available");
        return;
    };

    let engine = Arc::new(GovernanceEngine::new());
    let api = Arc::new(GovernanceDashboardApi::new(
        engine,
        storage.clone(),
        config::config::DeploymentConfig::default(),
    ));

    let tenant_id = TenantId::new(unique_id("t")).unwrap();
    let ctx = TenantContext {
        tenant_id: tenant_id.clone(),
        user_id: mk_core::types::UserId::new(unique_id("u")).unwrap(),
        agent_id: Some(unique_id("a")),
        roles: vec![],
        target_tenant_id: None,
    };

    let unit1_id = unique_id("u");
    let unit2_id = unique_id("u");
    let event1 = GovernanceEvent::UnitCreated {
        unit_id: unit1_id.clone(),
        unit_type: UnitType::Project,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        timestamp: 1000,
    };
    let event2 = GovernanceEvent::UnitCreated {
        unit_id: unit2_id.clone(),
        unit_type: UnitType::Project,
        tenant_id: tenant_id.clone(),
        parent_id: None,
        timestamp: 2000,
    };

    storage.publish(event1).await.unwrap();
    storage.publish(event2).await.unwrap();

    let all_replayed = replay_events(api.clone(), &ctx, 500, 10).await.unwrap();
    assert_eq!(all_replayed.len(), 2);

    let partial_replayed = replay_events(api.clone(), &ctx, 1500, 10).await.unwrap();
    assert_eq!(partial_replayed.len(), 1);
    if let GovernanceEvent::UnitCreated { unit_id, .. } = &partial_replayed[0] {
        assert_eq!(unit_id, &unit2_id);
    } else {
        panic!("Expected UnitCreated event");
    }
}
