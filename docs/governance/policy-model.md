# Policy Model Documentation

This document describes the Aeterna governance system's policy model, Cedar integration, and role-based access control (RBAC) implementation.

## Overview

The Aeterna governance system provides hierarchical policy management with rule-based validation, Cedar authorization integration, and multi-tenant role management. Policies flow down through organizational layers and can be merged, overridden, or intersected based on defined strategies.

## Policy Structure

### Core Policy Definition

```rust
pub struct Policy {
    pub id: String,                                    // Unique policy identifier
    pub name: String,                                  // Human-readable policy name
    pub description: Option<String>,                    // Optional policy description
    pub layer: KnowledgeLayer,                         // Organizational layer
    pub mode: PolicyMode,                              // Enforcement mode
    pub merge_strategy: RuleMergeStrategy,              // How to merge with parent policies
    pub rules: Vec<PolicyRule>,                        // List of constraint rules
    pub metadata: HashMap<String, serde_json::Value>   // Additional policy metadata
}
```

### Policy Properties

| Property | Type | Description |
|----------|------|-------------|
| `id` | `String` | Unique identifier for the policy |
| `name` | `String` | Human-readable policy name |
| `description` | `Option<String>` | Optional detailed description |
| `layer` | `KnowledgeLayer` | Organizational layer (Tenant, Org, Team, Project) |
| `mode` | `PolicyMode` | Optional or Mandatory enforcement |
| `merge_strategy` | `RuleMergeStrategy` | How to combine with parent policies |
| `rules` | `Vec<PolicyRule>` | List of constraint rules |
| `metadata` | `HashMap<String, serde_json::Value>` | Additional metadata for versioning, embeddings, etc. |

## Rule Definition

### Core Rule Structure

```rust
pub struct PolicyRule {
    pub id: String,                                    // Unique rule identifier
    pub rule_type: RuleType,                            // Allow or Deny
    pub target: ConstraintTarget,                       // What the rule applies to
    pub operator: ConstraintOperator,                  // How to evaluate the constraint
    pub value: serde_json::Value,                      // Expected value or pattern
    pub severity: ConstraintSeverity,                   // Violation severity level
    pub message: String                                 // Human-readable violation message
}
```

### Rule Properties

| Property | Type | Description |
|----------|------|-------------|
| `id` | `String` | Unique identifier for the rule |
| `rule_type` | `RuleType` | Allow or Deny rule type |
| `target` | `ConstraintTarget` | Target of the constraint (File, Code, Dependency, Import, Config) |
| `operator` | `ConstraintOperator` | Evaluation operator |
| `value` | `serde_json::Value` | Expected value or pattern |
| `severity` | `ConstraintSeverity` | Severity level for violations |
| `message` | `String` | Human-readable violation message |

## Constraint Operators

### MustExist
- **Purpose**: Requires a target to exist in context
- **Use Case**: Ensuring required files or configurations are present
- **Example**: `MustExist` on `"README.md"` file

### MustNotExist
- **Purpose**: Requires a target to be absent from context
- **Use Case**: Preventing forbidden files or configurations
- **Example**: `MustNotExist` on `"secrets.txt"` file

### MustUse
- **Purpose**: Requires a specific value to be present or used
- **Use Case**: Enforcing dependency usage, requiring specific imports
- **Example**: `MustUse` `"log4j"` dependency

### MustNotUse
- **Purpose**: Prohibits a specific value from being used
- **Use Case**: Banning deprecated or unsafe dependencies
- **Example**: `MustNotUse` `"unsafe-lib"` dependency

### MustMatch
- **Purpose**: Requires content to match a regex pattern
- **Use Case**: Enforcing code style, naming conventions
- **Example**: `MustMatch` `"^# ADR"` for ADR headers

### MustNotMatch
- **Purpose**: Prohibits content from matching a regex pattern
- **Use Case**: Preventing anti-patterns in code
- **Example**: `MustNotMatch` `"console\.log"` in production code

## Constraint Targets

| Target | Description | Context Key |
|--------|-------------|-------------|
| `File` | File path validation | `"path"` |
| `Code` | Source code content validation | `"content"` |
| `Dependency` | Package dependency validation | `"dependencies"` |
| `Import` | Module import validation | `"imports"` |
| `Config` | Configuration file validation | `"config"` |

## Constraint Severity

| Severity | Description | Impact |
|----------|-------------|--------|
| `Info` | Informational violation | 0.1 drift score |
| `Warn` | Warning violation | 0.5 drift score |
| `Block` | Blocking violation | 1.0 drift score |

## Rule Types

| Type | Description | Evaluation Logic |
|------|-------------|-----------------|
| `Allow` | Condition must be met | Violation when condition is NOT met |
| `Deny` | Condition must not be met | Violation when condition IS met |

## Policy Mode

| Mode | Description | Behavior |
|------|-------------|----------|
| `Optional` | Policy is advisory | Can be overridden by lower layers |
| `Mandatory` | Policy is required | Cannot be overridden (except with Override strategy) |

## Rule Merge Strategies

### Override
- **Behavior**: Completely replaces existing policy rules
- **Use Case**: When lower layers need to completely replace parent policies
- **Precedence**: Lower layer policies take precedence

### Merge
- **Behavior**: Combines rules from both policies
- **Use Case**: When adding new rules without removing existing ones
- **Implementation**: Adds unique rules, merges metadata

### Intersect
- **Behavior**: Keeps only rules present in both policies
- **Use Case**: When policies should be more restrictive
- **Implementation**: Removes rules not present in incoming policy

## Knowledge Layer Hierarchy

Policies flow down through organizational layers:

```
Tenant (highest precedence)
    ↓
Organization
    ↓
Team
    ↓
Project (lowest precedence)
```

### Layer Precedence
- Tenant policies have the highest precedence
- Project policies have the lowest precedence
- Lower layers can override higher layers using appropriate merge strategies
- Mandatory policies from higher layers cannot be overridden (except with Override strategy)

## Role-Based Access Control (RBAC)

### Role Hierarchy

| Role | Precedence | Display Name | Description |
|------|------------|--------------|-------------|
| `Admin` | 4 | Admin | Full system access |
| `Architect` | 3 | Architect | System architecture and policy design |
| `TechLead` | 2 | Tech Lead | Technical leadership and team management |
| `Developer` | 1 | Developer | Standard development access |
| `Agent` | 0 | Agent | AI agent with delegated permissions |

### Role Permissions

Roles determine access levels for different operations:
- **Admin**: Can manage all aspects of the system
- **Architect**: Can design and modify policies, manage knowledge repository
- **TechLead**: Can manage team resources, enforce policies
- **Developer**: Standard development and knowledge access
- **Agent**: Delegated permissions based on user context

## Cedar Integration

### Cedar Schema Definition

The system uses Cedar for authorization with the following schema:

```cedar
entity type User;

entity type Unit {
    parent: Unit,
    tenant_id: String
};

entity type Role {
    name: String
};

action ViewKnowledge, EditKnowledge, DeleteKnowledge, AdministerUnit;
```

### Authorization Rules

```cedar
// Allow users to view knowledge if they are members of the unit (or parent units)
permit (
    principal,
    action == Action::"ViewKnowledge",
    resource
)
when {
    principal in resource.members
};

// Allow Unit Admins to administer the unit
permit (
    principal,
    action == Action::"AdministerUnit",
    resource
)
when {
    principal has role && principal.role == Role::"UnitAdmin"
};
```

### Cedar Authorizer Implementation

The `CedarAuthorizer` provides:
- Permission checking for users and agents
- Delegation support for agent actions
- Integration with the Cedar policy engine
- Entity and policy management

### Alternative: Permit.io Integration

For external authorization, the system supports Permit.io:
- RESTful API integration
- Role management
- Permission checking
- Multi-tenant support

## Policy Evaluation

### Evaluation Process

1. **Policy Resolution**: Gather policies from all applicable layers
2. **Policy Merging**: Apply merge strategies to combine policies
3. **Rule Evaluation**: Evaluate each rule against the context
4. **Violation Collection**: Collect all rule violations
5. **Result Generation**: Return validation result with violations

### Context Structure

Policies are evaluated against a context containing:
```json
{
    "path": "file/path/to/validate",
    "content": "file or code content",
    "dependencies": ["dep1", "dep2"],
    "imports": ["module1", "module2"],
    "config": { "key": "value" },
    "unitId": "organizational-unit-id",
    "projectId": "project-identifier"
}
```

### Validation Result

```rust
pub struct ValidationResult {
    pub is_valid: bool,                              // Overall validation status
    pub violations: Vec<PolicyViolation>              // List of violations
}

pub struct PolicyViolation {
    pub rule_id: String,                             // Rule that was violated
    pub policy_id: String,                           // Policy containing the rule
    pub severity: ConstraintSeverity,                 // Violation severity
    pub message: String,                              // Violation message
    pub context: HashMap<String, serde_json::Value>   // Evaluation context
}
```

## Drift Detection

The system provides drift detection capabilities:
- Semantic similarity analysis using embeddings
- LLM-based drift analysis
- Policy version checking
- Continuous monitoring

### Drift Score Calculation

Drift scores are calculated based on violation severity:
- `Block`: 1.0 points
- `Warn`: 0.5 points
- `Info`: 0.1 points

Final drift score is the minimum of the total score and 1.0.

## Policy Examples

### Security Baseline Policy

```rust
let security_policy = Policy {
    id: "security-baseline".to_string(),
    name: "Security Baseline".to_string(),
    description: Some("Core security requirements".to_string()),
    layer: KnowledgeLayer::Tenant,
    mode: PolicyMode::Mandatory,
    merge_strategy: RuleMergeStrategy::Merge,
    rules: vec![
        PolicyRule {
            id: "no-unsafe-deps".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustNotUse,
            value: serde_json::json!("unsafe-lib"),
            severity: ConstraintSeverity::Block,
            message: "unsafe-lib is prohibited for security reasons".to_string(),
        },
        PolicyRule {
            id: "require-readme".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::File,
            operator: ConstraintOperator::MustExist,
            value: serde_json::json!("README.md"),
            severity: ConstraintSeverity::Warn,
            message: "Project must have a README.md file".to_string(),
        }
    ],
    metadata: HashMap::new(),
};
```

### Dependency Allowlist Policy

```rust
let dependency_policy = Policy {
    id: "dependency-allowlist".to_string(),
    name: "Approved Dependencies".to_string(),
    description: Some("Only approved dependencies may be used".to_string()),
    layer: KnowledgeLayer::Org,
    mode: PolicyMode::Mandatory,
    merge_strategy: RuleMergeStrategy::Intersect,
    rules: vec![
        PolicyRule {
            id: "approved-deps".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Dependency,
            operator: ConstraintOperator::MustUse,
            value: serde_json::json!(["serde", "tokio", "reqwest"]),
            severity: ConstraintSeverity::Block,
            message: "Only approved dependencies are allowed".to_string(),
        }
    ],
    metadata: HashMap::new(),
};
```

### Code Pattern Enforcement Policy

```rust
let code_pattern_policy = Policy {
    id: "code-patterns".to_string(),
    name: "Code Pattern Enforcement".to_string(),
    description: Some("Enforces coding standards and patterns".to_string()),
    layer: KnowledgeLayer::Team,
    mode: PolicyMode::Optional,
    merge_strategy: RuleMergeStrategy::Merge,
    rules: vec![
        PolicyRule {
            id: "rust-error-handling".to_string(),
            rule_type: RuleType::Allow,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustMatch,
            value: serde_json::json!(r"Result<.*, .*>|Option<.*>"),
            severity: ConstraintSeverity::Warn,
            message: "Use Result or Option for error handling".to_string(),
        },
        PolicyRule {
            id: "no-debug-prints".to_string(),
            rule_type: RuleType::Deny,
            target: ConstraintTarget::Code,
            operator: ConstraintOperator::MustNotMatch,
            value: serde_json::json!(r"println!|dbg!|debug_print!"),
            severity: ConstraintSeverity::Info,
            message: "Remove debug print statements".to_string(),
        }
    ],
    metadata: HashMap::new(),
};
```

## Governance Events

The system emits governance events for auditing and monitoring:

### Event Types

| Event | Description |
|-------|-------------|
| `UnitCreated` | New organizational unit created |
| `UnitUpdated` | Organizational unit updated |
| `UnitDeleted` | Organizational unit deleted |
| `RoleAssigned` | Role assigned to user |
| `RoleRemoved` | Role removed from user |
| `PolicyUpdated` | Policy created or updated |
| `PolicyDeleted` | Policy deleted |
| `DriftDetected` | Drift detected in project |

### Event Structure

```rust
pub enum GovernanceEvent {
    UnitCreated { unit_id: String, unit_type: UnitType, tenant_id: TenantId, parent_id: Option<String>, timestamp: i64 },
    RoleAssigned { user_id: UserId, unit_id: String, role: Role, tenant_id: TenantId, timestamp: i64 },
    PolicyUpdated { policy_id: String, layer: KnowledgeLayer, tenant_id: TenantId, timestamp: i64 },
    DriftDetected { project_id: String, tenant_id: TenantId, drift_score: f32, timestamp: i64 },
    // ... other event types
}
```

## Implementation Notes

### Performance Considerations

- Policy evaluation is optimized for hierarchical resolution
- Caching is used for frequently accessed policies
- Batch evaluation is supported for multiple validations
- Embedding-based analysis is optional and configurable

### Extensibility

- Custom constraint operators can be added via trait implementations
- New authorization providers can implement the `AuthorizationService` trait
- Policy merge strategies are extensible through the `RuleMergeStrategy` enum
- Event publishers can be customised via the `EventPublisher` trait

### Error Handling

- Policy validation errors are captured in `ValidationResult`
- Authorization errors are type-safe through the `AuthorizationService` trait
- Drift analysis failures are logged but don't prevent operation
- Schema validation errors are caught early in the policy loading process

## Best Practices

### Policy Design

1. **Start with tenant-level mandatory policies** for core requirements
2. **Use org-level policies** for department-specific constraints
3. **Apply team-level policies** for coding standards and patterns
4. **Reserve project-level policies** for project-specific requirements
5. **Use appropriate merge strategies** to balance control and flexibility

### Rule Design

1. **Prefer Allow rules** for positive requirements
2. **Use Deny rules** for clear prohibitions
3. **Set appropriate severity levels** based on impact
4. **Write clear, actionable violation messages**
5. **Use specific targets** to minimize evaluation overhead

### Cedar Integration

1. **Define clear entity hierarchies** in the schema
2. **Use role-based access** for scalable authorization
3. **Implement delegation** for agent permissions
4. **Validate policies** before deployment
5. **Monitor authorization failures** for security insights

This policy model provides a comprehensive framework for governing AI agent behavior, ensuring compliance with organizational standards while maintaining flexibility for diverse use cases.