# Governance System API Reference

This document provides comprehensive API documentation for the governance system in the Aeterna Memory-Knowledge System.

## Overview

The governance system provides policy management, validation, and drift detection capabilities for the Memory-Knowledge System. It supports multi-tenancy through tenant contexts and three deployment modes: Local, Hybrid, and Remote.

## Deployment Modes

### Local Mode

All governance operations are performed locally without remote communication. Requires a `GovernanceEngine` instance.

### Hybrid Mode

Combines local governance with remote synchronization. Uses local cache and syncs changes to remote governance service. Requires both `GovernanceEngine` instance and `remote_url` configuration.

### Remote Mode

All operations are performed through a remote governance service. Requires `remote_url` configuration.

## MCP Tools

The governance system exposes the following MCP tools for agent interaction:

### 1. governance_unit_create

Creates a new organizational unit (Organization, Team, or Project).

**Parameters:**

- `name` (string, required): Name of the unit
- `unit_type` (string, required): Type of the unit. Values: `organization`, `team`, `project`
- `parent_id` (string, optional): Parent unit ID
- `tenantContext` (TenantContext, optional): Tenant context information
- `metadata` (object, optional): Optional metadata

**Response:**

```json
{
    "success": true,
    "unit_id": "uuid-string"
}
```

**Example Usage:**

```rust
let params = json!({
    "name": "Engineering Team",
    "unit_type": "team",
    "parent_id": "org-123",
    "tenantContext": {
        "tenantId": "acme-corp",
        "userId": "user-456"
    }
});

let result = tool.call(params).await?;
```

### 2. governance_policy_add

Adds or updates a policy for an organizational unit.

**Parameters:**

- `unit_id` (string, required): Unit ID to attach policy to
- `policy` (object, required): Policy definition
- `tenantContext` (TenantContext, optional): Tenant context information

**Response:**

```json
{
    "success": true,
    "policy_id": "policy-uuid"
}
```

**Example Usage:**

```rust
let policy = Policy {
    id: "security-policy".to_string(),
    name: "Security Standards".to_string(),
    description: Some("Security constraints for all projects".to_string()),
    layer: KnowledgeLayer::Tenant,
    mode: PolicyMode::Mandatory,
    merge_strategy: RuleMergeStrategy::Merge,
    rules: vec![
        PolicyRule {
            id: "no-unsafe-deps".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: json!("unsafe-lib"),
            severity: ConstraintSeverity::Block,
            message: "Unsafe libraries are forbidden".to_string(),
        }
    ],
    metadata: HashMap::new(),
};

let params = json!({
    "unit_id": "unit-123",
    "policy": policy,
    "tenantContext": {
        "tenantId": "acme-corp",
        "userId": "user-456"
    }
});

let result = tool.call(params).await?;
```

### 3. governance_role_assign

Assigns a role to a user within a specific organizational unit.

**Parameters:**

- `user_id` (string, required): User ID
- `unit_id` (string, required): Unit ID
- `role` (string, required): Role to assign. Values: `platform_admin`, `tenant_admin`, `admin`, `architect`, `tech_lead`, `developer`, `viewer`, `agent`
- `tenantContext` (TenantContext, optional): Tenant context information

**Response:**

```json
{
    "success": true
}
```

**Example Usage:**

```rust
let params = json!({
    "user_id": "user-456",
    "unit_id": "team-789",
    "role": "developer",
    "tenantContext": {
        "tenantId": "acme-corp",
        "userId": "user-456"
    }
});

let result = tool.call(params).await?;
```

### 4. governance_role_remove

Removes a role from a user within a specific organizational unit.

**Parameters:**

- `user_id` (string, required): User ID
- `unit_id` (string, required): Unit ID
- `role` (string, required): Role to remove. Values: `platform_admin`, `tenant_admin`, `admin`, `architect`, `tech_lead`, `developer`, `viewer`, `agent`
- `tenantContext` (TenantContext, optional): Tenant context information

**Response:**

```json
{
    "success": true
}
```

**Example Usage:**

```rust
let params = json!({
    "user_id": "user-456",
    "unit_id": "team-789",
    "role": "developer",
    "tenantContext": {
        "tenantId": "acme-corp",
        "userId": "user-456"
    }
});

let result = tool.call(params).await?;
```

### 5. governance_hierarchy_navigate

Navigates the organizational hierarchy (ancestors or descendants) for a unit.

**Parameters:**

- `unit_id` (string, required): Starting Unit ID
- `direction` (string, required): Navigation direction. Values: `ancestors`, `descendants`
- `tenantContext` (TenantContext, optional): Tenant context information

**Response:**

```json
{
    "success": true,
    "units": [
        {
            "id": "unit-123",
            "name": "Parent Unit",
            "unitType": "organization",
            "parentId": "unit-456",
            "tenantId": "acme-corp",
            "metadata": {},
            "createdAt": 1640995200,
            "updatedAt": 1640995200
        }
    ]
}
```

**Example Usage:**

```rust
let params = json!({
    "unit_id": "team-789",
    "direction": "ancestors",
    "tenantContext": {
        "tenantId": "acme-corp",
        "userId": "user-456"
    }
});

let result = tool.call(params).await?;
```

## Core APIs

### GovernanceEngine

The main engine for policy validation and drift detection.

#### Methods

##### add_policy(policy: Policy)

Adds a policy to the engine for validation.

**Parameters:**

- `policy`: Policy object with rules and metadata

**Example:**

```rust
let mut engine = GovernanceEngine::new();
engine.add_policy(security_policy);
```

##### validate(target_layer: KnowledgeLayer, context: &HashMap<String, serde_json::Value>) -> ValidationResult

Validates content against policies for a specific knowledge layer.

**Parameters:**

- `target_layer`: The knowledge layer to validate against
- `context`: HashMap containing content and metadata to validate

**Returns:**

- `ValidationResult`: Contains validation status and any violations

**Example:**

```rust
let mut context = HashMap::new();
context.insert("dependencies".to_string(), json!(["safe-lib", "unsafe-lib"]));
context.insert("content".to_string(), json!("# ADR 001\n..."));

let result = engine.validate(KnowledgeLayer::Project, &context);
if !result.is_valid {
    for violation in &result.violations {
        println!("Violation: {} - {}", violation.rule_id, violation.message);
    }
}
```

##### validate_with_context(target_layer: KnowledgeLayer, context: &HashMap<String, serde_json::Value>, tenant_ctx: Option<&TenantContext>) -> ValidationResult

Enhanced validation with tenant context and hierarchy support.

**Parameters:**

- `target_layer`: The knowledge layer to validate against
- `context`: HashMap containing content and metadata to validate
- `tenant_ctx`: Optional tenant context for hierarchical policy resolution

**Returns:**

- `ValidationResult`: Contains validation status and any violations

##### check_drift(tenant_ctx: &TenantContext, project_id: &str, context: &HashMap<String, serde_json::Value>) -> Result<f32, anyhow::Error>

Analyzes content for policy drift and returns a drift score (0.0-1.0).

**Parameters:**

- `tenant_ctx`: Tenant context
- `project_id`: Project identifier
- `context`: HashMap containing content and metadata to analyze

**Returns:**

- `f32`: Drift score where 0.0 = no drift, 1.0 = maximum drift

**Example:**

```rust
let mut context = HashMap::new();
context.insert("content".to_string(), json!("some code content"));
context.insert("version_hash".to_string(), json!("abc123"));

let drift_score = engine.check_drift(&tenant_ctx, "project-456", &context).await?;
if drift_score > 0.5 {
    println!("High drift detected: {:.2}", drift_score);
}
```

### GovernanceClient

Client interface for governance operations across deployment modes.

#### Methods

##### validate(ctx: &TenantContext, layer: KnowledgeLayer, context: &HashMap<String, serde_json::Value>) -> Result<ValidationResult>

Validates content against policies using the configured deployment mode.

##### get_drift_status(ctx: &TenantContext, project_id: &str) -> Result<Option<DriftResult>>

Retrieves the latest drift analysis result for a project.

##### list_proposals(ctx: &TenantContext, layer: Option<KnowledgeLayer>) -> Result<Vec<KnowledgeEntry>>

Lists governance proposals, optionally filtered by knowledge layer.

##### replay_events(ctx: &TenantContext, since_timestamp: i64, limit: usize) -> Result<Vec<GovernanceEvent>>

Replays governance events since a given timestamp for auditing.

## Data Structures

### Policy

```rust
pub struct Policy {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub layer: KnowledgeLayer,
    pub mode: PolicyMode,
    pub merge_strategy: RuleMergeStrategy,
    pub rules: Vec<PolicyRule>,
    pub metadata: HashMap<String, serde_json::Value>
}
```

### PolicyRule

```rust
pub struct PolicyRule {
    pub id: String,
    pub rule_type: RuleType,
    pub target: ConstraintTarget,
    pub operator: ConstraintOperator,
    pub value: serde_json::Value,
    pub severity: ConstraintSeverity,
    pub message: String
}
```

### PolicyViolation

```rust
pub struct PolicyViolation {
    pub rule_id: String,
    pub policy_id: String,
    pub severity: ConstraintSeverity,
    pub message: String,
    pub context: HashMap<String, serde_json::Value>
}
```

### ValidationResult

```rust
pub struct ValidationResult {
    pub is_valid: bool,
    pub violations: Vec<PolicyViolation>
}
```

### DriftResult

```rust
pub struct DriftResult {
    pub project_id: String,
    pub tenant_id: TenantId,
    pub drift_score: f32,
    pub violations: Vec<PolicyViolation>,
    pub timestamp: i64
}
```

### TenantContext

```rust
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub agent_id: Option<String>
}
```

### OrganizationalUnit

```rust
pub struct OrganizationalUnit {
    pub id: String,
    pub name: String,
    pub unit_type: UnitType,
    pub parent_id: Option<String>,
    pub tenant_id: TenantId,
    pub metadata: HashMap<String, serde_json::Value>,
    pub created_at: i64,
    pub updated_at: i64
}
```

## Enums

### KnowledgeLayer

- `Tenant`: Tenant-level policies
- `Org`: Organization-level policies
- `Team`: Team-level policies
- `Project`: Project-level policies

### ConstraintSeverity

- `Info`: Informational violation (0.1 drift weight)
- `Warn`: Warning violation (0.5 drift weight)
- `Block`: Blocking violation (1.0 drift weight)

### ConstraintOperator

- `MustUse`: Content must use specified value
- `MustNotUse`: Content must not use specified value
- `MustMatch`: Content must match regex pattern
- `MustNotMatch`: Content must not match regex pattern
- `MustExist`: Content must exist
- `MustNotExist`: Content must not exist

### ConstraintTarget

- `File`: Target is file path
- `Code`: Target is code content
- `Dependency`: Target is dependency list
- `Import`: Target is import statements
- `Config`: Target is configuration

### Role

- `PlatformAdmin`: Cross-tenant admin role (precedence: 6)
- `TenantAdmin`: Tenant-scoped admin role (precedence: 5)
- `Viewer`: Read-only tenant role
- `Developer`: Basic developer role (precedence: 1)
- `TechLead`: Tech lead role (precedence: 2)
- `Architect`: Architect role (precedence: 3)
- `Admin`: Admin role (precedence: 4)
- `Agent`: AI agent role (precedence: 0)

### UnitType

- `Organization`: Organization unit
- `Team`: Team unit
- `Project`: Project unit

## Authentication Requirements

All governance API calls require tenant context authentication:

1. **Tenant ID**: Identifies the tenant/organization
2. **User ID**: Identifies the user making the request
3. **Agent ID** (optional): Identifies the AI agent if applicable

Headers for remote mode:

```
X-Tenant-Id: your-tenant-id
X-User-Id: your-user-id
```

## Error Handling

### MCP Tool Errors

- `-32602`: Invalid input parameters
- `-32601`: Tool not found
- `-32000`: Internal server error
- `-32001`: Request timeout

### GovernanceClient Errors

- `Network(reqwest::Error)`: Network connectivity issues
- `Api(String)`: API-specific errors from remote service
- `Serialization(serde_json::Error)`: JSON serialization/deserialization errors
- `Internal(String)`: Internal implementation errors
- `RemoteUnavailable`: Remote service unavailable (using cached data)
- `SyncConflict(String)`: Synchronization conflicts between local and remote

## Multi-Tenancy

The governance system supports multi-tenancy through:

1. **Tenant Isolation**: Each tenant has separate policies, units, and roles
2. **Hierarchical Inheritance**: Policies inherit from parent units within the same tenant
3. **Role-Based Access**: Users can have different roles across different units and tenants

### Tenant Context Creation

```rust
use mk_core::types::{TenantId, UserId, TenantContext};

let tenant_id = TenantId::new("acme-corp".to_string()).unwrap();
let user_id = UserId::new("john-doe".to_string()).unwrap();
let ctx = TenantContext::new(tenant_id, user_id);

// With agent context
let agent_ctx = TenantContext::with_agent(
    tenant_id,
    user_id,
    "agent-assistant-1".to_string()
);
```

## Policy Inheritance and Resolution

Policies are resolved hierarchically from Tenant → Org → Team → Project layers:

1. **Collection**: Gather all applicable policies from hierarchy
2. **Merging**: Apply merge strategies (Override, Merge, Intersect)
3. **Validation**: Evaluate rules against content

### Merge Strategies

- `Override`: Higher-level policies completely override lower-level ones
- `Merge`: Combine rules from all levels, higher-level takes precedence on conflicts
- `Intersect`: Keep only rules that exist in all levels

### Example Policy Resolution

```
Tenant Policy (Mandatory):
- Rule A: Must not use unsafe dependencies

Team Policy (Optional):
- Rule B: Must use specific logging library

Project Policy (Optional):
- Rule A: Must not use unsafe dependencies (inherited)
- Rule B: Must use specific logging library (inherited)
```

## Drift Detection

Drift detection analyzes content for policy violations and semantic contradictions:

### Drift Score Calculation

```
Drift Score = (sum of violation weights) / (maximum possible score)

Violation Weights:
- Info violations: 0.1
- Warn violations: 0.5
- Block violations: 1.0
```

### Analysis Types

1. **Rule-based**: Evaluates content against defined policy rules
2. **Semantic**: Uses embeddings to detect contradictions with policy intent
3. **LLM-based**: Uses language models for complex reasoning about violations

## Usage Examples

### Complete Policy Management Workflow

```rust
use knowledge::governance::GovernanceEngine;
use mk_core::types::*;

// 1. Create governance engine
let mut engine = GovernanceEngine::new();

// 2. Define security policy
let security_policy = Policy {
    id: "tenant-security".to_string(),
    name: "Tenant Security Standards".to_string(),
    description: Some("Security constraints for all projects".to_string()),
    layer: KnowledgeLayer::Tenant,
    mode: PolicyMode::Mandatory,
    merge_strategy: RuleMergeStrategy::Merge,
    rules: vec![
        PolicyRule {
            id: "no-unsafe-deps".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: json!("unsafe-lib"),
            severity: ConstraintSeverity::Block,
            message: "Unsafe libraries are forbidden".to_string(),
        },
        PolicyRule {
            id: "adr-format".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: json!("^# ADR"),
            severity: ConstraintSeverity::Warn,
            message: "ADRs must start with '# ADR'".to_string(),
        }
    ],
    metadata: HashMap::new(),
};

// 3. Add policy to engine
engine.add_policy(security_policy);

// 4. Validate content
let mut context = HashMap::new();
context.insert("dependencies".to_string(), json!(["safe-lib", "another-safe-lib"]));
context.insert("content".to_string(), json!("# ADR 001: Use Rust\n..."));

let result = engine.validate(KnowledgeLayer::Project, &context);
if result.is_valid {
    println!("Content passes all policy checks");
} else {
    for violation in &result.violations {
        println!("Violation: {} - {}", violation.rule_id, violation.message);
    }
}

// 5. Check for drift
let tenant_ctx = TenantContext::new(
    TenantId::new("acme-corp".to_string()).unwrap(),
    UserId::new("developer-123".to_string()).unwrap()
);

let drift_score = engine.check_drift(
    &tenant_ctx,
    "my-project",
    &context
).await?;

println!("Drift score: {:.2}", drift_score);
```

### Client Creation for Different Deployment Modes

```rust
use knowledge::governance_client::{create_governance_client, GovernanceClientKind};
use config::DeploymentConfig;

// Local mode
let local_config = DeploymentConfig {
    mode: "local".to_string(),
    remote_url: None,
    sync_enabled: true,
};
let local_client = create_governance_client(&local_config, Some(engine))?;

// Hybrid mode
let hybrid_config = DeploymentConfig {
    mode: "hybrid".to_string(),
    remote_url: Some("https://governance.example.com".to_string()),
    sync_enabled: true,
};
let hybrid_client = create_governance_client(&hybrid_config, Some(engine))?;

// Remote mode
let remote_config = DeploymentConfig {
    mode: "remote".to_string(),
    remote_url: Some("https://governance.example.com".to_string()),
    sync_enabled: false,
};
let remote_client = create_governance_client(&remote_config, None)?;
```

This API reference provides comprehensive coverage of the governance system's capabilities for policy management, validation, and drift detection in the Aeterna Memory-Knowledge System.
