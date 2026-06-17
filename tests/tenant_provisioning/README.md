# Tenant Provisioning Consistency Suite

This directory implements the **consistency acceptance suite** from B2
§13 of the `harden-tenant-provisioning` change.

## Goal

For every fixture in [`scenarios/`](./scenarios), the manifest must
produce **the same rendered tenant state** regardless of how it was
submitted:

1. **CLI path** — `aeterna tenant apply --file <fixture>.yaml`
2. **Server path** — `POST /admin/tenants/provision` (direct API call
   with a test-minted PlatformAdmin or scoped service token)
3. **UI path** — Admin UI Create-Tenant wizard driven by Playwright,
   submitting with `X-Aeterna-Client-Kind: ui`

All three runners then call `aeterna tenant render --slug <slug>` and
diff the output against an expected baseline, allow-listing
`createdAt`, `updatedAt`, and any UUID fields. Any drift across
runners is a suite failure.

## Layout

```
tests/tenant_provisioning/
├── README.md              # this file
├── scenarios/             # the canonical fixture set (§13.1)
│   ├── 01-bootstrap.yaml
│   ├── 02-add-root-org.yaml
│   ├── 03-rotate-reference.yaml
│   ├── 04-noop-reapply.yaml
│   └── 05-prune.yaml
└── run_validate.sh        # fast pre-check (§13.7) — no Docker needed
```

## Current Status

| Task   | Component                                             | Status |
|--------|-------------------------------------------------------|--------|
| §13.1  | Scenario fixtures (5, covering bootstrap → prune)     | ✅     |
| §13.2  | API runner (in-process router via `cargo test`)       | ✅     |
| §13.3  | `runner_cli.rs` (spawns `aeterna tenant apply`)       | ✅     |
| §13.4  | `runner_ui.rs` (Playwright against `/admin/*`)        | ✅     |
| §13.5  | Render-diff assertions with allowlist                 | ✅     |
| §13.6  | CI job `consistency-matrix` (api+cli+ui)              | ✅     |
| §13.7  | CI job running structural pre-check (jq)              | ✅     |

The API runner lives at
[`cli/tests/tenant_provisioning_consistency_test.rs`](../../cli/tests/tenant_provisioning_consistency_test.rs).
It runs every fixture sequentially through the in-process axum router
against a testcontainer Postgres and structurally diffs each rendered
tenant against the input manifest.

The CLI runner lives at
[`cli/tests/tenant_provisioning_consistency_cli_test.rs`](../../cli/tests/tenant_provisioning_consistency_cli_test.rs).
It binds the same axum router on a random `127.0.0.1:0` port, then
spawns the real `aeterna` binary (`tenant apply` + `tenant render`)
against that URL, with a seeded PlatformAdmin user and an HS256-minted
JWT exported as `AETERNA_API_TOKEN`. Same fixture set, same
`redact_volatile` allowlist, same `assert_round_trip` invariant — but
exercising the binary's wire shape (including the
`X-Aeterna-Client-Kind: cli` header that the API runner cannot reach).

The UI runner lives at
[`admin-ui/e2e/tenant-consistency-runner.spec.ts`](../../admin-ui/e2e/tenant-consistency-runner.spec.ts).
It is render-side only: it mocks `GET /admin/tenants/{slug}/manifest`
to return each fixture verbatim, then drives the Manifest tab and
asserts that the UI's YAML serializer (`manifestToYaml`) round-trips
every structural marker — slug, name, generation, scenario label,
every `config.fields[*]` key, every `config.secretReferences[*]`
logical name, and every top-level `hierarchy[*].name`. Download
filename is pinned to `<slug>.manifest.redacted.yaml` per §14.3.
The runner is hermetic (Vite + `page.route()` only — no backend, no
Docker), so it complements the JSON-shaped §13.2/§13.3 runners by
covering the YAML serializer they cannot reach.

The CLI runner (§13.3) reuses the same fixture set and the same
`redact_volatile` allowlist as §13.2; its absence today is a
harness-shape gap, not a correctness gap.

## Scenario Design Notes

The five fixtures form a **sequenced narrative** against the same
tenant slug (`acme-bootstrap`), so the full suite also exercises the
second-apply, no-op, and prune paths:

1. `01-bootstrap`   — create
2. `02-add-root-org` — extend (hierarchy add)
3. `03-rotate-reference` — modify (secret reference flip)
4. `04-noop-reapply` — byte-identical re-submit of (3), expects `no_op`
5. `05-prune`       — remove a hierarchy branch added in (2)

Each fixture's header documents its own acceptance criteria.

## Running the Fast Pre-Check Locally

```bash
./tests/tenant_provisioning/run_validate.sh
```

This runs a **client-local, server-less structural check** against
every fixture:

1. Valid JSON.
2. `apiVersion: aeterna.io/v1` + `kind: TenantManifest`.
3. Non-empty `tenant.slug` and `tenant.name`.
4. `metadata.labels.suite == "consistency"` so stray manifests can't
   accidentally opt into the suite.

Full server-backed validation (`POST
/admin/tenants/provision?dryRun=true`) is the job of the per-runner
tests §13.2–§13.4. Keeping this lane client-local means pre-commit
hooks and the fast CI job never need Docker, Postgres, or a built
`aeterna` binary — only `jq`.

## Why JSON and not YAML

Every fixture is JSON because `aeterna tenant validate` / `tenant apply`
POST their input as JSON to `/admin/tenants/provision`. The operator
guide shows YAML for readability, but the wire format is JSON.
Keeping the fixtures in JSON eliminates a YAML-to-JSON round-trip
from the suite, and makes diffs against `aeterna tenant render`
output (also JSON) byte-comparable after allow-listed fields are
stripped.

Each fixture's intent and acceptance criteria are documented in the
"Scenario Design Notes" section above — the JSON file itself carries
only the `metadata.labels.suite` marker.
