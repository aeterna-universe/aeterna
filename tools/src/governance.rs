use crate::tools::Tool;
use async_trait::async_trait;
use knowledge::governance::GovernanceEngine;
use knowledge::manager::KnowledgeManager;
use mk_core::types::PromotionRequestStatus;
use mk_core::types::{GovernanceEvent, OrganizationalUnit, Role, TenantContext, UnitType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use validator::Validate;

/// Tool to create a new organizational unit.
pub struct UnitCreateTool {
    backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    governance_engine: Arc<GovernanceEngine>,
}

impl UnitCreateTool {
    pub fn new(
        backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
        governance_engine: Arc<GovernanceEngine>,
    ) -> Self {
        Self {
            backend,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct UnitCreateParams {
    pub name: String,
    pub unit_type: String,
    pub parent_id: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[async_trait]
impl Tool for UnitCreateTool {
    fn name(&self) -> &str {
        "governance_unit_create"
    }

    fn description(&self) -> &str {
        "Create a new organizational unit (Organization, Team, or Project)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Name of the unit" },
                "unit_type": {
                    "type": "string",
                    "enum": ["tenant", "organization", "team", "project"],
                    "description": "Type of the unit"
                },
                "parent_id": { "type": "string", "description": "Parent unit ID" },
                "metadata": { "type": "object", "description": "Optional metadata" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["name", "unit_type"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: UnitCreateParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let unit_type = match p.unit_type.as_str() {
            "tenant" | "organization" => UnitType::Organization,
            "team" => UnitType::Team,
            "project" => UnitType::Project,
            _ => return Err("Invalid unit type".into()),
        };

        let unit = OrganizationalUnit {
            id: uuid::Uuid::new_v4().to_string(),
            name: p.name,
            unit_type,
            parent_id: p.parent_id,
            tenant_id: ctx.tenant_id.clone(),
            metadata: p.metadata,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source_owner: mk_core::types::RecordSource::Admin,
        };

        self.backend.create_unit(&unit).await?;

        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::UnitCreated {
                unit_id: unit.id.clone(),
                unit_type: unit.unit_type,
                tenant_id: ctx.tenant_id.clone(),
                parent_id: unit.parent_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true,
            "unit_id": unit.id
        }))
    }
}

/// Tool to add a policy to an organizational unit.
pub struct UnitPolicyAddTool {
    backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    governance_engine: Arc<GovernanceEngine>,
}

impl UnitPolicyAddTool {
    pub fn new(
        backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
        governance_engine: Arc<GovernanceEngine>,
    ) -> Self {
        Self {
            backend,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct UnitPolicyAddParams {
    pub unit_id: String,
    pub policy: mk_core::types::Policy,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for UnitPolicyAddTool {
    fn name(&self) -> &str {
        "governance_policy_add"
    }

    fn description(&self) -> &str {
        "Add or update a policy for an organizational unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "unit_id": { "type": "string", "description": "Unit ID to attach policy to" },
                "policy": { "type": "object", "description": "Policy definition" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["unit_id", "policy"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: UnitPolicyAddParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        self.backend
            .add_unit_policy(&ctx, &p.unit_id, &p.policy)
            .await?;

        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::PolicyUpdated {
                policy_id: p.policy.id.clone(),
                layer: p.policy.layer,
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true,
            "policy_id": p.policy.id
        }))
    }
}

/// Tool to assign a role to a user within an organizational unit.
pub struct UserRoleAssignTool {
    backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    governance_engine: Arc<GovernanceEngine>,
}

impl UserRoleAssignTool {
    pub fn new(
        backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
        governance_engine: Arc<GovernanceEngine>,
    ) -> Self {
        Self {
            backend,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct UserRoleAssignParams {
    pub user_id: String,
    pub unit_id: String,
    pub role: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for UserRoleAssignTool {
    fn name(&self) -> &str {
        "governance_role_assign"
    }

    fn description(&self) -> &str {
        "Assign a role to a user within a specific organizational unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "User ID" },
                "unit_id": { "type": "string", "description": "Unit ID" },
                "role": {
                    "type": "string",
                    "enum": ["developer", "techlead", "architect", "admin", "agent"],
                    "description": "Role to assign"
                },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["user_id", "unit_id", "role"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: UserRoleAssignParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let user_id = mk_core::types::UserId::new(p.user_id).ok_or("Invalid user ID")?;
        let role: Role = p.role.parse()?;

        self.backend
            .assign_role(&user_id, &ctx.tenant_id, &p.unit_id, role.clone().into())
            .await?;

        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::RoleAssigned {
                user_id: user_id.clone(),
                unit_id: p.unit_id.clone(),
                role: role.into(),
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true
        }))
    }
}

/// Tool to remove a role from a user within an organizational unit.
pub struct UserRoleRemoveTool {
    backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    governance_engine: Arc<GovernanceEngine>,
}

impl UserRoleRemoveTool {
    pub fn new(
        backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
        governance_engine: Arc<GovernanceEngine>,
    ) -> Self {
        Self {
            backend,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct UserRoleRemoveParams {
    pub user_id: String,
    pub unit_id: String,
    pub role: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for UserRoleRemoveTool {
    fn name(&self) -> &str {
        "governance_role_remove"
    }

    fn description(&self) -> &str {
        "Remove a role from a user within a specific organizational unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "user_id": { "type": "string", "description": "User ID" },
                "unit_id": { "type": "string", "description": "Unit ID" },
                "role": {
                    "type": "string",
                    "enum": ["developer", "techlead", "architect", "admin", "agent"],
                    "description": "Role to remove"
                },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["user_id", "unit_id", "role"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: UserRoleRemoveParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;
        let user_id = mk_core::types::UserId::new(p.user_id).ok_or("Invalid user ID")?;
        let role: Role = p.role.parse()?;

        self.backend
            .remove_role(&user_id, &ctx.tenant_id, &p.unit_id, role.clone().into())
            .await?;

        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::RoleRemoved {
                user_id: user_id.clone(),
                unit_id: p.unit_id.clone(),
                role: role.into(),
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true
        }))
    }
}

pub struct HierarchyNavigateTool {
    backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
}

impl HierarchyNavigateTool {
    pub fn new(
        backend: Arc<dyn mk_core::traits::StorageBackend<Error = storage::postgres::PostgresError>>,
    ) -> Self {
        Self { backend }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct HierarchyNavigateParams {
    pub unit_id: String,
    pub direction: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for HierarchyNavigateTool {
    fn name(&self) -> &str {
        "governance_hierarchy_navigate"
    }

    fn description(&self) -> &str {
        "Navigate the organizational hierarchy (ancestors or descendants) for a unit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "unit_id": { "type": "string", "description": "Starting Unit ID" },
                "direction": {
                    "type": "string",
                    "enum": ["ancestors", "descendants"],
                    "description": "Navigation direction"
                },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["unit_id", "direction"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: HierarchyNavigateParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let units = match p.direction.as_str() {
            "ancestors" => self.backend.get_ancestors(ctx, &p.unit_id).await?,
            "descendants" => self.backend.get_descendants(ctx, &p.unit_id).await?,
            _ => return Err("Invalid direction".into()),
        };

        Ok(json!({
            "success": true,
            "units": units
        }))
    }
}

// =============================================================================
// GOVERNANCE WORKFLOW TOOLS - Approval Request Management
// =============================================================================

use storage::governance::{
    AuditFilters, CreateApprovalRequest, CreateDecision, CreateGovernanceRole, Decision,
    GovernanceConfig, GovernanceStorage, PrincipalType, RequestFilters, RequestStatus, RiskLevel,
};

/// Tool to configure governance settings for a scope (tenant, org, team,
/// project).
pub struct GovernanceConfigureTool {
    storage: Arc<GovernanceStorage>,
    governance_engine: Arc<GovernanceEngine>,
}

impl GovernanceConfigureTool {
    pub fn new(storage: Arc<GovernanceStorage>, governance_engine: Arc<GovernanceEngine>) -> Self {
        Self {
            storage,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceConfigureParams {
    /// Tenant ID for tenant-level config
    pub tenant_id: Option<String>,
    /// Organization ID for org-level config
    pub org_id: Option<String>,
    /// Team ID for team-level config
    pub team_id: Option<String>,
    /// Project ID for project-level config
    pub project_id: Option<String>,
    /// Approval mode: single, quorum, unanimous
    pub approval_mode: Option<String>,
    /// Minimum number of approvers required
    pub min_approvers: Option<i32>,
    /// Timeout in hours before request expires
    pub timeout_hours: Option<i32>,
    /// Auto-approve low-risk requests
    pub auto_approve_low_risk: Option<bool>,
    /// Enable escalation workflow
    pub escalation_enabled: Option<bool>,
    /// Hours before escalation triggers
    pub escalation_timeout_hours: Option<i32>,
    /// Email/contact for escalations
    pub escalation_contact: Option<String>,
    /// Policy-specific settings (JSON)
    pub policy_settings: Option<Value>,
    /// Knowledge-specific settings (JSON)
    pub knowledge_settings: Option<Value>,
    /// Memory-specific settings (JSON)
    pub memory_settings: Option<Value>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceConfigureTool {
    fn name(&self) -> &str {
        "governance_configure"
    }

    fn description(&self) -> &str {
        "Configure governance settings (approval mode, thresholds, escalation) for a scope."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenant_id": { "type": "string", "description": "Tenant ID" },
                "org_id": { "type": "string", "description": "Organization ID" },
                "team_id": { "type": "string", "description": "Team ID" },
                "project_id": { "type": "string", "description": "Project ID" },
                "approval_mode": {
                    "type": "string",
                    "enum": ["single", "quorum", "unanimous"],
                    "description": "How approvals are counted"
                },
                "min_approvers": { "type": "integer", "minimum": 1, "description": "Minimum approvers required" },
                "timeout_hours": { "type": "integer", "minimum": 1, "description": "Request expiration hours" },
                "auto_approve_low_risk": { "type": "boolean", "description": "Auto-approve low-risk requests" },
                "escalation_enabled": { "type": "boolean", "description": "Enable escalation workflow" },
                "escalation_timeout_hours": { "type": "integer", "description": "Hours before escalation" },
                "escalation_contact": { "type": "string", "description": "Escalation contact email" },
                "policy_settings": { "type": "object", "description": "Policy-specific config" },
                "knowledge_settings": { "type": "object", "description": "Knowledge-specific config" },
                "memory_settings": { "type": "object", "description": "Memory-specific config" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "anyOf": [
                { "required": ["tenant_id"] },
                { "required": ["org_id"] },
                { "required": ["team_id"] },
                { "required": ["project_id"] }
            ]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceConfigureParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        // Get existing config or use defaults
        let existing = self
            .storage
            .get_effective_config(
                p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
                p.org_id.as_ref().and_then(|s| s.parse().ok()),
                p.team_id.as_ref().and_then(|s| s.parse().ok()),
                p.project_id.as_ref().and_then(|s| s.parse().ok()),
            )
            .await
            .unwrap_or_default();

        // Build new config with overrides
        let config = GovernanceConfig {
            id: existing.id,
            tenant_id: p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
            org_id: p.org_id.as_ref().and_then(|s| s.parse().ok()),
            team_id: p.team_id.as_ref().and_then(|s| s.parse().ok()),
            project_id: p.project_id.as_ref().and_then(|s| s.parse().ok()),
            approval_mode: p
                .approval_mode
                .as_ref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(existing.approval_mode),
            min_approvers: p.min_approvers.unwrap_or(existing.min_approvers),
            timeout_hours: p.timeout_hours.unwrap_or(existing.timeout_hours),
            auto_approve_low_risk: p
                .auto_approve_low_risk
                .unwrap_or(existing.auto_approve_low_risk),
            escalation_enabled: p.escalation_enabled.unwrap_or(existing.escalation_enabled),
            escalation_timeout_hours: p
                .escalation_timeout_hours
                .unwrap_or(existing.escalation_timeout_hours),
            escalation_contact: p.escalation_contact.or(existing.escalation_contact),
            policy_settings: p.policy_settings.unwrap_or(existing.policy_settings),
            knowledge_settings: p.knowledge_settings.unwrap_or(existing.knowledge_settings),
            memory_settings: p.memory_settings.unwrap_or(existing.memory_settings),
        };

        let config_id = self.storage.upsert_config(&config).await?;

        // Publish governance event
        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::ConfigUpdated {
                config_id: config_id.to_string(),
                scope: format!(
                    "tenant={:?},org={:?},team={:?},project={:?}",
                    p.tenant_id, p.org_id, p.team_id, p.project_id
                ),
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true,
            "config_id": config_id.to_string(),
            "config": config
        }))
    }
}

/// Tool to get effective governance configuration for a scope.
pub struct GovernanceConfigGetTool {
    storage: Arc<GovernanceStorage>,
}

impl GovernanceConfigGetTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceConfigGetParams {
    pub tenant_id: Option<String>,
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceConfigGetTool {
    fn name(&self) -> &str {
        "governance_config_get"
    }

    fn description(&self) -> &str {
        "Get effective governance configuration for a scope (with inheritance)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenant_id": { "type": "string" },
                "org_id": { "type": "string" },
                "team_id": { "type": "string" },
                "project_id": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceConfigGetParams = serde_json::from_value(params)?;
        p.validate()?;

        let config = self
            .storage
            .get_effective_config(
                p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
                p.org_id.as_ref().and_then(|s| s.parse().ok()),
                p.team_id.as_ref().and_then(|s| s.parse().ok()),
                p.project_id.as_ref().and_then(|s| s.parse().ok()),
            )
            .await?;

        Ok(json!({
            "success": true,
            "config": config
        }))
    }
}

/// Tool to create a new approval request.
pub struct GovernanceRequestCreateTool {
    storage: Arc<GovernanceStorage>,
    governance_engine: Arc<GovernanceEngine>,
}

impl GovernanceRequestCreateTool {
    pub fn new(storage: Arc<GovernanceStorage>, governance_engine: Arc<GovernanceEngine>) -> Self {
        Self {
            storage,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRequestCreateParams {
    /// Type of request: policy, knowledge, memory, role, config
    pub request_type: String,
    /// Target type being modified (e.g., "policy", "adr", "memory")
    pub target_type: String,
    /// Optional target ID
    pub target_id: Option<String>,
    /// Scope IDs
    pub tenant_id: Option<String>,
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    /// Human-readable title
    pub title: String,
    /// Description of the change
    pub description: Option<String>,
    /// The actual payload/change data
    pub payload: Value,
    /// Risk level: low, medium, high, critical
    pub risk_level: Option<String>,
    /// Requestor info
    pub requestor_id: String,
    pub requestor_email: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRequestCreateTool {
    fn name(&self) -> &str {
        "governance_request_create"
    }

    fn description(&self) -> &str {
        "Create a new approval request for a governance action (policy change, knowledge update, \
         etc.)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_type": {
                    "type": "string",
                    "enum": ["policy", "knowledge", "memory", "role", "config"],
                    "description": "Type of approval request"
                },
                "target_type": { "type": "string", "description": "What is being changed" },
                "target_id": { "type": "string", "description": "ID of target being changed" },
                "tenant_id": { "type": "string" },
                "org_id": { "type": "string" },
                "team_id": { "type": "string" },
                "project_id": { "type": "string" },
                "title": { "type": "string", "description": "Human-readable title" },
                "description": { "type": "string", "description": "Change description" },
                "payload": { "type": "object", "description": "The change payload" },
                "risk_level": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "critical"],
                    "default": "medium"
                },
                "requestor_id": { "type": "string", "description": "UUID of requestor" },
                "requestor_email": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["request_type", "target_type", "title", "payload", "requestor_id"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRequestCreateParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        // Get effective config to determine required approvals
        let config = self
            .storage
            .get_effective_config(
                p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
                p.org_id.as_ref().and_then(|s| s.parse().ok()),
                p.team_id.as_ref().and_then(|s| s.parse().ok()),
                p.project_id.as_ref().and_then(|s| s.parse().ok()),
            )
            .await
            .unwrap_or_default();

        let risk_level: RiskLevel = p
            .risk_level
            .as_ref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default();

        // Check for auto-approval
        if config.auto_approve_low_risk && risk_level == RiskLevel::Low {
            // Create request as already approved
            let request = CreateApprovalRequest {
                request_type: p.request_type.parse().map_err(|e: String| e)?,
                target_type: p.target_type,
                target_id: p.target_id,
                tenant_id: p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
                org_id: p.org_id.as_ref().and_then(|s| s.parse().ok()),
                team_id: p.team_id.as_ref().and_then(|s| s.parse().ok()),
                project_id: p.project_id.as_ref().and_then(|s| s.parse().ok()),
                title: p.title,
                description: p.description,
                payload: p.payload,
                risk_level,
                requestor_type: PrincipalType::User,
                requestor_id: p.requestor_id.parse()?,
                requestor_email: p.requestor_email,
                required_approvals: 0, // Auto-approved
                timeout_hours: Some(config.timeout_hours),
            };

            let created = self.storage.create_request(&request).await?;

            return Ok(json!({
                "success": true,
                "auto_approved": true,
                "request": created,
                "message": "Low-risk request auto-approved per governance config"
            }));
        }

        let request = CreateApprovalRequest {
            request_type: p.request_type.parse().map_err(|e: String| e)?,
            target_type: p.target_type,
            target_id: p.target_id,
            tenant_id: p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
            org_id: p.org_id.as_ref().and_then(|s| s.parse().ok()),
            team_id: p.team_id.as_ref().and_then(|s| s.parse().ok()),
            project_id: p.project_id.as_ref().and_then(|s| s.parse().ok()),
            title: p.title.clone(),
            description: p.description,
            payload: p.payload,
            risk_level,
            requestor_type: PrincipalType::User,
            requestor_id: p.requestor_id.parse()?,
            requestor_email: p.requestor_email,
            required_approvals: config.min_approvers,
            timeout_hours: Some(config.timeout_hours),
        };

        let created = self.storage.create_request(&request).await?;

        // Publish event
        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::RequestCreated {
                request_id: created.id.to_string(),
                request_type: created.request_type.to_string(),
                title: p.title,
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        Ok(json!({
            "success": true,
            "auto_approved": false,
            "request": created
        }))
    }
}

/// Tool to approve an approval request.
pub struct GovernanceApproveTool {
    storage: Arc<GovernanceStorage>,
    governance_engine: Arc<GovernanceEngine>,
}

impl GovernanceApproveTool {
    pub fn new(storage: Arc<GovernanceStorage>, governance_engine: Arc<GovernanceEngine>) -> Self {
        Self {
            storage,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceApproveParams {
    /// Request ID or request number (e.g., "REQ-000001")
    pub request_id: String,
    /// Approver info
    pub approver_id: String,
    pub approver_email: Option<String>,
    /// Optional comment
    pub comment: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceApproveTool {
    fn name(&self) -> &str {
        "governance_approve"
    }

    fn description(&self) -> &str {
        "Approve a pending governance request."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_id": { "type": "string", "description": "Request ID or number" },
                "approver_id": { "type": "string", "description": "Approver UUID" },
                "approver_email": { "type": "string" },
                "comment": { "type": "string", "description": "Approval comment" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["request_id", "approver_id"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceApproveParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        // Find request by ID or number
        let request = if p.request_id.starts_with("REQ-") {
            self.storage
                .get_request_by_number(&p.request_id)
                .await?
                .ok_or("Request not found")?
        } else {
            self.storage
                .get_request(p.request_id.parse()?)
                .await?
                .ok_or("Request not found")?
        };

        // Validate request is pending
        if request.status != RequestStatus::Pending {
            return Err(
                format!("Request is not pending, current status: {}", request.status).into(),
            );
        }

        // Add approval decision
        let decision = CreateDecision {
            request_id: request.id,
            approver_type: PrincipalType::User,
            approver_id: p.approver_id.parse()?,
            approver_email: p.approver_email,
            decision: Decision::Approve,
            comment: p.comment,
        };

        let approval = self.storage.add_decision(&decision).await?;

        // Get updated request to check if fully approved
        let updated_request = self
            .storage
            .get_request(request.id)
            .await?
            .ok_or("Request not found after approval")?;

        let fully_approved =
            updated_request.current_approvals >= updated_request.required_approvals;

        // Publish event
        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::RequestApproved {
                request_id: request.id.to_string(),
                approver_id: p.approver_id,
                fully_approved,
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        // Log audit
        let _ = self
            .storage
            .log_audit(
                "approve",
                Some(request.id),
                Some(&request.target_type),
                request.target_id.as_deref(),
                PrincipalType::User,
                Some(decision.approver_id),
                decision.approver_email.as_deref(),
                json!({
                    "decision": "approve",
                    "comment": decision.comment,
                    "current_approvals": updated_request.current_approvals,
                    "required_approvals": updated_request.required_approvals
                }),
                // #44.d §2.5 — tools-layer governance is instance-scoped:
                // there is no request-scoped TenantContext here. Platform-
                // scope attribution (NULL) is the correct semantic.
                None,
            )
            .await;

        Ok(json!({
            "success": true,
            "approval": approval,
            "request": updated_request,
            "fully_approved": fully_approved,
            "remaining_approvals": std::cmp::max(0, updated_request.required_approvals - updated_request.current_approvals)
        }))
    }
}

/// Tool to reject an approval request.
pub struct GovernanceRejectTool {
    storage: Arc<GovernanceStorage>,
    governance_engine: Arc<GovernanceEngine>,
}

impl GovernanceRejectTool {
    pub fn new(storage: Arc<GovernanceStorage>, governance_engine: Arc<GovernanceEngine>) -> Self {
        Self {
            storage,
            governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRejectParams {
    pub request_id: String,
    pub rejector_id: String,
    pub rejector_email: Option<String>,
    /// Reason for rejection (required)
    #[validate(length(min = 1))]
    pub reason: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRejectTool {
    fn name(&self) -> &str {
        "governance_reject"
    }

    fn description(&self) -> &str {
        "Reject a pending governance request."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_id": { "type": "string", "description": "Request ID or number" },
                "rejector_id": { "type": "string", "description": "Rejector UUID" },
                "rejector_email": { "type": "string" },
                "reason": { "type": "string", "minLength": 1, "description": "Rejection reason" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["request_id", "rejector_id", "reason"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRejectParams = serde_json::from_value(params)?;
        p.validate()?;

        let ctx = p.tenant_context.ok_or("Missing tenant context")?;

        // Find request
        let request = if p.request_id.starts_with("REQ-") {
            self.storage
                .get_request_by_number(&p.request_id)
                .await?
                .ok_or("Request not found")?
        } else {
            self.storage
                .get_request(p.request_id.parse()?)
                .await?
                .ok_or("Request not found")?
        };

        if request.status != RequestStatus::Pending {
            return Err(
                format!("Request is not pending, current status: {}", request.status).into(),
            );
        }

        // Add rejection decision
        let decision = CreateDecision {
            request_id: request.id,
            approver_type: PrincipalType::User,
            approver_id: p.rejector_id.parse()?,
            approver_email: p.rejector_email.clone(),
            decision: Decision::Reject,
            comment: Some(p.reason.clone()),
        };

        let rejection = self.storage.add_decision(&decision).await?;

        // Mark request as rejected
        let rejected_request = self.storage.reject_request(request.id, &p.reason).await?;

        // Publish event
        let _ = self
            .governance_engine
            .publish_event(GovernanceEvent::RequestRejected {
                request_id: request.id.to_string(),
                rejector_id: p.rejector_id.clone(),
                reason: p.reason.clone(),
                tenant_id: ctx.tenant_id.clone(),
                timestamp: chrono::Utc::now().timestamp(),
            })
            .await;

        // Log audit
        let _ = self
            .storage
            .log_audit(
                "reject",
                Some(request.id),
                Some(&request.target_type),
                request.target_id.as_deref(),
                PrincipalType::User,
                Some(decision.approver_id),
                p.rejector_email.as_deref(),
                json!({
                    "decision": "reject",
                    "reason": p.reason
                }),
                None, // #44.d §2.5 — tools-layer (see approve path).
            )
            .await;

        Ok(json!({
            "success": true,
            "rejection": rejection,
            "request": rejected_request
        }))
    }
}

/// Tool to list pending approval requests.
pub struct GovernanceRequestListTool {
    storage: Arc<GovernanceStorage>,
    knowledge_manager: Option<Arc<KnowledgeManager>>,
}

impl GovernanceRequestListTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self {
            storage,
            knowledge_manager: None,
        }
    }

    pub fn with_knowledge_manager(mut self, knowledge_manager: Arc<KnowledgeManager>) -> Self {
        self.knowledge_manager = Some(knowledge_manager);
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRequestListParams {
    pub request_type: Option<String>,
    pub tenant_id: Option<String>,
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    pub requestor_id: Option<String>,
    pub limit: Option<i32>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRequestListTool {
    fn name(&self) -> &str {
        "governance_request_list"
    }

    fn description(&self) -> &str {
        "List pending approval requests with optional filters."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_type": {
                    "type": "string",
                    "enum": ["policy", "knowledge", "memory", "role", "config"]
                },
                "tenant_id": { "type": "string" },
                "org_id": { "type": "string" },
                "team_id": { "type": "string" },
                "project_id": { "type": "string" },
                "requestor_id": { "type": "string" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 50 },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRequestListParams = serde_json::from_value(params)?;
        p.validate()?;

        let filters = RequestFilters {
            request_type: p.request_type.as_ref().and_then(|s| s.parse().ok()),
            tenant_id: p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
            org_id: p.org_id.as_ref().and_then(|s| s.parse().ok()),
            team_id: p.team_id.as_ref().and_then(|s| s.parse().ok()),
            project_id: p.project_id.as_ref().and_then(|s| s.parse().ok()),
            requestor_id: p.requestor_id.as_ref().and_then(|s| s.parse().ok()),
            limit: p.limit,
        };

        let requests = self.storage.list_pending_requests(&filters).await?;

        let promotion_summary = if let (Some(manager), Some(ctx)) =
            (self.knowledge_manager.as_ref(), p.tenant_context)
        {
            let pending = manager
                .list_promotion_requests(ctx.clone(), Some(PromotionRequestStatus::PendingReview))
                .await?
                .len();
            let approved = manager
                .list_promotion_requests(ctx.clone(), Some(PromotionRequestStatus::Approved))
                .await?
                .len();
            let rejected = manager
                .list_promotion_requests(ctx, Some(PromotionRequestStatus::Rejected))
                .await?
                .len();
            Some(json!({
                "pendingCount": pending,
                "approvedCount": approved,
                "rejectedCount": rejected,
            }))
        } else {
            None
        };

        Ok(json!({
            "success": true,
            "count": requests.len(),
            "requests": requests,
            "promotionSummary": promotion_summary
        }))
    }
}

/// Tool to get details of a specific request including decisions.
pub struct GovernanceRequestGetTool {
    storage: Arc<GovernanceStorage>,
}

impl GovernanceRequestGetTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRequestGetParams {
    pub request_id: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRequestGetTool {
    fn name(&self) -> &str {
        "governance_request_get"
    }

    fn description(&self) -> &str {
        "Get detailed information about a specific approval request including all decisions."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "request_id": { "type": "string", "description": "Request ID or number" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["request_id"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRequestGetParams = serde_json::from_value(params)?;
        p.validate()?;

        // Find request
        let request = if p.request_id.starts_with("REQ-") {
            self.storage
                .get_request_by_number(&p.request_id)
                .await?
                .ok_or("Request not found")?
        } else {
            self.storage
                .get_request(p.request_id.parse()?)
                .await?
                .ok_or("Request not found")?
        };

        // Get all decisions
        let decisions = self.storage.get_decisions(request.id).await?;

        Ok(json!({
            "success": true,
            "request": request,
            "decisions": decisions,
            "approval_progress": {
                "current": request.current_approvals,
                "required": request.required_approvals,
                "remaining": std::cmp::max(0, request.required_approvals - request.current_approvals)
            }
        }))
    }
}

/// Tool to list audit log entries.
pub struct GovernanceAuditListTool {
    storage: Arc<GovernanceStorage>,
}

impl GovernanceAuditListTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceAuditListParams {
    /// Filter by action type
    pub action: Option<String>,
    /// Filter by actor
    pub actor_id: Option<String>,
    /// Filter by target type
    pub target_type: Option<String>,
    /// How many days back to search (default 30)
    pub days_back: Option<i64>,
    /// Max results
    pub limit: Option<i32>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceAuditListTool {
    fn name(&self) -> &str {
        "governance_audit_list"
    }

    fn description(&self) -> &str {
        "List governance audit log entries with filters."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "description": "Filter by action (approve, reject, etc.)" },
                "actor_id": { "type": "string", "description": "Filter by actor UUID" },
                "target_type": { "type": "string", "description": "Filter by target type" },
                "days_back": { "type": "integer", "minimum": 1, "maximum": 365, "default": 30 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceAuditListParams = serde_json::from_value(params)?;
        p.validate()?;

        let days = p.days_back.unwrap_or(30);
        let since = chrono::Utc::now() - chrono::Duration::days(days);

        let filters = AuditFilters {
            action: p.action,
            actor_id: p.actor_id.as_ref().and_then(|s| s.parse().ok()),
            target_type: p.target_type,
            since,
            limit: p.limit,
            // #44.d §2.5 — tools-layer audit queries are instance-scoped:
            // no request-time tenant filter applied here.
            acting_as_tenant_id: None,
            // B2 §11.1 — MCP tool callers don't surface the new filter
            // dimensions today; leaving them None preserves pre-migration
            // list behaviour. Expose via dedicated params once the tool
            // contract explicitly calls for it.
            via: None,
            client_version: None,
            manifest_hash: None,
            generation: None,
            dry_run: None,
        };

        let entries = self.storage.list_audit_logs(&filters).await?;

        Ok(json!({
            "success": true,
            "count": entries.len(),
            "entries": entries
        }))
    }
}

/// Tool to assign a governance role.
pub struct GovernanceRoleAssignTool {
    storage: Arc<GovernanceStorage>,
    _governance_engine: Arc<GovernanceEngine>,
}

impl GovernanceRoleAssignTool {
    pub fn new(storage: Arc<GovernanceStorage>, governance_engine: Arc<GovernanceEngine>) -> Self {
        Self {
            storage,
            _governance_engine: governance_engine,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRoleAssignParams {
    /// Principal type: user, agent, system
    pub principal_type: String,
    /// Principal UUID
    pub principal_id: String,
    /// Role name (e.g., "approver", "admin", "auditor")
    pub role: String,
    /// Scope
    pub tenant_id: Option<String>,
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    pub project_id: Option<String>,
    /// Who is granting this role
    pub granted_by: String,
    /// Optional expiration
    pub expires_in_days: Option<i64>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRoleAssignTool {
    fn name(&self) -> &str {
        "governance_principal_role_assign"
    }

    fn description(&self) -> &str {
        "Assign a governance role (approver, admin, auditor) to a user or agent."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "principal_type": {
                    "type": "string",
                    "enum": ["user", "agent", "system"]
                },
                "principal_id": { "type": "string", "description": "UUID of principal" },
                "role": {
                    "type": "string",
                    "enum": ["approver", "admin", "auditor", "policy_admin", "knowledge_admin"],
                    "description": "Governance role"
                },
                "tenant_id": { "type": "string" },
                "org_id": { "type": "string" },
                "team_id": { "type": "string" },
                "project_id": { "type": "string" },
                "granted_by": { "type": "string", "description": "UUID of granter" },
                "expires_in_days": { "type": "integer", "description": "Optional expiration in days" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["principal_type", "principal_id", "role", "granted_by"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRoleAssignParams = serde_json::from_value(params)?;
        p.validate()?;

        let _ctx = p.tenant_context.ok_or("Missing tenant context")?;

        let expires_at = p
            .expires_in_days
            .map(|d| chrono::Utc::now() + chrono::Duration::days(d));

        let role = CreateGovernanceRole {
            principal_type: p.principal_type.parse().map_err(|e: String| e)?,
            principal_id: p.principal_id.parse()?,
            role: p.role.clone(),
            tenant_id: p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
            org_id: p.org_id.as_ref().and_then(|s| s.parse().ok()),
            team_id: p.team_id.as_ref().and_then(|s| s.parse().ok()),
            project_id: p.project_id.as_ref().and_then(|s| s.parse().ok()),
            granted_by: p.granted_by.parse()?,
            expires_at,
        };

        let role_id = self.storage.assign_role(&role).await?;

        // Log audit
        let _ = self
            .storage
            .log_audit(
                "role_assigned",
                None,
                Some("role"),
                Some(&role_id.to_string()),
                PrincipalType::User,
                Some(role.granted_by),
                None,
                json!({
                    "role": p.role,
                    "principal_type": p.principal_type,
                    "principal_id": p.principal_id
                }),
                None, // #44.d §2.5 — tools-layer (see approve path).
            )
            .await;

        Ok(json!({
            "success": true,
            "role_id": role_id.to_string()
        }))
    }
}

/// Tool to revoke a governance role.
pub struct GovernanceRoleRevokeTool {
    storage: Arc<GovernanceStorage>,
}

impl GovernanceRoleRevokeTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRoleRevokeParams {
    pub principal_id: String,
    pub role: String,
    pub revoked_by: String,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRoleRevokeTool {
    fn name(&self) -> &str {
        "governance_role_revoke"
    }

    fn description(&self) -> &str {
        "Revoke a governance role from a user or agent."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "principal_id": { "type": "string", "description": "UUID of principal" },
                "role": { "type": "string", "description": "Role to revoke" },
                "revoked_by": { "type": "string", "description": "UUID of revoker" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            },
            "required": ["principal_id", "role", "revoked_by"]
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRoleRevokeParams = serde_json::from_value(params)?;
        p.validate()?;

        let _ctx = p.tenant_context.ok_or("Missing tenant context")?;

        self.storage
            .revoke_role(p.principal_id.parse()?, &p.role, p.revoked_by.parse()?)
            .await?;

        // Log audit
        let _ = self
            .storage
            .log_audit(
                "role_revoked",
                None,
                Some("role"),
                None,
                PrincipalType::User,
                Some(p.revoked_by.parse()?),
                None,
                json!({
                    "role": p.role,
                    "principal_id": p.principal_id
                }),
                None, // #44.d §2.5 — tools-layer (see approve path).
            )
            .await;

        Ok(json!({
            "success": true
        }))
    }
}

/// Tool to list governance roles.
pub struct GovernanceRoleListTool {
    storage: Arc<GovernanceStorage>,
}

impl GovernanceRoleListTool {
    pub fn new(storage: Arc<GovernanceStorage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Validate)]
pub struct GovernanceRoleListParams {
    pub tenant_id: Option<String>,
    pub org_id: Option<String>,
    pub team_id: Option<String>,
    #[serde(rename = "tenantContext")]
    pub tenant_context: Option<TenantContext>,
}

#[async_trait]
impl Tool for GovernanceRoleListTool {
    fn name(&self) -> &str {
        "governance_role_list"
    }

    fn description(&self) -> &str {
        "List governance roles for a scope."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tenant_id": { "type": "string" },
                "org_id": { "type": "string" },
                "team_id": { "type": "string" },
                "tenantContext": { "$ref": "#/definitions/TenantContext" }
            }
        })
    }

    async fn call(&self, params: Value) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        let p: GovernanceRoleListParams = serde_json::from_value(params)?;
        p.validate()?;

        let roles = self
            .storage
            .list_roles(
                p.tenant_id.as_ref().and_then(|s| s.parse().ok()),
                p.org_id.as_ref().and_then(|s| s.parse().ok()),
                p.team_id.as_ref().and_then(|s| s.parse().ok()),
            )
            .await?;

        Ok(json!({
            "success": true,
            "count": roles.len(),
            "roles": roles
        }))
    }
}

// =============================================================================
// Unit tests for governance tools
// =============================================================================
//
// These tests cover:
//   1. Pure methods (`name`, `description`, `input_schema`) — no I/O.
//   2. Full `call()` round-trips for the 5 StorageBackend-backed tools using
//      MockStorageBackend so no Postgres instance is required.
//
// The GovernanceStorage-backed tools (GovernanceConfigure*, GovernanceRequest*,
// etc.) require a live PgPool and are tested separately as integration tests.

#[cfg(test)]
mod tests {
    use super::*;
    use mk_core::traits::StorageBackend;
    use mk_core::types::RecordSource;
    use std::sync::Arc;
    use testing::mock_storage::MockStorageBackend;

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    fn make_engine() -> Arc<GovernanceEngine> {
        Arc::new(GovernanceEngine::new())
    }

    fn make_backend() -> Arc<MockStorageBackend> {
        MockStorageBackend::arc()
    }

    fn tenant_ctx_json(tenant: &str, user: &str) -> Value {
        serde_json::json!({
            "tenant_id": tenant,
            "user_id": user
        })
    }

    // -------------------------------------------------------------------------
    // UnitCreateTool
    // -------------------------------------------------------------------------

    #[test]
    fn test_unit_create_tool_pure_methods() {
        let tool = UnitCreateTool::new(make_backend(), make_engine());
        assert_eq!(tool.name(), "governance_unit_create");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "name"));
        assert!(required.iter().any(|v| v == "unit_type"));
    }

    #[tokio::test]
    async fn test_unit_create_tool_call_succeeds() {
        let backend = make_backend();
        let tool = UnitCreateTool::new(backend.clone(), make_engine());

        let result = tool
            .call(serde_json::json!({
                "name": "Acme Platform",
                "unit_type": "tenant",
                "tenantContext": tenant_ctx_json("acme", "user-1")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        let unit_id = result["unit_id"].as_str().unwrap();
        assert!(!unit_id.is_empty());

        // Confirm the unit was actually stored
        let units = backend.list_all_units().await.unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].name, "Acme Platform");
    }

    #[tokio::test]
    async fn test_unit_create_tool_with_parent_id() {
        let backend = make_backend();
        let tool = UnitCreateTool::new(backend.clone(), make_engine());

        // Create parent first
        tool.call(serde_json::json!({
            "name": "Parent Corp",
            "unit_type": "tenant",
            "tenantContext": tenant_ctx_json("t", "u")
        }))
        .await
        .unwrap();

        let parent_id = backend.list_all_units().await.unwrap()[0].id.clone();

        // Create child referencing parent
        let result = tool
            .call(serde_json::json!({
                "name": "Platform Eng",
                "unit_type": "organization",
                "parent_id": parent_id,
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        let units = backend.list_all_units().await.unwrap();
        assert_eq!(units.len(), 2);
        let child = units.iter().find(|u| u.name == "Platform Eng").unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(parent_id.as_str()));
    }

    #[tokio::test]
    async fn test_unit_create_tool_invalid_unit_type_returns_error() {
        let tool = UnitCreateTool::new(make_backend(), make_engine());
        let err = tool
            .call(serde_json::json!({
                "name": "X",
                "unit_type": "INVALID_TYPE",
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_unit_create_tool_missing_tenant_context_returns_error() {
        let tool = UnitCreateTool::new(make_backend(), make_engine());
        let err = tool
            .call(serde_json::json!({
                "name": "X",
                "unit_type": "team"
                // no tenantContext
            }))
            .await;
        assert!(err.is_err());
    }

    // -------------------------------------------------------------------------
    // UnitPolicyAddTool
    // -------------------------------------------------------------------------

    #[test]
    fn test_unit_policy_add_tool_pure_methods() {
        let tool = UnitPolicyAddTool::new(make_backend(), make_engine());
        assert_eq!(tool.name(), "governance_policy_add");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "unit_id"));
        assert!(required.iter().any(|v| v == "policy"));
    }

    #[tokio::test]
    async fn test_unit_policy_add_tool_call_succeeds() {
        let backend = make_backend();
        let tool = UnitPolicyAddTool::new(backend.clone(), make_engine());

        let result = tool
            .call(serde_json::json!({
                "unit_id": "unit-abc",
                "policy": {
                    "id": "pol-1",
                    "name": "Test Policy",
                    "layer": "Team",
                    "mode": "Mandatory",
                    "merge_strategy": "Merge",
                    "rules": [],
                    "metadata": {}
                },
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert_eq!(result["policy_id"], "pol-1");

        // Confirm stored
        let ctx = mk_core::types::TenantContext {
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            user_id: mk_core::types::UserId::new("u".to_string()).unwrap(),
            agent_id: None,
            roles: vec![],
            target_tenant_id: None,
        };
        let stored = backend.get_unit_policies(ctx, "unit-abc").await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "pol-1");
    }

    // -------------------------------------------------------------------------
    // UserRoleAssignTool
    // -------------------------------------------------------------------------

    #[test]
    fn test_user_role_assign_tool_pure_methods() {
        let tool = UserRoleAssignTool::new(make_backend(), make_engine());
        assert_eq!(tool.name(), "governance_role_assign");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "user_id"));
        assert!(required.iter().any(|v| v == "unit_id"));
        assert!(required.iter().any(|v| v == "role"));
    }

    #[tokio::test]
    async fn test_user_role_assign_tool_call_succeeds() {
        let backend = make_backend();
        let tool = UserRoleAssignTool::new(backend.clone(), make_engine());

        let result = tool
            .call(serde_json::json!({
                "user_id": "alice",
                "unit_id": "unit-x",
                "role": "developer",
                "tenantContext": tenant_ctx_json("t1", "alice")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);

        // Confirm role stored
        let key = "alice:t1:unit-x";
        let roles = backend.roles.get(key).unwrap();
        assert!(roles.contains(&mk_core::types::Role::Developer.into()));
    }

    #[tokio::test]
    async fn test_user_role_assign_tool_invalid_role_returns_error() {
        let tool = UserRoleAssignTool::new(make_backend(), make_engine());
        let err = tool
            .call(serde_json::json!({
                "user_id": "alice",
                "unit_id": "unit-x",
                "role": "SUPER_ADMIN",
                "tenantContext": tenant_ctx_json("t1", "alice")
            }))
            .await;
        assert!(err.is_err());
    }

    // -------------------------------------------------------------------------
    // UserRoleRemoveTool
    // -------------------------------------------------------------------------

    #[test]
    fn test_user_role_remove_tool_pure_methods() {
        let tool = UserRoleRemoveTool::new(make_backend(), make_engine());
        assert_eq!(tool.name(), "governance_role_remove");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "user_id"));
        assert!(required.iter().any(|v| v == "role"));
    }

    #[tokio::test]
    async fn test_user_role_remove_tool_call_removes_role() {
        let backend = make_backend();
        let assign_tool = UserRoleAssignTool::new(backend.clone(), make_engine());
        let remove_tool = UserRoleRemoveTool::new(backend.clone(), make_engine());

        // First assign
        assign_tool
            .call(serde_json::json!({
                "user_id": "bob",
                "unit_id": "unit-y",
                "role": "techlead",
                "tenantContext": tenant_ctx_json("t1", "bob")
            }))
            .await
            .unwrap();

        let key = "bob:t1:unit-y";
        assert!(
            backend
                .roles
                .get(key)
                .unwrap()
                .contains(&mk_core::types::Role::TechLead.into())
        );

        // Then remove
        let result = remove_tool
            .call(serde_json::json!({
                "user_id": "bob",
                "unit_id": "unit-y",
                "role": "techlead",
                "tenantContext": tenant_ctx_json("t1", "bob")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        assert!(backend.roles.get(key).unwrap().is_empty());
    }

    // -------------------------------------------------------------------------
    // HierarchyNavigateTool
    // -------------------------------------------------------------------------

    #[test]
    fn test_hierarchy_navigate_tool_pure_methods() {
        let tool = HierarchyNavigateTool::new(make_backend());
        assert_eq!(tool.name(), "governance_hierarchy_navigate");
        assert!(!tool.description().is_empty());
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "unit_id"));
        assert!(required.iter().any(|v| v == "direction"));
        // Verify direction enum values
        let direction_enum = schema["properties"]["direction"]["enum"]
            .as_array()
            .unwrap();
        assert!(direction_enum.iter().any(|v| v == "ancestors"));
        assert!(direction_enum.iter().any(|v| v == "descendants"));
    }

    #[tokio::test]
    async fn test_hierarchy_navigate_tool_ancestors() {
        let backend = make_backend();

        // Seed: gp -> parent -> child
        let gp = mk_core::types::OrganizationalUnit {
            id: "gp".to_string(),
            name: "GrandParent".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: None,
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        let parent = mk_core::types::OrganizationalUnit {
            id: "parent".to_string(),
            name: "Parent".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: Some("gp".to_string()),
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        let child = mk_core::types::OrganizationalUnit {
            id: "child".to_string(),
            name: "Child".to_string(),
            unit_type: mk_core::types::UnitType::Team,
            parent_id: Some("parent".to_string()),
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        backend.create_unit(&gp).await.unwrap();
        backend.create_unit(&parent).await.unwrap();
        backend.create_unit(&child).await.unwrap();

        let tool = HierarchyNavigateTool::new(backend.clone());
        let result = tool
            .call(serde_json::json!({
                "unit_id": "child",
                "direction": "ancestors",
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        let units = result["units"].as_array().unwrap();
        let ids: Vec<&str> = units.iter().map(|u| u["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"child"));
        assert!(ids.contains(&"parent"));
        assert!(ids.contains(&"gp"));
    }

    #[tokio::test]
    async fn test_hierarchy_navigate_tool_descendants() {
        let backend = make_backend();

        let root = mk_core::types::OrganizationalUnit {
            id: "root".to_string(),
            name: "Root".to_string(),
            unit_type: mk_core::types::UnitType::Organization,
            parent_id: None,
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        let child1 = mk_core::types::OrganizationalUnit {
            id: "c1".to_string(),
            name: "Child1".to_string(),
            unit_type: mk_core::types::UnitType::Team,
            parent_id: Some("root".to_string()),
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        let child2 = mk_core::types::OrganizationalUnit {
            id: "c2".to_string(),
            name: "Child2".to_string(),
            unit_type: mk_core::types::UnitType::Team,
            parent_id: Some("root".to_string()),
            tenant_id: mk_core::types::TenantId::new("t".to_string()).unwrap(),
            metadata: std::collections::HashMap::new(),
            created_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap(),
            source_owner: RecordSource::Admin,
        };
        backend.create_unit(&root).await.unwrap();
        backend.create_unit(&child1).await.unwrap();
        backend.create_unit(&child2).await.unwrap();

        let tool = HierarchyNavigateTool::new(backend.clone());
        let result = tool
            .call(serde_json::json!({
                "unit_id": "root",
                "direction": "descendants",
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await
            .unwrap();

        assert_eq!(result["success"], true);
        let units = result["units"].as_array().unwrap();
        assert_eq!(units.len(), 2);
        let ids: Vec<&str> = units.iter().map(|u| u["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"c1"));
        assert!(ids.contains(&"c2"));
    }

    #[tokio::test]
    async fn test_hierarchy_navigate_tool_invalid_direction_returns_error() {
        let tool = HierarchyNavigateTool::new(make_backend());
        let err = tool
            .call(serde_json::json!({
                "unit_id": "some-id",
                "direction": "sideways",
                "tenantContext": tenant_ctx_json("t", "u")
            }))
            .await;
        assert!(err.is_err());
    }
}
