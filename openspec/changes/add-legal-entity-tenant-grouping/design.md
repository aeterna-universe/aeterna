## Context

Aeterna's `Tenant` is *both* the data-plane isolation boundary (RLS, backups,
migrations, quotas) and — by accident of history — the only top-level
organisational entity in the system. That dual role works as long as every
customer is one organisation. It breaks the moment a customer is a corporate
group with subsidiaries, which is the typical shape of any treasury customer
larger than a startup.

The rc.9 architecture review surfaced this in the context of an admin-ui Create
dialog. The narrow question was "should the dialog let me create a Tenant
under another Tenant so I can model subsidiaries?". The answer turned out to
be no, but the *reason* required articulating the right model:

  - data isolation between subsidiaries is the *correct* default — different
    ERPs, different compliance regimes, different data-residency, different
    backup schedules — so each subsidiary should be its own tenant;
  - but the customer-visible group identity ("Acme Holding") still needs to
    exist somewhere, otherwise sales, billing, customer success, and admin UX
    have to reconstruct it from spreadsheets.

v1.5.x ships a metadata-only seed for this proposal: migration 033 adds a
nullable `tenants.legal_entity_name` text column. That column is not the
feature — this proposal is. The column exists so that this proposal can land
without asking customer success to redo data entry. When this proposal's
migration runs, every distinct non-NULL value of `legal_entity_name` becomes
a row in the new `legal_entities` table, and the column is dropped.

The existing infrastructure that this proposal extends:

  - `TenantStore` and `TenantRecord` (proven shape; the new
    `LegalEntityStore` mirrors it).
  - The `require_platform_admin` / `require_tenant_admin` family of auth
    helpers (the new `require_legal_entity_admin_for(id)` slots in alongside).
  - The Utopia OpenAPI registry (one more nest of routes).
  - The audit-logging helper `audit_tenant_action` (the cross-tenant rollup
    handlers call it once per underlying tenant they touch).
  - The admin-ui's existing tenant switcher (gains a Legal Entity grouping
    level above it).

## Goals / Non-Goals

**Goals:**

  - First-class persistence for the Legal Entity concept (a `legal_entities`
    row per corporate entity, FK from `tenants`).
  - A `LegalEntityAdmin` principal that can read across the tenants of one
    legal entity, *without* loosening RLS at the tenant boundary.
  - Cross-tenant rollup endpoints (read-only) so the admin UI can show a
    single dashboard per legal entity.
  - Lossless promotion of the v1.5.x `legal_entity_name` text column into the
    new schema.
  - CLI parity with the API.
  - Per-tenant audit trail completeness preserved — a rollup that touches N
    tenants writes N audit rows, not one.

**Non-Goals:**

  - Any change to RLS policy or to what data lives at the tenant boundary.
    Tenants remain the data isolation unit; legal entities are *above* that.
  - Recursive same-type parenting inside a tenant (Tenant → Tenant → …).
    Explicitly rejected; documented in the proposal's "Why".
  - Many-to-many tenant ↔ legal entity. The FK is single-valued; a tenant
    has at most one legal entity.
  - Cross-LE moves of tenants ("Acme sells subsidiary X to Globex Group").
    This is its own proposal because of audit/data-residency questions.
  - Billing system integration. The contract metadata fields on
    `legal_entities` are pure record-keeping; no invoicing logic lands here.
  - Per-LE quotas, per-LE backup policies, per-LE migrations. Those remain
    per-tenant; aggregate views can be computed but enforcement is not
    pulled up to the LE layer.

## Decisions

### Decision 1: Legal Entity is **not** an RLS boundary

The `LegalEntityAdmin` principal cannot directly observe row-level data in any
tenant. The `legal_entities` table is consulted at the *handler* level to
resolve which tenants the principal may aggregate across, and then the handler
issues N separate, individually scoped queries against each tenant's data
with the standard tenant context set. RLS policies are *unchanged*.

  - **Why**: RLS is enforced at the database level by `current_setting('aeterna.tenant_id')`;
    introducing a "legal_entity" axis into RLS would either require
    multi-axis policies (a known footgun — PostgreSQL row policies become
    very hard to reason about with two predicates) or a privileged role that
    bypasses tenant scoping (which is *exactly* the kind of cross-tenant
    leak vector we don't want). Keeping the cross-tenant aggregation in
    handler code preserves the simple, well-understood RLS invariant.
  - **Trade-off**: Handler-level aggregation costs N queries instead of one
    join. For the scale we expect (a holding tenant has tens of
    subsidiaries, not thousands), this is fine; the queries can run in
    parallel via `tokio::join!` for the rollup endpoints, and the result is
    cached at the LE-summary cache layer (see decision 5).
  - **Alternative considered**: Add a `legal_entity_id` GUC alongside the
    tenant GUC and write RLS policies of the form
    `tenant_id = current_setting('aeterna.tenant_id') OR
     (legal_entity_id = current_setting('aeterna.legal_entity_id') AND ...)`.
    Rejected: it conflates principal authorisation with row visibility, makes
    every row policy in the system harder to audit, and is not reversible
    once production data is touching it.

### Decision 2: Strict 1:N (tenant has at most one legal entity)

A tenant has either zero or one legal entity. Many-to-many is explicitly
rejected.

  - **Why**: Tenants represent a *legal* entity's operations. A subsidiary
    that genuinely belongs to two parents simultaneously does exist (joint
    ventures), but that's a corporate accounting fiction with messy
    consequences for billing, audit, and right-to-delete. We want one MSA,
    one billing relationship, one customer-success owner per tenant.
  - **Migration path if we ever change our minds**: Promote `legal_entity_id`
    from a column on `tenants` to a join table `tenant_legal_entities`. The
    storage layer's tenant queries already pass through a wrapper that we
    can switch from a single FK lookup to an N-row aggregation. Not free,
    but contained.

### Decision 3: Migrate the v1.5.x text column losslessly

The v1.5.x `tenants.legal_entity_name` text column is **the** seed data for
this proposal's migration. Every distinct non-NULL value becomes a
`legal_entities` row; the column is then dropped.

  - **Why**: The text column was introduced in migration 033 specifically so
    sales/ops could record the corporate hierarchy in v1.5.x without waiting
    for this proposal. Dropping that data on the floor would re-introduce
    the manual-data-entry problem the column was meant to solve.
  - **Mechanics** (one transaction):
        INSERT INTO legal_entities (name, slug, source_owner)
            SELECT DISTINCT legal_entity_name,
                   slugify(legal_entity_name),
                   'admin'
            FROM tenants WHERE legal_entity_name IS NOT NULL;
        UPDATE tenants t SET legal_entity_id = le.id
            FROM legal_entities le WHERE t.legal_entity_name = le.name;
        DROP INDEX idx_tenants_legal_entity_name;
        ALTER TABLE tenants DROP COLUMN legal_entity_name;
  - **Idempotency**: the migration uses `IF NOT EXISTS` for additive changes
    and is safe to re-run (the column drop is the irreversible step; on
    re-run, the column is already gone, and `IF EXISTS` makes it a no-op).

### Decision 4: `ON DELETE SET NULL`, not CASCADE

Deleting a legal entity sets owning tenants' `legal_entity_id` to NULL
(de-grouping them). It does **not** delete the tenants.

  - **Why**: A tenant is a customer-paying, RLS-isolated unit; deleting all
    of Acme Holding's tenants because someone ran `DELETE FROM legal_entities
    WHERE id = ...` is the kind of operator-error blast radius we make a
    priority of avoiding. Tenants must be deactivated explicitly, one at a
    time, through the existing tenant deactivation flow.
  - **Soft-delete the LE itself**: Mirror the tenant `status` pattern (active,
    inactive). Hard-deletion of an LE row is gated behind
    `--force-hard-delete` on the CLI and an explicit confirmation on the
    admin UI; in both cases all owning tenants must already be detached.

### Decision 5: Rollup endpoint result caching

The `GET /api/v1/legal-entities/{id}/summary` endpoint is cached for 60
seconds in Redis behind a key `le_summary:{id}`. Mutations that could
invalidate it (tenant attach/detach, tenant deactivation) explicitly
`DEL` the key.

  - **Why**: This endpoint can be polled by the admin UI dashboard. With N
    tenants behind one LE, an uncached call costs N queries; if 5 admins are
    looking at the dashboard simultaneously that's 5N queries every refresh.
    A 60s TTL trades trivial freshness for substantial back-end load relief.
  - **Why 60s and not longer**: Customer-success scenarios commonly involve
    "I just attached/detached a tenant; show me the new summary". 60s feels
    fast, and explicit invalidation on attach/detach makes the common-case
    interactions feel instantaneous.

### Decision 6: Per-tenant audit completeness for cross-tenant reads

A `GET /api/v1/legal-entities/{id}/summary` that touches N tenants writes N
`audit_action` rows, one per underlying tenant scope, *not* one row at the
LE level.

  - **Why**: Tenant-level auditors must be able to answer "who read data from
    my tenant" by querying tenant-scoped audit logs. If we collapsed the
    rollup into a single LE-level audit row, that question becomes
    unanswerable from the tenant scope, breaking SOC-2 audit story.
  - **What about audit volume**: Each row is a few hundred bytes; N=20
    subsidiaries × a per-minute dashboard refresh × 8h × 5 admins =
    ~50K rows/day across all tenants. Within the existing audit retention
    envelope (audit logs already roll to S3 after 30 days; see
    `add-day2-operations`).

## Risks

  - **Risk**: A LegalEntityAdmin principal exfiltrates data by repeatedly
    calling rollup endpoints with different filters until they reconstruct
    per-tenant detail.
  - **Mitigation**: Rollup endpoints expose *aggregations*, not row-level
    data. The handler enforces a minimum aggregation cardinality (e.g.
    "open incidents" returns a count, not a list of incident IDs); detail
    drill-down requires a tenant-scoped principal.

  - **Risk**: The migration that drops `tenants.legal_entity_name` runs against
    a cluster where some tenant rows have a `legal_entity_name` value that
    `slugify` collapses to the same slug as another (e.g. "Acme Holding" and
    "acme holding"). The `legal_entities.slug UNIQUE` constraint would fire
    and the migration would fail mid-way.
  - **Mitigation**: The migration script first runs a pre-flight
    `SELECT COUNT(*) FROM (SELECT DISTINCT slugify(legal_entity_name) FROM
    tenants WHERE legal_entity_name IS NOT NULL GROUP BY 1 HAVING COUNT(*)
    > 1)` and aborts with a clear message and a manual remediation script
    if it returns > 0. Documented in the migration header.

  - **Risk**: The new `LegalEntityAdmin` principal is recognised by the
    server but the admin-ui doesn't yet render the LE navigation, so the
    principal sees the regular tenant switcher and is confused.
  - **Mitigation**: Server and admin-ui ship in lockstep; the auth subject
    extension is feature-flagged (`features.legal_entity_admin = false` in
    the server config) until admin-ui rolls out the navigation. CLI
    commands work without the UI.

## Open Questions

  - Should `LegalEntityAdmin` be able to *create* tenants under their LE
    without PlatformAdmin involvement? Initial answer: no, tenant creation
    is a PlatformAdmin operation because of provisioning side-effects
    (Qdrant collection, DuckDB graph, IdP setup). Revisit after first ten
    LE-using customers.
  - Should the LE detail page expose a "detach all tenants" button? Initial
    answer: no; require per-tenant detach to keep the blast radius small.
