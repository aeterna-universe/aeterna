# github-org-sync Specification

## Purpose
TBD - created by archiving change add-github-org-sync. Update Purpose after archive.
## Requirements
### Requirement: GitHub Organization Provider
The system SHALL implement a GitHub Organization provider for the IdP sync framework that maps GitHub Org structure into Aeterna's organizational hierarchy using GitHub App certificate-based authentication.

The provider MUST implement the existing `IdpClient` trait (`list_users`, `list_groups`, `get_group_members`, `get_user`) without modifying the trait interface.

#### Scenario: GitHub App authentication
- **WHEN** the GitHub provider is initialized with app_id, installation_id, and PEM private key
- **THEN** the provider SHALL mint a JWT, exchange it for an installation access token via the GitHub API
- **AND** the provider SHALL cache the token and refresh it automatically before expiry (within 5 minutes of the 1-hour expiration)

#### Scenario: Token refresh on expiry
- **WHEN** a cached installation token is within 5 minutes of expiry
- **THEN** the provider SHALL mint a new JWT and obtain a fresh installation token before making the API call
- **AND** the provider SHALL rebuild its HTTP client with the new token

#### Scenario: Authentication failure
- **WHEN** the PEM key is invalid or the GitHub App is not installed on the target organization
- **THEN** the provider SHALL return an `AuthenticationError` with a descriptive message
- **AND** the provider SHALL NOT retry authentication failures

### Requirement: GitHub Hierarchy Mapping
The system SHALL map GitHub Organization structure to Aeterna's four-level hierarchy according to the following rules:

- GitHub Organization → Tenant (root scope)
- Top-level GitHub Teams (no parent team) → Organization (business unit)
- Nested GitHub Teams (have a parent team) → Team (working group)
- GitHub Organization Members → Users

#### Scenario: Full hierarchy sync
- **WHEN** an admin triggers a full sync for GitHub org `acme-corp`
- **THEN** the system SHALL create a Tenant unit named `acme-corp` if it does not exist
- **AND** the system SHALL create Organization units for each top-level team
- **AND** the system SHALL create Team units for each nested team, parented under the correct Organization
- **AND** the system SHALL create or update User records for all org members

#### Scenario: Team nesting detection
- **WHEN** the provider fetches teams via `GET /orgs/{org}/teams`
- **THEN** teams with `parent: null` SHALL be classified as Organizations
- **AND** teams with a non-null `parent` field SHALL be classified as Teams under the parent Organization

#### Scenario: Idempotent sync
- **WHEN** a full sync runs and the hierarchy already exists from a previous sync
- **THEN** the system SHALL update existing units (name changes, reparenting) without creating duplicates
- **AND** the system SHALL use GitHub team slug as the stable identifier for matching

### Requirement: GitHub Member Role Mapping
The system SHALL map GitHub organization and team roles to Aeterna governance roles.

| GitHub Role | Aeterna Role |
|---|---|
| Organization owner | Admin |
| Organization member | Developer |
| Team maintainer | TechLead |
| Team member | Developer |

A user's effective Aeterna role MUST be the highest role across all their memberships.

#### Scenario: Org owner mapped to Admin
- **WHEN** a GitHub user has the `admin` role in the organization
- **THEN** the user SHALL be assigned the Admin role in Aeterna
- **AND** the Admin role SHALL apply at the Tenant level

#### Scenario: Team maintainer mapped to TechLead
- **WHEN** a GitHub user has the `maintainer` role in a team
- **THEN** the user SHALL be assigned the TechLead role for that team's corresponding unit
- **AND** if the user is also a regular member in another team, they SHALL retain the higher TechLead role

#### Scenario: Member deactivation on removal
- **WHEN** a GitHub user is removed from the organization
- **THEN** the system SHALL deactivate the user's Aeterna account
- **AND** the system SHALL remove all team memberships for the deactivated user

### Requirement: On-Demand Sync API
The system SHALL expose a REST API endpoint for administrators to trigger GitHub org synchronization on demand.

#### Credential Resolution Order
- Tenant-scoped sync and fan-out sync SHALL resolve credentials in this order:
  1. Tenant config fields and secret references (`github.org_name`, `github.app_id`, `github.installation_id`, `github.app_pem`)
  2. Environment variable fallback (`AETERNA_GITHUB_ORG_NAME`, `AETERNA_GITHUB_APP_ID`, `AETERNA_GITHUB_INSTALLATION_ID`, `AETERNA_GITHUB_APP_PEM`)
- The tenant-scoped endpoint `POST /api/v1/admin/tenants/{tenant}/sync/github` SHALL be available for a single tenant by name or UUID.

#### Scenario: Trigger sync via API
- **WHEN** an admin sends `POST /api/v1/admin/sync/github` with a valid admin API key
- **THEN** the system SHALL execute a full two-phase sync (hierarchy creation + membership sync)
- **AND** the system SHALL return a tenant result list that includes `SyncReport` data for each successful tenant sync

#### Scenario: Per-tenant config takes precedence over env vars
- **WHEN** a tenant has `github.org_name`, `github.app_id`, `github.installation_id`, and `github.app_pem` configured
- **AND** instance-level GitHub sync environment variables are also set
- **THEN** the system SHALL use the tenant-scoped config and secret values for that tenant sync
- **AND** it SHALL ignore conflicting environment variable values for that tenant

#### Scenario: Trigger tenant-scoped sync via API
- **WHEN** an admin sends `POST /api/v1/admin/tenants/{tenant}/sync/github`
- **THEN** the system SHALL resolve `{tenant}` by tenant name or UUID
- **AND** the system SHALL execute a full two-phase sync only for that tenant
- **AND** the system SHALL return a `SyncReport` JSON response with counts of created, updated, and deactivated entities

#### Scenario: Unauthorized sync attempt
- **WHEN** a non-admin user sends `POST /api/v1/admin/sync/github`
- **THEN** the system SHALL return HTTP 403 Forbidden
- **AND** the system SHALL NOT initiate any sync operation

#### Scenario: Sync while already running
- **WHEN** a sync is already in progress and another sync request arrives
- **THEN** the system SHALL return HTTP 409 Conflict with a message indicating sync is in progress

### Requirement: Incremental Webhook Sync
The system SHALL process GitHub webhook events for near-real-time hierarchy updates without requiring a full sync.

The webhook handler MUST share the same `X-Hub-Signature-256` verification as existing GitHub webhook events.

#### Scenario: Organization member added
- **WHEN** GitHub sends an `organization` webhook with action `member_added`
- **THEN** the system SHALL create or activate the user in Aeterna
- **AND** the system SHALL assign the appropriate role based on the member's org role

#### Scenario: Team created
- **WHEN** GitHub sends a `team` webhook with action `created`
- **THEN** the system SHALL create the corresponding Organization or Team unit based on whether the team has a parent
- **AND** the system SHALL set the correct parent unit in the hierarchy

#### Scenario: Team membership changed
- **WHEN** GitHub sends a `membership` webhook with action `added` or `removed`
- **THEN** the system SHALL add or remove the team membership in Aeterna
- **AND** the system SHALL recalculate the user's effective role

#### Scenario: Team deleted
- **WHEN** GitHub sends a `team` webhook with action `deleted`
- **THEN** the system SHALL soft-delete (deactivate) the corresponding unit
- **AND** the system SHALL NOT delete child units or member records

### Requirement: GitHub Provider Configuration
The system SHALL support GitHub org sync configuration via the `IdpProvider` enum with a `GitHub` variant.

#### Scenario: Valid GitHub configuration
- **WHEN** the IdP sync config specifies `type: "github"` with `org_name`, `app_id`, `installation_id`, and `private_key_pem`
- **THEN** the system SHALL construct a `GitHubClient` and register it with the sync service
- **AND** the system SHALL validate the PEM key format on startup

#### Scenario: Optional team filter
- **WHEN** the GitHub config includes a `team_filter` regex pattern
- **THEN** the system SHALL only sync teams whose names match the filter
- **AND** the system SHALL log skipped teams at debug level

#### Scenario: Missing required fields
- **WHEN** the GitHub config is missing `org_name`, `app_id`, or `installation_id`
- **THEN** the system SHALL return a `ConfigError` at startup with a clear message identifying the missing field

### Requirement: Verified Sync Webhook Boundary
The system SHALL verify webhook authenticity before processing sync-triggering GitHub organization events.

#### Scenario: Team or membership event without valid signature
- **WHEN** a `team`, `membership`, `member`, or `organization` webhook arrives without the required valid signature
- **THEN** the system SHALL reject the event before any incremental sync or tenant mutation work begins
- **AND** no sync job SHALL be scheduled from that event

### Requirement: Sync Coexistence with Tenant Administration Control Plane
The system SHALL allow GitHub or IdP-managed hierarchy sync to coexist with manual tenant administration without overwriting admin-owned tenant configuration.

#### Scenario: Sync preserves tenant repository binding
- **WHEN** a GitHub organization sync updates a tenant, organization, or team hierarchy
- **THEN** the sync SHALL preserve the tenant's admin-managed knowledge repository binding
- **AND** the sync report SHALL identify that the binding was left unchanged

#### Scenario: Sync preserves verified tenant mappings unless explicitly sync-owned
- **WHEN** a GitHub organization sync updates tenant-linked identity data
- **THEN** admin-managed verified tenant mappings SHALL remain unchanged unless an explicit source-ownership rule marks them as sync-owned
- **AND** the sync report SHALL identify any skipped mapping mutations caused by source ownership
