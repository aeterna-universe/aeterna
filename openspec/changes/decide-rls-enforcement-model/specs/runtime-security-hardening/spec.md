## MODIFIED Requirements

### Requirement: Backend-Specific Persistence Isolation

The system SHALL define and enforce tenant isolation explicitly for each persistence backend used in production-capable deployments. On PostgreSQL, the authoritative enforcement layer SHALL be row-level security policies evaluated against a transaction-scoped `app.tenant_id` GUC; the repository layer's `WHERE tenant_id = $N` clauses remain as required second-layer defense in depth.

#### Scenario: PostgreSQL tenant isolation is enforced by row-level security

- **WHEN** PostgreSQL stores tenant-scoped data
- **THEN** the default application database role SHALL NOT have `BYPASSRLS` and SHALL NOT be the table owner of any tenant-scoped table
- **AND** every tenant-scoped table SHALL have `ENABLE ROW LEVEL SECURITY` with at least one policy keyed on `current_setting('app.tenant_id', true)`
- **AND** every request-scoped query path SHALL acquire its database connection through the `with_tenant_context` helper that opens an explicit transaction, issues `SET LOCAL app.tenant_id = $1`, runs the query body, and commits or rolls back
- **AND** every repository-layer query against a tenant-scoped table SHALL additionally include an explicit `WHERE tenant_id = $N` clause (or equivalent governance-root scope clause for governance tables) as defense in depth on top of the RLS policy
- **AND** documentation (`AGENTS.md`, `DEVELOPER_GUIDE.md`) SHALL state the two-layer model: RLS is authoritative, the `WHERE` clause is required defense in depth, neither is optional

#### Scenario: RLS policies are authored and kept valid

- **WHEN** a migration adds a tenant-scoped table to PostgreSQL
- **THEN** that migration SHALL `ENABLE ROW LEVEL SECURITY` on the table
- **AND** SHALL define at least one policy keyed on the canonical session variable `app.tenant_id` (with governance tables mapped onto that same tenant root)
- **AND** SHALL grant `SELECT, INSERT, UPDATE, DELETE` on the table to the `aeterna_app` application role
- **AND** the policies SHALL remain valid under the CI enforcement suite defined in "RLS End-to-End Verification"

#### Scenario: Qdrant tenant isolation is enforced by storage routing

- **WHEN** Qdrant stores tenant-scoped vectors or payloads
- **THEN** the storage layer SHALL route operations through tenant-scoped collections, mandatory tenant filters, or both
- **AND** it SHALL reject or prevent queries that could return vectors from another tenant

#### Scenario: Redis tenant isolation is enforced by storage namespaces

- **WHEN** Redis stores tenant-scoped working memory, session data, checkpoints, streams, or caches
- **THEN** the storage layer SHALL use tenant-scoped key namespaces and access wrappers for those keys
- **AND** callers SHALL NOT read or mutate another tenant's Redis data without an explicit authorized tenant context

## ADDED Requirements

### Requirement: Two-Role PostgreSQL Access Model

The system SHALL provision two distinct PostgreSQL roles and SHALL route every application connection through one of them. The default role (`aeterna_app`) SHALL NOT have `BYPASSRLS`; the administrative role (`aeterna_admin`) SHALL have `BYPASSRLS` and SHALL be used only through a narrow, audited helper. No third connection path SHALL exist.

#### Scenario: Both roles exist with the correct attributes

- **WHEN** the migration suite is applied against a fresh database
- **THEN** the role `aeterna_app` SHALL exist with `rolbypassrls = false` and `rolsuper = false`
- **AND** the role `aeterna_admin` SHALL exist with `rolbypassrls = true` and `rolsuper = false`
- **AND** `aeterna_app` SHALL hold `USAGE` on the application schema and `SELECT, INSERT, UPDATE, DELETE` on every table that has `ENABLE ROW LEVEL SECURITY`
- **AND** `aeterna_admin` SHALL hold `USAGE` on the application schema and `SELECT, INSERT, UPDATE, DELETE` on all application tables

#### Scenario: Application wires two connection pools

- **WHEN** the application builds its shared state
- **THEN** `AppState` SHALL expose `pool: PgPool` that opens connections as `aeterna_app` using `DATABASE_URL`
- **AND** SHALL expose `admin_pool: PgPool` that opens connections as `aeterna_admin` using `DATABASE_URL_ADMIN`
- **AND** the admin pool SHALL be size-capped (`max_connections = 4` or lower) to make accidental hot-loop use of the admin pool a visible capacity signal rather than a silent privilege escalation

### Requirement: Per-Request Transaction-Scoped Tenant Context

Every code path that reads from or writes to a tenant-scoped PostgreSQL table via the default pool SHALL acquire its connection through a helper that opens an explicit transaction, sets `app.tenant_id` transaction-locally, and commits or rolls back before returning the connection. Direct `state.pool.acquire()` or `state.pool.begin()` on tenant-scoped paths SHALL be forbidden.

#### Scenario: Request handlers use with_tenant_context

- **WHEN** a request handler queries a tenant-scoped table under a resolved tenant context
- **THEN** the handler SHALL call `with_tenant_context(&ctx, |tx| async { … })`
- **AND** the helper SHALL issue `BEGIN`, `SET LOCAL app.tenant_id = $1`, the handler body, and `COMMIT`/`ROLLBACK` in that order
- **AND** the handler SHALL NOT call `state.pool.acquire()` or `state.pool.begin()` directly on a tenant-scoped path

#### Scenario: Async per-tenant workers carry their own tenant context

- **WHEN** an async worker (sync, per-tenant backup, per-tenant GDPR export) processes a job that touches tenant-scoped tables
- **THEN** the worker SHALL read the tenant identifier from its job record
- **AND** SHALL construct a `TenantContext` via `TenantContext::from_scheduled_job(tenant_id, job_id)`
- **AND** SHALL acquire its connection through `with_tenant_context` using that context
- **AND** a job that touches tenant-scoped tables without an associated tenant identifier SHALL fail fast rather than running unscoped

#### Scenario: Direct pool access is lint-enforced

- **WHEN** a contributor introduces a call to `state.pool.acquire()` or `state.pool.begin()` outside of `with_tenant_context`
- **THEN** the CI lint SHALL fail the build unless the call site carries `#[allow(direct_pool_access)]` with a justification comment documenting why the path is tenant-context-free

### Requirement: Administrative Cross-Tenant Access

The system SHALL route every legitimate cross-tenant operation (PlatformAdmin `?tenant=*` reads, scheduled cross-tenant maintenance, migration runner) through a single `with_admin_context` helper on a dedicated admin pool. Every call SHALL be audited by the helper itself so cross-tenant access is always traceable.

#### Scenario: PlatformAdmin cross-tenant reads use the admin helper

- **WHEN** a PlatformAdmin request arrives with `?tenant=*` (or the deprecated `?tenant=all`)
- **THEN** the handler SHALL resolve the request context to `CrossTenantAll` and SHALL acquire its connection through `with_admin_context(&ctx, |tx| …)`
- **AND** SHALL NOT use `with_tenant_context` (there is no single tenant to scope to)
- **AND** the helper SHALL open the transaction on `state.admin_pool` and SHALL NOT issue `SET LOCAL app.tenant_id`

#### Scenario: Scheduled cross-tenant jobs use the system sentinel context

- **WHEN** a scheduled job runs cross-tenant maintenance (audit compaction, global rate-limit sweep, cross-tenant analytics rollup)
- **THEN** it SHALL construct a `TenantContext` via `TenantContext::system_ctx()`
- **AND** SHALL acquire its connection through `with_admin_context(&system_ctx, |tx| …)`
- **AND** the `system_ctx` sentinel SHALL carry `actor_type = 'system'` so audit rows are attributed to internal system work rather than a human actor

#### Scenario: Scheduled per-tenant jobs enumerate via admin and dispatch via tenant

- **WHEN** a scheduled job runs per-tenant work (sync, per-tenant backup, per-tenant GDPR export)
- **THEN** the scheduler SHALL enumerate active tenants via a one-shot `with_admin_context(&system_ctx, |tx| …)` call
- **AND** SHALL dispatch the per-tenant body through `with_tenant_context(&TenantContext::from_scheduled_job(t, job_id), |tx| …)` for each tenant
- **AND** the per-tenant body SHALL NOT touch the admin pool

#### Scenario: Every admin access is audited

- **WHEN** any code path enters `with_admin_context`
- **THEN** the helper SHALL write a `governance_audit_log` row before returning, carrying `actor_id`, `actor_type`, `admin_scope = true`, and `acting_as_tenant_id = NULL`
- **AND** the audit write SHALL be inside the helper so no per-call-site bookkeeping is required
- **AND** a code path that reaches `state.admin_pool` without going through `with_admin_context` SHALL fail the direct-pool-access CI lint

### Requirement: Tenant Session Variable Hygiene

Every code path that writes the `app.tenant_id` PostgreSQL session variable SHALL use transaction-scoped writes so the setting cannot survive on a pooled connection after the originating request completes. Legacy alternate GUC names SHALL NOT be written by application code; migration 024 normalized the policies and Bundle A.1 normalizes the app-side writes.

#### Scenario: set_config uses transaction scope

- **WHEN** the application calls `set_config('app.tenant_id', $1, ?)` (or equivalent `SET LOCAL`)
- **THEN** the call SHALL pass `true` as the third argument (transaction-local) and SHALL be inside an explicit `BEGIN`
- **AND** the application SHALL NOT pass `false` (session-scoped) anywhere outside of an explicitly-allowlisted diagnostic path

#### Scenario: Canonical GUC namespace is enforced

- **WHEN** application code reads or writes a tenant session variable
- **THEN** it SHALL use the name `app.tenant_id` exclusively
- **AND** SHALL NOT reference legacy alternate GUC names outside of migration files
- **AND** a compile-time grep test SHALL fail the build if a new reference to a legacy GUC name is introduced in non-migration source

#### Scenario: Test suite catches session-scope misuse

- **WHEN** a new query is introduced that writes `app.tenant_id` with session scope
- **THEN** the `session_variable_hygiene` test SHALL fail by detecting a pooled-connection leak: it acquires a connection, sets context to tenant A, returns the connection, acquires a fresh connection, and asserts `current_setting('app.tenant_id', true)` returns empty rather than tenant A's identifier

### Requirement: RLS End-to-End Verification

The system SHALL maintain an integration test suite that exercises every RLS-enabled PostgreSQL table under the non-BYPASSRLS `aeterna_app` role on every CI run, plus every admin-scoped handler under the `aeterna_admin` role. This is the permanent regression guard that prevents future refactors from silently regressing the model.

#### Scenario: Grant pre-flight enumerates RLS tables

- **WHEN** the integration test suite starts
- **THEN** a pre-flight query SHALL enumerate every table in `pg_tables` where `rowsecurity = true`
- **AND** SHALL assert that `aeterna_app` has been granted at minimum `SELECT` on each such table
- **AND** SHALL fail the run with a message naming the missing table if any grant is absent

#### Scenario: RLS isolation is verified per table

- **WHEN** the integration test suite runs
- **THEN** for every RLS table, there SHALL be a positive test (connect as `aeterna_app`, `BEGIN`, `set_config(..., true)`, `SELECT *`, assert only that tenant's rows)
- **AND** a negative test (same connection without `set_config`, assert zero rows)
- **AND** a cross-tenant test (seed rows for A and B, set context to A, query for B's rows by id, assert zero rows)

#### Scenario: Admin surface is verified under the admin role

- **WHEN** the integration test suite runs
- **THEN** every PlatformAdmin `?tenant=*` list endpoint SHALL be exercised against a test server wired with both pools
- **AND** the test SHALL probe the connection's `current_user` to assert the admin pool was chosen
- **AND** the response SHALL contain rows from multiple tenants
- **AND** the test SHALL assert a `governance_audit_log` row was written with `admin_scope = true` for each admin access

#### Scenario: Per-tenant handler surface is verified under the default role

- **WHEN** the integration test suite runs
- **THEN** per-tenant list endpoints (`/user`, `/project`, `/org`, `/govern/audit` without `?tenant=*`) SHALL be exercised against a test server whose default pool connects as `aeterna_app`
- **AND** each list SHALL return only the authenticated tenant's rows
- **AND** a request with no resolved tenant context SHALL return `400 select_tenant` (middleware) or an empty list (RLS) — never rows from another tenant
