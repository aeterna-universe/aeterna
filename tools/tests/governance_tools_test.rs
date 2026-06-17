use chrono::{Duration, Utc};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use storage::approval_workflow::{ApprovalWorkflow, WorkflowState};
use storage::governance::{
    ApprovalDecision, ApprovalMode, ApprovalRequest, AuditFilters, CreateApprovalRequest,
    CreateDecision, CreateGovernanceRole, Decision, GovernanceAuditEntry, GovernanceConfig,
    GovernanceRole, PrincipalType, RequestFilters, RequestStatus, RequestType, RiskLevel,
};
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct MockGovernanceStorage {
    configs: Arc<RwLock<HashMap<String, GovernanceConfig>>>,
    requests: Arc<RwLock<HashMap<Uuid, ApprovalRequest>>>,
    decisions: Arc<RwLock<HashMap<Uuid, Vec<ApprovalDecision>>>>,
    roles: Arc<RwLock<Vec<GovernanceRole>>>,
    audit_logs: Arc<RwLock<Vec<GovernanceAuditEntry>>>,
    request_counter: Arc<RwLock<u64>>,
}

impl MockGovernanceStorage {
    pub fn new() -> Self {
        Self {
            configs: Arc::new(RwLock::new(HashMap::new())),
            requests: Arc::new(RwLock::new(HashMap::new())),
            decisions: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(Vec::new())),
            audit_logs: Arc::new(RwLock::new(Vec::new())),
            request_counter: Arc::new(RwLock::new(0)),
        }
    }

    fn scope_key(
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> String {
        format!(
            "{:?}:{:?}:{:?}:{:?}",
            tenant_id, org_id, team_id, project_id
        )
    }

    pub async fn get_effective_config(
        &self,
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
        project_id: Option<Uuid>,
    ) -> Result<GovernanceConfig, String> {
        let configs = self.configs.read().await;
        let key = Self::scope_key(tenant_id, org_id, team_id, project_id);

        if let Some(config) = configs.get(&key) {
            return Ok(config.clone());
        }

        Ok(GovernanceConfig::default())
    }

    pub async fn upsert_config(&self, config: &GovernanceConfig) -> Result<Uuid, String> {
        let mut configs = self.configs.write().await;
        let key = Self::scope_key(
            config.tenant_id,
            config.org_id,
            config.team_id,
            config.project_id,
        );
        let id = config.id.unwrap_or_else(Uuid::new_v4);
        let mut stored = config.clone();
        stored.id = Some(id);
        configs.insert(key, stored);
        Ok(id)
    }

    pub async fn create_request(
        &self,
        request: &CreateApprovalRequest,
    ) -> Result<ApprovalRequest, String> {
        let mut counter = self.request_counter.write().await;
        *counter += 1;
        let request_number = format!("REQ-{:06}", *counter);

        let id = Uuid::new_v4();
        let now = Utc::now();
        let expires_at = request
            .timeout_hours
            .map(|h| now + Duration::hours(h as i64));

        let approval_request = ApprovalRequest {
            id,
            request_number,
            request_type: request.request_type,
            target_type: request.target_type.clone(),
            target_id: request.target_id.clone(),
            tenant_id: request.tenant_id,
            org_id: request.org_id,
            team_id: request.team_id,
            project_id: request.project_id,
            title: request.title.clone(),
            description: request.description.clone(),
            payload: request.payload.clone(),
            risk_level: request.risk_level,
            requestor_type: request.requestor_type,
            requestor_id: request.requestor_id,
            requestor_email: request.requestor_email.clone(),
            required_approvals: request.required_approvals,
            current_approvals: 0,
            status: RequestStatus::Pending,
            created_at: now,
            updated_at: now,
            expires_at,
            resolved_at: None,
            resolution_reason: None,
            applied_at: None,
            applied_by: None,
        };

        let mut requests = self.requests.write().await;
        requests.insert(id, approval_request.clone());
        Ok(approval_request)
    }

    pub async fn get_request(&self, request_id: Uuid) -> Result<Option<ApprovalRequest>, String> {
        let requests = self.requests.read().await;
        Ok(requests.get(&request_id).cloned())
    }

    pub async fn get_request_by_number(
        &self,
        request_number: &str,
    ) -> Result<Option<ApprovalRequest>, String> {
        let requests = self.requests.read().await;
        Ok(requests
            .values()
            .find(|r| r.request_number == request_number)
            .cloned())
    }

    pub async fn list_pending_requests(
        &self,
        filters: &RequestFilters,
    ) -> Result<Vec<ApprovalRequest>, String> {
        let requests = self.requests.read().await;
        let limit = filters.limit.unwrap_or(50);

        let filtered: Vec<ApprovalRequest> = requests
            .values()
            .filter(|r| r.status == RequestStatus::Pending)
            .filter(|r| filters.tenant_id.is_none_or(|id| r.tenant_id == Some(id)))
            .filter(|r| filters.org_id.is_none_or(|id| r.org_id == Some(id)))
            .filter(|r| filters.team_id.is_none_or(|id| r.team_id == Some(id)))
            .filter(|r| filters.project_id.is_none_or(|id| r.project_id == Some(id)))
            .filter(|r| filters.requestor_id.is_none_or(|id| r.requestor_id == id))
            .filter(|r| filters.request_type.is_none_or(|t| r.request_type == t))
            .take(limit as usize)
            .cloned()
            .collect();

        Ok(filtered)
    }

    pub async fn add_decision(
        &self,
        decision: &CreateDecision,
    ) -> Result<ApprovalDecision, String> {
        let id = Uuid::new_v4();
        let approval_decision = ApprovalDecision {
            id,
            request_id: decision.request_id,
            approver_type: decision.approver_type,
            approver_id: decision.approver_id,
            approver_email: decision.approver_email.clone(),
            decision: decision.decision,
            comment: decision.comment.clone(),
            created_at: Utc::now(),
        };

        let mut decisions = self.decisions.write().await;
        decisions
            .entry(decision.request_id)
            .or_insert_with(Vec::new)
            .push(approval_decision.clone());

        if decision.decision == Decision::Approve {
            let mut requests = self.requests.write().await;
            if let Some(request) = requests.get_mut(&decision.request_id) {
                request.current_approvals += 1;
                request.updated_at = Utc::now();

                if request.current_approvals >= request.required_approvals {
                    request.status = RequestStatus::Approved;
                    request.resolved_at = Some(Utc::now());
                }
            }
        }

        Ok(approval_decision)
    }

    pub async fn reject_request(
        &self,
        request_id: Uuid,
        reason: &str,
    ) -> Result<ApprovalRequest, String> {
        let mut requests = self.requests.write().await;
        let request = requests.get_mut(&request_id).ok_or("Request not found")?;

        request.status = RequestStatus::Rejected;
        request.resolved_at = Some(Utc::now());
        request.resolution_reason = Some(reason.to_string());
        request.updated_at = Utc::now();

        Ok(request.clone())
    }

    pub async fn cancel_request(&self, request_id: Uuid) -> Result<ApprovalRequest, String> {
        let mut requests = self.requests.write().await;
        let request = requests.get_mut(&request_id).ok_or("Request not found")?;

        request.status = RequestStatus::Cancelled;
        request.resolved_at = Some(Utc::now());
        request.updated_at = Utc::now();

        Ok(request.clone())
    }

    pub async fn mark_applied(
        &self,
        request_id: Uuid,
        applied_by: Uuid,
    ) -> Result<ApprovalRequest, String> {
        let mut requests = self.requests.write().await;
        let request = requests.get_mut(&request_id).ok_or("Request not found")?;

        request.applied_at = Some(Utc::now());
        request.applied_by = Some(applied_by);
        request.updated_at = Utc::now();

        Ok(request.clone())
    }

    pub async fn get_decisions(&self, request_id: Uuid) -> Result<Vec<ApprovalDecision>, String> {
        let decisions = self.decisions.read().await;
        Ok(decisions.get(&request_id).cloned().unwrap_or_default())
    }

    pub async fn assign_role(&self, role: &CreateGovernanceRole) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let governance_role = GovernanceRole {
            id,
            principal_type: role.principal_type,
            principal_id: role.principal_id,
            role: role.role.clone(),
            tenant_id: role.tenant_id,
            org_id: role.org_id,
            team_id: role.team_id,
            project_id: role.project_id,
            granted_by: role.granted_by,
            granted_at: Utc::now(),
            expires_at: role.expires_at,
            revoked_at: None,
            revoked_by: None,
        };

        let mut roles = self.roles.write().await;
        roles.push(governance_role);
        Ok(id)
    }

    pub async fn revoke_role(
        &self,
        principal_id: Uuid,
        role: &str,
        revoked_by: Uuid,
    ) -> Result<(), String> {
        let mut roles = self.roles.write().await;
        for r in roles.iter_mut() {
            if r.principal_id == principal_id && r.role == role && r.revoked_at.is_none() {
                r.revoked_at = Some(Utc::now());
                r.revoked_by = Some(revoked_by);
                return Ok(());
            }
        }
        Err("Role not found".to_string())
    }

    pub async fn list_roles(
        &self,
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
    ) -> Result<Vec<GovernanceRole>, String> {
        let roles = self.roles.read().await;
        let filtered: Vec<GovernanceRole> = roles
            .iter()
            .filter(|r| r.revoked_at.is_none())
            .filter(|r| tenant_id.is_none_or(|id| r.tenant_id == Some(id)))
            .filter(|r| org_id.is_none_or(|id| r.org_id == Some(id)))
            .filter(|r| team_id.is_none_or(|id| r.team_id == Some(id)))
            .cloned()
            .collect();
        Ok(filtered)
    }

    pub async fn log_audit(
        &self,
        action: &str,
        request_id: Option<Uuid>,
        target_type: Option<&str>,
        target_id: Option<&str>,
        actor_type: PrincipalType,
        actor_id: Option<Uuid>,
        actor_email: Option<&str>,
        details: serde_json::Value,
    ) -> Result<Uuid, String> {
        let id = Uuid::new_v4();
        let entry = GovernanceAuditEntry {
            id,
            action: action.to_string(),
            request_id,
            target_type: target_type.map(String::from),
            target_id: target_id.map(String::from),
            actor_type,
            actor_id,
            actor_email: actor_email.map(String::from),
            details,
            old_values: None,
            new_values: None,
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
            created_at: Utc::now(),
        };

        let mut logs = self.audit_logs.write().await;
        logs.push(entry);
        Ok(id)
    }

    pub async fn list_audit_logs(
        &self,
        filters: &AuditFilters,
    ) -> Result<Vec<GovernanceAuditEntry>, String> {
        let logs = self.audit_logs.read().await;
        let limit = filters.limit.unwrap_or(50);

        let filtered: Vec<GovernanceAuditEntry> = logs
            .iter()
            .filter(|e| filters.action.as_ref().is_none_or(|a| &e.action == a))
            .filter(|e| filters.actor_id.is_none_or(|id| e.actor_id == Some(id)))
            .filter(|e| {
                filters
                    .target_type
                    .as_ref()
                    .is_none_or(|t| e.target_type.as_ref() == Some(t))
            })
            .filter(|e| e.created_at >= filters.since)
            .take(limit as usize)
            .cloned()
            .collect();

        Ok(filtered)
    }

    pub async fn check_has_role(
        &self,
        principal_id: Uuid,
        role: &str,
        tenant_id: Option<Uuid>,
        org_id: Option<Uuid>,
        team_id: Option<Uuid>,
    ) -> Result<bool, String> {
        let roles = self.roles.read().await;
        let has_role = roles.iter().any(|r| {
            r.principal_id == principal_id
                && r.role == role
                && r.revoked_at.is_none()
                && (tenant_id.is_none() || r.tenant_id == tenant_id)
                && (org_id.is_none() || r.org_id == org_id)
                && (team_id.is_none() || r.team_id == team_id)
        });
        Ok(has_role)
    }
}

impl Default for MockGovernanceStorage {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// GovernanceConfig Tests
// =============================================================================

#[cfg(test)]
mod governance_config_tests {
    use super::*;

    #[tokio::test]
    async fn test_upsert_and_get_config() {
        let storage = MockGovernanceStorage::new();
        let tenant_id = Uuid::new_v4();

        let config = GovernanceConfig {
            id: None,
            tenant_id: Some(tenant_id),
            org_id: None,
            team_id: None,
            project_id: None,
            approval_mode: ApprovalMode::Quorum,
            min_approvers: 3,
            timeout_hours: 48,
            auto_approve_low_risk: true,
            escalation_enabled: true,
            escalation_timeout_hours: 24,
            escalation_contact: Some("admin@example.com".to_string()),
            policy_settings: json!({"require_approval": true}),
            knowledge_settings: json!({}),
            memory_settings: json!({}),
        };

        let id = storage.upsert_config(&config).await.unwrap();
        assert!(!id.is_nil());

        let retrieved = storage
            .get_effective_config(Some(tenant_id), None, None, None)
            .await
            .unwrap();
        assert_eq!(retrieved.min_approvers, 3);
        assert_eq!(retrieved.approval_mode, ApprovalMode::Quorum);
        assert!(retrieved.auto_approve_low_risk);
    }

    #[tokio::test]
    async fn test_default_config_when_not_found() {
        let storage = MockGovernanceStorage::new();
        let config = storage
            .get_effective_config(Some(Uuid::new_v4()), None, None, None)
            .await
            .unwrap();

        assert_eq!(config.approval_mode, ApprovalMode::Quorum);
        assert_eq!(config.min_approvers, 2);
        assert!(!config.auto_approve_low_risk);
    }

    #[tokio::test]
    async fn test_approval_mode_parsing() {
        assert_eq!(
            "single".parse::<ApprovalMode>().unwrap(),
            ApprovalMode::Single
        );
        assert_eq!(
            "quorum".parse::<ApprovalMode>().unwrap(),
            ApprovalMode::Quorum
        );
        assert_eq!(
            "unanimous".parse::<ApprovalMode>().unwrap(),
            ApprovalMode::Unanimous
        );
        assert!("invalid".parse::<ApprovalMode>().is_err());
    }

    #[tokio::test]
    async fn test_config_update_overwrites() {
        let storage = MockGovernanceStorage::new();
        let tenant_id = Uuid::new_v4();

        // First config
        let config1 = GovernanceConfig {
            tenant_id: Some(tenant_id),
            min_approvers: 2,
            ..Default::default()
        };
        storage.upsert_config(&config1).await.unwrap();

        // Second config (update)
        let config2 = GovernanceConfig {
            tenant_id: Some(tenant_id),
            min_approvers: 5,
            ..Default::default()
        };
        storage.upsert_config(&config2).await.unwrap();

        let retrieved = storage
            .get_effective_config(Some(tenant_id), None, None, None)
            .await
            .unwrap();
        assert_eq!(retrieved.min_approvers, 5);
    }
}

// =============================================================================
// Approval Request Tests
// =============================================================================

#[cfg(test)]
mod approval_request_tests {
    use super::*;

    fn create_test_request(title: &str, requestor_id: Uuid) -> CreateApprovalRequest {
        CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "cedar_policy".to_string(),
            target_id: Some("policy-123".to_string()),
            tenant_id: Some(Uuid::new_v4()),
            org_id: None,
            team_id: None,
            project_id: None,
            title: title.to_string(),
            description: Some("Test request".to_string()),
            payload: json!({"policy": "permit(...);"}),
            risk_level: RiskLevel::Medium,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: Some("user@example.com".to_string()),
            required_approvals: 2,
            timeout_hours: Some(72),
        }
    }

    #[tokio::test]
    async fn test_create_request() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let request = create_test_request("Create security policy", requestor_id);
        let created = storage.create_request(&request).await.unwrap();

        assert!(!created.id.is_nil());
        assert!(created.request_number.starts_with("REQ-"));
        assert_eq!(created.title, "Create security policy");
        assert_eq!(created.status, RequestStatus::Pending);
        assert_eq!(created.current_approvals, 0);
        assert_eq!(created.required_approvals, 2);
        assert!(created.expires_at.is_some());
    }

    #[tokio::test]
    async fn test_get_request_by_id() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let request = create_test_request("Test policy", requestor_id);
        let created = storage.create_request(&request).await.unwrap();

        let retrieved = storage.get_request(created.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn test_get_request_by_number() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let request = create_test_request("Test policy", requestor_id);
        let created = storage.create_request(&request).await.unwrap();

        let retrieved = storage
            .get_request_by_number(&created.request_number)
            .await
            .unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().request_number, created.request_number);
    }

    #[tokio::test]
    async fn test_list_pending_requests_with_filters() {
        let storage = MockGovernanceStorage::new();
        let requestor1 = Uuid::new_v4();
        let requestor2 = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();

        // Create requests
        let mut req1 = create_test_request("Policy 1", requestor1);
        req1.tenant_id = Some(tenant_id);
        storage.create_request(&req1).await.unwrap();

        let mut req2 = create_test_request("Policy 2", requestor2);
        req2.tenant_id = Some(tenant_id);
        storage.create_request(&req2).await.unwrap();

        let req3 = create_test_request("Policy 3", requestor1);
        storage.create_request(&req3).await.unwrap();

        // Filter by tenant root
        let filters = RequestFilters {
            request_type: None,
            tenant_id: Some(tenant_id),
            org_id: None,
            team_id: None,
            project_id: None,
            requestor_id: None,
            limit: Some(10),
        };
        let results = storage.list_pending_requests(&filters).await.unwrap();
        assert_eq!(results.len(), 2);

        // Filter by requestor
        let filters = RequestFilters {
            request_type: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            requestor_id: Some(requestor1),
            limit: Some(10),
        };
        let results = storage.list_pending_requests(&filters).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_reject_request() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let request = create_test_request("Test policy", requestor_id);
        let created = storage.create_request(&request).await.unwrap();

        let rejected = storage
            .reject_request(created.id, "Does not meet security standards")
            .await
            .unwrap();

        assert_eq!(rejected.status, RequestStatus::Rejected);
        assert!(rejected.resolved_at.is_some());
        assert_eq!(
            rejected.resolution_reason,
            Some("Does not meet security standards".to_string())
        );
    }

    #[tokio::test]
    async fn test_cancel_request() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let request = create_test_request("Test policy", requestor_id);
        let created = storage.create_request(&request).await.unwrap();

        let cancelled = storage.cancel_request(created.id).await.unwrap();

        assert_eq!(cancelled.status, RequestStatus::Cancelled);
        assert!(cancelled.resolved_at.is_some());
    }

    #[tokio::test]
    async fn test_request_type_parsing() {
        assert_eq!(
            "policy".parse::<RequestType>().unwrap(),
            RequestType::Policy
        );
        assert_eq!(
            "knowledge".parse::<RequestType>().unwrap(),
            RequestType::Knowledge
        );
        assert_eq!(
            "memory".parse::<RequestType>().unwrap(),
            RequestType::Memory
        );
        assert_eq!("role".parse::<RequestType>().unwrap(), RequestType::Role);
        assert_eq!(
            "config".parse::<RequestType>().unwrap(),
            RequestType::Config
        );
        assert!("invalid".parse::<RequestType>().is_err());
    }

    #[tokio::test]
    async fn test_risk_level_parsing() {
        assert_eq!("low".parse::<RiskLevel>().unwrap(), RiskLevel::Low);
        assert_eq!("medium".parse::<RiskLevel>().unwrap(), RiskLevel::Medium);
        assert_eq!("high".parse::<RiskLevel>().unwrap(), RiskLevel::High);
        assert_eq!(
            "critical".parse::<RiskLevel>().unwrap(),
            RiskLevel::Critical
        );
        assert!("invalid".parse::<RiskLevel>().is_err());
    }
}

// =============================================================================
// Approval Decision Tests
// =============================================================================

#[cfg(test)]
mod approval_decision_tests {
    use super::*;

    #[tokio::test]
    async fn test_add_approval_decision() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();
        let approver_id = Uuid::new_v4();

        // Create request
        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Test".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 1,
            timeout_hours: None,
        };
        let created = storage.create_request(&request).await.unwrap();

        // Add approval
        let decision = CreateDecision {
            request_id: created.id,
            approver_type: PrincipalType::User,
            approver_id,
            approver_email: Some("approver@example.com".to_string()),
            decision: Decision::Approve,
            comment: Some("LGTM".to_string()),
        };
        let approval = storage.add_decision(&decision).await.unwrap();

        assert_eq!(approval.decision, Decision::Approve);
        assert_eq!(approval.comment, Some("LGTM".to_string()));

        // Request should be approved now (required: 1, current: 1)
        let updated_request = storage.get_request(created.id).await.unwrap().unwrap();
        assert_eq!(updated_request.status, RequestStatus::Approved);
        assert_eq!(updated_request.current_approvals, 1);
    }

    #[tokio::test]
    async fn test_quorum_approval() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();
        let approver1 = Uuid::new_v4();
        let approver2 = Uuid::new_v4();

        // Create request requiring 2 approvals
        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Test".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Medium,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 2,
            timeout_hours: None,
        };
        let created = storage.create_request(&request).await.unwrap();

        // First approval
        let decision1 = CreateDecision {
            request_id: created.id,
            approver_type: PrincipalType::User,
            approver_id: approver1,
            approver_email: None,
            decision: Decision::Approve,
            comment: None,
        };
        storage.add_decision(&decision1).await.unwrap();

        // Still pending
        let req = storage.get_request(created.id).await.unwrap().unwrap();
        assert_eq!(req.status, RequestStatus::Pending);
        assert_eq!(req.current_approvals, 1);

        // Second approval
        let decision2 = CreateDecision {
            request_id: created.id,
            approver_type: PrincipalType::User,
            approver_id: approver2,
            approver_email: None,
            decision: Decision::Approve,
            comment: None,
        };
        storage.add_decision(&decision2).await.unwrap();

        // Now approved
        let req = storage.get_request(created.id).await.unwrap().unwrap();
        assert_eq!(req.status, RequestStatus::Approved);
        assert_eq!(req.current_approvals, 2);
    }

    #[tokio::test]
    async fn test_get_decisions_for_request() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();
        let approver1 = Uuid::new_v4();
        let approver2 = Uuid::new_v4();

        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Test".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 2,
            timeout_hours: None,
        };
        let created = storage.create_request(&request).await.unwrap();

        // Add two decisions
        storage
            .add_decision(&CreateDecision {
                request_id: created.id,
                approver_type: PrincipalType::User,
                approver_id: approver1,
                approver_email: None,
                decision: Decision::Approve,
                comment: Some("Approved".to_string()),
            })
            .await
            .unwrap();

        storage
            .add_decision(&CreateDecision {
                request_id: created.id,
                approver_type: PrincipalType::User,
                approver_id: approver2,
                approver_email: None,
                decision: Decision::Abstain,
                comment: None,
            })
            .await
            .unwrap();

        let decisions = storage.get_decisions(created.id).await.unwrap();
        assert_eq!(decisions.len(), 2);
    }

    #[tokio::test]
    async fn test_mark_applied() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();
        let applier_id = Uuid::new_v4();

        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Test".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 1,
            timeout_hours: None,
        };
        let created = storage.create_request(&request).await.unwrap();

        // Approve
        storage
            .add_decision(&CreateDecision {
                request_id: created.id,
                approver_type: PrincipalType::User,
                approver_id: Uuid::new_v4(),
                approver_email: None,
                decision: Decision::Approve,
                comment: None,
            })
            .await
            .unwrap();

        // Mark applied
        let applied = storage.mark_applied(created.id, applier_id).await.unwrap();
        assert!(applied.applied_at.is_some());
        assert_eq!(applied.applied_by, Some(applier_id));
    }
}

// =============================================================================
// Governance Role Tests
// =============================================================================

#[cfg(test)]
mod governance_role_tests {
    use super::*;

    #[tokio::test]
    async fn test_assign_role() {
        let storage = MockGovernanceStorage::new();
        let principal_id = Uuid::new_v4();
        let granter_id = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();

        let role = CreateGovernanceRole {
            principal_type: PrincipalType::User,
            principal_id,
            role: "approver".to_string(),
            tenant_id: Some(tenant_id),
            org_id: None,
            team_id: None,
            project_id: None,
            granted_by: granter_id,
            expires_at: None,
        };

        let role_id = storage.assign_role(&role).await.unwrap();
        assert!(!role_id.is_nil());

        // Verify role exists
        let has_role = storage
            .check_has_role(principal_id, "approver", Some(tenant_id), None, None)
            .await
            .unwrap();
        assert!(has_role);
    }

    #[tokio::test]
    async fn test_revoke_role() {
        let storage = MockGovernanceStorage::new();
        let principal_id = Uuid::new_v4();
        let granter_id = Uuid::new_v4();
        let revoker_id = Uuid::new_v4();

        let role = CreateGovernanceRole {
            principal_type: PrincipalType::User,
            principal_id,
            role: "approver".to_string(),
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            granted_by: granter_id,
            expires_at: None,
        };

        storage.assign_role(&role).await.unwrap();

        // Role should exist
        let has_role = storage
            .check_has_role(principal_id, "approver", None, None, None)
            .await
            .unwrap();
        assert!(has_role);

        // Revoke
        storage
            .revoke_role(principal_id, "approver", revoker_id)
            .await
            .unwrap();

        // Role should no longer exist
        let has_role = storage
            .check_has_role(principal_id, "approver", None, None, None)
            .await
            .unwrap();
        assert!(!has_role);
    }

    #[tokio::test]
    async fn test_list_roles() {
        let storage = MockGovernanceStorage::new();
        let user1 = Uuid::new_v4();
        let user2 = Uuid::new_v4();
        let granter = Uuid::new_v4();
        let tenant_id = Uuid::new_v4();

        // Assign roles
        storage
            .assign_role(&CreateGovernanceRole {
                principal_type: PrincipalType::User,
                principal_id: user1,
                role: "approver".to_string(),
                tenant_id: Some(tenant_id),
                org_id: None,
                team_id: None,
                project_id: None,
                granted_by: granter,
                expires_at: None,
            })
            .await
            .unwrap();

        storage
            .assign_role(&CreateGovernanceRole {
                principal_type: PrincipalType::User,
                principal_id: user2,
                role: "admin".to_string(),
                tenant_id: Some(tenant_id),
                org_id: None,
                team_id: None,
                project_id: None,
                granted_by: granter,
                expires_at: None,
            })
            .await
            .unwrap();

        let roles = storage
            .list_roles(Some(tenant_id), None, None)
            .await
            .unwrap();
        assert_eq!(roles.len(), 2);
    }

    #[tokio::test]
    async fn test_principal_type_parsing() {
        assert_eq!(
            "user".parse::<PrincipalType>().unwrap(),
            PrincipalType::User
        );
        assert_eq!(
            "agent".parse::<PrincipalType>().unwrap(),
            PrincipalType::Agent
        );
        assert_eq!(
            "system".parse::<PrincipalType>().unwrap(),
            PrincipalType::System
        );
        assert!("invalid".parse::<PrincipalType>().is_err());
    }
}

// =============================================================================
// Audit Log Tests
// =============================================================================

#[cfg(test)]
mod audit_log_tests {
    use super::*;

    #[tokio::test]
    async fn test_log_audit_entry() {
        let storage = MockGovernanceStorage::new();
        let actor_id = Uuid::new_v4();

        let id = storage
            .log_audit(
                "config_updated",
                None,
                Some("governance_config"),
                Some("config-123"),
                PrincipalType::User,
                Some(actor_id),
                Some("admin@example.com"),
                json!({"changes": {"min_approvers": [2, 3]}}),
            )
            .await
            .unwrap();

        assert!(!id.is_nil());
    }

    #[tokio::test]
    async fn test_list_audit_logs_with_filters() {
        let storage = MockGovernanceStorage::new();
        let actor1 = Uuid::new_v4();
        let actor2 = Uuid::new_v4();

        // Create several audit entries
        storage
            .log_audit(
                "config_updated",
                None,
                Some("config"),
                None,
                PrincipalType::User,
                Some(actor1),
                None,
                json!({}),
            )
            .await
            .unwrap();

        storage
            .log_audit(
                "role_assigned",
                None,
                Some("role"),
                None,
                PrincipalType::User,
                Some(actor2),
                None,
                json!({}),
            )
            .await
            .unwrap();

        storage
            .log_audit(
                "request_approved",
                Some(Uuid::new_v4()),
                Some("request"),
                None,
                PrincipalType::User,
                Some(actor1),
                None,
                json!({}),
            )
            .await
            .unwrap();

        // Filter by action
        let filters = AuditFilters {
            action: Some("config_updated".to_string()),
            actor_id: None,
            target_type: None,
            since: Utc::now() - Duration::hours(1),
            limit: Some(50),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };
        let results = storage.list_audit_logs(&filters).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].action, "config_updated");

        // Filter by actor
        let filters = AuditFilters {
            action: None,
            actor_id: Some(actor1),
            target_type: None,
            since: Utc::now() - Duration::hours(1),
            limit: Some(50),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };
        let results = storage.list_audit_logs(&filters).await.unwrap();
        assert_eq!(results.len(), 2);

        // Filter by target_type
        let filters = AuditFilters {
            action: None,
            actor_id: None,
            target_type: Some("role".to_string()),
            since: Utc::now() - Duration::hours(1),
            limit: Some(50),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };
        let results = storage.list_audit_logs(&filters).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_audit_log_limit() {
        let storage = MockGovernanceStorage::new();

        // Create 5 entries
        for i in 0..5 {
            storage
                .log_audit(
                    &format!("action_{}", i),
                    None,
                    None,
                    None,
                    PrincipalType::System,
                    None,
                    None,
                    json!({}),
                )
                .await
                .unwrap();
        }

        // Limit to 3
        let filters = AuditFilters {
            action: None,
            actor_id: None,
            target_type: None,
            since: Utc::now() - Duration::hours(1),
            limit: Some(3),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };
        let results = storage.list_audit_logs(&filters).await.unwrap();
        assert_eq!(results.len(), 3);
    }
}

// =============================================================================
// Approval Workflow State Machine Tests (Integration with storage types)
// =============================================================================

#[cfg(test)]
mod workflow_integration_tests {
    use super::*;
    use storage::approval_workflow::{
        ApprovalEvent, ApprovalModeKind, ApprovalWorkflowContext, RiskLevelKind,
    };

    #[test]
    fn test_workflow_state_matches_request_status() {
        // Verify our workflow states map to storage RequestStatus
        let pending = WorkflowState::Pending {
            submitted_at: Utc::now(),
        };
        assert!(matches!(pending, WorkflowState::Pending { .. }));

        // RequestStatus::Pending corresponds to WorkflowState::Pending
        let status = RequestStatus::Pending;
        assert_eq!(status.to_string(), "pending");
    }

    #[test]
    fn test_workflow_with_mock_config() {
        // Create workflow with settings matching GovernanceConfig
        let config = GovernanceConfig {
            approval_mode: ApprovalMode::Quorum,
            min_approvers: 2,
            auto_approve_low_risk: true,
            ..Default::default()
        };

        let context = ApprovalWorkflowContext {
            request_id: Uuid::new_v4(),
            request_type: "policy".to_string(),
            required_approvals: config.min_approvers,
            current_approvals: 0,
            approval_mode: ApprovalModeKind::Quorum,
            timeout_hours: config.timeout_hours,
            auto_approve_low_risk: config.auto_approve_low_risk,
            risk_level: RiskLevelKind::Medium,
        };

        let workflow = ApprovalWorkflow::new(context);

        // Workflow starts in Draft state
        assert!(matches!(workflow.state, WorkflowState::Draft));
    }

    #[tokio::test]
    async fn test_workflow_approve_updates_storage() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();
        let approver_id = Uuid::new_v4();

        // Create request in storage
        let request = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Test".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 1,
            timeout_hours: None,
        };
        let created = storage.create_request(&request).await.unwrap();

        // Create workflow mirroring the request
        let context = ApprovalWorkflowContext {
            request_id: created.id,
            request_type: "policy".to_string(),
            required_approvals: 1,
            current_approvals: 0,
            approval_mode: ApprovalModeKind::Quorum,
            timeout_hours: 72,
            auto_approve_low_risk: false,
            risk_level: RiskLevelKind::Low,
        };
        let mut workflow = ApprovalWorkflow::new(context);

        // Submit the workflow
        workflow
            .handle(ApprovalEvent::Submit {
                requestor_id,
                submitted_at: Utc::now(),
            })
            .unwrap();

        // Approve in workflow
        workflow
            .handle(ApprovalEvent::Approve {
                approver_id,
                approved_at: Utc::now(),
                comment: Some("LGTM".to_string()),
            })
            .unwrap();

        // Workflow should be approved
        assert!(matches!(workflow.state, WorkflowState::Approved { .. }));

        // Now sync to storage
        storage
            .add_decision(&CreateDecision {
                request_id: created.id,
                approver_type: PrincipalType::User,
                approver_id,
                approver_email: None,
                decision: Decision::Approve,
                comment: Some("LGTM".to_string()),
            })
            .await
            .unwrap();

        // Storage should also be approved
        let updated = storage.get_request(created.id).await.unwrap().unwrap();
        assert_eq!(updated.status, RequestStatus::Approved);
    }

    #[test]
    fn test_workflow_auto_approve_low_risk() {
        let context = ApprovalWorkflowContext {
            request_id: Uuid::new_v4(),
            request_type: "config".to_string(),
            required_approvals: 2,
            current_approvals: 0,
            approval_mode: ApprovalModeKind::Quorum,
            timeout_hours: 48,
            auto_approve_low_risk: true,
            risk_level: RiskLevelKind::Low,
        };

        let mut workflow = ApprovalWorkflow::new(context);

        // Submit should auto-approve due to low risk + auto_approve_low_risk
        workflow
            .handle(ApprovalEvent::Submit {
                requestor_id: Uuid::new_v4(),
                submitted_at: Utc::now(),
            })
            .unwrap();

        assert!(matches!(workflow.state, WorkflowState::Approved { .. }));
    }

    #[test]
    fn test_workflow_reject() {
        let context = ApprovalWorkflowContext {
            request_id: Uuid::new_v4(),
            request_type: "policy".to_string(),
            required_approvals: 2,
            current_approvals: 0,
            approval_mode: ApprovalModeKind::Quorum,
            timeout_hours: 48,
            auto_approve_low_risk: false,
            risk_level: RiskLevelKind::High,
        };

        let mut workflow = ApprovalWorkflow::new(context);

        workflow
            .handle(ApprovalEvent::Submit {
                requestor_id: Uuid::new_v4(),
                submitted_at: Utc::now(),
            })
            .unwrap();

        workflow
            .handle(ApprovalEvent::Reject {
                rejector_id: Uuid::new_v4(),
                rejected_at: Utc::now(),
                reason: "Does not meet security standards".to_string(),
            })
            .unwrap();

        assert!(matches!(workflow.state, WorkflowState::Rejected { .. }));
    }
}

// =============================================================================
// Edge Cases and Error Handling Tests
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[tokio::test]
    async fn test_get_nonexistent_request() {
        let storage = MockGovernanceStorage::new();
        let result = storage.get_request(Uuid::new_v4()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_nonexistent_request_by_number() {
        let storage = MockGovernanceStorage::new();
        let result = storage
            .get_request_by_number("REQ-DOESNOTEXIST")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_reject_nonexistent_request() {
        let storage = MockGovernanceStorage::new();
        let result = storage.reject_request(Uuid::new_v4(), "reason").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_request() {
        let storage = MockGovernanceStorage::new();
        let result = storage.cancel_request(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_nonexistent_role() {
        let storage = MockGovernanceStorage::new();
        let result = storage
            .revoke_role(Uuid::new_v4(), "approver", Uuid::new_v4())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_empty_pending_requests() {
        let storage = MockGovernanceStorage::new();
        let filters = RequestFilters {
            request_type: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            requestor_id: None,
            limit: Some(10),
        };
        let results = storage.list_pending_requests(&filters).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_list_empty_roles() {
        let storage = MockGovernanceStorage::new();
        let results = storage.list_roles(None, None, None).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_list_empty_audit_logs() {
        let storage = MockGovernanceStorage::new();
        let filters = AuditFilters {
            action: None,
            actor_id: None,
            target_type: None,
            since: Utc::now() - Duration::hours(1),
            limit: Some(50),
            acting_as_tenant_id: None,
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };
        let results = storage.list_audit_logs(&filters).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_decisions_for_nonexistent_request() {
        let storage = MockGovernanceStorage::new();
        let results = storage.get_decisions(Uuid::new_v4()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_sequential_request_numbers() {
        let storage = MockGovernanceStorage::new();
        let requestor_id = Uuid::new_v4();

        let req1 = CreateApprovalRequest {
            request_type: RequestType::Policy,
            target_type: "policy".to_string(),
            target_id: None,
            tenant_id: None,
            org_id: None,
            team_id: None,
            project_id: None,
            title: "Request 1".to_string(),
            description: None,
            payload: json!({}),
            risk_level: RiskLevel::Low,
            requestor_type: PrincipalType::User,
            requestor_id,
            requestor_email: None,
            required_approvals: 1,
            timeout_hours: None,
        };

        let created1 = storage.create_request(&req1).await.unwrap();
        let created2 = storage.create_request(&req1).await.unwrap();
        let created3 = storage.create_request(&req1).await.unwrap();

        assert_eq!(created1.request_number, "REQ-000001");
        assert_eq!(created2.request_number, "REQ-000002");
        assert_eq!(created3.request_number, "REQ-000003");
    }
}

// =============================================================================
// Display Trait Tests
// =============================================================================

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn test_approval_mode_display() {
        assert_eq!(ApprovalMode::Single.to_string(), "single");
        assert_eq!(ApprovalMode::Quorum.to_string(), "quorum");
        assert_eq!(ApprovalMode::Unanimous.to_string(), "unanimous");
    }

    #[test]
    fn test_request_type_display() {
        assert_eq!(RequestType::Policy.to_string(), "policy");
        assert_eq!(RequestType::Knowledge.to_string(), "knowledge");
        assert_eq!(RequestType::Memory.to_string(), "memory");
        assert_eq!(RequestType::Role.to_string(), "role");
        assert_eq!(RequestType::Config.to_string(), "config");
    }

    #[test]
    fn test_risk_level_display() {
        assert_eq!(RiskLevel::Low.to_string(), "low");
        assert_eq!(RiskLevel::Medium.to_string(), "medium");
        assert_eq!(RiskLevel::High.to_string(), "high");
        assert_eq!(RiskLevel::Critical.to_string(), "critical");
    }

    #[test]
    fn test_principal_type_display() {
        assert_eq!(PrincipalType::User.to_string(), "user");
        assert_eq!(PrincipalType::Agent.to_string(), "agent");
        assert_eq!(PrincipalType::System.to_string(), "system");
    }

    #[test]
    fn test_request_status_display() {
        assert_eq!(RequestStatus::Pending.to_string(), "pending");
        assert_eq!(RequestStatus::Approved.to_string(), "approved");
        assert_eq!(RequestStatus::Rejected.to_string(), "rejected");
        assert_eq!(RequestStatus::Expired.to_string(), "expired");
        assert_eq!(RequestStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_decision_display() {
        assert_eq!(Decision::Approve.to_string(), "approve");
        assert_eq!(Decision::Reject.to_string(), "reject");
        assert_eq!(Decision::Abstain.to_string(), "abstain");
    }
}
