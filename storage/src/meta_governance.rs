//! Meta-Governance: Policies about policies
//!
//! This module defines who can govern at each level of the organizational
//! hierarchy. Meta-governance policies control:
//! - Who can create/approve policies at each layer (tenant, org, team,
//!   project)
//! - Delegation rules for AI agents (what they can do autonomously vs needing
//!   human approval)
//! - Escalation paths when approvers are unavailable
//! - Human confirmation gates for sensitive agent actions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::governance::{PrincipalType, RiskLevel};

const DEFAULT_SESSION_TIMEOUT_HOURS: u64 = 12;
const DEFAULT_ACTIONS_PER_MINUTE: u64 = 20;
const DEFAULT_ACTIONS_PER_HOUR: u64 = 300;
const DEFAULT_MEMORY_WRITES_PER_HOUR: u64 = 50;
const DEFAULT_GOVERNANCE_SUBMISSIONS_PER_DAY: u64 = 5;

/// Defines the governance layer where a meta-governance policy applies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceLayer {
    Tenant,
    Org,
    Team,
    Project,
}

impl std::fmt::Display for GovernanceLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GovernanceLayer::Tenant => write!(f, "tenant"),
            GovernanceLayer::Org => write!(f, "org"),
            GovernanceLayer::Team => write!(f, "team"),
            GovernanceLayer::Project => write!(f, "project"),
        }
    }
}

impl std::str::FromStr for GovernanceLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tenant" => Ok(GovernanceLayer::Tenant),
            "org" | "organization" => Ok(GovernanceLayer::Org),
            "team" => Ok(GovernanceLayer::Team),
            "project" => Ok(GovernanceLayer::Project),
            _ => Err(format!(
                "Invalid governance layer: {}. Use: tenant, org, team, project",
                s
            )),
        }
    }
}

/// The type of governance action being controlled.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GovernanceActionType {
    // Policy actions
    CreatePolicy,
    ApprovePolicy,
    RejectPolicy,
    DeletePolicy,
    // Knowledge actions
    ProposeKnowledge,
    ApproveKnowledge,
    EditKnowledge,
    DeleteKnowledge,
    // Memory actions
    PromoteMemory,
    DeleteMemory,
    // Role actions
    AssignRole,
    RevokeRole,
    // Governance config actions
    ModifyGovernanceConfig,
    // Meta-governance actions (most privileged)
    ModifyMetaGovernance,
}

impl std::fmt::Display for GovernanceActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            GovernanceActionType::CreatePolicy => "create_policy",
            GovernanceActionType::ApprovePolicy => "approve_policy",
            GovernanceActionType::RejectPolicy => "reject_policy",
            GovernanceActionType::DeletePolicy => "delete_policy",
            GovernanceActionType::ProposeKnowledge => "propose_knowledge",
            GovernanceActionType::ApproveKnowledge => "approve_knowledge",
            GovernanceActionType::EditKnowledge => "edit_knowledge",
            GovernanceActionType::DeleteKnowledge => "delete_knowledge",
            GovernanceActionType::PromoteMemory => "promote_memory",
            GovernanceActionType::DeleteMemory => "delete_memory",
            GovernanceActionType::AssignRole => "assign_role",
            GovernanceActionType::RevokeRole => "revoke_role",
            GovernanceActionType::ModifyGovernanceConfig => "modify_governance_config",
            GovernanceActionType::ModifyMetaGovernance => "modify_meta_governance",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for GovernanceActionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "create_policy" => Ok(GovernanceActionType::CreatePolicy),
            "approve_policy" => Ok(GovernanceActionType::ApprovePolicy),
            "reject_policy" => Ok(GovernanceActionType::RejectPolicy),
            "delete_policy" => Ok(GovernanceActionType::DeletePolicy),
            "propose_knowledge" => Ok(GovernanceActionType::ProposeKnowledge),
            "approve_knowledge" => Ok(GovernanceActionType::ApproveKnowledge),
            "edit_knowledge" => Ok(GovernanceActionType::EditKnowledge),
            "delete_knowledge" => Ok(GovernanceActionType::DeleteKnowledge),
            "promote_memory" => Ok(GovernanceActionType::PromoteMemory),
            "delete_memory" => Ok(GovernanceActionType::DeleteMemory),
            "assign_role" => Ok(GovernanceActionType::AssignRole),
            "revoke_role" => Ok(GovernanceActionType::RevokeRole),
            "modify_governance_config" => Ok(GovernanceActionType::ModifyGovernanceConfig),
            "modify_meta_governance" => Ok(GovernanceActionType::ModifyMetaGovernance),
            _ => Err(format!("Invalid governance action type: {}", s)),
        }
    }
}

/// A meta-governance policy that controls who can govern at a specific layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaGovernancePolicy {
    pub id: Uuid,
    pub layer: GovernanceLayer,
    pub scope_id: Option<Uuid>,
    pub min_role_for_governance: RoleLevel,
    /// Specific permissions for different action types
    pub action_permissions: Vec<ActionPermission>,
    /// Agent delegation rules for this layer
    pub agent_delegation: AgentDelegationConfig,
    /// Escalation configuration
    pub escalation_config: EscalationConfig,
    /// Whether this policy is active
    pub active: bool,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Updated timestamp
    pub updated_at: DateTime<Utc>,
    /// Who created this policy
    pub created_by: Uuid,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RoleLevel {
    Viewer = 0,
    #[default]
    Developer = 1,
    TechLead = 2,
    Architect = 3,
    Admin = 4,
}

impl std::fmt::Display for RoleLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoleLevel::Viewer => write!(f, "viewer"),
            RoleLevel::Developer => write!(f, "developer"),
            RoleLevel::TechLead => write!(f, "techlead"),
            RoleLevel::Architect => write!(f, "architect"),
            RoleLevel::Admin => write!(f, "admin"),
        }
    }
}

impl std::str::FromStr for RoleLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "viewer" => Ok(RoleLevel::Viewer),
            "developer" => Ok(RoleLevel::Developer),
            "techlead" | "tech_lead" => Ok(RoleLevel::TechLead),
            "architect" => Ok(RoleLevel::Architect),
            "admin" => Ok(RoleLevel::Admin),
            _ => Err(format!(
                "Invalid role level: {}. Use: viewer, developer, techlead, architect, admin",
                s
            )),
        }
    }
}

/// Permission configuration for a specific governance action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPermission {
    pub action: GovernanceActionType,
    pub min_role: Option<RoleLevel>,
    /// Whether agents can perform this action autonomously
    pub agent_autonomous: bool,
    /// Whether this action requires human confirmation even when agent is
    /// authorized
    pub requires_human_confirmation: bool,
    /// Risk levels where this action is restricted
    pub restricted_risk_levels: Vec<RiskLevel>,
}

/// Configuration for AI agent delegation at a governance layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDelegationConfig {
    /// Whether agents can act autonomously at this layer
    pub autonomous_enabled: bool,
    /// Maximum delegation depth from a human principal
    pub max_delegation_depth: i32,
    /// Capabilities agents can have at this layer
    pub allowed_capabilities: Vec<AgentCapability>,
    /// Actions that always require human confirmation (override autonomous)
    pub human_confirmation_required: Vec<GovernanceActionType>,
    /// Time limit for agent sessions (hours)
    pub session_timeout_hours: i32,
    /// Rate limits for agent actions
    pub rate_limits: Option<AgentRateLimits>,
}

impl Default for AgentDelegationConfig {
    fn default() -> Self {
        Self {
            autonomous_enabled: true,
            max_delegation_depth: 3,
            allowed_capabilities: vec![
                AgentCapability::MemoryRead,
                AgentCapability::MemoryWrite,
                AgentCapability::KnowledgeRead,
                AgentCapability::KnowledgePropose,
                AgentCapability::PolicyRead,
                AgentCapability::PolicySimulate,
                AgentCapability::GovernanceRead,
                AgentCapability::GovernanceSubmit,
                AgentCapability::OrgRead,
            ],
            human_confirmation_required: vec![
                GovernanceActionType::DeletePolicy,
                GovernanceActionType::DeleteKnowledge,
                GovernanceActionType::ModifyGovernanceConfig,
                GovernanceActionType::ModifyMetaGovernance,
                GovernanceActionType::AssignRole,
                GovernanceActionType::RevokeRole,
            ],
            session_timeout_hours: 24,
            rate_limits: Some(AgentRateLimits::default()),
        }
    }
}

/// Agent capabilities that can be granted or restricted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    // Memory capabilities
    MemoryRead,
    MemoryWrite,
    MemoryDelete,
    MemoryPromote,
    // Knowledge capabilities
    KnowledgeRead,
    KnowledgePropose,
    KnowledgeEdit,
    // Policy capabilities
    PolicyRead,
    PolicyCreate,
    PolicySimulate,
    // Governance capabilities
    GovernanceRead,
    GovernanceSubmit,
    GovernanceApprove, // Rarely granted to agents
    // Organization capabilities
    OrgRead,
    // Agent-to-agent capabilities
    AgentRegister,
    AgentDelegate,
}

impl std::fmt::Display for AgentCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AgentCapability::MemoryRead => "memory:read",
            AgentCapability::MemoryWrite => "memory:write",
            AgentCapability::MemoryDelete => "memory:delete",
            AgentCapability::MemoryPromote => "memory:promote",
            AgentCapability::KnowledgeRead => "knowledge:read",
            AgentCapability::KnowledgePropose => "knowledge:propose",
            AgentCapability::KnowledgeEdit => "knowledge:edit",
            AgentCapability::PolicyRead => "policy:read",
            AgentCapability::PolicyCreate => "policy:create",
            AgentCapability::PolicySimulate => "policy:simulate",
            AgentCapability::GovernanceRead => "governance:read",
            AgentCapability::GovernanceSubmit => "governance:submit",
            AgentCapability::GovernanceApprove => "governance:approve",
            AgentCapability::OrgRead => "org:read",
            AgentCapability::AgentRegister => "agent:register",
            AgentCapability::AgentDelegate => "agent:delegate",
        };
        write!(f, "{}", s)
    }
}

impl std::str::FromStr for AgentCapability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "memory:read" => Ok(AgentCapability::MemoryRead),
            "memory:write" => Ok(AgentCapability::MemoryWrite),
            "memory:delete" => Ok(AgentCapability::MemoryDelete),
            "memory:promote" => Ok(AgentCapability::MemoryPromote),
            "knowledge:read" => Ok(AgentCapability::KnowledgeRead),
            "knowledge:propose" => Ok(AgentCapability::KnowledgePropose),
            "knowledge:edit" => Ok(AgentCapability::KnowledgeEdit),
            "policy:read" => Ok(AgentCapability::PolicyRead),
            "policy:create" => Ok(AgentCapability::PolicyCreate),
            "policy:simulate" => Ok(AgentCapability::PolicySimulate),
            "governance:read" => Ok(AgentCapability::GovernanceRead),
            "governance:submit" => Ok(AgentCapability::GovernanceSubmit),
            "governance:approve" => Ok(AgentCapability::GovernanceApprove),
            "org:read" => Ok(AgentCapability::OrgRead),
            "agent:register" => Ok(AgentCapability::AgentRegister),
            "agent:delegate" => Ok(AgentCapability::AgentDelegate),
            _ => Err(format!("Invalid agent capability: {}", s)),
        }
    }
}

/// Rate limits for agent actions to prevent abuse.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRateLimits {
    /// Maximum actions per minute
    pub actions_per_minute: i32,
    /// Maximum actions per hour
    pub actions_per_hour: i32,
    /// Maximum governance submissions per day
    pub governance_submissions_per_day: i32,
    /// Maximum memory writes per hour
    pub memory_writes_per_hour: i32,
}

impl Default for AgentRateLimits {
    fn default() -> Self {
        Self {
            actions_per_minute: 30,
            actions_per_hour: 500,
            governance_submissions_per_day: 10,
            memory_writes_per_hour: 100,
        }
    }
}

/// Configuration for escalation when approvers are unavailable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationConfig {
    /// Whether escalation is enabled
    pub enabled: bool,
    /// Hours to wait before first escalation
    pub initial_timeout_hours: i32,
    /// Escalation tiers (ordered by priority)
    pub tiers: Vec<EscalationTier>,
    /// What happens if all escalation tiers fail
    pub fallback_action: EscalationFallback,
    /// Send reminders before escalation
    pub reminder_intervals_hours: Vec<i32>,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            initial_timeout_hours: 24,
            tiers: vec![
                EscalationTier {
                    name: "Team Lead Escalation".to_string(),
                    timeout_hours: 12,
                    escalate_to: EscalationTarget::RoleInScope(RoleLevel::TechLead),
                    notification_channels: vec![NotificationChannel::Email],
                },
                EscalationTier {
                    name: "Architect Escalation".to_string(),
                    timeout_hours: 24,
                    escalate_to: EscalationTarget::RoleInScope(RoleLevel::Architect),
                    notification_channels: vec![
                        NotificationChannel::Email,
                        NotificationChannel::Slack,
                    ],
                },
                EscalationTier {
                    name: "Admin Escalation".to_string(),
                    timeout_hours: 48,
                    escalate_to: EscalationTarget::RoleInScope(RoleLevel::Admin),
                    notification_channels: vec![
                        NotificationChannel::Email,
                        NotificationChannel::Slack,
                        NotificationChannel::PagerDuty,
                    ],
                },
            ],
            fallback_action: EscalationFallback::ExpireRequest,
            reminder_intervals_hours: vec![12, 18, 23],
        }
    }
}

/// A tier in the escalation chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationTier {
    /// Human-readable name for this tier
    pub name: String,
    /// Hours to wait at this tier before moving to next
    pub timeout_hours: i32,
    /// Who to escalate to
    pub escalate_to: EscalationTarget,
    /// How to notify escalation targets
    pub notification_channels: Vec<NotificationChannel>,
}

/// Target for escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscalationTarget {
    RoleInScope(RoleLevel),
    SpecificUser(Uuid),
    ParentScope,
    CustomGroup(String),
}

/// What happens when all escalation tiers fail.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EscalationFallback {
    /// Expire the request (default)
    ExpireRequest,
    /// Auto-approve (risky, use only for low-risk items)
    AutoApprove,
    /// Keep waiting indefinitely
    WaitIndefinitely,
    /// Notify emergency contacts
    NotifyEmergency,
}

/// Notification channels for escalation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationChannel {
    Email,
    Slack,
    MsTeams,
    PagerDuty,
    Webhook,
}

/// A request for human confirmation of an agent action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanConfirmationRequest {
    pub id: Uuid,
    /// The agent requesting confirmation
    pub agent_id: Uuid,
    /// The action the agent wants to perform
    pub action: GovernanceActionType,
    /// Human-readable description of what the agent wants to do
    pub action_description: String,
    /// The target of the action (policy ID, knowledge ID, etc.)
    pub target_type: String,
    pub target_id: Option<String>,
    /// Risk assessment
    pub risk_level: RiskLevel,
    /// Why this requires confirmation
    pub confirmation_reason: ConfirmationReason,
    /// Context about the agent's session
    pub agent_context: serde_json::Value,
    /// Users who can approve this request
    pub authorized_approvers: Vec<Uuid>,
    /// Status of the confirmation request
    pub status: ConfirmationStatus,
    /// Timeout for confirmation (after which request is auto-denied)
    pub expires_at: DateTime<Utc>,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Resolution timestamp
    pub resolved_at: Option<DateTime<Utc>>,
    /// Who resolved (if human)
    pub resolved_by: Option<Uuid>,
    /// Resolution comment
    pub resolution_comment: Option<String>,
}

/// Why human confirmation is required.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationReason {
    /// Action is in the always-confirm list
    PolicyRequired,
    /// Action has high risk level
    HighRisk,
    /// Agent delegation depth exceeded soft limit
    DelegationDepthWarning,
    /// Rate limit approaching
    RateLimitWarning,
    /// Action affects multiple scopes
    CrossScopeAction,
    /// First time agent performs this action type
    FirstTimeAction,
    /// Manual request by agent
    AgentRequested,
}

impl std::fmt::Display for ConfirmationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmationReason::PolicyRequired => write!(f, "Policy requires human confirmation"),
            ConfirmationReason::HighRisk => write!(f, "Action classified as high risk"),
            ConfirmationReason::DelegationDepthWarning => {
                write!(f, "Agent delegation depth near limit")
            }
            ConfirmationReason::RateLimitWarning => write!(f, "Agent approaching rate limit"),
            ConfirmationReason::CrossScopeAction => write!(f, "Action affects multiple scopes"),
            ConfirmationReason::FirstTimeAction => {
                write!(f, "First time agent performs this action")
            }
            ConfirmationReason::AgentRequested => write!(f, "Agent requested human oversight"),
        }
    }
}

/// Status of a human confirmation request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmationStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
}

impl std::fmt::Display for ConfirmationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfirmationStatus::Pending => write!(f, "pending"),
            ConfirmationStatus::Approved => write!(f, "approved"),
            ConfirmationStatus::Denied => write!(f, "denied"),
            ConfirmationStatus::Expired => write!(f, "expired"),
            ConfirmationStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::str::FromStr for ConfirmationStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pending" => Ok(ConfirmationStatus::Pending),
            "approved" => Ok(ConfirmationStatus::Approved),
            "denied" => Ok(ConfirmationStatus::Denied),
            "expired" => Ok(ConfirmationStatus::Expired),
            "cancelled" => Ok(ConfirmationStatus::Cancelled),
            _ => Err(format!("Invalid confirmation status: {}", s)),
        }
    }
}

pub fn create_default_policies() -> Vec<MetaGovernancePolicy> {
    let now = Utc::now();
    let system_user = Uuid::nil();

    vec![
        MetaGovernancePolicy {
            id: Uuid::new_v4(),
            layer: GovernanceLayer::Tenant,
            scope_id: None,
            min_role_for_governance: RoleLevel::Admin,
            action_permissions: vec![
                ActionPermission {
                    action: GovernanceActionType::CreatePolicy,
                    min_role: Some(RoleLevel::Architect),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![],
                },
                ActionPermission {
                    action: GovernanceActionType::ApprovePolicy,
                    min_role: Some(RoleLevel::Admin),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![],
                },
                ActionPermission {
                    action: GovernanceActionType::ModifyMetaGovernance,
                    min_role: Some(RoleLevel::Admin),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![],
                },
            ],
            agent_delegation: AgentDelegationConfig {
                autonomous_enabled: false,
                max_delegation_depth: 1,
                allowed_capabilities: vec![
                    AgentCapability::KnowledgeRead,
                    AgentCapability::PolicyRead,
                    AgentCapability::GovernanceRead,
                    AgentCapability::OrgRead,
                ],
                human_confirmation_required: vec![
                    GovernanceActionType::CreatePolicy,
                    GovernanceActionType::ApprovePolicy,
                    GovernanceActionType::RejectPolicy,
                    GovernanceActionType::DeletePolicy,
                    GovernanceActionType::ProposeKnowledge,
                    GovernanceActionType::ApproveKnowledge,
                    GovernanceActionType::EditKnowledge,
                    GovernanceActionType::DeleteKnowledge,
                    GovernanceActionType::AssignRole,
                    GovernanceActionType::RevokeRole,
                    GovernanceActionType::ModifyGovernanceConfig,
                    GovernanceActionType::ModifyMetaGovernance,
                ],
                session_timeout_hours: 8,
                rate_limits: Some(AgentRateLimits {
                    actions_per_minute: 10,
                    actions_per_hour: 100,
                    governance_submissions_per_day: 2,
                    memory_writes_per_hour: 20,
                }),
            },
            escalation_config: EscalationConfig {
                enabled: true,
                initial_timeout_hours: 12,
                tiers: vec![EscalationTier {
                    name: "Admin Escalation".to_string(),
                    timeout_hours: 12,
                    escalate_to: EscalationTarget::RoleInScope(RoleLevel::Admin),
                    notification_channels: vec![
                        NotificationChannel::Email,
                        NotificationChannel::Slack,
                        NotificationChannel::PagerDuty,
                    ],
                }],
                fallback_action: EscalationFallback::NotifyEmergency,
                reminder_intervals_hours: vec![6, 10],
            },
            active: true,
            created_at: now,
            updated_at: now,
            created_by: system_user,
        },
        MetaGovernancePolicy {
            id: Uuid::new_v4(),
            layer: GovernanceLayer::Org,
            scope_id: None,
            min_role_for_governance: RoleLevel::Architect,
            action_permissions: vec![
                ActionPermission {
                    action: GovernanceActionType::CreatePolicy,
                    min_role: Some(RoleLevel::TechLead),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::ApprovePolicy,
                    min_role: Some(RoleLevel::Architect),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![],
                },
                ActionPermission {
                    action: GovernanceActionType::ProposeKnowledge,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![RiskLevel::High, RiskLevel::Critical],
                },
            ],
            agent_delegation: AgentDelegationConfig {
                autonomous_enabled: true,
                max_delegation_depth: 2,
                allowed_capabilities: vec![
                    AgentCapability::MemoryRead,
                    AgentCapability::MemoryWrite,
                    AgentCapability::KnowledgeRead,
                    AgentCapability::KnowledgePropose,
                    AgentCapability::PolicyRead,
                    AgentCapability::PolicySimulate,
                    AgentCapability::GovernanceRead,
                    AgentCapability::GovernanceSubmit,
                    AgentCapability::OrgRead,
                ],
                human_confirmation_required: vec![
                    GovernanceActionType::DeletePolicy,
                    GovernanceActionType::DeleteKnowledge,
                    GovernanceActionType::AssignRole,
                    GovernanceActionType::RevokeRole,
                    GovernanceActionType::ModifyGovernanceConfig,
                ],
                session_timeout_hours: DEFAULT_SESSION_TIMEOUT_HOURS as i32,
                rate_limits: Some(AgentRateLimits {
                    actions_per_minute: DEFAULT_ACTIONS_PER_MINUTE as i32,
                    actions_per_hour: DEFAULT_ACTIONS_PER_HOUR as i32,
                    governance_submissions_per_day: DEFAULT_GOVERNANCE_SUBMISSIONS_PER_DAY as i32,
                    memory_writes_per_hour: DEFAULT_MEMORY_WRITES_PER_HOUR as i32,
                }),
            },
            escalation_config: EscalationConfig::default(),
            active: true,
            created_at: now,
            updated_at: now,
            created_by: system_user,
        },
        MetaGovernancePolicy {
            id: Uuid::new_v4(),
            layer: GovernanceLayer::Team,
            scope_id: None,
            min_role_for_governance: RoleLevel::TechLead,
            action_permissions: vec![
                ActionPermission {
                    action: GovernanceActionType::CreatePolicy,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![RiskLevel::High, RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::ApprovePolicy,
                    min_role: Some(RoleLevel::TechLead),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![],
                },
                ActionPermission {
                    action: GovernanceActionType::ProposeKnowledge,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::PromoteMemory,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![],
                },
            ],
            agent_delegation: AgentDelegationConfig {
                autonomous_enabled: true,
                max_delegation_depth: 3,
                allowed_capabilities: vec![
                    AgentCapability::MemoryRead,
                    AgentCapability::MemoryWrite,
                    AgentCapability::MemoryPromote,
                    AgentCapability::KnowledgeRead,
                    AgentCapability::KnowledgePropose,
                    AgentCapability::PolicyRead,
                    AgentCapability::PolicyCreate,
                    AgentCapability::PolicySimulate,
                    AgentCapability::GovernanceRead,
                    AgentCapability::GovernanceSubmit,
                    AgentCapability::OrgRead,
                ],
                human_confirmation_required: vec![
                    GovernanceActionType::DeletePolicy,
                    GovernanceActionType::DeleteKnowledge,
                    GovernanceActionType::DeleteMemory,
                    GovernanceActionType::ModifyGovernanceConfig,
                ],
                session_timeout_hours: 24,
                rate_limits: Some(AgentRateLimits::default()),
            },
            escalation_config: EscalationConfig::default(),
            active: true,
            created_at: now,
            updated_at: now,
            created_by: system_user,
        },
        MetaGovernancePolicy {
            id: Uuid::new_v4(),
            layer: GovernanceLayer::Project,
            scope_id: None,
            min_role_for_governance: RoleLevel::Developer,
            action_permissions: vec![
                ActionPermission {
                    action: GovernanceActionType::CreatePolicy,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::ApprovePolicy,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: false,
                    requires_human_confirmation: true,
                    restricted_risk_levels: vec![RiskLevel::High, RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::ProposeKnowledge,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![],
                },
                ActionPermission {
                    action: GovernanceActionType::ApproveKnowledge,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![RiskLevel::High, RiskLevel::Critical],
                },
                ActionPermission {
                    action: GovernanceActionType::PromoteMemory,
                    min_role: Some(RoleLevel::Developer),
                    agent_autonomous: true,
                    requires_human_confirmation: false,
                    restricted_risk_levels: vec![],
                },
            ],
            agent_delegation: AgentDelegationConfig {
                autonomous_enabled: true,
                max_delegation_depth: 3,
                allowed_capabilities: vec![
                    AgentCapability::MemoryRead,
                    AgentCapability::MemoryWrite,
                    AgentCapability::MemoryDelete,
                    AgentCapability::MemoryPromote,
                    AgentCapability::KnowledgeRead,
                    AgentCapability::KnowledgePropose,
                    AgentCapability::KnowledgeEdit,
                    AgentCapability::PolicyRead,
                    AgentCapability::PolicyCreate,
                    AgentCapability::PolicySimulate,
                    AgentCapability::GovernanceRead,
                    AgentCapability::GovernanceSubmit,
                    AgentCapability::OrgRead,
                    AgentCapability::AgentRegister,
                    AgentCapability::AgentDelegate,
                ],
                human_confirmation_required: vec![
                    GovernanceActionType::DeletePolicy,
                    GovernanceActionType::ModifyGovernanceConfig,
                ],
                session_timeout_hours: 48,
                rate_limits: Some(AgentRateLimits {
                    actions_per_minute: 60,
                    actions_per_hour: 1000,
                    governance_submissions_per_day: 20,
                    memory_writes_per_hour: 200,
                }),
            },
            escalation_config: EscalationConfig {
                enabled: true,
                initial_timeout_hours: 48,
                tiers: vec![EscalationTier {
                    name: "Team Escalation".to_string(),
                    timeout_hours: 24,
                    escalate_to: EscalationTarget::ParentScope,
                    notification_channels: vec![NotificationChannel::Email],
                }],
                fallback_action: EscalationFallback::ExpireRequest,
                reminder_intervals_hours: vec![24, 36],
            },
            active: true,
            created_at: now,
            updated_at: now,
            created_by: system_user,
        },
    ]
}

/// Result of a meta-governance authorization check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationResult {
    pub allowed: bool,
    pub reason: String,
    pub requires_human_confirmation: bool,
    pub escalation_required: bool,
    pub warnings: Vec<String>,
}

impl MetaGovernancePolicy {
    pub fn check_authorization(
        &self,
        principal_type: PrincipalType,
        principal_role: RoleLevel,
        action: GovernanceActionType,
        risk_level: RiskLevel,
        delegation_depth: Option<i32>,
    ) -> AuthorizationResult {
        let mut warnings = Vec::new();
        let mut requires_human_confirmation = false;

        let action_permission = self.action_permissions.iter().find(|p| p.action == action);

        let min_role = action_permission
            .and_then(|p| p.min_role)
            .unwrap_or(self.min_role_for_governance);

        if principal_role < min_role {
            return AuthorizationResult {
                allowed: false,
                reason: format!(
                    "Role {} is insufficient. Minimum required: {}",
                    principal_role, min_role
                ),
                requires_human_confirmation: false,
                escalation_required: false,
                warnings: vec![],
            };
        }

        if let Some(permission) = action_permission {
            if permission.restricted_risk_levels.contains(&risk_level) {
                return AuthorizationResult {
                    allowed: false,
                    reason: format!(
                        "Action {} is restricted at risk level {}",
                        action, risk_level
                    ),
                    requires_human_confirmation: false,
                    escalation_required: false,
                    warnings: vec![],
                };
            }

            if permission.requires_human_confirmation {
                requires_human_confirmation = true;
            }
        }

        if principal_type == PrincipalType::Agent {
            let delegation = &self.agent_delegation;

            if !delegation.autonomous_enabled {
                return AuthorizationResult {
                    allowed: false,
                    reason: format!("Agents cannot act autonomously at {} layer", self.layer),
                    requires_human_confirmation: true,
                    escalation_required: false,
                    warnings: vec![],
                };
            }

            if let Some(depth) = delegation_depth {
                if depth > delegation.max_delegation_depth {
                    return AuthorizationResult {
                        allowed: false,
                        reason: format!(
                            "Delegation depth {} exceeds maximum {} for {} layer",
                            depth, delegation.max_delegation_depth, self.layer
                        ),
                        requires_human_confirmation: true,
                        escalation_required: false,
                        warnings: vec![],
                    };
                }

                if depth == delegation.max_delegation_depth {
                    warnings.push(format!(
                        "Delegation depth at maximum ({}). Further delegation not allowed.",
                        depth
                    ));
                }
            }

            if delegation.human_confirmation_required.contains(&action) {
                requires_human_confirmation = true;
            }

            if let Some(permission) = action_permission
                && !permission.agent_autonomous
                && !requires_human_confirmation
            {
                requires_human_confirmation = true;
                warnings.push(format!(
                    "Action {} requires human confirmation when performed by agents",
                    action
                ));
            }
        }

        AuthorizationResult {
            allowed: true,
            reason: "Authorized".to_string(),
            requires_human_confirmation,
            escalation_required: false,
            warnings,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct MetaGovernancePolicyRow {
    id: Uuid,
    layer: String,
    scope_id: Option<Uuid>,
    min_role_for_governance: String,
    action_permissions: serde_json::Value,
    agent_delegation: serde_json::Value,
    escalation_config: serde_json::Value,
    active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    created_by: Uuid,
}

impl From<MetaGovernancePolicyRow> for MetaGovernancePolicy {
    fn from(row: MetaGovernancePolicyRow) -> Self {
        Self {
            id: row.id,
            layer: row.layer.parse().unwrap_or(GovernanceLayer::Project),
            scope_id: row.scope_id,
            min_role_for_governance: row
                .min_role_for_governance
                .parse()
                .unwrap_or(RoleLevel::Developer),
            action_permissions: serde_json::from_value(row.action_permissions).unwrap_or_default(),
            agent_delegation: serde_json::from_value(row.agent_delegation)
                .unwrap_or_else(|_| AgentDelegationConfig::default()),
            escalation_config: serde_json::from_value(row.escalation_config)
                .unwrap_or_else(|_| EscalationConfig::default()),
            active: row.active,
            created_at: row.created_at,
            updated_at: row.updated_at,
            created_by: row.created_by,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct ConfirmationRequestRow {
    id: Uuid,
    agent_id: Uuid,
    action: String,
    action_description: String,
    target_type: String,
    target_id: Option<String>,
    risk_level: String,
    confirmation_reason: String,
    agent_context: serde_json::Value,
    authorized_approvers: serde_json::Value,
    status: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    resolved_at: Option<DateTime<Utc>>,
    resolved_by: Option<Uuid>,
    resolution_comment: Option<String>,
}

/// Storage for meta-governance policies and human confirmation requests.
pub struct MetaGovernanceStorage {
    pool: PgPool,
}

impl MetaGovernanceStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn get_effective_policy(
        &self,
        layer: GovernanceLayer,
        scope_id: Option<Uuid>,
    ) -> Result<Option<MetaGovernancePolicy>, sqlx::Error> {
        if let Some(sid) = scope_id {
            let row: Option<MetaGovernancePolicyRow> = sqlx::query_as(
                r#"
                SELECT * FROM meta_governance_policies
                WHERE layer = $1 AND scope_id = $2 AND active = true
                ORDER BY updated_at DESC
                LIMIT 1
                "#,
            )
            .bind(layer.to_string())
            .bind(sid)
            .fetch_optional(&self.pool)
            .await?;

            if let Some(row) = row {
                return Ok(Some(row.into()));
            }
        }

        let row: Option<MetaGovernancePolicyRow> = sqlx::query_as(
            r#"
            SELECT * FROM meta_governance_policies
            WHERE layer = $1 AND scope_id IS NULL AND active = true
            ORDER BY updated_at DESC
            LIMIT 1
            "#,
        )
        .bind(layer.to_string())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Into::into))
    }

    /// Create or update a meta-governance policy.
    pub async fn upsert_policy(&self, policy: &MetaGovernancePolicy) -> Result<Uuid, sqlx::Error> {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO meta_governance_policies (
                id, layer, scope_id, min_role_for_governance,
                action_permissions, agent_delegation, escalation_config,
                active, created_at, updated_at, created_by
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (layer, scope_id) WHERE scope_id IS NOT NULL
            DO UPDATE SET
                min_role_for_governance = EXCLUDED.min_role_for_governance,
                action_permissions = EXCLUDED.action_permissions,
                agent_delegation = EXCLUDED.agent_delegation,
                escalation_config = EXCLUDED.escalation_config,
                active = EXCLUDED.active,
                updated_at = NOW()
            RETURNING id
            "#,
        )
        .bind(policy.id)
        .bind(policy.layer.to_string())
        .bind(policy.scope_id)
        .bind(policy.min_role_for_governance.to_string())
        .bind(serde_json::to_value(&policy.action_permissions).unwrap_or_default())
        .bind(serde_json::to_value(&policy.agent_delegation).unwrap_or_default())
        .bind(serde_json::to_value(&policy.escalation_config).unwrap_or_default())
        .bind(policy.active)
        .bind(policy.created_at)
        .bind(Utc::now())
        .bind(policy.created_by)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Create a human confirmation request.
    pub async fn create_confirmation_request(
        &self,
        request: &HumanConfirmationRequest,
    ) -> Result<Uuid, sqlx::Error> {
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO human_confirmation_requests (
                id, agent_id, action, action_description,
                target_type, target_id, risk_level, confirmation_reason,
                agent_context, authorized_approvers, status,
                expires_at, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
            RETURNING id
            "#,
        )
        .bind(request.id)
        .bind(request.agent_id)
        .bind(request.action.to_string())
        .bind(&request.action_description)
        .bind(&request.target_type)
        .bind(&request.target_id)
        .bind(request.risk_level.to_string())
        .bind(format!("{:?}", request.confirmation_reason))
        .bind(&request.agent_context)
        .bind(serde_json::to_value(&request.authorized_approvers).unwrap_or_default())
        .bind(request.status.to_string())
        .bind(request.expires_at)
        .bind(request.created_at)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0)
    }

    /// Get a pending confirmation request.
    pub async fn get_confirmation_request(
        &self,
        request_id: Uuid,
    ) -> Result<Option<HumanConfirmationRequest>, sqlx::Error> {
        let row: Option<ConfirmationRequestRow> =
            sqlx::query_as("SELECT * FROM human_confirmation_requests WHERE id = $1")
                .bind(request_id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|r| HumanConfirmationRequest {
            id: r.id,
            agent_id: r.agent_id,
            action: r
                .action
                .parse()
                .unwrap_or(GovernanceActionType::CreatePolicy),
            action_description: r.action_description,
            target_type: r.target_type,
            target_id: r.target_id,
            risk_level: r.risk_level.parse().unwrap_or_default(),
            confirmation_reason: match r.confirmation_reason.as_str() {
                "PolicyRequired" => ConfirmationReason::PolicyRequired,
                "HighRisk" => ConfirmationReason::HighRisk,
                "DelegationDepthWarning" => ConfirmationReason::DelegationDepthWarning,
                "RateLimitWarning" => ConfirmationReason::RateLimitWarning,
                "CrossScopeAction" => ConfirmationReason::CrossScopeAction,
                "FirstTimeAction" => ConfirmationReason::FirstTimeAction,
                _ => ConfirmationReason::AgentRequested,
            },
            agent_context: r.agent_context,
            authorized_approvers: serde_json::from_value(r.authorized_approvers)
                .unwrap_or_default(),
            status: r.status.parse().unwrap_or(ConfirmationStatus::Pending),
            expires_at: r.expires_at,
            created_at: r.created_at,
            resolved_at: r.resolved_at,
            resolved_by: r.resolved_by,
            resolution_comment: r.resolution_comment,
        }))
    }

    /// List pending confirmation requests for an approver.
    pub async fn list_pending_confirmations(
        &self,
        approver_id: Uuid,
        limit: i32,
    ) -> Result<Vec<HumanConfirmationRequest>, sqlx::Error> {
        let rows: Vec<ConfirmationRequestRow> = sqlx::query_as(
            r#"
            SELECT * FROM human_confirmation_requests
            WHERE status = 'pending'
              AND expires_at > NOW()
              AND authorized_approvers @> $1::jsonb
            ORDER BY created_at DESC
            LIMIT $2
            "#,
        )
        .bind(serde_json::json!([approver_id]))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| HumanConfirmationRequest {
                id: r.id,
                agent_id: r.agent_id,
                action: r
                    .action
                    .parse()
                    .unwrap_or(GovernanceActionType::CreatePolicy),
                action_description: r.action_description,
                target_type: r.target_type,
                target_id: r.target_id,
                risk_level: r.risk_level.parse().unwrap_or_default(),
                confirmation_reason: match r.confirmation_reason.as_str() {
                    "PolicyRequired" => ConfirmationReason::PolicyRequired,
                    "HighRisk" => ConfirmationReason::HighRisk,
                    "DelegationDepthWarning" => ConfirmationReason::DelegationDepthWarning,
                    "RateLimitWarning" => ConfirmationReason::RateLimitWarning,
                    "CrossScopeAction" => ConfirmationReason::CrossScopeAction,
                    "FirstTimeAction" => ConfirmationReason::FirstTimeAction,
                    _ => ConfirmationReason::AgentRequested,
                },
                agent_context: r.agent_context,
                authorized_approvers: serde_json::from_value(r.authorized_approvers)
                    .unwrap_or_default(),
                status: r.status.parse().unwrap_or(ConfirmationStatus::Pending),
                expires_at: r.expires_at,
                created_at: r.created_at,
                resolved_at: r.resolved_at,
                resolved_by: r.resolved_by,
                resolution_comment: r.resolution_comment,
            })
            .collect())
    }

    /// Resolve a confirmation request (approve/deny).
    pub async fn resolve_confirmation(
        &self,
        request_id: Uuid,
        approved: bool,
        resolved_by: Uuid,
        comment: Option<String>,
    ) -> Result<(), sqlx::Error> {
        let status = if approved {
            ConfirmationStatus::Approved
        } else {
            ConfirmationStatus::Denied
        };

        sqlx::query(
            r#"
            UPDATE human_confirmation_requests
            SET status = $2,
                resolved_at = NOW(),
                resolved_by = $3,
                resolution_comment = $4
            WHERE id = $1 AND status = 'pending'
            "#,
        )
        .bind(request_id)
        .bind(status.to_string())
        .bind(resolved_by)
        .bind(comment)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Expire old confirmation requests.
    pub async fn expire_old_requests(&self) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE human_confirmation_requests
            SET status = 'expired'
            WHERE status = 'pending' AND expires_at < NOW()
            "#,
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_governance_layer_display() {
        assert_eq!(GovernanceLayer::Tenant.to_string(), "tenant");
        assert_eq!(GovernanceLayer::Org.to_string(), "org");
        assert_eq!(GovernanceLayer::Team.to_string(), "team");
        assert_eq!(GovernanceLayer::Project.to_string(), "project");
    }

    #[test]
    fn test_governance_layer_parse() {
        assert_eq!(
            "tenant".parse::<GovernanceLayer>().unwrap(),
            GovernanceLayer::Tenant
        );
        assert_eq!(
            "organization".parse::<GovernanceLayer>().unwrap(),
            GovernanceLayer::Org
        );
        assert_eq!(
            "Team".parse::<GovernanceLayer>().unwrap(),
            GovernanceLayer::Team
        );
        assert!("invalid".parse::<GovernanceLayer>().is_err());
    }

    #[test]
    fn test_governance_role_ordering() {
        assert!(RoleLevel::Viewer < RoleLevel::Developer);
        assert!(RoleLevel::Developer < RoleLevel::TechLead);
        assert!(RoleLevel::TechLead < RoleLevel::Architect);
        assert!(RoleLevel::Architect < RoleLevel::Admin);
    }

    #[test]
    fn test_agent_capability_display() {
        assert_eq!(AgentCapability::MemoryRead.to_string(), "memory:read");
        assert_eq!(
            AgentCapability::KnowledgePropose.to_string(),
            "knowledge:propose"
        );
        assert_eq!(
            AgentCapability::PolicySimulate.to_string(),
            "policy:simulate"
        );
    }

    #[test]
    fn test_agent_capability_parse() {
        assert_eq!(
            "memory:read".parse::<AgentCapability>().unwrap(),
            AgentCapability::MemoryRead
        );
        assert_eq!(
            "governance:approve".parse::<AgentCapability>().unwrap(),
            AgentCapability::GovernanceApprove
        );
        assert!("invalid:cap".parse::<AgentCapability>().is_err());
    }

    #[test]
    fn test_default_policies_created() {
        let policies = create_default_policies();
        assert_eq!(policies.len(), 4);

        let layers: Vec<_> = policies.iter().map(|p| p.layer).collect();
        assert!(layers.contains(&GovernanceLayer::Tenant));
        assert!(layers.contains(&GovernanceLayer::Org));
        assert!(layers.contains(&GovernanceLayer::Team));
        assert!(layers.contains(&GovernanceLayer::Project));
    }

    #[test]
    fn test_company_policy_most_restrictive() {
        let policies = create_default_policies();
        let tenant = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Tenant)
            .unwrap();

        assert_eq!(tenant.min_role_for_governance, RoleLevel::Admin);
        assert!(!tenant.agent_delegation.autonomous_enabled);
        assert_eq!(tenant.agent_delegation.max_delegation_depth, 1);
    }

    #[test]
    fn test_project_policy_most_permissive() {
        let policies = create_default_policies();
        let project = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Project)
            .unwrap();

        assert_eq!(project.min_role_for_governance, RoleLevel::Developer);
        assert!(project.agent_delegation.autonomous_enabled);
        assert_eq!(project.agent_delegation.max_delegation_depth, 3);
    }

    #[test]
    fn test_authorization_check_role_insufficient() {
        let policies = create_default_policies();
        let tenant = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Tenant)
            .unwrap();

        let result = tenant.check_authorization(
            PrincipalType::User,
            RoleLevel::Developer,
            GovernanceActionType::ApprovePolicy,
            RiskLevel::Medium,
            None,
        );

        assert!(!result.allowed);
        assert!(result.reason.contains("insufficient"));
    }

    #[test]
    fn test_authorization_check_user_allowed() {
        let policies = create_default_policies();
        let project = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Project)
            .unwrap();

        let result = project.check_authorization(
            PrincipalType::User,
            RoleLevel::Developer,
            GovernanceActionType::ProposeKnowledge,
            RiskLevel::Low,
            None,
        );

        assert!(result.allowed);
    }

    #[test]
    fn test_authorization_check_agent_no_autonomous() {
        let policies = create_default_policies();
        let tenant = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Tenant)
            .unwrap();

        let result = tenant.check_authorization(
            PrincipalType::Agent,
            RoleLevel::Admin, // Even admin agent can't act autonomously
            GovernanceActionType::CreatePolicy,
            RiskLevel::Low,
            Some(1),
        );

        assert!(!result.allowed);
        assert!(result.reason.contains("cannot act autonomously"));
    }

    #[test]
    fn test_authorization_check_agent_delegation_depth_exceeded() {
        let policies = create_default_policies();
        let project = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Project)
            .unwrap();

        let result = project.check_authorization(
            PrincipalType::Agent,
            RoleLevel::Developer,
            GovernanceActionType::ProposeKnowledge,
            RiskLevel::Low,
            Some(5), // Exceeds max of 3
        );

        assert!(!result.allowed);
        assert!(result.reason.contains("Delegation depth"));
    }

    #[test]
    fn test_authorization_check_agent_requires_confirmation() {
        let policies = create_default_policies();
        let team = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Team)
            .unwrap();

        let result = team.check_authorization(
            PrincipalType::Agent,
            RoleLevel::TechLead,
            GovernanceActionType::DeletePolicy,
            RiskLevel::Low,
            Some(1),
        );

        assert!(result.allowed);
        assert!(result.requires_human_confirmation);
    }

    #[test]
    fn test_authorization_check_risk_level_restricted() {
        let policies = create_default_policies();
        let org = policies
            .iter()
            .find(|p| p.layer == GovernanceLayer::Org)
            .unwrap();

        let result = org.check_authorization(
            PrincipalType::User,
            RoleLevel::TechLead,
            GovernanceActionType::CreatePolicy,
            RiskLevel::Critical, // Restricted at org level
            None,
        );

        assert!(!result.allowed);
        assert!(result.reason.contains("restricted"));
    }

    #[test]
    fn test_escalation_config_default() {
        let config = EscalationConfig::default();
        assert!(config.enabled);
        assert_eq!(config.tiers.len(), 3);
        assert_eq!(config.fallback_action, EscalationFallback::ExpireRequest);
    }

    #[test]
    fn test_agent_rate_limits_default() {
        let limits = AgentRateLimits::default();
        assert_eq!(limits.actions_per_minute, 30);
        assert_eq!(limits.actions_per_hour, 500);
        assert_eq!(limits.governance_submissions_per_day, 10);
    }

    #[test]
    fn test_confirmation_status_roundtrip() {
        for status in [
            ConfirmationStatus::Pending,
            ConfirmationStatus::Approved,
            ConfirmationStatus::Denied,
            ConfirmationStatus::Expired,
            ConfirmationStatus::Cancelled,
        ] {
            let s = status.to_string();
            assert_eq!(s.parse::<ConfirmationStatus>().unwrap(), status);
        }
    }

    #[test]
    fn test_confirmation_reason_display() {
        assert_eq!(
            ConfirmationReason::PolicyRequired.to_string(),
            "Policy requires human confirmation"
        );
        assert_eq!(
            ConfirmationReason::HighRisk.to_string(),
            "Action classified as high risk"
        );
    }

    #[test]
    fn test_governance_action_type_roundtrip() {
        for action in [
            GovernanceActionType::CreatePolicy,
            GovernanceActionType::ApprovePolicy,
            GovernanceActionType::ProposeKnowledge,
            GovernanceActionType::PromoteMemory,
            GovernanceActionType::ModifyMetaGovernance,
        ] {
            let s = action.to_string();
            assert_eq!(s.parse::<GovernanceActionType>().unwrap(), action);
        }
    }

    #[test]
    fn test_agent_delegation_config_default_capabilities() {
        let config = AgentDelegationConfig::default();
        assert!(
            config
                .allowed_capabilities
                .contains(&AgentCapability::MemoryRead)
        );
        assert!(
            config
                .allowed_capabilities
                .contains(&AgentCapability::KnowledgePropose)
        );
        assert!(
            config
                .allowed_capabilities
                .contains(&AgentCapability::PolicySimulate)
        );
        // GovernanceApprove should NOT be in default capabilities
        assert!(
            !config
                .allowed_capabilities
                .contains(&AgentCapability::GovernanceApprove)
        );
    }

    #[test]
    fn test_role_level_display_parse_and_invalid() {
        assert_eq!(RoleLevel::Viewer.to_string(), "viewer");
        assert_eq!(RoleLevel::Developer.to_string(), "developer");
        assert_eq!(RoleLevel::TechLead.to_string(), "techlead");
        assert_eq!(RoleLevel::Architect.to_string(), "architect");
        assert_eq!(RoleLevel::Admin.to_string(), "admin");

        assert_eq!("viewer".parse::<RoleLevel>().unwrap(), RoleLevel::Viewer);
        assert_eq!(
            "developer".parse::<RoleLevel>().unwrap(),
            RoleLevel::Developer
        );
        assert_eq!(
            "tech_lead".parse::<RoleLevel>().unwrap(),
            RoleLevel::TechLead
        );
        assert_eq!(
            "architect".parse::<RoleLevel>().unwrap(),
            RoleLevel::Architect
        );
        assert_eq!("admin".parse::<RoleLevel>().unwrap(), RoleLevel::Admin);
        assert!("owner".parse::<RoleLevel>().is_err());
    }

    #[test]
    fn test_governance_action_type_display_parse_and_invalid() {
        let cases = [
            (GovernanceActionType::RejectPolicy, "reject_policy"),
            (GovernanceActionType::DeletePolicy, "delete_policy"),
            (GovernanceActionType::ApproveKnowledge, "approve_knowledge"),
            (GovernanceActionType::EditKnowledge, "edit_knowledge"),
            (GovernanceActionType::DeleteKnowledge, "delete_knowledge"),
            (GovernanceActionType::DeleteMemory, "delete_memory"),
            (GovernanceActionType::AssignRole, "assign_role"),
            (GovernanceActionType::RevokeRole, "revoke_role"),
            (
                GovernanceActionType::ModifyGovernanceConfig,
                "modify_governance_config",
            ),
        ];

        for (action, value) in cases {
            assert_eq!(action.to_string(), value);
            assert_eq!(value.parse::<GovernanceActionType>().unwrap(), action);
        }

        assert!("launch_missiles".parse::<GovernanceActionType>().is_err());
    }

    #[test]
    fn test_agent_capability_display_parse_fuller_matrix() {
        let cases = [
            (AgentCapability::MemoryWrite, "memory:write"),
            (AgentCapability::MemoryDelete, "memory:delete"),
            (AgentCapability::MemoryPromote, "memory:promote"),
            (AgentCapability::KnowledgeRead, "knowledge:read"),
            (AgentCapability::KnowledgeEdit, "knowledge:edit"),
            (AgentCapability::PolicyCreate, "policy:create"),
            (AgentCapability::GovernanceRead, "governance:read"),
            (AgentCapability::GovernanceSubmit, "governance:submit"),
            (AgentCapability::OrgRead, "org:read"),
            (AgentCapability::AgentRegister, "agent:register"),
            (AgentCapability::AgentDelegate, "agent:delegate"),
        ];

        for (capability, value) in cases {
            assert_eq!(capability.to_string(), value);
            assert_eq!(value.parse::<AgentCapability>().unwrap(), capability);
        }

        assert!("agent:teleport".parse::<AgentCapability>().is_err());
    }

    #[test]
    fn test_escalation_defaults_cover_all_tiers_and_limits() {
        let config = EscalationConfig::default();

        assert!(config.enabled);
        assert_eq!(config.initial_timeout_hours, 24);
        assert_eq!(config.reminder_intervals_hours, vec![12, 18, 23]);
        assert_eq!(config.tiers.len(), 3);
        assert_eq!(config.tiers[0].name, "Team Lead Escalation");
        assert_eq!(config.tiers[0].timeout_hours, 12);
        assert!(matches!(
            config.tiers[0].escalate_to,
            EscalationTarget::RoleInScope(RoleLevel::TechLead)
        ));
        assert_eq!(
            config.tiers[0].notification_channels,
            vec![NotificationChannel::Email]
        );
        assert_eq!(config.tiers[1].name, "Architect Escalation");
        assert_eq!(
            config.tiers[1].notification_channels,
            vec![NotificationChannel::Email, NotificationChannel::Slack]
        );
        assert_eq!(config.tiers[2].name, "Admin Escalation");
        assert_eq!(
            config.tiers[2].notification_channels,
            vec![
                NotificationChannel::Email,
                NotificationChannel::Slack,
                NotificationChannel::PagerDuty,
            ]
        );
    }

    #[test]
    fn test_agent_rate_limits_default_includes_memory_limit() {
        let limits = AgentRateLimits::default();
        assert_eq!(limits.memory_writes_per_hour, 100);
    }

    #[test]
    fn test_confirmation_reason_display_all_remaining_variants() {
        assert_eq!(
            ConfirmationReason::DelegationDepthWarning.to_string(),
            "Agent delegation depth near limit"
        );
        assert_eq!(
            ConfirmationReason::RateLimitWarning.to_string(),
            "Agent approaching rate limit"
        );
        assert_eq!(
            ConfirmationReason::CrossScopeAction.to_string(),
            "Action affects multiple scopes"
        );
        assert_eq!(
            ConfirmationReason::FirstTimeAction.to_string(),
            "First time agent performs this action"
        );
        assert_eq!(
            ConfirmationReason::AgentRequested.to_string(),
            "Agent requested human oversight"
        );
    }
}
