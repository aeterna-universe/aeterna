## ADDED Requirements

### Requirement: Account Resource Above Tenant
The platform SHALL persist a first-class `Account` record above `Tenant`. An account represents the customer or owning organization that may operate multiple isolated tenants. An account has a stable UUID id, a unique slug, a human-readable name, optional metadata, and SHALL NOT act as an RLS boundary.

#### Scenario: Creating an account
- **WHEN** a PlatformAdmin sends `POST /api/v1/accounts` with body `{ "name": "Acme", "slug": "acme" }`
- **THEN** the server SHALL persist an `accounts` row with the supplied name and slug, generated UUID id, and timestamps
- **AND** the server SHALL return `201 Created` with the full `AccountRecord`

#### Scenario: Listing accounts
- **WHEN** a PlatformAdmin sends `GET /api/v1/accounts`
- **THEN** the server SHALL return all non-deleted accounts with their tenant counts
- **AND** each account record SHALL exclude tenant data-plane content

### Requirement: Tenant Attachment to Account
A tenant SHALL have at most one account. Attachment and detachment are explicit operations on the tenant record, and detaching a tenant SHALL NOT delete or mutate tenant data beyond removing the account reference.

#### Scenario: Attach tenant to account
- **WHEN** a PlatformAdmin sends `POST /api/v1/tenants/{slug}/account` with body `{ "accountId": "<uuid>" }`
- **AND** the tenant and account both exist
- **THEN** the server SHALL set `tenants.account_id` to the supplied UUID
- **AND** the response SHALL return the updated tenant including its account reference

#### Scenario: Detach tenant from account
- **WHEN** a PlatformAdmin sends `DELETE /api/v1/tenants/{slug}/account`
- **THEN** the server SHALL set `tenants.account_id` to `NULL`
- **AND** the server SHALL return `200 OK`
- **AND** the tenant's other configuration and data SHALL remain unchanged

### Requirement: Account-Owned Tenant Environments
The system SHALL allow an account to own multiple tenants distinguished by environment metadata. A tenant MAY declare an environment label such as `dev`, `test`, `staging`, or `prod`, and account-oriented APIs/UI SHALL surface that label when listing the account's tenants.

#### Scenario: Tenant exposes environment label
- **WHEN** a tenant record has `environment = "prod"`
- **THEN** `GET /api/v1/tenants/{slug}` SHALL include `environment: "prod"`
- **AND** account-oriented tenant listings SHALL show that same environment label

#### Scenario: Account tenant list is grouped for operators
- **WHEN** a caller sends `GET /api/v1/accounts/{id}/tenants`
- **THEN** the response SHALL list only tenants attached to that account
- **AND** each tenant entry SHALL include id, slug, name, status, and environment
