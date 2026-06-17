## Context

Aeterna currently mixes two separate concerns into one tree:

1. **Tenant isolation** — the real operational boundary for RLS, backups, quotas, provisioning, sync, and runtime configuration.
2. **Business hierarchy** — currently modeled inside a tenant as `Tenant -> Organization -> Team -> Project`.

That shape is backwards for the way most operators actually deploy the system. A customer account or organization typically owns several isolated tenants (`dev`, `test`, `prod`, sometimes region-specific or subsidiary-specific), and each tenant then contains its own organizational hierarchy.

The current shape also complicates the ongoing #130 cleanup:

- modern hierarchy tables still retain a legacy wrapper layer even though tenant is already the effective root for most runtime paths,
- GitHub sync has to invent a tenant-internal tenant node,
- reverse-render and role resolution still need compatibility logic around tenant-rooted hierarchy state,
- manifests force an extra level that usually adds no business value.

This design intentionally treats the earlier `add-legal-entity-tenant-grouping` proposal as a stepping stone, not the final model. The target model is:

```text
Account
└── Tenant (dev/test/prod/...)
    └── Organization
        └── Team
            └── Project
```

## Goals / Non-Goals

**Goals:**

- Introduce a canonical **Account** layer above tenants.
- Make `Tenant` the explicit root of the in-tenant hierarchy.
- Remove `Tenant` from tenant-facing hierarchy contracts and storage.
- Let tenant manifests, sync, admin APIs, and OPAL views all agree on `Tenant -> Organization -> Team -> Project`.
- Support the common deployment pattern of one account owning several environment-specific tenants.
- Fail closed for ambiguous migrations rather than inventing compatibility shims.

**Non-Goals:**

- Preserving backward compatibility with tenant-rooted manifests or APIs.
- Auto-collapsing tenants that currently depend on multiple active legacy root rows under one tenant.
- Reworking tenant RLS boundaries; tenants remain the isolation root.
- Introducing many-to-many account↔tenant relationships.
- Changing GitHub's external model; the sync layer still adapts GitHub's hierarchy into Aeterna's supported tenant-root tree.

## Decisions

### Decision 1: `Account` is the canonical layer above tenant

A new `accounts` table becomes the customer/account-organization layer above `tenants`. A tenant has at most one `account_id`, and an account owns many tenants.

- **Why**: this matches the real operational model (`dev`, `test`, `prod` as separate tenants under one customer/account) and keeps tenant isolation semantics intact.
- **Alternative considered**: continuing with `legal_entities` as the public model. Rejected because the requested conceptual model is broader than billing/legal metadata; the system needs a canonical operator-facing account abstraction, not just a legal-entity side table.
- **Migration note**: existing `tenants.legal_entity_name` seed data is promoted into `accounts` rows instead of a separate `legal_entities` concept.

### Decision 2: Tenant-root hierarchy becomes `Organization -> Team -> Project`

The tenant-internal `Tenant` layer is removed from the canonical schema and APIs. `organizations` gain a direct `tenant_id` FK, `teams` continue to reference `organizations`, and `projects` continue to reference `teams`.

- **Why**: tenant is already the real root for provisioning, auth context, RLS, and sync targeting. Keeping a tenant node inside each tenant duplicates that root and forces unnecessary translation everywhere.
- **Alternative considered**: keep the legacy wrapper as an optional cosmetic layer. Rejected because it preserves the same mismatch and keeps #130 half-finished forever.

### Decision 3: Migration is strict and blocks multi-tenant tenants

Automatic migration SHALL only proceed for tenants that have zero or one active tenant in the old hierarchy.

- **When zero legacy wrapper rows exist**: only the account migration runs; tenant-root org/team/project remains empty.
- **When exactly one legacy wrapper row exists**: organizations formerly under that root are re-parented directly to the tenant.
- **When multiple active legacy wrapper rows exist**: migration aborts for that tenant with an explicit operator action required.

- **Why**: collapsing multiple legacy wrapper rows into one tenant root is lossy and semantically ambiguous. The user's stated preference is to simplify rather than preserve every backward-compatible corner case.
- **Alternative considered**: flatten all legacy wrapper rows by concatenating names or inserting synthetic org prefixes. Rejected as silent data corruption.

### Decision 4: Role scope storage moves off legacy OU/tenant assumptions

`user_roles.unit_id -> organizational_units.id` is not compatible with the target model. Role grants move to typed resource scopes aligned with the tenant-root hierarchy (`instance`, `tenant`, `organization`, `team`, `project`).

- **Why**: the target model removes tenant-internal tenant nodes and continues the broader #130 effort to eliminate `organizational_units` runtime dependence.
- **Alternative considered**: keep `user_roles` on OU while flattening only the read side. Rejected because it leaves the model split and undermines the migration.

### Decision 5: GitHub organization sync targets the tenant root, not a synthetic tenant

GitHub sync SHALL treat the target tenant as the root and map:

- GitHub Organization → Tenant sync target (no created hierarchy node)
- Top-level GitHub Teams → Organization
- Nested GitHub Teams → Team
- GitHub organization members → Users / memberships

For deeper GitHub nesting, all descendants remain teams under the nearest mapped organization root unless and until a separate sub-team model is introduced.

- **Why**: creating a synthetic tenant node just to satisfy the old hierarchy is exactly the inversion this change removes.
- **Alternative considered**: map GitHub org to Account. Rejected because sync is still configured and executed per tenant; one GitHub org may intentionally back only one tenant environment.

### Decision 6: Tenant manifests and control-plane payloads expose account + environment explicitly

Tenant create/show/provision payloads gain optional account reference and environment metadata. The hierarchy section starts at organizations.

- **Why**: if the platform wants operators to think in `Account -> Tenant(dev/test/prod)`, the contract has to say that directly.
- **Alternative considered**: infer environment only from tenant slug naming conventions. Rejected because it is brittle and impossible to validate reliably.

## Risks / Trade-offs

- **Migration failure for multi-tenant tenants** → Provide a preflight check and explicit operator remediation guide; fail before destructive schema steps.
- **Large cross-cutting refactor** → Sequence rollout as: account schema, org re-parenting, view rewrite, role-scope rewrite, API/manifest cutover, then tenant removal.
- **Spec overlap with `add-legal-entity-tenant-grouping`** → Mark this change as superseding that direction; do not implement both models in parallel.
- **Admin/UI confusion during rollout** → Land account-aware tenant listing and manifest changes in the same release as the storage migration; do not expose partial terminology.
- **GitHub nested-team mismatch remains imperfect** → Document the flattening rule explicitly; defer arbitrary team nesting to a separate proposal.

## Migration Plan

1. Add `accounts` and `tenants.account_id`, plus `tenants.environment`.
2. Backfill accounts from `tenants.legal_entity_name` where present.
3. Add `organizations.tenant_id`, backfill it through the legacy wrapper's tenant link, and update uniqueness/indexes.
4. Rewrite `v_hierarchy` and `v_user_permissions` to expose `tenant -> organization -> team -> project` without tenant columns.
5. Migrate role-scope persistence away from `organizational_units` and tenant-rooted lookups.
6. Cut over provisioning, reverse-render, admin APIs, GitHub sync, and OPAL fetch paths to the new model.
7. Drop the legacy wrapper table and remaining legacy OU/tenant compatibility paths.

Rollback strategy is migration-stage dependent:

- before view/API cutover: restore old queries and leave additive columns in place;
- after role-schema cutover: rollback requires restoring from backup rather than a partial down-migration, so rollout MUST be gated by preflight checks and staging validation.

## Open Questions

- Should `tenant.environment` be a strict enum (`dev`, `test`, `staging`, `prod`) or a validated free-form string with recommended values?
- Should account creation be implicit during tenant provisioning when the manifest supplies a new account slug, or should provisioning require the account to exist first?
