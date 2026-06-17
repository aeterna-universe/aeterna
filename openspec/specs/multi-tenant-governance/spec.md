# multi-tenant-governance Specification

## Purpose
TBD - created by archiving change add-multi-tenant-governance. Update Purpose after archive.
## Requirements
### Requirement: Tenant Isolation

The system SHALL enforce hard isolation at the tenant boundary for all memory and knowledge operations.

Each tenant MUST have:
- Unique tenant identifier
- Isolated data storage (logical or physical)
- Independent configuration

#### Scenario: Tenant boundary enforcement
- **WHEN** a user from Tenant A attempts to access data belonging to Tenant B
- **THEN** the system SHALL reject the request with an authorization error
- **AND** the system SHALL NOT reveal whether the target resource exists

#### Scenario: Cross-tenant search isolation
- **WHEN** a user performs a vector similarity search
- **THEN** the system SHALL only return results from the user's tenant
- **AND** embeddings from other tenants SHALL NOT influence the search results

#### Scenario: Tenant configuration segregation
- **WHEN** the system stores or mutates tenant-specific configuration and secret references
- **THEN** each tenant SHALL have an independently addressable configuration surface bound to that tenant's unique identifier
- **AND** GlobalAdmin and TenantAdmin mutations SHALL be restricted to the tenant and ownership scope authorized for the caller
- **AND** the system SHALL reject tenant configuration references that cross tenant boundaries or expose raw secret values outside the tenant's approved secret container

#### Scenario: Shared provider connections respect tenant visibility boundaries
- **WHEN** the system exposes a platform-managed Git provider connection to one or more tenants
- **THEN** only explicitly allowed tenants SHALL be able to reference that connection in tenant configuration
- **AND** tenants that are not granted visibility SHALL receive an authorization or validation error without disclosure of hidden connection details

### Requirement: Hierarchical Organization Structure

The system SHALL support a four-level organizational hierarchy within each tenant:
1. Tenant (root scope)
2. Organization (business unit)
3. Team (working group)
4. Project (codebase/repository)

The hierarchy MUST be bootstrappable from an external identity provider (GitHub, Okta, Azure AD) via the IdP sync framework, in addition to manual creation via the API.

#### Scenario: Hierarchy navigation
- **WHEN** a user queries for knowledge at the Team level
- **THEN** the system SHALL include inherited knowledge from Organization and Tenant levels
- **AND** the system SHALL mark each result with its originating hierarchy level

#### Scenario: Hierarchy creation
- **WHEN** an admin creates a new Team under an Organization
- **THEN** the Team SHALL inherit default policies from the parent Organization
- **AND** the Team SHALL be visible to all Organization members with appropriate permissions

#### Scenario: IdP-bootstrapped hierarchy
- **WHEN** an IdP sync provider (GitHub, Okta, or Azure AD) is configured
- **THEN** the system SHALL create the Tenant, Organization, and Team units automatically from the IdP's group/team structure
- **AND** manually-created units SHALL coexist with IdP-synced units without conflict
- **AND** IdP-synced units SHALL be tagged with their source provider for audit purposes

### Requirement: Relationship-Based Access Control

The system SHALL implement Cedar-based Role-Based Access Control with 8 roles (Viewer, Agent, Developer, TechLead, Architect, Admin, TenantAdmin, PlatformAdmin) for fine-grained permissions within and across tenants.

Supported roles:
- **Viewer**: Read-only access to memories, knowledge, policies, governance requests, and organization structure
- **Agent**: Inherits permissions from the user it acts on behalf of
- **Developer**: Can add memories, propose knowledge, view resources, register and delegate to agents
- **Tech Lead**: Can approve promotions, manage team knowledge, manage members
- **Architect**: Can reject proposals, force corrections, review drift, create policies
- **Admin**: Full tenant management access (legacy alias for TenantAdmin)
- **TenantAdmin**: Explicit full tenant-scoped administration, tenant config/secrets, role delegation within tenant
- **PlatformAdmin**: Cross-tenant super admin, tenant lifecycle, shared Git provider connections

#### Scenario: Role-based knowledge approval
- **WHEN** a Developer proposes promoting a memory to team knowledge
- **THEN** a Tech Lead or Architect from that team or higher hierarchy MUST approve
- **AND** the Developer SHALL NOT be able to self-approve their own proposals

#### Scenario: Architect rejection with feedback
- **WHEN** an Architect rejects a knowledge proposal
- **THEN** the system SHALL require a rejection reason
- **AND** the system SHALL notify the proposer with the rejection reason
- **AND** the proposal status SHALL change to "rejected"

#### Scenario: LLM agent as architect
- **WHEN** an LLM agent is configured with Architect role
- **THEN** the agent SHALL be able to approve or reject proposals programmatically
- **AND** all agent actions SHALL be audited with the agent's identity

#### Scenario: Viewer role has read-only access
- **WHEN** a user has the Viewer role
- **THEN** the user SHALL be able to view memories, knowledge, policies, governance requests, and organization structure
- **AND** the user SHALL NOT be able to create, modify, delete, or approve any resource

#### Scenario: TenantAdmin manages tenant-scoped resources
- **WHEN** a user has the TenantAdmin role on a specific tenant
- **THEN** the user SHALL have full administrative access within that tenant
- **AND** the user SHALL be able to manage tenant config, secrets, hierarchy, and role assignments within the tenant
- **AND** the user SHALL NOT have access to other tenants or platform-wide operations

### Requirement: Governance Event Streaming

The system SHALL emit real-time events for all governance-related actions via Redis Streams.

Event types:
- `KnowledgeProposed`: New knowledge proposal submitted
- `KnowledgeApproved`: Proposal approved by authorized role
- `KnowledgeRejected`: Proposal rejected with reason
- `MemoryPromoted`: Memory promoted to higher layer
- `DriftDetected`: Semantic drift identified
- `PolicyViolation`: Policy compliance failure

#### Scenario: Real-time governance notification
- **WHEN** a knowledge proposal is submitted
- **THEN** the system SHALL emit a `KnowledgeProposed` event within 100ms
- **AND** all subscribed governance dashboards SHALL receive the event

#### Scenario: Event persistence
- **WHEN** a governance event is emitted
- **THEN** the event SHALL be persisted for audit purposes
- **AND** the event SHALL include timestamp, actor, action, and affected resources

### Requirement: Semantic Drift Detection

The system SHALL detect semantic drift between project-level knowledge/memories and higher-level organizational standards.

Drift types:
- **Contradicting**: Project knowledge contradicts organization policy
- **Missing**: Required policies not present at project level
- **Stale**: References to deprecated or outdated knowledge
- **Pattern Deviation**: Significant deviation from approved patterns

#### Scenario: Real-time contradiction detection
- **WHEN** a new knowledge item is created that contradicts an existing higher-level policy
- **THEN** the system SHALL flag the item with a drift warning
- **AND** the system SHALL calculate a drift score based on semantic similarity

#### Scenario: Missing policy detection
- **WHEN** a project lacks required policies defined at the organization level
- **THEN** the system SHALL identify the missing policies during sync
- **AND** the system SHALL generate a compliance report

#### Scenario: Drift score calculation
- **WHEN** drift detection runs for a project
- **THEN** the system SHALL calculate: `drift_score = sum(severity_weight * item_drift) / total_items`
- **AND** `item_drift = 1 - cosine_similarity(project_embedding, reference_embedding)`

### Requirement: Batch Governance Analysis

The system SHALL perform scheduled batch analysis for complex governance checks that require LLM-based semantic comparison.

Schedule:
- **Hourly**: Quick drift scan for active projects
- **Daily**: Full drift analysis with LLM semantic comparison
- **Weekly**: Comprehensive governance report generation

#### Scenario: Hourly drift scan
- **WHEN** the hourly batch job runs
- **THEN** the system SHALL scan all projects modified in the last hour
- **AND** the system SHALL perform vector-based drift detection only (no LLM calls)

#### Scenario: Daily semantic analysis
- **WHEN** the daily batch job runs
- **THEN** the system SHALL perform LLM-based semantic comparison for flagged items
- **AND** the system SHALL update drift scores with refined analysis
- **AND** the system SHALL cache results to avoid redundant LLM calls

#### Scenario: Weekly governance report
- **WHEN** the weekly batch job runs
- **THEN** the system SHALL generate a governance report per organization
- **AND** the report SHALL include drift trends, approval rates, and policy compliance

### Requirement: Deployment Mode Support

The system SHALL support three deployment modes to accommodate different usage patterns:

| Mode | Local Components | Central Components |
|------|------------------|-------------------|
| **Local** | All (Redis, PG, Qdrant, Aeterna) | None |
| **Hybrid** | Working/Session memory only | Episodic+, Knowledge, Governance |
| **Remote** | None | All |

#### Scenario: Hybrid mode sync
- **WHEN** running in Hybrid mode
- **THEN** local Working and Session memories SHALL sync to central Episodic memory
- **AND** central Knowledge queries SHALL be available locally
- **AND** governance events SHALL originate from the central server only

#### Scenario: Local mode isolation
- **WHEN** running in Local mode
- **THEN** all operations SHALL be self-contained
- **AND** no external network calls SHALL be made for governance
- **AND** drift detection SHALL run against local knowledge only

#### Scenario: Remote mode delegation
- **WHEN** running in Remote mode
- **THEN** all operations SHALL delegate to the central server
- **AND** the local instance SHALL act as a thin client
- **AND** authentication SHALL be required for all operations

### Requirement: Governance Dashboard API

The system SHALL expose REST API endpoints for governance dashboards to query and visualize governance data.

Endpoints:
- `GET /api/v1/governance/drift/{project_id}` - Project drift status
- `GET /api/v1/governance/proposals` - Pending proposals list
- `GET /api/v1/governance/reports/{org_id}` - Organization reports
- `POST /api/v1/governance/proposals/{id}/approve` - Approve proposal
- `POST /api/v1/governance/proposals/{id}/reject` - Reject proposal

#### Scenario: Drift status query
- **WHEN** a dashboard queries drift status for a project
- **THEN** the system SHALL return current drift score, flagged items, and trend data
- **AND** the response SHALL include last scan timestamp

#### Scenario: Proposal approval via API
- **WHEN** an authorized user approves a proposal via API
- **THEN** the system SHALL validate the user's role permissions
- **AND** the system SHALL emit a `KnowledgeApproved` event
- **AND** the system SHALL update the knowledge repository

### Requirement: Tenant Context Propagation

The system SHALL propagate tenant context through all operations using a TenantContext structure.

```rust
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub user_id: UserId,
    pub roles: Vec<Role>,
    pub hierarchy_path: HierarchyPath,
}
```

#### Scenario: Context injection
- **WHEN** a request arrives at any tenant-scoped API or tool boundary
- **THEN** the system SHALL extract and validate TenantContext from authenticated identity or trusted boundary data
- **AND** the context SHALL include the caller's effective roles and hierarchy path for downstream authorization decisions

#### Scenario: Context validation failure
- **WHEN** a request lacks valid TenantContext
- **THEN** the system SHALL reject the request with 401 Unauthorized
- **AND** the system SHALL NOT process any data operations

#### Scenario: MCP tool invocation validates tenant context
- **WHEN** an MCP tool is invoked with tenant context in the JSON payload
- **THEN** the runtime SHALL validate that the supplied tenant context matches the authenticated identity
- **AND** the request SHALL be rejected if the supplied tenant context cannot be verified

### Requirement: Policy Inheritance

The system SHALL support policy inheritance from higher hierarchy levels to lower levels.

Policies can be:
- **Mandatory**: Must be present at all child levels (inherited automatically)
- **Optional**: Can be overridden at child levels
- **Forbidden**: Cannot be overridden (enforced from parent)

#### Scenario: Mandatory policy inheritance
- **WHEN** a Tenant defines a mandatory security policy
- **THEN** all Organizations, Teams, and Projects under that Tenant SHALL inherit the policy
- **AND** the policy SHALL NOT be deletable at lower levels

#### Scenario: Optional policy override
- **WHEN** a Team overrides an optional Organization policy
- **THEN** the Team's policy version SHALL take precedence for that Team
- **AND** the override SHALL be tracked for audit purposes

#### Scenario: Forbidden policy enforcement
- **WHEN** a lower level attempts to create a policy that contradicts a forbidden parent policy
- **THEN** the system SHALL reject the creation
- **AND** the system SHALL return the conflicting parent policy in the error response

### Requirement: Tenant Data Isolation Security (MT-C1)
The system SHALL implement defense-in-depth for tenant data isolation.

#### Scenario: Query Parameterization
- **WHEN** database queries are executed
- **THEN** all queries MUST use parameterized statements
- **AND** tenant_id MUST be included in all WHERE clauses

#### Scenario: Row-Level Security
- **WHEN** PostgreSQL tables contain multi-tenant data
- **THEN** row-level security (RLS) policies MUST be enabled
- **AND** RLS MUST enforce tenant isolation at the database level
- **AND** runtime hot paths MUST activate the database tenant context needed for those RLS policies

#### Scenario: Penetration Testing
- **WHEN** new tenant isolation features are deployed
- **THEN** penetration testing MUST verify cross-tenant isolation
- **AND** test results MUST be documented

### Requirement: RBAC Policy Testing (MT-C2)
The system SHALL have comprehensive automated testing for role-based access control.

#### Scenario: RBAC Integration Tests
- **WHEN** CI runs
- **THEN** integration tests MUST verify all role-action-resource combinations
- **AND** tests MUST cover positive and negative authorization cases

#### Scenario: Permission Matrix Validation
- **WHEN** RBAC policies are modified
- **THEN** a permission matrix MUST be generated
- **AND** matrix MUST be reviewed before deployment

#### Scenario: Role Escalation Prevention
- **WHEN** testing role permissions
- **THEN** tests MUST verify privilege escalation is not possible
- **AND** tests MUST verify role hierarchy is enforced correctly

### Requirement: Drift Detection Tuning (MT-C3)
The system SHALL provide controls to reduce drift detection false positives.

#### Scenario: Configurable Drift Threshold
- **WHEN** drift detection runs
- **THEN** drift threshold MUST be configurable per project
- **AND** default threshold MUST be conservative (0.2 similarity difference)

#### Scenario: Drift Suppression Rules
- **WHEN** a known legitimate drift pattern exists
- **THEN** suppression rules MUST allow marking it as acceptable
- **AND** suppressed drifts MUST be tracked separately in reports

#### Scenario: Drift Confidence Scoring
- **WHEN** drift is detected
- **THEN** system MUST provide confidence score for each drift item
- **AND** low-confidence drifts MUST be flagged for manual review

### Requirement: Event Streaming Reliability (MT-H1)
The system SHALL implement reliable delivery for governance events.

#### Scenario: Event Persistence
- **WHEN** governance events are emitted
- **THEN** events MUST be persisted to durable storage before acknowledgment
- **AND** persistence failure MUST be retried with exponential backoff

#### Scenario: At-Least-Once Delivery
- **WHEN** consumers receive events
- **THEN** delivery MUST be at-least-once with idempotency keys
- **AND** consumers MUST handle duplicate events gracefully

#### Scenario: Dead Letter Queue
- **WHEN** event delivery fails after max retries
- **THEN** events MUST be moved to dead letter queue
- **AND** DLQ MUST be monitored with alerts

### Requirement: Batch Job Coordination (MT-H2)
The system SHALL prevent batch job scheduling conflicts.

#### Scenario: Distributed Locking
- **WHEN** batch jobs are scheduled
- **THEN** distributed locks MUST prevent concurrent execution
- **AND** locks MUST have TTL to prevent deadlocks

#### Scenario: Job Deduplication
- **WHEN** a job is triggered while previous instance is running
- **THEN** system MUST skip the duplicate run
- **AND** skip event MUST be logged for monitoring

#### Scenario: Job Timeout
- **WHEN** a batch job exceeds timeout (default: 30 minutes)
- **THEN** job MUST be terminated gracefully
- **AND** partial results MUST be persisted

### Requirement: Tenant Context Safety (MT-H3)
The system SHALL enforce mandatory tenant context propagation.

#### Scenario: Middleware Enforcement
- **WHEN** requests are processed
- **THEN** middleware MUST require valid TenantContext
- **AND** requests without context MUST be rejected immediately

#### Scenario: Fail-Closed Policy
- **WHEN** TenantContext extraction fails
- **THEN** system MUST fail closed (reject request)
- **AND** MUST NOT fall back to default tenant
- **AND** MUST NOT assign a synthetic system user as a replacement caller identity

#### Scenario: Context Audit Trail
- **WHEN** operations are performed
- **THEN** TenantContext MUST be logged with each operation
- **AND** audit logs MUST enable forensic reconstruction

### Requirement: Authorization Fallback (MT-H4)
The system SHALL implement fallback for external authorization service failures.

#### Scenario: Local Policy Cache
- **WHEN** Permit.io/external auth service is unavailable
- **THEN** system MUST use locally cached policies
- **AND** cache MUST be refreshed when service recovers

#### Scenario: OPA/Cedar Fallback
- **WHEN** configuring authorization
- **THEN** system MUST support local OPA or Cedar as fallback
- **AND** policy sync between Permit.io and local MUST be automated

#### Scenario: Graceful Degradation
- **WHEN** authorization is degraded
- **THEN** system MUST log degradation mode
- **AND** read operations MAY continue with cached permissions

### Requirement: Dashboard API Security (MT-H5)
The system SHALL secure dashboard API endpoints.

#### Scenario: JWT Validation
- **WHEN** dashboard API requests are received
- **THEN** JWT tokens MUST be validated
- **AND** expired tokens MUST be rejected

#### Scenario: OPAL Integration
- **WHEN** using OPAL for policy updates
- **THEN** dashboard auth MUST integrate with OPAL auth
- **AND** API keys MUST be rotatable

#### Scenario: CORS Configuration
- **WHEN** dashboard frontends access API
- **THEN** CORS MUST be configured with allowed origins
- **AND** production MUST not allow wildcard origins

### Requirement: Federated Identity Tenant Context Mapping
The system SHALL derive tenant context for authenticated interactive users from trusted Okta-backed identity claims and configured mapping rules.

#### Scenario: Authenticated user receives tenant context
- **WHEN** an authenticated interactive request reaches Aeterna with the required trusted identity fields
- **THEN** the system SHALL resolve the user's tenant context using configured claim and mapping rules
- **AND** the resolved tenant context SHALL be made available to downstream authorization checks

#### Scenario: Tenant mapping cannot be resolved
- **WHEN** the system cannot derive a valid tenant context from the authenticated user's trusted identity fields
- **THEN** the request SHALL be rejected as unauthorized
- **AND** the system SHALL NOT assign a default tenant

### Requirement: Okta Group Authorization Mapping
The system SHALL map trusted Okta group claims into Aeterna roles and policy inputs used for authorization.

#### Scenario: Group claims map to roles
- **WHEN** an authenticated user presents trusted Okta group claims
- **THEN** the system SHALL translate those groups into configured Aeterna roles or policy attributes
- **AND** authorization decisions SHALL use the translated roles or attributes

#### Scenario: Missing required group mapping fails closed
- **WHEN** an operation requires a role or policy attribute that cannot be derived from the authenticated user's trusted group claims
- **THEN** the system SHALL deny the operation
- **AND** the system SHALL record the authorization failure for audit or troubleshooting

### Requirement: Administrative Authority Boundaries
The system SHALL distinguish cross-tenant platform administration from tenant-scoped administration without weakening tenant isolation.

#### Scenario: Platform admin manages tenant lifecycle without implicit tenant content access
- **WHEN** a platform administrator creates, updates, or deactivates a tenant
- **THEN** the system SHALL allow the lifecycle mutation across tenants
- **AND** the platform administrator SHALL NOT implicitly gain read or write access to that tenant's memory or knowledge content outside an explicit tenant-scoped request path

#### Scenario: Tenant admin is limited to one tenant
- **WHEN** a tenant administrator attempts a cross-tenant lifecycle or scoped-administration action
- **THEN** the system SHALL reject the request with an authorization error
- **AND** the audit trail SHALL record the denied cross-tenant attempt

### Requirement: Verified Tenant Resolution for Administrative Onboarding
The system SHALL resolve tenant association for onboarding and administrative bootstrap from explicit tenant selection, sync-derived mappings, or admin-approved verified mappings.

#### Scenario: Verified domain mapping resolves a tenant
- **WHEN** an onboarding or bootstrap flow matches exactly one admin-approved email-domain mapping for a tenant
- **THEN** the system SHALL allow that mapping to select the tenant automatically
- **AND** the audit trail SHALL record the mapping source used for the tenant resolution

#### Scenario: Ambiguous or missing mapping fails closed
- **WHEN** no verified tenant mapping exists or multiple mappings match the same onboarding request
- **THEN** the system SHALL require explicit tenant selection or administrator intervention
- **AND** the system SHALL NOT infer a tenant from an unverified email suffix alone

### Requirement: Canonical Administrative Role Catalog
The system SHALL maintain one canonical administrative role catalog across runtime types, CLI validation, API schemas, and authorization policy bundles.

#### Scenario: Role catalog inspection is consistent
- **WHEN** an operator inspects the supported role catalog through the API, CLI, or policy-inspection surface
- **THEN** each surface SHALL report the same role identifiers and scope rules
- **AND** the catalog SHALL include any special cross-tenant or read-only administrative roles supported by the deployment

#### Scenario: Policy bundle with unknown role is rejected
- **WHEN** the authorization policy bundle references a role that is not part of the canonical role catalog
- **THEN** policy validation SHALL fail before the bundle becomes active
- **AND** the validation error SHALL identify the unknown role reference

