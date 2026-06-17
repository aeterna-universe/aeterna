pub mod account_store;
pub mod approval_workflow;
pub mod budget_storage;
pub mod cascade;
pub mod code_graph;
pub mod dead_letter;
pub mod encryption;
pub mod events;
pub mod gdpr;
pub mod git_provider_connection_store;
pub mod governance;
pub mod graph;
pub mod graph_duckdb;
pub mod graph_event_log;
pub mod graph_projector;
pub mod graph_verify;
pub mod graphrag;
pub mod hierarchy_store;
pub mod kms;
pub mod kms_integration;
pub mod meta_governance;
pub mod migrations;
pub mod observability;
pub mod policy_evaluator;
pub mod positional_index;
pub mod postgres;
pub mod query_builder;
pub mod quota;
pub mod reconciliation;
pub mod redis;
pub mod redis_store;
pub mod redis_version_guard;
pub mod remediation_store;
pub mod repo_manager;
pub mod retention;
pub mod rlm_weights;
pub mod rls_migration;
pub mod secret_backend;
pub mod secret_provider;
pub mod shard_manager;
pub mod shard_router;
pub mod tenant_config_provider;
pub mod tenant_router;
pub mod tenant_store;

// Re-export Redis lock types for job coordination
pub use account_store::AccountStore;
pub use redis::{JobSkipReason, LockResult};

// Re-export generic Redis-backed store
pub use redis_store::RedisStore;

// Re-export budget storage types
pub use budget_storage::{BudgetStorage, BudgetStorageError, StoredBudget, StoredUsage};

// Re-export governance types
pub use governance::{
    ApprovalDecision, ApprovalMode, ApprovalRequest, AuditExtensions, AuditFilters,
    CreateApprovalRequest, CreateDecision, CreateGovernanceRole, Decision, GovernanceAuditEntry,
    GovernanceConfig, GovernanceRole, GovernanceStorage, PrincipalType, RequestFilters,
    RequestStatus, RequestType, RiskLevel,
};

// Re-export approval workflow state machine
pub use approval_workflow::{
    ApprovalDecisionRecord, ApprovalEvent, ApprovalModeKind, ApprovalWorkflow,
    ApprovalWorkflowContext, RiskLevelKind, WorkflowError, WorkflowState, create_workflow,
};

// Re-export meta-governance types
pub use meta_governance::{
    ActionPermission, AgentCapability, AgentDelegationConfig, AgentRateLimits, AuthorizationResult,
    ConfirmationReason, ConfirmationStatus, EscalationConfig, EscalationFallback, EscalationTarget,
    EscalationTier, GovernanceActionType, GovernanceLayer, HumanConfirmationRequest,
    MetaGovernancePolicy, MetaGovernanceStorage, NotificationChannel, RoleLevel,
    create_default_policies,
};

// Re-export Git provider connection store types
pub use git_provider_connection_store::{
    GitProviderConnectionError, InMemoryGitProviderConnectionStore,
};

// Re-export RLM weight storage types
pub use rlm_weights::{
    PostgresRlmWeightStorage, RlmWeightStorage, RlmWeightStorageError, StoredPolicyState,
};

// Re-export encryption types
pub use encryption::{EncryptedData, EncryptionConfig, EncryptionError, EncryptionManager};

// Re-export KMS types
pub use kms_integration::{
    AwsKmsProvider, KmsClient, KmsConfig, KmsError, KmsKeyMetadata, KmsProvider, LocalKmsProvider,
};

// Re-export cascade delete types
pub use cascade::{CascadeDeleter, CascadeError, CascadeReport, TenantPurgeReport};

// Re-export GDPR types
pub use gdpr::{
    AnonymizationStrategy, GdprAuditLog, GdprConsent, GdprError, GdprOperations,
    PostgresGdprStorage, UserDataExport,
};

// Re-export tenant sharding types
pub use repo_manager::{
    CleanupLog, CreateRepository, CreateRequest, IndexMetadata, RepoRequest, RepoRequestStatus,
    RepoStorage, Repository, RepositoryStatus, RepositoryType, SyncStrategy, UsageMetrics,
};
pub use shard_manager::{ShardError, ShardInfo, ShardManager, ShardStatistics, ShardStatus};
pub use tenant_router::{TenantRouter, TenantShard, TenantSize};
