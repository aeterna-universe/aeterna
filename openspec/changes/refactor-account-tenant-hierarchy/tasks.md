## 1. Account model and tenant metadata

- [x] 1.1 Add `accounts` schema, `tenants.account_id`, and `tenants.environment` migration(s)
- [x] 1.2 Backfill accounts from existing tenant legal-entity seed data and attach matching tenants
- [x] 1.3 Extend tenant storage/types/API projections with account reference and environment metadata
- [x] 1.4 Add account CRUD + tenant attach/detach control-plane endpoints and CLI support

## 2. Flatten tenant-root hierarchy

- [x] 2.1 Add `organizations.tenant_id` and backfill it from the existing tenant chain
- [x] 2.2 Add migration preflight that blocks tenants with more than one active tenant from automatic flattening
- [x] 2.3 Rewrite hierarchy persistence/read helpers to use `Tenant -> Organization -> Team -> Project`
- [x] 2.4 Rewrite `v_hierarchy` and `v_user_permissions` for the tenant-root hierarchy shape

## 3. Remove tenant-rooted runtime assumptions

- [x] 3.1 Update tenant provisioning manifests, validation, and reverse-rendering to use account/env metadata and organization-root hierarchies
- [x] 3.2 Update role-scope persistence and resolution to stop depending on tenant-rooted OU ids
- [x] 3.3 Update bootstrap and tenant admin control-plane paths to create/manage only tenant/org/team/project scopes
- [x] 3.4 Remove tenant-specific runtime joins from OPAL fetch, role listing, and related admin surfaces

## 4. Sync and integration paths

- [x] 4.1 Update GitHub sync mapping to target the tenant root and stop creating synthetic tenant nodes
- [x] 4.2 Update sync-to-governance bridging to emit tenant/org/team/project-scoped roles under the new model
- [x] 4.3 Add migration-safe tests for tenant-root hierarchy sync, manifests, and role rendering

## 5. Cleanup and rollout

- [x] 5.1 Remove obsolete tenant-layer compatibility paths after tenant-root flows are green
- [x] 5.2 Update admin UI navigation to present `Account -> Tenant(environment)` and tenant-root hierarchy management
- [x] 5.3 Update docs and OpenSpec references, including a note that this change supersedes `add-legal-entity-tenant-grouping`
- [x] 5.4 Validate with `openspec validate refactor-account-tenant-hierarchy --strict` and full targeted test suites
