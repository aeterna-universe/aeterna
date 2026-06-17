## MODIFIED Requirements

### Requirement: GitHub Hierarchy Mapping
The system SHALL map GitHub Organization structure to Aeterna's tenant-root hierarchy according to the following rules:

- GitHub Organization → Tenant sync target (no synthetic hierarchy node)
- Top-level GitHub Teams (no parent team) → Organization
- Nested GitHub Teams (have a parent team) → Team
- GitHub Organization Members → Users

#### Scenario: Full hierarchy sync
- **WHEN** an admin triggers a full sync for GitHub org `acme-corp` targeting tenant `acme-prod`
- **THEN** the system SHALL treat tenant `acme-prod` as the hierarchy root
- **AND** the system SHALL create Organization units for each top-level team
- **AND** the system SHALL create Team units for each nested team, parented under the correct Organization
- **AND** the system SHALL create or update User records for all org members
- **AND** the system SHALL NOT create a tenant-internal Tenant node

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
