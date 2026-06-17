# Admin API — Cross-Tenant Listing

> Status: **stable** for all four values of `?tenant=` (`*`, `all`, `<slug>`, `<uuid>`) on every endpoint listed below.
>
> Shipped in [RFC #56](https://github.com/kikokikok/aeterna/pull/56) · Tracked in [#44.d](https://github.com/kikokikok/aeterna/issues/44).

This document is the canonical reference for the `?tenant=` query parameter across admin list endpoints. It is the source of truth that a future OpenAPI schema will be generated from.

## Endpoints covered

| Endpoint         | Since PR | §     | `?tenant=*` envelope | Per-row `tenantId`/`tenantSlug` |
|------------------|----------|-------|----------------------|---------------------------------|
| `GET /admin/tenants` | #63  | §2.1 | ✅                    | N/A (items _are_ tenants)       |
| `GET /user`      | #64      | §2.2 | ✅                    | ✅                               |
| `GET /project`   | #65      | §2.3 | ✅                    | ✅                               |
| `GET /org`       | #66      | §2.4 | ✅                    | ✅                               |
| `GET /govern/audit` | #68 + Bundle D | §2.5 | ✅                    | ✅ (see [Audit notes](#audit-notes)) |

## The `?tenant=` grammar

| Value                      | Meaning                                                                  | Gate                    | Response shape                 |
|----------------------------|--------------------------------------------------------------------------|-------------------------|--------------------------------|
| _absent_ or `?tenant=`     | Tenant-scoped to the caller's tenant (from `X-Tenant-ID` / default).    | Existing auth gate      | Legacy bare array (unchanged)  |
| `?tenant=*`                | Cross-tenant listing — items from every active tenant.                  | **PlatformAdmin**       | Envelope (see below)           |
| `?tenant=all`              | Deprecated alias for `*`. Emits a `compat` warning log. Will be removed in a future minor. | **PlatformAdmin** | Envelope                       |
| `?tenant=<slug-or-uuid>`   | Single-foreign-tenant listing. Resolves via the tenant store; unknown values → `404 tenant_not_found`. | **PlatformAdmin** (or member of the target tenant) | Envelope with `scope:"tenant"` |

## The cross-tenant response envelope

When `?tenant=*` (or its alias) resolves successfully, the response body uses a stable envelope that replaces the legacy bare array:

```json
{
  "success": true,
  "scope":   "all",
  "items": [
    {
      "id":          "…",
      "name":        "…",
      "tenantId":    "tenant-uuid",
      "tenantSlug":  "acme",
      "tenantName":  "Acme Corp",
      "…":           "… (endpoint-specific fields)"
    }
  ]
}
```

### Contract guarantees

For `/user`, `/project`, and `/org`, the following are locked by the [§4.1 contract test](../../cli/tests/server_runtime_test.rs) (search `assert_cross_tenant_envelope_contract`):

1. HTTP status is `200`.
2. `body.success == true`.
3. `body.scope == "all"`.
4. `body.items` is an array.
5. Every item has a **non-empty string** `tenantId`.
6. Every item has a **non-empty string** `tenantSlug`.
7. When data spans multiple tenants, `items` reflects that (the contract test asserts `≥ min_tenants` distinct tenant ids — catches accidental single-tenant collapse).

Ordering is stable across pages: `(tenant_id ASC, name ASC, id ASC)`.

### Audit notes

`GET /govern/audit` carries per-row tenant attribution via `governance_audit_log.acting_as_tenant_id` (populated on every tenant-scoped governance write since Bundle D — see [#44.d tasks §2.5](../../openspec/changes/add-cross-tenant-admin-listing/tasks.md)). The wire shape matches the cross-tenant envelope (`tenantId`, `tenantSlug`, `actingAsTenantId` on each item), and `?tenant=<slug>` filters rows via the same column.

Two backward-compatibility notes specific to this endpoint:

- **Pre-Bundle-D rows** (rows written before `acting_as_tenant_id` was threaded through every `log_audit` call site) have `acting_as_tenant_id = NULL`. Under `?tenant=*` they surface with `tenantId: null` / `tenantSlug: null` / `actingAsTenantId: null`; under `?tenant=<slug>` they are filtered out by the SQL `= $tenant_id` clause. This is the only correct behavior — the write-time attribution does not exist so the read layer cannot fabricate it.
- **Tools-layer writes** (callers from the `mk-tools` crate that aren't attached to a request tenant) pass `acting_as_tenant_id = None` deliberately; those rows behave identically to pre-Bundle-D rows (visible under `scope=all`, excluded from `scope=tenant`).

The audit endpoint has a dedicated test (`list_audit_cross_tenant_scope_gates_and_filter_compose`) rather than using the shared §4.1 helper, because the helper asserts every item carries non-empty tenant decoration — a property the audit endpoint cannot uphold in the presence of NULL-attributed rows.

## Error responses

| HTTP | `error` code             | When                                                                |
|------|--------------------------|---------------------------------------------------------------------|
| `403` | `forbidden_scope`       | `?tenant=*` or `?tenant=all` called by a non-PlatformAdmin.         |
| `404` | `tenant_not_found`      | `?tenant=<slug-or-uuid>` resolved to no active tenant.              |
| `501` | `scope_not_implemented` | Reserved for future endpoints that haven't yet implemented `?tenant=<slug>`. All five shipped endpoints support it. |
| `400` | `scope_not_allowed_for_write` | Future guard for write operations under `?tenant=*` (§5.8).   |

All error bodies follow the standard error envelope:

```json
{
  "success": false,
  "error":   "forbidden_scope",
  "message": "Human-readable description",
  "required_role": "PlatformAdmin"
}
```

## Filter composition

Endpoint-specific query parameters (e.g. `?actor=`, `?since=`, `?action=` on `/govern/audit`; `?tenant=` on `/org`) **compose with `?tenant=*`**. The envelope is a response-shape-only transform applied after the storage query. There is no special casing: the same filter rules apply in tenant-scoped and cross-tenant modes.

## Backward compatibility

- Omitting `?tenant=` is always equivalent to the pre-RFC behavior. No existing client is impacted by the introduction of the parameter.
- `?tenant=` with an empty string value is treated as absent (defensive for clients sending unfilled form fields).
- The `?tenant=all` alias is intentionally accepted so clients that tried the feature before it was formally stabilized don't break; they now see a `compat` warning log directing them to `?tenant=*`.

## Related

- RFC: [PR #56](https://github.com/kikokikok/aeterna/pull/56)
- Resolver implementation: [`cli/src/server/context.rs`](../../cli/src/server/context.rs) (search `resolve_list_scope`)
- Contract test: [`cli/tests/server_runtime_test.rs`](../../cli/tests/server_runtime_test.rs) (search `assert_cross_tenant_envelope_contract`)
- RLS boundary guard: [`storage/tests/rls_boundary_test.rs`](../../storage/tests/rls_boundary_test.rs)
- CLI integration: see [CLI flags](#cli-flags) below.

## CLI flags

The `?tenant=` grammar is surfaced to the `aeterna` CLI via two mutually-exclusive flags, flattened into every list command that supports cross-tenant scoping:

| Flag              | Server query                 | Envelope emitted        |
|-------------------|------------------------------|-------------------------|
| (none)            | `GET /…` (no `?tenant=`)     | legacy bare array       |
| `--all-tenants`   | `GET /…?tenant=*`            | `scope: "all"`          |
| `--tenant <slug>` | `GET /…?tenant=<slug>`       | `scope: "tenant"`       |

Commands that honor these flags today:

- `aeterna user list [--all-tenants | --tenant <slug>]`
- `aeterna org list  [--all-tenants | --tenant <slug>]`
- `aeterna govern audit [--all-tenants | --tenant <slug>]`

Combining the two flags is rejected at parse time (clap `conflicts_with`) — combining them is always a client bug.

In cross-tenant views the human-readable output grows a leading `[tenant]` column so items that would otherwise look ambiguous (e.g. two orgs with the same name in different tenants) stay distinguishable. `--json` mode emits the raw server envelope unchanged so automation can rely on the exact contract documented above.

## Deprecated: `X-Target-Tenant-Id` header

Prior to #44.d, PlatformAdmins could target a foreign tenant by setting the `X-Target-Tenant-Id` request header. That path is now **deprecated** in favor of the `?tenant=<slug>` query parameter documented above.

- The header is **still honored** — existing CI scripts and support tools won't break when this change ships.
- Each request carrying the header emits a `tracing::warn!(target = "compat", header = "X-Target-Tenant-Id", replacement = "?tenant=<slug-or-*>", ...)` log line including a prefix of the raw value so operators can correlate with client traffic and find stragglers.
- Removal is planned for a future minor version — tracked in the follow-up §8 work. Clients should migrate to `?tenant=<slug>` / `?tenant=*` at their earliest convenience.

Why replace it: the header was opaque at the URL level (invisible in access logs, browser devtools, curl transcripts) and bypassed the per-endpoint scope-gate that `?tenant=` now enforces uniformly (`forbidden_scope` / `scope_not_implemented` / `tenant_not_found`). The query parameter is explicit, testable, and subject to the §4.1 envelope contract test.
