# Tasks

## 1. Decision ratification

- [ ] 1.1 Review `proposal.md` and `design.md` with reviewing architect
- [ ] 1.2 Record the decision on issue #58 as a top-level comment referencing this change
- [ ] 1.3 Lock the dual-role/dual-pool design (see `design.md` §4) as the target state

## 2. Bundle A.1 — Hazard fixes (prerequisite, independently shippable)

**Goal:** close H1 and H2 before the role flip. No behavior change under BYPASSRLS; essential invariant once BYPASSRLS is gone. Target: 1 PR, ≤ 1 week.

- [ ] 2.1 **H1 — `storage/src/postgres.rs:63`.** Change `set_config('app.tenant_id', $1, false)` → `set_config('app.tenant_id', $1, true)`. Document that `activate_tenant_context` requires an open transaction; add a `debug_assert!` at entry that verifies the connection is inside a transaction.
- [ ] 2.2 **H1 — `cli/src/server/backup_api.rs:1597`.** Move the `set_config` call inside the existing `BEGIN` scope and flip the third argument to `true`.
- [ ] 2.3 **H1 — `cli/src/server/gdpr.rs:407,490,580`.** `true` is already passed; verify each is inside an explicit transaction and add a module-level `// INVARIANT:` comment pinning the expectation.
- [ ] 2.4 **H2 — dual-GUC normalization on the app side.** Grep the entire codebase for the canonical tenant GUC plus the three legacy alternate names. Normalize every app-side `set_config` and `current_setting` call to `app.tenant_id`. Migration files are not touched (they're historical record).
- [ ] 2.5 **H1 regression test — `storage/tests/session_variable_hygiene.rs`.** Acquire a connection, set `app.tenant_id` via `activate_tenant_context`, return it, acquire a fresh connection, assert `current_setting('app.tenant_id', true)` returns empty. Fails if anyone reverts to session-scope.
- [ ] 2.6 **H2 regression test — `storage/tests/guc_namespace.rs`.** Compile-time grep (via `include_str!` over every `*.rs` under `cli/src/` and `storage/src/`) that asserts no legacy GUC name appears outside the migrations directory.

## 3. Bundle A.2 — Roles, migrations, dual pools, CI verification (independently shippable)

**Goal:** stand up both roles, wire both pools, land both helpers, ship the permanent RLS regression guard. `DATABASE_URL` stays on the current role. Target: 1 PR, ≤ 2 weeks.

### 3.1 Migration

- [ ] 3.1.1 `storage/migrations/025_add_app_roles.sql`. Idempotent `CREATE ROLE` via `DO $$ … pg_roles WHERE rolname = … $$`.
- [ ] 3.1.2 `CREATE ROLE aeterna_app LOGIN NOBYPASSRLS PASSWORD :'app_password'`.
- [ ] 3.1.3 `CREATE ROLE aeterna_admin LOGIN BYPASSRLS PASSWORD :'admin_password'`.
- [ ] 3.1.4 `GRANT USAGE ON SCHEMA public TO aeterna_app, aeterna_admin`.
- [ ] 3.1.5 `GRANT SELECT, INSERT, UPDATE, DELETE` on every table currently having `rowsecurity = true` to `aeterna_app`. Enumerated explicitly (no `GRANT … ON ALL TABLES` catch-all) so missing a grant on a new RLS table fails the A.2.5.1 pre-flight.
- [ ] 3.1.6 `GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO aeterna_admin` — `aeterna_admin` is BYPASSRLS and operates globally, so a broad grant is correct.
- [ ] 3.1.7 `GRANT USAGE, SELECT` on every sequence both roles need.
- [ ] 3.1.8 Migration header documents the dual-role design and points to `openspec/changes/decide-rls-enforcement-model/proposal.md`.

### 3.2 AppState + helpers

- [ ] 3.2.1 `AppState` grows `admin_pool: PgPool` alongside the existing `pool: PgPool`.
- [ ] 3.2.2 Admin pool size capped at 4 (`PgPoolOptions::new().max_connections(4)`) — admin operations are rare, constraining the pool makes accidental hot-loop use of the admin pool a visible error.
- [ ] 3.2.3 Admin pool connection string sourced from `DATABASE_URL_ADMIN` env var. Default: same host/db as `DATABASE_URL`, credentials swapped to `aeterna_admin` / `APP_DB_ADMIN_PASSWORD`.
- [ ] 3.2.4 `storage/src/postgres.rs::with_tenant_context` — signature `async fn with_tenant_context<F, T>(&self, ctx: &TenantContext, body: F) -> Result<T>` where `F: FnOnce(&mut Transaction<'_, Postgres>) -> BoxFuture<Result<T>>`. Implementation: `self.pool.begin()` → `SET LOCAL app.tenant_id = $1` → `body(&mut tx)` → `tx.commit()`. Rustdoc states this is the only valid way to reach `state.pool` on an RLS-protected path.
- [ ] 3.2.5 `storage/src/postgres.rs::with_admin_context` — same shape but uses `self.admin_pool.begin()` and does NOT issue `SET LOCAL`. Rustdoc states every call is admin-only and is auto-audited per A.2.2.7.
- [ ] 3.2.6 `TenantContext::system_ctx()` sentinel constructor — represents "no human actor, internal system." Carries `actor_type = 'system'` for audit attribution.
- [ ] 3.2.7 `TenantContext::from_scheduled_job(tenant_id, job_id)` constructor — used by per-tenant scheduled work (see proposal.md "Scheduled jobs pattern").
- [ ] 3.2.8 Admin audit wrapper: every `with_admin_context` call records an audit row with `actor_id`, `actor_type`, `admin_scope = true`, `acting_as_tenant_id = NULL`. Implementation goes in the helper itself so it cannot be bypassed.

### 3.3 CI verification suite

- [ ] 3.3.1 `cli/tests/rls_enforcement_test.rs` — pre-flight: enumerate `pg_tables WHERE rowsecurity = true`; assert every such table has `SELECT` granted to `aeterna_app`; fail with a named-table error if any grant is missing.
- [ ] 3.3.2 For every RLS table: positive path (connect as `aeterna_app`, `BEGIN`, `set_config('app.tenant_id', tenant_a, true)`, `SELECT *`, assert only tenant_a's rows).
- [ ] 3.3.3 For every RLS table: negative path (same connection without `set_config`, assert zero rows).
- [ ] 3.3.4 For every RLS table: cross-tenant path (seed rows for A and B, set context to A, `SELECT * WHERE id = $b_id`, assert zero rows).
- [ ] 3.3.5 `cli/tests/rls_admin_surface_test.rs` — PlatformAdmin list endpoints (`/user`, `/project`, `/org`, `/govern/audit`) with `?tenant=*` exercised through a test server wired with both pools. Assert the admin pool is chosen (via a probe that checks the connection's `current_user`) and that cross-tenant rows are returned.
- [ ] 3.3.6 `cli/tests/rls_handler_smoke.rs` — per-tenant list endpoints exercised with a test server whose default pool connects as `aeterna_app`. Assert each list returns only the authenticated tenant's rows; assert a request with no resolved tenant context returns `400 select_tenant` (middleware) or empty list (RLS).
- [ ] 3.3.7 `cli/tests/admin_pool_access_lint.rs` — compile-time grep asserting no code path outside `with_admin_context` references `state.admin_pool` directly. Warn-level in A.2; graduates to deny in A.3's last wave.

### 3.4 Docs

- [ ] 3.4.1 `DEVELOPER_GUIDE.md` — document the two-helper pattern, the scheduled-jobs pattern, and the `system_ctx` sentinel.
- [ ] 3.4.2 `AGENTS.md` — add a short "RLS enforcement" section: RLS is authoritative, app-layer `WHERE tenant_id = ?` is required DiD, direct pool access is forbidden outside the two helpers.

## 4. Bundle A.3 — Call-site refactor + cutover (multi-PR, wave-by-wave)

**Goal:** thread every RLS-protected query through `with_tenant_context`; thread every admin-scope query through `with_admin_context`. Each wave is an independent PR, each wave runs against the A.2 CI suite. Final wave flips `DATABASE_URL` to `aeterna_app`. Target: 4–6 PRs, ≤ 3 weeks.

- [ ] 4.1 **Wave 1 — `user_api.rs` + `team_api.rs` + `org_api.rs` + `project_api.rs`.** Refactor list/get/create/update/delete paths to `with_tenant_context`. Per-request admin paths (`?tenant=*`) route through `with_admin_context`. ~25 call sites.
- [ ] 4.2 **Wave 2 — `govern_api.rs` + `govern_policy.rs` + audit write paths.** ~15 call sites. Audit write paths use whichever helper the originating handler is in — `with_tenant_context` for tenant-scoped writes, `with_admin_context` for admin writes.
- [ ] 4.3 **Wave 3 — `memory_api.rs` + `knowledge_api.rs` + ingest paths.** ~20 call sites.
- [ ] 4.4 **Wave 4 — `sync.rs` + `webhook.rs` + async worker paths.** Workers use `with_tenant_context(&TenantContext::from_scheduled_job(t, job_id), …)` for per-tenant work and `with_admin_context(&system_ctx, …)` for cross-tenant maintenance. ~15 call sites.
- [ ] 4.5 **Wave 5 — `backup_api.rs` + `gdpr.rs` + admin paths.** Convert manual `set_config` call sites (now fixed by A.1) to the helper pattern. Global admin operations route through `with_admin_context`. ~10 call sites.
- [ ] 4.6 **Wave 6 — sweep + cutover.** Final wave:
  - [ ] 4.6.1 `storage/tools/find_unwrapped_queries.sh` audit — any remaining direct `self.pool.acquire()` / `self.pool.begin()` / `self.admin_pool.*` outside the helpers either (a) gets refactored or (b) carries `#[allow(direct_pool_access)]` with a justification comment.
  - [ ] 4.6.2 Flip `DATABASE_URL` in `docker-compose.yml` and every `.env.example` to `postgres://aeterna_app:${APP_DB_PASSWORD}@…`.
  - [ ] 4.6.3 Add `DATABASE_URL_ADMIN=postgres://aeterna_admin:${APP_DB_ADMIN_PASSWORD}@…` to every `.env.example`.
  - [ ] 4.6.4 Delete the orphan `activate_tenant_context` calls in `cli/src/server/sync.rs:131,256` (Hazard H3).
  - [ ] 4.6.5 Graduate both lints (direct-pool-access in A.2.3.7 and admin-pool-access) from warn to deny.
  - [ ] 4.6.6 Update `AGENTS.md` to state the final model is in effect; update `DEVELOPER_GUIDE.md` with `with_tenant_context` / `with_admin_context` as canonical.
  - [ ] 4.6.7 Close issue #58 referencing the merged wave 6 PR.
