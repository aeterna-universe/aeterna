# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [0.8.0-rc.7] — 2026-04-29

### BREAKING CHANGES

- **Governance wire format: snake_case → camelCase (struct fields) +
  PascalCase (bare enum variant tags).** All governance types
  (`GovernanceEvent`, `RemediationRequest`, `ApprovalRecord`, audit entries)
  now serialize their **struct fields** as camelCase
  (e.g. `resource_type` → `resourceType`, `created_at` → `createdAt`).
  **Externally tagged enum variants** (e.g. `GovernanceEvent::UnitCreated`)
  use serde's default representation, which means the JSON tag is the
  Rust variant name verbatim — **`"UnitCreated"`, not `"unitCreated"`**
  — and similarly for `KnowledgeLayer` (`"Tenant"`, `"Org"`, `"Team"`,
  `"Project"`), `KnowledgeType` (`"Adr"`, `"Pattern"`, `"Hindsight"`, …),
  `Role` (`"Admin"`, `"Architect"`, …), and other enums under
  `mk_core::types`. One deliberate exception: `ResourceType` keeps
  `#[serde(rename_all = "lowercase")]` (`"organization"`, `"session"`,
  `"tenant"`) for backwards-compatibility with stored RBAC grants.
  Affected endpoints:
    - `GET /api/v1/govern/audit`
    - `GET /api/v1/govern/pending`
    - `GET /api/v1/admin/lifecycle/remediations`
    - All governance webhook payloads

- **Paginated list endpoints return `{ items, total, limit, offset }` envelopes**
  instead of bare arrays. Affected endpoints:
    - `GET /api/v1/admin/tenants`
    - `GET /api/v1/user`
    - `GET /api/v1/govern/policies`
    - `GET /api/v1/govern/audit`
    - `GET /api/v1/govern/pending`
    - `GET /api/v1/govern/roles`
    - `GET /api/v1/knowledge/promotions`
    - `GET /api/v1/admin/exports`
    - `GET /api/v1/admin/imports`
    - `GET /api/v1/admin/git-provider-connections`
    - `POST /api/v1/knowledge/query` (response shape unchanged but `offset`
      added to request body)
    - `POST /api/v1/memory/search` (`offset` added to request body)
    - `POST /api/v1/memory/list` (`offset` added to request body)

### Added

- Shared `<PaginationBar>` React component for admin-ui list pages.
- `offset` field on `POST /memory/search`, `POST /memory/list`, and
  `POST /knowledge/query` request bodies.
- `limit` and `offset` query parameters on `GET /admin/exports`,
  `GET /admin/imports`, `GET /govern/pending`,
  `GET /knowledge/promotions`, and `GET /admin/git-provider-connections`.
- Max cap of 500 on `GET /sync/pull` limit parameter.
- `LIMIT` clauses on `list_project_team_assignments`,
  `list_suppressions`, and other unbounded storage queries.

### Fixed

- `GET /govern/pending` no longer hard-codes `limit: 100`; respects
  caller-supplied `limit`/`offset` query params.
- `POST /memory/list` uses `list_from_layer` with proper offset/limit
  instead of fetching all then truncating.
- Admin-UI `TenantDetailPage` unwraps `{ tenant: ... }` envelope from
  `GET /admin/tenants/:id`.
- Admin-UI `KnowledgeSearchPage` uses `KnowledgeItem` type and correct
  camelCase field references.
- Admin-UI `MemorySearchPage` sends correct feedback request body
  (`rewardType`, `score`, `layer`) to `POST /memory/:id/feedback`.
- Admin-UI `LifecyclePage` handles both bare array and `{ items }`
  envelope for remediations.
- `governance_events` insert: cast `PersistentEvent.id` (text) → `uuid`
  before binding to `INSERT INTO governance_events(id …)`. Eliminates
  the `BootstrapCompleted` warning observed on rc.6 and lets the event
  table actually receive bootstrap rows. (`storage/src/postgres.rs`,
  carried in #177).
- Hermetic CLI e2e helpers — `helpers::aeterna_cli()` now scrubs
  `HOME`, `XDG_CONFIG_HOME`, and `AETERNA_TOKEN` from the child env so
  developer creds in `~/.aeterna/` cannot leak into a passing test
  pretending the unauth path works. (`cli/tests/helpers/`, carried in
  #177).
- Redis `summary_cache_key` lowercases the `SummaryDepth` segment so
  cache keys remain stable across the PascalCase enum flip — without
  this, every cached summary would silently miss on upgrade.
  (`storage/src/redis.rs`).

### Test infrastructure (#178)

Six test files carried camelCase assertions/fixtures predating the
wire-format refactor and were failing on `cargo test --workspace --lib`
even though the changed-feature integration tests passed. Fixed:

- `mk_core/src/types.rs` — 12 inline `test_*_serialization` tests +
  `role_identifier_tests` updated to expect PascalCase enum tags.
- `tools/src/redis_publisher.rs` — `GovernanceEvent` serialization
  asserts now expect `"UnitCreated"` / `"DriftDetected"`. Misleading
  comments removed.
- `tools/src/governance.rs` — `unit_policy.add` JSON fixture: layer/
  mode/merge_strategy values bumped to PascalCase.
- `memory/src/reasoning.rs` — `ReasoningStrategy` mock fixture
  `"exhaustive"` → `"Exhaustive"`.
- `cli/src/server/role_grants.rs` — restored intentional
  `ResourceType` lowercase serialization with a clarifying comment.

Workspace test count: **3,144 passing across 19 suites** (was 17
failures before the alignment).

### Migration Notes

Clients consuming the admin API must update:

1. Parse camelCase fields in governance struct payloads.
2. Parse PascalCase enum tags (`"UnitCreated"`, `"Tenant"`, `"Adr"`,
   `"Admin"`, …) — this is serde-default for all `mk_core::types` enums
   that don't carry an explicit `rename_all`.
3. Read list results from `response.items` instead of the top-level array.
4. Use `response.total` for pagination UI; pass `limit`/`offset` query
   params (or body fields for POST endpoints) to page through results.

### Known follow-ups

- Soak rc.7 in a staging environment to validate admin-ui consumes
  the PascalCase enum tags correctly. If admin-ui needs adjustments,
  cut rc.8 with the UI fix.
