## Why

Aeterna's current hierarchy still reflects an inverted model: `Tenant -> Organization -> Team -> Project`, while also lacking a canonical account layer above tenants. In practice, operators usually need one customer account or organization with multiple isolated tenants (for example `dev`, `test`, and `prod`) — and then an in-tenant hierarchy that starts at `Organization`, not an extra tenant-internal root node. The current model makes tenant manifests, GitHub sync, role scoping, and issue #130's migration work harder than they need to be.

## What Changes

- Introduce a canonical **Account** layer above `Tenant`, so one account can own many tenants.
- Add optional tenant environment metadata so account-owned tenants can be identified as `dev`, `test`, `staging`, `prod`, or another operator-defined environment label.
- Flatten the in-tenant hierarchy to `Tenant -> Organization -> Team -> Project`.
- **BREAKING** Remove the legacy tenant-internal root node from tenant-facing hierarchy contracts: manifests, control-plane APIs, sync mappings, reverse-render output, OPAL hierarchy views, and role-resolution paths SHALL stop treating an extra root node as an in-tenant hierarchy unit.
- Migrate existing data into the new model:
    - backfill accounts from the existing legal-entity seed data,
    - attach tenants to accounts,
    - lift organizations under the tenant root,
    - reject automatic migration for tenants that still depend on multiple active legacy root rows under one tenant.
- Supersede the earlier `add-legal-entity-tenant-grouping` direction with a cleaner, canonical account-oriented model.

## Capabilities

### New Capabilities

- `account-grouping`: First-class account/customer-organization layer above tenants, including tenant attachment and environment-aware tenant grouping.

### Modified Capabilities

- `tenant-provisioning`: Tenant manifests attach tenants to accounts and declare `organization -> team -> project` hierarchies without an extra in-tenant root layer.
- `github-org-sync`: GitHub organization sync maps into tenant-root organizations and teams instead of creating synthetic tenant-internal root rows.
- `tenant-admin-control-plane`: Tenant admin APIs/CLI/admin UI present tenants as account-owned environments and manage only organization/team/project hierarchy within a tenant.

## Impact

- **Affected code**:
    - `storage/migrations/` — new migrations for `accounts`, tenant/account backfill, organization re-parenting, and removal of legacy root-hierarchy assumptions.
    - `storage/src/hierarchy_store.rs`, `storage/src/postgres.rs`, `storage/src/tenant_store.rs` — canonical hierarchy and tenant/account persistence.
    - `cli/src/server/tenant_api.rs`, `cli/src/server/manifest_render.rs`, `cli/src/server/role_grants.rs`, `cli/src/server/bootstrap.rs` — provisioning, reverse-render, role resolution, bootstrap.
    - `idp-sync/src/github.rs`, `idp-sync/src/sync.rs` — tenant-root GitHub hierarchy mapping.
    - `opal-fetcher/src/*` and `storage/migrations/028_tenant_scoped_hierarchy.sql` successors — hierarchy and permission views.
    - `admin-ui/src/*` and CLI tenant/account commands — account-oriented navigation and tenant environment display.
- **Breaking API / manifest surface**:
    - `hierarchy:` roots become organizations instead of legacy wrapper roots.
    - tenant payloads gain optional account and environment fields.
    - account APIs replace ad-hoc legal-entity grouping.
- **Operational impact**:
    - tenants with more than one active legacy root node cannot be auto-flattened safely and require explicit operator migration before this change can complete.
