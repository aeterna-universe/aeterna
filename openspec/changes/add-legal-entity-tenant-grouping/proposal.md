> Superseded by `refactor-account-tenant-hierarchy`, which establishes `Account -> Tenant -> Organization -> Team -> Project` as the canonical model.

## Why

Aeterna treasury customers operate as **corporate groups** — a holding tenant plus N
subsidiaries that may run different ERPs, different cash-management workflows, different
compliance regimes, and different geographic data-residency requirements. Today the
platform has no first-class concept for this:

1. **`Tenant` is the only top-level entity.** It is also our RLS boundary, our migration
   boundary, our backup boundary, our quota boundary. That is correct — each subsidiary
   should keep its data isolated from siblings, and "one tenant per subsidiary" is the
   right answer for data plane concerns.
2. **There is no rollup _above_ tenant.** Sales, billing, customer success, and the
   admin UI all need to know "all the tenants belonging to Acme Holding", but nothing in
   the schema captures that fact. Today the only signal is the
   `tenants.legal_entity_name` text column added in v1.5.x by migration 033 — a
   metadata seed precisely so this proposal could land without losing pre-existing data.
3. **An alternative was considered and rejected.** Modelling subsidiaries as recursive
   `Tenant → Tenant` parenting _inside_ a single tenant was the obvious shortcut. It
   was rejected during the rc.9 architecture review because (a) it forces all
   subsidiaries to share RLS, schema, migrations, backups, and retention; (b) it
   conflates a corporate-structure question with a data-isolation question that already
   has a clean answer; (c) it creates two ways to model the same relationship in the
   product (tenant-level grouping vs. unit-level recursion), which is a bug factory; and
   (d) it duplicates the latent ancestor-cycle / matrix-validation surface area that
   was already a source of bugs (see `update_unit` fix in the same rc.9 sweep).
4. **Customer-facing pain.** Without a Legal Entity layer there is no consolidated
   "all my tenants" view in the admin UI, no single MSA-to-tenants link, no way for a
   customer's central treasury team to scan health across all subsidiaries from one
   pane, and no way for sales to see their portfolio without joining external
   spreadsheets to the platform.

## What Changes

- Add a first-class **`legal_entities`** table at the same persistence tier as
  `tenants`, with one row per corporate legal entity (e.g. "Acme Holding", "Globex
  Industries"). One legal entity owns N tenants; a tenant has at most one legal entity
  (1:N, not M:N). Promotion of the v1.5.x `tenants.legal_entity_name` text column to a
  proper `legal_entity_id` UUID FK is part of this migration.
- Add a **`LegalEntityAdmin`** principal type that can read across all tenants of one
  legal entity. Critically, this is **NOT an RLS bypass** — the principal cannot
  observe row-level data in any tenant directly. The handler walks
  `legal_entities → tenants` and runs _separate_ per-tenant scoped queries, then
  aggregates results. RLS remains enforced unchanged at the tenant boundary.
- Add a **cross-tenant rollup API** under `/api/v1/legal-entities/{id}/...` exposing
  read-only aggregations (tenant list, health summary, open-incident count, storage
  usage rollup, license-seat usage rollup). Write paths stay tenant-scoped.
- Add **admin-ui navigation** with a Legal Entity grouping level above the existing
  tenant switcher: a Legal Entity Admin sees their LE in the top nav, can drill into
  any of its tenants, and can view a cross-tenant dashboard for the LE.
- Add **CLI commands** mirroring the API: `aeterna legal-entity create`,
  `aeterna legal-entity list`, `aeterna legal-entity attach <tenant-slug>`,
  `aeterna legal-entity detach <tenant-slug>`.
- Add **billing/contract metadata fields** on `legal_entities` (MSA reference, contract
  start/end, primary billing contact). Pure metadata for now — does not couple to any
  invoicing system; that's left for a separate billing proposal.

## Capabilities

### New Capabilities

- `legal-entity-grouping`: First-class corporate-entity layer above the tenant
  boundary. Owns the `legal_entities` table, the `LegalEntityAdmin` principal, the
  cross-tenant rollup API, the admin-ui Legal Entity navigation, and the CLI commands.
  Explicitly does NOT bypass tenant RLS — cross-tenant aggregation is implemented as
  N per-tenant scoped queries, not as a privileged join.

### Modified Capabilities

- `tenant-management`: `tenants` table gains a nullable `legal_entity_id` FK; the
  `legal_entity_name` text column added in v1.5.x by migration 033 is migrated into
  rows of `legal_entities` and then dropped. `TenantRecord` gains an optional
  `legal_entity` reference.
- `auth`: New principal subject `LegalEntityAdmin` with scoped read access; the auth
  layer learns to resolve this principal and to refuse it on any write path that is
  not explicitly listed as cross-LE-permitted.
- `admin-ui`: Tenant switcher gains an optional Legal Entity grouping level; a new
  Legal Entity dashboard view lists tenants and shows aggregated health.
- `cli`: Adds the `legal-entity` subcommand group.
- `audit`: All cross-tenant rollup reads are individually audited — one audit row per
  underlying tenant scope they touched, not one for the rollup endpoint, so the
  per-tenant audit trail remains complete.

## Impact

- **Affected code**:
    - `storage/migrations/` — new migration creating `legal_entities`, adding
      `tenants.legal_entity_id`, populating it from `legal_entity_name`, and dropping
      the old text column. Lossless migration; see design doc.
    - `storage/src/legal_entity_store.rs` (new) — LegalEntityStore mirroring
      TenantStore's shape.
    - `storage/src/tenant_store.rs` — join `legal_entities` into the tenant queries.
    - `mk_core/src/types.rs` — `LegalEntityRecord`, principal subject extension.
    - `cli/src/server/legal_entity_api.rs` (new) — routes under
      `/api/v1/legal-entities`.
    - `cli/src/server/auth/...` — LegalEntityAdmin principal recognition.
    - `admin-ui/src/...` — navigation refactor, LE dashboard view.
    - `cli/src/commands/legal_entity.rs` (new) — CLI subcommands.
- **Out of scope** (deliberately):
    - Billing system integration (only metadata fields land here).
    - Cross-LE moves of tenants ("Acme sells subsidiary X to Globex") — needs a
      separate proposal because of audit-trail and data-residency questions.
    - Many-to-many (a tenant belonging to multiple LEs) — explicitly not supported;
      the FK is single-valued.
