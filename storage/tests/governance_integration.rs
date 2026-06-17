//! Integration tests for the governance event system and multi-publisher.

use mk_core::traits::{AuthorizationService, EventPublisher, StorageBackend};
use mk_core::types::{
    GovernanceEvent, OrganizationalUnit, RecordSource, TenantContext, TenantId, UserId,
};
use std::sync::Arc;
use storage::events::RedisPublisher;
use storage::postgres::PostgresBackend;
use testing::{postgres, redis, unique_id};

#[tokio::test]
async fn test_governance_event_propagation() {
    let (Some(pg_fixture), Some(redis_fixture)) = (postgres().await, redis().await) else {
        eprintln!("Skipping governance integration test: Docker not available");
        return;
    };

    let pg_backend = Arc::new(PostgresBackend::new(pg_fixture.url()).await.unwrap());
    pg_backend.initialize_schema().await.unwrap();

    let stream_name = unique_id("gov_events");
    let redis_publisher = Arc::new(RedisPublisher::new(redis_fixture.url(), &stream_name).unwrap());

    let mut rx = redis_publisher.subscribe(&[&stream_name]).await.unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let tenant_id = TenantId::new(unique_id("tenant")).unwrap();
    let project_id = unique_id("project");
    let event = GovernanceEvent::DriftDetected {
        project_id: project_id.clone(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.75,
        timestamp: chrono::Utc::now().timestamp(),
    };

    pg_backend.publish(event.clone()).await.unwrap();
    redis_publisher.publish(event.clone()).await.unwrap();

    let received = tokio::time::timeout(tokio::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("Timeout waiting for event")
        .expect("Channel closed");

    if let GovernanceEvent::DriftDetected { drift_score, .. } = received {
        assert_eq!(drift_score, 0.75);
    } else {
        panic!("Wrong event type received");
    }
}

#[tokio::test]
async fn test_full_governance_workflow() {
    let Some(pg_fixture) = postgres().await else {
        eprintln!("Skipping governance workflow test: Docker not available");
        return;
    };

    let pg_backend = Arc::new(PostgresBackend::new(pg_fixture.url()).await.unwrap());
    pg_backend.initialize_schema().await.unwrap();

    let tenant_id = TenantId::new(unique_id("comp")).unwrap();
    let user_id = UserId::new(unique_id("user")).unwrap();
    let agent_id = unique_id("agent");

    let org_id = unique_id("org");
    let team_id = unique_id("team");
    let proj_id = unique_id("proj");

    let org = OrganizationalUnit {
        id: org_id.clone(),
        name: "Organization 1".into(),
        unit_type: mk_core::types::UnitType::Organization,
        parent_id: None,
        tenant_id: tenant_id.clone(),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    pg_backend.create_unit(&org).await.unwrap();

    let team = OrganizationalUnit {
        id: team_id.clone(),
        name: "Team 1".into(),
        unit_type: mk_core::types::UnitType::Team,
        parent_id: Some(org_id.clone()),
        tenant_id: tenant_id.clone(),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    pg_backend.create_unit(&team).await.unwrap();

    let project = OrganizationalUnit {
        id: proj_id.clone(),
        name: "Project 1".into(),
        unit_type: mk_core::types::UnitType::Project,
        parent_id: Some(team_id.clone()),
        tenant_id: tenant_id.clone(),
        metadata: std::collections::HashMap::new(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        source_owner: RecordSource::Admin,
    };
    pg_backend.create_unit(&project).await.unwrap();

    let cedar_policies = format!(
        r#"
            permit(principal == User::"{}", action == Action::"ActAs", resource == User::"{}");
            permit(principal == User::"{}", action == Action::"Update", resource == Unit::"{}");
        "#,
        agent_id,
        user_id.as_str(),
        user_id.as_str(),
        proj_id
    );
    let cedar_schema = "{}";
    let authorizer =
        adapters::auth::cedar::CedarAuthorizer::new(&cedar_policies, cedar_schema).unwrap();

    let ctx = TenantContext::with_agent(tenant_id.clone(), user_id.clone(), agent_id);
    let allowed = authorizer
        .check_permission(&ctx, "Update", &format!("Unit::\"{}\"", proj_id))
        .await
        .unwrap();
    assert!(allowed);

    let job_name = unique_id("drift_scan");
    pg_backend
        .record_job_status(
            &job_name,
            tenant_id.as_str(),
            "completed",
            None,
            chrono::Utc::now().timestamp() - 100,
            Some(chrono::Utc::now().timestamp()),
        )
        .await
        .unwrap();

    let drift = mk_core::types::DriftResult {
        project_id: proj_id.clone(),
        tenant_id: tenant_id.clone(),
        drift_score: 0.2,
        confidence: 1.0,
        violations: vec![],
        suppressed_violations: vec![],
        requires_manual_review: false,
        timestamp: chrono::Utc::now().timestamp(),
    };
    pg_backend.store_drift_result(drift).await.unwrap();

    let engine =
        Arc::new(knowledge::governance::GovernanceEngine::new().with_storage(pg_backend.clone()));
    let deployment_config = config::DeploymentConfig::default();
    let api = Arc::new(knowledge::api::GovernanceDashboardApi::new(
        engine,
        pg_backend.clone(),
        deployment_config,
    ));

    let drift_status = knowledge::api::get_drift_status(api.clone(), &ctx, &proj_id)
        .await
        .unwrap();
    assert!(drift_status.is_some());
    assert_eq!(drift_status.unwrap().drift_score, 0.2);

    let jobs = knowledge::api::get_job_status(api, &ctx, Some(&job_name))
        .await
        .unwrap();
    assert!(!jobs.as_array().unwrap().is_empty());
}
