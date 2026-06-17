# Decide RLS Enforcement Model

## Why

The database layer has 22 tables with `ENABLE ROW LEVEL SECURITY` and 5 with `FORCE ROW LEVEL SECURITY` across `storage/migrations/004_enable_rls.sql`, `016_governance_rls.sql`, and `024_normalize_rls_session_variables.sql`. Every policy is keyed off `current_setting('app.tenant_id', true)`. `PostgresBackend::activate_tenant_context` is called in exactly 2 code paths (both in `cli/src/server/sync.rs`). The remaining 100+ query sites never set the session variable.

Tests pass anyway because the shipped `DATABASE_URL` points at the `postgres` superuser (implicit BYPASSRLS), so every RLS policy silently no-ops. The actual tenant isolation is the `WHERE tenant_id = $N` clauses in repository code. Any handler that forgets that clause leaks cross-tenant rows and the RLS policies we wrote do nothing to stop it.

There is no production environment or live user base yet. The window to fix the enforcement model without operational ceremony is now. Issue #58 requires a recorded decision under `openspec/changes/` before implementation.

## What Changes

This change ratifies **Option A — activate RLS on every request-scoped connection** and records it in the `runtime-security-hardening` capability. Three options are analyzed in `design.md`; the decision summary is normative:

- **Option A (chosen).** Non-BYPASSRLS application role + transaction-scoped `SET LOCAL app.tenant_id` + repository refactor so every query acquires its connection through a `with_tenant_context` helper. Real database-enforced isolation.
- **Option B (rejected).** Drop all RLS policies, app-layer-only. Honest but loses all defense in depth.
- **Option C (rejected).** RLS as a CI-only gate, prod stays BYPASSRLS. Captures regression value but leaves the runtime defenseless. The CI suite from C survives as a permanent regression guard inside Bundle A.2.

Option A is the right call even pre-production: it locks in the enforcement model now while the blast radius of a refactor is zero, rather than inheriting the BYPASSRLS posture into launch.

### Dual-role, dual-pool design

Option A is implemented with **two PostgreSQL roles and two connection pools**, not one. This is integral to the design, not an operational afterthought:

| Role            | Pool                          | `BYPASSRLS` | Used by                                                                              |
| --------------- | ----------------------------- | ----------- | ------------------------------------------------------------------------------------ |
| `aeterna_app`   | `state.pool` (default, large) | No          | Every per-tenant request; async workers running per-tenant jobs                      |
| `aeterna_admin` | `state.admin_pool` (size=4)   | Yes         | PlatformAdmin with `?tenant=*`; scheduled cross-tenant maintenance; migration runner |

Request routing is driven by two explicit helpers:

```
with_tenant_context(&ctx, |tx| …)   // default pool, BEGIN + SET LOCAL app.tenant_id + body + COMMIT
with_admin_context(&ctx, |tx| …)    // admin pool, BEGIN + body + COMMIT (no SET LOCAL; RLS bypassed by role)
```

The router picks based on `ctx.scope`:

- `Tenant(_)` or `CrossTenantSingle(_)` → `with_tenant_context` (99% of traffic)
- `CrossTenantAll` and actor is `PlatformAdmin` → `with_admin_context`
- Scheduled jobs → see pattern below

Every `with_admin_context` call is auto-audited (actor + `acting_as_tenant_id = NULL` + `admin_scope = true`) so cross-tenant reads are traceable. A CI lint forbids any code path from reaching `state.admin_pool` without going through `with_admin_context`.

### Scheduled jobs pattern

Scheduled work splits into two shapes; the helpers compose to cover both:

- **Per-tenant jobs** (sync, per-tenant backup, per-tenant GDPR export) — the scheduler uses `with_admin_context` for a one-shot `SELECT id FROM tenants WHERE active`, then dispatches each tenant's work through `with_tenant_context(&TenantContext::from_scheduled_job(t, job_id), …)`. RLS enforces scope on the per-tenant body.
- **Truly cross-tenant jobs** (audit log compaction, cross-tenant analytics rollups, global rate-limit sweeps) — run entirely under `with_admin_context(&system_ctx, …)`. `system_ctx` is a sentinel `TenantContext` that represents "no human actor, internal system"; audit rows written during these jobs have `actor_type = 'system'`.

### Staged implementation (3 bundles)

The implementation lands in three stacked PRs; see `tasks.md`:

- **Bundle A.1 — Hazard fixes (prerequisite).** Close H1 (session-scope `set_config(..., false)` on pooled connections) and H2 (legacy alternate GUC names not fully normalized on the app side). Safe to merge immediately; no behavior change under BYPASSRLS, essential invariant once BYPASSRLS is gone.
- **Bundle A.2 — Roles + migration + dual pools + CI verification.** Migration 025 creates both roles with correct grants. `AppState` grows a second pool. `with_tenant_context` + `with_admin_context` helpers land with a compile-time exclusion (direct `state.pool`/`state.admin_pool` access is lint-warned). CI suite runs every RLS table end-to-end under `aeterna_app`, and every admin-scoped handler under `aeterna_admin`.
- **Bundle A.3 — Call-site refactor + cutover.** Refactor the ~100 repository query sites in 4–6 domain-themed waves (user → org → project → team → governance → audit → sync → backup/gdpr), each wave a standalone PR against the base branch. Final wave flips `DATABASE_URL` to `aeterna_app`, wires the admin pool URL, deletes the orphan `activate_tenant_context` calls in `sync.rs` (Hazard H3), and graduates the direct-pool-access lint from warn to deny.

Bundles A.1 and A.2 are independent; either can land first. A.3's waves depend on A.2. The final commit of A.3's last wave is the only one that changes runtime behavior from "tests run under RLS" to "everything runs under RLS" — but since there is no prod and no users, that commit is a one-line env change, not an operational event.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `runtime-security-hardening`: rewrites the PostgreSQL isolation requirement to make row-level security the authoritative runtime control; adds requirements for the two-role/two-pool model, per-request transaction-scoped tenant context, administrative cross-tenant access via the admin pool + helper, session variable hygiene, and end-to-end RLS verification.

## Not In Scope

- Replacing the `app.tenant_id` + legacy alternate GUC namespace with a single unified variable across all migrations. Migration 024 normalized the policies; Bundle A.1 closes the app-side writes. A full retroactive GUC consolidation is deferred as a separate change.
- Changing the tenant resolution middleware. This change enforces at the DB layer; the request-scoped tenant context is assumed to already be correct when it reaches the pool.
- Removing app-layer `WHERE tenant_id = ?` clauses. They remain as required defense in depth on top of RLS; `AGENTS.md` is updated in Bundle A.3's last wave to describe both layers as mandatory, neither optional.
- Operational rollout mechanics (canary, feature flags, rollback runbooks). Not applicable pre-production; would be a separate concern if and when a production deployment is stood up.
