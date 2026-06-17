use knowledge::governance::GovernanceEngine;
use mk_core::types::{TenantContext, TenantId, UserId};
use serde_json::json;
use std::sync::Arc;
use storage::governance::{
    CreateApprovalRequest, GovernanceStorage, PrincipalType, RequestStatus, RequestType, RiskLevel,
};
use testing::{postgres, unique_id};
use tools::governance::{GovernanceApproveTool, GovernanceRejectTool};
use tools::tools::Tool;
use uuid::Uuid;

fn tenant_context_json(tenant: &str, user: &str) -> serde_json::Value {
    json!({
        "tenant_id": tenant,
        "user_id": user,
    })
}

fn make_tenant_context(tenant: &str, user: &str) -> TenantContext {
    TenantContext::new(
        TenantId::new(tenant.to_string()).unwrap(),
        UserId::new(user.to_string()).unwrap(),
    )
}

fn create_test_request(requestor_id: Uuid, tenant_id: Uuid) -> CreateApprovalRequest {
    CreateApprovalRequest {
        request_type: RequestType::Policy,
        target_type: "policy".to_string(),
        target_id: Some(unique_id("policy-target")),
        tenant_id: Some(tenant_id),
        org_id: None,
        team_id: None,
        project_id: None,
        title: "Test governance integrity request".to_string(),
        description: Some(
            "Ensure requestors cannot approve or reject their own request".to_string(),
        ),
        payload: json!({"policy": "permit(principal, action, resource);"}),
        risk_level: RiskLevel::Medium,
        requestor_type: PrincipalType::User,
        requestor_id,
        requestor_email: Some("requestor@example.com".to_string()),
        required_approvals: 1,
        timeout_hours: Some(24),
    }
}

#[tokio::test]
async fn governance_approve_tool_rejects_requestor_self_approval() {
    let Some(pg_fixture) = postgres().await else {
        eprintln!("Skipping governance integrity approve test: Docker not available");
        return;
    };

    let postgres = Arc::new(
        storage::postgres::PostgresBackend::new(pg_fixture.url())
            .await
            .unwrap(),
    );
    postgres.initialize_schema().await.unwrap();

    let storage = Arc::new(GovernanceStorage::new(postgres.pool().clone()));
    let requestor_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let created = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();

    let tool = GovernanceApproveTool::new(storage.clone(), Arc::new(GovernanceEngine::new()));
    let err = tool
        .call(json!({
            "request_id": created.id.to_string(),
            "approver_id": requestor_id.to_string(),
            "approver_email": "requestor@example.com",
            "comment": "self approve attempt",
            "tenantContext": tenant_context_json(&tenant_id.to_string(), "requestor-user")
        }))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Requestor cannot approve their own governance request")
    );

    let stored_request = storage.get_request(created.id).await.unwrap().unwrap();
    assert_eq!(stored_request.status, RequestStatus::Pending);
    assert_eq!(stored_request.current_approvals, 0);

    let decisions = storage.get_decisions(created.id).await.unwrap();
    assert!(decisions.is_empty());
}

#[tokio::test]
async fn governance_reject_tool_rejects_requestor_self_rejection() {
    let Some(pg_fixture) = postgres().await else {
        eprintln!("Skipping governance integrity reject test: Docker not available");
        return;
    };

    let postgres = Arc::new(
        storage::postgres::PostgresBackend::new(pg_fixture.url())
            .await
            .unwrap(),
    );
    postgres.initialize_schema().await.unwrap();

    let storage = Arc::new(GovernanceStorage::new(postgres.pool().clone()));
    let requestor_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let created = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();

    let tool = GovernanceRejectTool::new(storage.clone(), Arc::new(GovernanceEngine::new()));
    let err = tool
        .call(json!({
            "request_id": created.id.to_string(),
            "rejector_id": requestor_id.to_string(),
            "rejector_email": "requestor@example.com",
            "reason": "self reject attempt",
            "tenantContext": tenant_context_json(&tenant_id.to_string(), "requestor-user")
        }))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Requestor cannot reject their own governance request")
    );

    let stored_request = storage.get_request(created.id).await.unwrap().unwrap();
    assert_eq!(stored_request.status, RequestStatus::Pending);
    assert_eq!(stored_request.resolution_reason, None);

    let decisions = storage.get_decisions(created.id).await.unwrap();
    assert!(decisions.is_empty());
}

#[tokio::test]
async fn governance_integrity_allows_distinct_approver_and_rejector() {
    let Some(pg_fixture) = postgres().await else {
        eprintln!("Skipping governance integrity control test: Docker not available");
        return;
    };

    let postgres = Arc::new(
        storage::postgres::PostgresBackend::new(pg_fixture.url())
            .await
            .unwrap(),
    );
    postgres.initialize_schema().await.unwrap();

    let storage = Arc::new(GovernanceStorage::new(postgres.pool().clone()));
    let requestor_id = Uuid::new_v4();
    let approver_id = Uuid::new_v4();
    let rejector_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let tenant_context = make_tenant_context(&tenant_id.to_string(), "requestor-user");

    let approval_request = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();
    let approve_tool =
        GovernanceApproveTool::new(storage.clone(), Arc::new(GovernanceEngine::new()));
    let approved = approve_tool
        .call(json!({
            "request_id": approval_request.id.to_string(),
            "approver_id": approver_id.to_string(),
            "approver_email": "approver@example.com",
            "comment": "approved by separate user",
            "tenantContext": tenant_context_json(tenant_context.tenant_id.as_str(), tenant_context.user_id.as_str())
        }))
        .await
        .unwrap();

    assert_eq!(approved["success"], true);
    assert_eq!(approved["fully_approved"], true);

    let approved_request = storage
        .get_request(approval_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approved_request.status, RequestStatus::Approved);
    assert_eq!(approved_request.current_approvals, 1);

    let reject_request = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();
    let reject_tool = GovernanceRejectTool::new(storage.clone(), Arc::new(GovernanceEngine::new()));
    let rejected = reject_tool
        .call(json!({
            "request_id": reject_request.id.to_string(),
            "rejector_id": rejector_id.to_string(),
            "rejector_email": "rejector@example.com",
            "reason": "rejected by separate user",
            "tenantContext": tenant_context_json(tenant_context.tenant_id.as_str(), tenant_context.user_id.as_str())
        }))
        .await
        .unwrap();

    assert_eq!(rejected["success"], true);

    let rejected_request = storage
        .get_request(reject_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rejected_request.status, RequestStatus::Rejected);
    assert_eq!(
        rejected_request.resolution_reason.as_deref(),
        Some("rejected by separate user")
    );
}

#[tokio::test]
async fn governance_integrity_keeps_requestor_denied_across_multiple_pending_requests() {
    let Some(pg_fixture) = postgres().await else {
        eprintln!("Skipping governance integrity multi-request test: Docker not available");
        return;
    };

    let postgres = Arc::new(
        storage::postgres::PostgresBackend::new(pg_fixture.url())
            .await
            .unwrap(),
    );
    postgres.initialize_schema().await.unwrap();

    let storage = Arc::new(GovernanceStorage::new(postgres.pool().clone()));
    let requestor_id = Uuid::new_v4();
    let distinct_approver_id = Uuid::new_v4();
    let tenant_id = Uuid::new_v4();
    let first_request = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();
    let second_request = storage
        .create_request(&create_test_request(requestor_id, tenant_id))
        .await
        .unwrap();

    let approve_tool =
        GovernanceApproveTool::new(storage.clone(), Arc::new(GovernanceEngine::new()));

    let self_approval_error = approve_tool
        .call(json!({
            "request_id": first_request.id.to_string(),
            "approver_id": requestor_id.to_string(),
            "approver_email": "requestor@example.com",
            "comment": "still trying to self approve",
            "tenantContext": tenant_context_json(&tenant_id.to_string(), "requestor-user")
        }))
        .await
        .unwrap_err();

    assert!(
        self_approval_error
            .to_string()
            .contains("Requestor cannot approve their own governance request")
    );

    let approved_second_request = approve_tool
        .call(json!({
            "request_id": second_request.id.to_string(),
            "approver_id": distinct_approver_id.to_string(),
            "approver_email": "approver@example.com",
            "comment": "approved by a distinct user",
            "tenantContext": tenant_context_json(&tenant_id.to_string(), "requestor-user")
        }))
        .await
        .unwrap();

    assert_eq!(approved_second_request["success"], true);

    let persisted_first_request = storage
        .get_request(first_request.id)
        .await
        .unwrap()
        .unwrap();
    let persisted_second_request = storage
        .get_request(second_request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted_first_request.status, RequestStatus::Pending);
    assert_eq!(persisted_first_request.current_approvals, 0);
    assert_eq!(persisted_second_request.status, RequestStatus::Approved);
    assert_eq!(persisted_second_request.current_approvals, 1);
}
