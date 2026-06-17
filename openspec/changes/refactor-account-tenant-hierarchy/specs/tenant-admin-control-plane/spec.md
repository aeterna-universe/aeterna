## MODIFIED Requirements

### Requirement: Tenant Lifecycle Administration Control Plane
The system SHALL provide supported API and CLI control-plane workflows for tenant lifecycle administration, including optional account attachment and environment metadata.

#### Scenario: Platform admin creates a tenant
- **WHEN** a platform administrator creates a new tenant through the supported API or CLI
- **THEN** the system SHALL persist the tenant record with a unique tenant identifier, lifecycle metadata, optional account reference, and optional environment label
- **AND** the system SHALL allow bootstrapping an initial tenant administrator without requiring direct database edits

#### Scenario: Platform admin lists and inspects tenants
- **WHEN** a platform administrator lists or shows tenants through the supported control plane
- **THEN** the system SHALL return persisted tenant records, current operational status, account reference, and environment metadata
- **AND** the response SHALL exclude tenant content data that is not required for lifecycle administration

#### Scenario: Tenant admin cannot mutate another tenant
- **WHEN** a tenant-scoped administrator attempts to create, update, or deactivate a different tenant
- **THEN** the system SHALL reject the request with an authorization error
- **AND** no cross-tenant mutation SHALL occur

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
