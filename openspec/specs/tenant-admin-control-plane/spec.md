# tenant-admin-control-plane Specification

## Purpose
TBD - created by archiving change add-tenant-admin-control-plane. Update Purpose after archive.
## Requirements
### Requirement: Tenant Lifecycle Administration Control Plane
The system SHALL provide supported API and CLI control-plane workflows for tenant lifecycle administration.

#### Scenario: Platform admin creates a tenant
- **WHEN** a platform administrator creates a new tenant through the supported API or CLI
- **THEN** the system SHALL persist the tenant record with a unique tenant identifier and metadata
- **AND** the system SHALL allow bootstrapping an initial tenant administrator without requiring direct database edits

#### Scenario: Platform admin lists and inspects tenants
- **WHEN** a platform administrator lists or shows tenants through the supported control plane
- **THEN** the system SHALL return persisted tenant records and current operational status
- **AND** the response SHALL exclude tenant content data that is not required for lifecycle administration

#### Scenario: Tenant admin cannot mutate another tenant
- **WHEN** a tenant-scoped administrator attempts to create, update, or deactivate a different tenant
- **THEN** the system SHALL reject the request with an authorization error
- **AND** no cross-tenant mutation SHALL occur

### Requirement: Explicit Tenant Target Selection for Platform Administrators
The system SHALL require platform administrators to select an explicit tenant target before executing tenant-scoped administrative operations.

#### Scenario: Platform admin selects tenant target for scoped administration
- **WHEN** a platform administrator chooses a tenant target through the API request or supported CLI context override
- **THEN** subsequent tenant-scoped administration for that request SHALL execute against the selected tenant only
- **AND** the audit trail SHALL record which tenant target was selected

#### Scenario: Platform admin omits tenant target for tenant-scoped operation
- **WHEN** a platform administrator invokes a tenant-scoped membership, hierarchy, repository, or permission operation without an explicit tenant target
- **THEN** the system SHALL reject the request with a validation or authorization error
- **AND** the system SHALL NOT infer a tenant target from hidden ambient context alone

### Requirement: Hierarchy Administration Control Plane
The system SHALL provide supported API and CLI workflows for organization, team, project, and membership administration within a tenant.

#### Scenario: Tenant admin creates organizational units
- **WHEN** a tenant administrator creates an organization, team, or project through the supported control plane
- **THEN** the system SHALL persist the unit in the tenant hierarchy
- **AND** inherited policies and parent-child relationships SHALL be preserved according to the hierarchy rules

#### Scenario: Tenant admin manages membership and scoped roles
- **WHEN** a tenant administrator adds, removes, or updates a member role at tenant, organization, team, or project scope
- **THEN** the system SHALL persist the scoped membership change
- **AND** the system SHALL emit an auditable role or membership event for the mutation

### Requirement: Tenant Knowledge Repository Administration
The system SHALL provide supported API and CLI workflows to inspect, configure, and validate the canonical knowledge repository binding for a tenant.

#### Scenario: Tenant admin configures repository binding
- **WHEN** a tenant administrator sets the tenant's knowledge repository binding through the supported control plane
- **THEN** the system SHALL persist the binding configuration for that tenant
- **AND** the binding SHALL include enough information to resolve tenant knowledge operations without relying on process-global defaults

#### Scenario: Repository binding validation failure
- **WHEN** an administrator submits an invalid repository binding configuration
- **THEN** the system SHALL reject the change with actionable validation errors
- **AND** the previous valid binding SHALL remain unchanged

#### Scenario: Repository binding inspection shows effective tenant configuration
- **WHEN** an administrator inspects a tenant repository binding through the supported control plane
- **THEN** the system SHALL return the persisted binding fields, validation status, and source ownership metadata
- **AND** the response SHALL avoid exposing raw secret material

### Requirement: Role Administration and Permission Inspection
The system SHALL provide supported API and CLI workflows to assign roles, revoke roles, list roles, inspect effective permissions, and inspect the role-to-permission matrix.

#### Scenario: Scoped role assignment succeeds
- **WHEN** an authorized administrator assigns a role to a principal at a valid scope
- **THEN** the system SHALL persist the role assignment
- **AND** subsequent role and permission queries SHALL reflect the change

#### Scenario: Effective permission inspection
- **WHEN** an authorized operator requests the effective permissions for a principal at a given scope
- **THEN** the system SHALL evaluate the active role assignments and policy bundle for that principal and scope
- **AND** the response SHALL identify the permissions granted and denied by the current model

#### Scenario: Role-to-permission matrix inspection
- **WHEN** an authorized operator inspects the role matrix for the active policy bundle
- **THEN** the system SHALL return the permissions associated with each supported role
- **AND** the matrix SHALL reflect the same policy definitions used for runtime authorization decisions

#### Scenario: Unsupported role assignment is rejected consistently
- **WHEN** an administrator attempts to assign a role that is not part of the canonical role catalog
- **THEN** the system SHALL reject the request consistently across API, CLI, and policy validation surfaces
- **AND** the error SHALL identify the supported roles for that deployment

### Requirement: Honest Administrative Command Behavior
The system SHALL ensure live tenant-administration command paths return persisted results, real authorization failures, or explicit validation errors instead of preview output.

#### Scenario: Live CLI admin path returns real result
- **WHEN** an operator runs a supported live tenant-administration CLI command without `--dry-run`
- **THEN** the CLI SHALL execute the real backend-backed mutation or query
- **AND** it SHALL return persisted result data or the actual backend error rather than example output or placeholder success

### Requirement: Black-Box End-to-End Administration Coverage
The system SHALL maintain Newman/Postman end-to-end coverage for every supported tenant-admin and platform-admin HTTP workflow added by this capability.

#### Scenario: Supported admin workflow exists in Newman suite
- **WHEN** the system supports a tenant lifecycle, repository-binding, role-administration, or permission-inspection workflow through the HTTP control plane
- **THEN** the Postman collection and Newman runner SHALL include an asserted end-to-end scenario for that workflow
- **AND** the scenario SHALL exercise the same public HTTP contract used by operators rather than private test-only hooks

#### Scenario: Denial and validation paths are covered in Newman suite
- **WHEN** a workflow has a documented authorization-boundary or validation-failure behavior
- **THEN** the Newman suite SHALL include negative scenarios for those failures
- **AND** the assertions SHALL verify the documented error semantics for that workflow

