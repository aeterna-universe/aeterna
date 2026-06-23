# Aeterna Developer Guide

Comprehensive guide for developing features, debugging issues, and working with the Aeterna codebase.

---

## Project Structure

### Crate Responsibilities

| Crate | Purpose | Key Files |
|---|---|---|
| `mk_core` | Shared types, traits, domain primitives | `types.rs` (Role, MemoryLayer, TenantContext), `traits.rs` |
| `memory` | Memory system with 7 layers, provider registry, reasoning | `manager.rs`, `provider_registry.rs`, `reasoning.rs` |
| `knowledge` | Knowledge repository with Git backing, governance | `manager.rs`, `governance.rs`, `tenant_repo_resolver.rs` |
| `sync` | Memory-Knowledge sync bridge, WebSocket server | `bridge.rs`, `websocket.rs` |
| `tools` | MCP tool interface, tool registry, tool implementations | `tools.rs`, `server.rs`, `memory.rs`, `knowledge.rs`, `graph/` |
| `adapters` | Ecosystem adapters (OpenCode, LangChain, Cedar auth) | `opencode.rs`, `langchain.rs`, `auth/cedar.rs` |
| `storage` | Storage backends (Postgres, Qdrant, Redis, DuckDB) | `postgres.rs`, `graph_duckdb.rs`, `tenant_store.rs` |
| `config` | Configuration management, hot-reload | `lib.rs` |
| `errors` | Error handling framework | `lib.rs` |
| `utils` | Common utilities | `lib.rs` |
| `context` | Context compression (CCA) | `lib.rs` |
| `cli` | CLI binary + Axum HTTP server | `main.rs`, `server/` |
| `agent-a2a` | Agent-to-Agent protocol via Radkit | `lib.rs` |
| `opal-fetcher` | OPAL policy sync | `lib.rs` |
| `observability` | OpenTelemetry + Prometheus metrics | `lib.rs` |
| `testing` | Shared test fixtures and helpers | `lib.rs` |
| `idp-sync` | Identity provider sync (Okta webhooks) | `lib.rs`, `webhook_router` |
| `backup` | Backup/restore (archive, NDJSON, S3) | `archive.rs`, `manifest.rs`, `ndjson.rs`, `s3.rs` |

### Server Modules (`cli/src/server/`)

| Module | Purpose |
|---|---|
| `mod.rs` | `AppState` struct, auth context extraction helpers |
| `router.rs` | Route tree assembly, middleware stack |
| `bootstrap.rs` | Server initialization, service wiring |
| `plugin_auth.rs` | GitHub OAuth + JWT issuance/refresh |
| `auth_middleware.rs` | JWT validation middleware layer |
| `memory_api.rs` | Memory CRUD and search endpoints |
| `knowledge_api.rs` | Knowledge operations |
| `govern_api.rs` | Governance dashboard API |
| `backup_api.rs` | Export/import job management |
| `tenant_api.rs` | Tenant CRUD |
| `org_api.rs` | Organization management |
| `team_api.rs` | Team management |
| `project_api.rs` | Project management |
| `user_api.rs` | User management |
| `role_grants.rs` | Role administration (nested under `/admin`) |
| `sessions.rs` | Session management |
| `sync.rs` | Memory-Knowledge sync endpoints |
| `webhooks.rs` | Webhook endpoints |
| `mcp_transport.rs` | MCP protocol transport |
| `health.rs` | Health, liveness, readiness probes |
| `metrics.rs` | Prometheus metrics setup |
| `admin_sync.rs` | Admin synchronization |

---

## Common Development Tasks

### Adding a New Feature

Follow the OpenSpec workflow:

1. **Create OpenSpec change**: Scaffold `openspec/changes/<change-id>/` with `proposal.md`, `tasks.md`, and delta specs.
2. **Validate**: `openspec validate <change-id> --strict`
3. **Write failing tests first** (TDD): Create test functions that assert the expected behavior.
4. **Implement**: Follow `tasks.md` sequentially, making tests pass one by one.
5. **Verify coverage**: `cargo tarpaulin -p <crate-name>` to ensure >= 80%.
6. **Archive**: After deployment, `openspec archive <change-id> --yes`.

### Adding a New API Endpoint

1. **Choose the appropriate server module** in `cli/src/server/`. Create a new module if the endpoint group does not fit an existing one.

2. **Define the route handler**:
```rust
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use std::sync::Arc;
use super::{AppState, authenticated_tenant_context};

pub async fn my_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<MyRequest>,
) -> Result<Json<MyResponse>, (StatusCode, Json<serde_json::Value>)> {
    let ctx = authenticated_tenant_context(&state, &headers)
        .await
        .map_err(|r| /* convert to tuple */)?;
    // ... implementation
    Ok(Json(response))
}
```

3. **Register the route** in the module's `router()` function:
```rust
pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/v1/my-resource", post(my_handler))
        .with_state(state)
}
```

4. **Merge into the main router** in `cli/src/server/router.rs`:
```rust
// In build_router(), add to protected_api:
.merge(my_module::router(state.clone()))
```

5. **Auth patterns**: All protected routes use `authenticated_tenant_context()` which extracts `TenantContext` from JWT or headers. For role-based access control:
```rust
if !ctx.roles.iter().any(|r| matches!(r, RoleIdentifier::Known(Role::TenantAdmin | Role::PlatformAdmin))) {
    return Err((StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"}))));
}
```

6. **Error response format**: All API errors return JSON:
```json
{ "error": "error_code", "message": "Human-readable description" }
```

### Adding a New Storage Table

1. **Create a migration file** in `storage/migrations/` with the next sequential number:
```
018_my_new_table.sql
```

2. **Always include RLS policy**:
```sql
CREATE TABLE my_table (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    -- ... columns
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE my_table ENABLE ROW LEVEL SECURITY;
CREATE POLICY my_table_tenant_isolation ON my_table
    USING (tenant_id = current_setting('app.tenant_id', true));
```

3. **Add the sqlx query functions** in the appropriate storage module (e.g., `storage/src/postgres.rs` or a new module).

4. **Set `app.tenant_id`** before any query:
```rust
sqlx::query("SELECT set_config('app.tenant_id', $1, true)")
    .bind(&tenant_id.as_str())
    .execute(&*pool)
    .await?;
```

### Working with the Backup System

The backup system (`backup/` crate + `cli/src/server/backup_api.rs`) handles export and import of tenant data.

**Archive format**: tar.gz containing:
- `manifest.json` -- Metadata, entity counts, export scope
- `*.ndjson` -- One JSON object per line per entity type
- `checksums.sha256` -- SHA-256 hash of each file

**Key types**:
- `BackupManifest` (`backup/src/manifest.rs`) -- Archive metadata
- `ArchiveWriter` / `ArchiveReader` (`backup/src/archive.rs`) -- Tar/gzip I/O
- `ExportDestination` (`backup/src/destination.rs`) -- Local or S3 target
- `JobStatus` (`cli/src/server/backup_api.rs`) -- Pending/Running/Completed/Failed/Cancelled

**Adding a new entity type to backup**:
1. Add the entity to `EntityCounts` in `backup/src/manifest.rs`.
2. Add NDJSON serialization for the entity in the export task.
3. Add NDJSON deserialization in the import handler.
4. Update checksum verification to include the new file.

### Working with Per-Tenant Providers

The `TenantProviderRegistry` (`memory/src/provider_registry.rs`) manages per-tenant LLM and embedding services.

**Config field names** are defined in `memory/src/provider_registry.rs::config_keys`:
- `llm_provider`, `llm_model`, `llm_api_key`
- `llm_google_project_id`, `llm_google_location`, `llm_bedrock_region`
- `embedding_provider`, `embedding_model`, `embedding_api_key`
- `embedding_google_project_id`, `embedding_google_location`, `embedding_bedrock_region`

**Cache behavior**: The registry uses `DashMap` with TTL-based expiration. To force cache invalidation after a config change, the cache entry for the tenant is evicted on config update.

**Adding a new provider**:
1. Add a new variant to `LlmProviderType` / `EmbeddingProviderType` in `memory/src/llm/factory.rs` or `memory/src/embedding/factory.rs`.
2. Implement the config struct (e.g., `MyProviderLlmConfig`).
3. Add a match arm in `create_llm_service()` / `create_embedding_service()`.
4. Add the new provider type to the `config_keys` documentation.

### Working with the Admin UI

The admin UI is a React SPA in `admin-ui/`.

**Tech stack**: React 19, Vite, TypeScript, Tailwind CSS 4, TanStack Query, React Router 7, Lucide icons.

**Development**:
```bash
cd admin-ui
npm install
npm run dev     # Dev server at http://localhost:5173, proxies /api to :8080
```

**Adding a new page**:
1. Create a new directory under `admin-ui/src/pages/<section>/`.
2. Create the page component (e.g., `MyPage.tsx`).
3. Add the route in the router configuration.
4. Use `apiClient` from `@/api/client` for API calls.
5. Use TanStack Query hooks for data fetching:
```tsx
import { useQuery } from '@tanstack/react-query';
import { apiClient } from '@/api/client';

export function MyPage() {
  const { data, isLoading } = useQuery({
    queryKey: ['my-data'],
    queryFn: () => apiClient.get('/api/v1/my-resource').then(r => r.json()),
  });
  // ...
}
```

**Auth context**: Use the `AuthContext` from `@/auth/AuthContext.tsx` to access:
- `user` -- Current user profile
- `isAuthenticated`, `isPlatformAdmin`, `isTenantAdmin`
- `activeTenantId` -- Currently selected tenant
- `login()`, `logout()` -- Auth actions

**Directory structure**:
```
admin-ui/src/
  api/          # API client and type definitions
  auth/         # Auth context, login page, protected routes, token manager
  components/   # Shared UI components
  hooks/        # Custom React hooks
  layouts/      # Page layouts (AdminLayout)
  lib/          # Utility functions
  pages/        # Page components organized by section
    admin/      # System health, admin tools
    dashboard/  # Main dashboard
    governance/ # Governance workflows
    knowledge/  # Knowledge management
    memory/     # Memory browser
    organizations/
    policies/
    settings/
    tenants/
    users/
```

---

## ReplicaSet-Safe Patterns

Aeterna runs as a Kubernetes ReplicaSet with multiple replicas. Every feature must work correctly when N instances run behind a load balancer. Here are the patterns to follow.

**Adding shared state:**
```rust
// WRONG -- only visible to one replica
static STORE: LazyLock<RwLock<HashMap<String, Job>>> = LazyLock::new(..);

// RIGHT -- visible to all replicas via Redis
let store = RedisStore::new(redis_conn, "aeterna:jobs");
store.set("job-123", &job, Some(86400)).await?;
```

**Adding a background task:**
```rust
// WRONG -- every replica runs simultaneously
tokio::spawn(async { run_cleanup().await; });

// RIGHT -- only one replica runs via distributed lock
let lock_key = "aeterna:lifecycle:cleanup";
if redis_try_lock(&conn, lock_key, 3600).await? {
    run_cleanup().await;
    // Lock auto-expires after TTL
}
```

**Adding a cache:**
```rust
// OK -- per-instance cache with TTL, tolerate staleness
let cache: DashMap<String, CachedEntry<T>> = DashMap::new();
// Check TTL on read, re-resolve from DB if expired
```

**Key question before every feature**: "Does this work with 3 replicas behind a load balancer?"

---

## Debugging

### Structured Logging

Configure logging with the `RUST_LOG` environment variable:

```bash
# Default (info level)
RUST_LOG=info cargo run -p aeterna-cli -- serve

# Debug level for specific crates
RUST_LOG=aeterna=debug,memory=debug,sqlx=warn cargo run -p aeterna-cli -- serve

# Trace level for maximum detail
RUST_LOG=trace cargo run -p aeterna-cli -- serve
```

All log entries include structured fields (tenant_id, request_id, etc.) via `tracing` spans.

### Prometheus Metrics

When the server is running, metrics are available at the `/metrics` endpoint:

```bash
curl http://localhost:8080/metrics
```

Key metrics:
- `http_requests_total{method, path, status}` -- Request counter
- `http_request_duration_ms{method, path, status}` -- Latency histogram

### Health Endpoints

| Endpoint | Purpose | Expected Response |
|---|---|---|
| `GET /health` | Basic liveness | 200 OK |
| `GET /live` | Kubernetes liveness probe | 200 OK |
| `GET /ready` | Readiness (includes backend checks) | 200 OK or 503 |

### Database Debugging

```bash
# Connect to PostgreSQL
docker compose exec postgres psql -U aeterna -d aeterna

# Check RLS is active
SET app.tenant_id = 'my-tenant';
SELECT * FROM memories LIMIT 5;

# Check Qdrant collections
curl http://localhost:6333/collections
```

### Common Issues

**"Plugin auth JWT secret is not configured"**: Set `AETERNA_PLUGIN_AUTH_JWT_SECRET` environment variable or disable plugin auth for development.

**Empty search results**: Verify the embedding service is configured. Check `RUST_LOG=memory::provider_registry=debug` for provider resolution logs.

**RLS blocking queries**: Ensure `SET app.tenant_id` is called before every query. Check migration `004_enable_rls.sql` for policy definitions.

**Admin UI not loading at /admin**: Build the admin UI first (`cd admin-ui && npm run build`) or set `AETERNA_ADMIN_UI_PATH` to the dist directory.

---

## Cross-Tenant Operations (#44.d)

PlatformAdmins can list resources across every tenant on the instance by passing `?tenant=*` to any of the migrated list endpoints:

| Endpoint                | Effect of `?tenant=*`                                      |
|-------------------------|------------------------------------------------------------|
| `GET /admin/tenants`    | Lists all tenants (was always cross-tenant).               |
| `GET /user`             | Users from every tenant, decorated with `tenantId`/`tenantSlug`. |
| `GET /project`          | Project organizational units, same decoration.             |
| `GET /org`              | Organization units, same decoration.                       |
| `GET /govern/audit`     | Audit log across tenants (no per-row tenant decoration — see canonical doc). |

**Who can call it?** PlatformAdmin only. Non-admins get `403 forbidden_scope`. Callers without the `?tenant=` param see the legacy tenant-scoped behavior unchanged.

**Response envelope:** `{ "success": true, "scope": "all", "items": [...] }`. Every item (except on `/govern/audit`) has a non-empty `tenantId` and `tenantSlug` — this is locked by the §4.1 contract test in `cli/tests/server_runtime_test.rs`.

**Safety net:** `storage/tests/rls_boundary_test.rs` fails if any cross-tenant-readable table is ever accidentally RLS-enabled (which would silently return empty results instead of data). If you touch migrations for `tenants` / `users` / `organizational_units` / `governance_audit_log` / `referential_audit_log`, run this test locally.

**Planned work:**
- `?tenant=<slug>` (single-foreign-tenant listing): currently `501 scope_not_implemented`, scheduled in the cross-tenant listing change.
- CLI flags: `--all-tenants` and `--tenant <slug>` on `aeterna admin {users,projects,orgs,audit} list` — see [tasks §6](../openspec/changes/add-cross-tenant-admin-listing/tasks.md#6-cli-updates-separate-pr-tracked-here-for-cross-reference).
- Per-row tenant decoration for `/govern/audit` once `acting_as_tenant_id` is surfaced in `AuditRow`.

**Canonical reference:** [`docs/api/admin.md`](api/admin.md) documents the full grammar, error codes, envelope guarantees, and ordering invariants. Any OpenAPI schema we generate in the future should use that document as its source of truth.

**Common debugging tip:** if `?tenant=*` returns an empty `items[]` array but you expect data, check:
1. The caller actually has the `platform_admin` role (not just an `admin` role scoped to a single tenant).
2. No migration has RLS-enabled one of the tables (the RLS boundary test catches this).
3. The target tenants are `status = 'active'` in `tenants` (soft-deleted tenants are excluded from `?tenant=*`).

---

## Tenant Secrets & KMS

Tenant-scoped secret material (OpenAI keys, GitHub tokens, embedding-provider credentials) lives in the envelope-encrypted `tenant_secrets` Postgres table. Three layers back it:

| Layer | Crate | Summary |
|---|---|---|
| `SecretBytes` + `SecretReference` | `mk_core::secret` | Zeroize-on-drop plaintext carrier + tagged enum reference |
| `KmsProvider` (trait) + `AwsKmsProvider` / `LocalKmsProvider` | `storage::kms` | Wraps/unwraps per-row 32-byte DEKs |
| `SecretBackend` (trait) + `PostgresSecretBackend` | `storage::secret_backend` | put/get/delete/list; AES-256-GCM rows + KMS-wrapped DEK |

### In application code

```rust
use storage::secret_backend::build_secret_backend_from_env;
use mk_core::SecretBytes;

let backend = build_secret_backend_from_env(pool.clone()).await?;

// write
let reference = backend
    .put(tenant_db_id, "openai-api-key", SecretBytes::from_string(api_key))
    .await?;

// read (plaintext is only alive inside `SecretBytes` — drop it promptly)
let plaintext = backend.get(&reference).await?;
client.set_api_key(plaintext.expose());
drop(plaintext);
```

### In tests

Prefer `InMemorySecretBackend` for unit tests; it avoids KMS and Postgres entirely. Integration tests that exercise the real path gate on `testcontainers` + Docker availability (see `storage/tests/secret_backend_integration.rs`).

### Runtime configuration

| Env | Meaning |
|---|---|
| `AETERNA_KMS_PROVIDER=aws\|local` | Selects KMS backend (default: `local`) |
| `AETERNA_KMS_AWS_KEY_ARN` | CMK ARN when `provider=aws` |
| `AETERNA_LOCAL_KMS_KEY` | Base64-encoded 32 bytes when `provider=local` |

In Kubernetes these are all produced from the Helm `kms:` values block. See `website/docs/helm/kms.md`.

### Don'ts

- **Do not** `format!("{plaintext:?}")` or JSON-serialize a `SecretBytes` expecting the bytes to appear — they will always render as `<redacted>`. Use `.expose()` explicitly when bytes are actually needed.
- **Do not** cache `SecretBytes` in a long-lived struct — the zeroize guarantee is undermined if the value outlives its use.
- **Do not** invent new secret-bearing structs. If you need to persist something sensitive, store it through `SecretBackend` and hold a `SecretReference`.

### Pointers

- Concept page: `docs/architecture/secret-backend.md`
- Operator Helm guide: `website/docs/helm/kms.md`
- Rotation runbook: `docs/guides/secret-rotation.md`
- Design decisions: `openspec/changes/harden-tenant-provisioning/design.md` (D1–D8)

---

## Key Specs

Before working on a specific area, read the relevant OpenSpec specification:

| Area | Spec Location |
|---|---|
| Memory system | `openspec/specs/memory-system/` |
| Knowledge repository | `openspec/specs/knowledge-repository/` |
| OpenCode integration | `openspec/specs/opencode-integration/` |
| Multi-tenant governance | `openspec/specs/multi-tenant-governance/` |
| Storage layer | `openspec/specs/storage/` |
| Codesearch integration | `openspec/specs/codesearch-integration/` |
| Plugin auth | `openspec/specs/opencode-plugin-auth/` |
| Tenant config provider | `openspec/specs/tenant-config-provider/` |
| Server runtime | `openspec/specs/server-runtime/` |

Use `openspec show <spec-name>` to view any spec, or browse the `openspec/specs/` directory directly.

## RLS Enforcement & the Dual-Pool Model (issue #58)

*Added in Bundle A.2 of the RLS enforcement change. See `openspec/changes/decide-rls-enforcement-model/design.md` for the architectural rationale.*

### Two roles, two pools, two helpers

| Layer | Tenant path | Admin path |
|-------|-------------|------------|
| PG role | `aeterna_app` (NOBYPASSRLS) | `aeterna_admin` (BYPASSRLS) |
| Pool | `backend.pool()` | `backend.admin_pool()` |
| Helper | `with_tenant_context(&ctx, \|tx\| …)` | `with_admin_context(&ctx, action, \|tx\| …)` |
| Sets `app.tenant_id` | Yes (`SET LOCAL`) | No |
| Auto-audits | No | Yes (`admin_scope = TRUE`) |
| Use for | 99% of tenant-scoped traffic | PlatformAdmin cross-tenant / scheduled cross-tenant / migrations |

### Writing a tenant-scoped handler

```rust
let rows = backend.with_tenant_context(&ctx, |tx| {
    Box::pin(async move {
        sqlx::query_as::<_, Row>("SELECT … FROM users WHERE tenant_id = $1")
            .bind(&tenant_id)
            .fetch_all(&mut **tx)
            .await
            .map_err(Into::into)
    })
}).await?;
```

The `WHERE tenant_id = $1` clause stays — RLS is the floor, the explicit filter is the required defense-in-depth ceiling. `storage/tests/rls_enforcement_test.rs` surfaces any divergence.

### Writing an admin-scope handler

```rust
let rows = backend.with_admin_context(&ctx, "admin.user.list", |tx| {
    Box::pin(async move {
        sqlx::query_as::<_, Row>("SELECT … FROM users")  // no WHERE tenant_id
            .fetch_all(&mut **tx)
            .await
            .map_err(Into::into)
    })
}).await?;
```

`action` (the second argument) is the audit event name (e.g. `"admin.user.list"`). It lands in `governance_audit_log.action` alongside `admin_scope = TRUE` and `acting_as_tenant_id` from `ctx.target_tenant_id`.

### Scheduled-jobs pattern

Scheduled cross-tenant work:

```rust
backend.with_admin_context(
    &TenantContext::system_ctx(),
    "sched.global_audit_compaction",
    |tx| Box::pin(async move { /* cross-tenant sweep */ Ok(()) }),
).await?;
```

Scheduled per-tenant work — enumerate via admin, dispatch via tenant:

```rust
let tenant_ids = backend.with_admin_context(
    &TenantContext::system_ctx(),
    "sched.enumerate_tenants",
    |tx| Box::pin(async move {
        sqlx::query_scalar::<_, String>("SELECT id FROM tenants WHERE active")
            .fetch_all(&mut **tx).await.map_err(Into::into)
    }),
).await?;

for tid in tenant_ids {
    let ctx = TenantContext::from_scheduled_job(TenantId::new(tid)?, "nightly_reindex");
    backend.with_tenant_context(&ctx, |tx| {
        Box::pin(async move { /* per-tenant work */ Ok(()) })
    }).await?;
}
```

`TenantContext::system_ctx()` is the sentinel for no-human-actor scheduler work (`user_id = "system"`, `tenant_id = "__root__"`). `TenantContext::from_scheduled_job(tenant, job_id)` encodes the job name into `user_id` as `system:<job_id>` so the audit trail distinguishes different scheduled jobs hitting the same tenant.

### Local dev setup

Migration `025_add_app_roles.sql` creates both roles with `PASSWORD NULL`. It is **superuser-aware**: when run by a superuser (local dev with the `postgres` superuser), it creates / repairs the roles itself; when run by a non-superuser (the CNPG-managed `aeterna` owner in production — which has `CREATEROLE` but not `SUPERUSER`), it skips the `BYPASSRLS`/`NOBYPASSRLS` statements with a WARNING and expects the roles to already exist (provisioned by the prereqs layer's CNPG `managed.roles`). See issue #194.

**Local dev (superuser path)** — after running migrations locally, set passwords:

```bash
psql $DATABASE_URL -c "ALTER ROLE aeterna_app WITH PASSWORD 'devapp'"
psql $DATABASE_URL -c "ALTER ROLE aeterna_admin WITH PASSWORD 'devadmin'"
export DATABASE_URL_ADMIN="postgres://aeterna_admin:***@localhost:5432/aeterna"
```

**Production / greenfield (non-superuser path)** — the `aeterna-prereqs` Helm chart provisions `aeterna_app` (NOBYPASSRLS) and `aeterna_admin` (BYPASSRLS) via CloudNativePG `managed.roles`, which the operator applies as a superuser. The migration then only handles GRANTs / the `admin_scope` column / index. No `ALTER ROLE … SUPERUSER` workaround is needed. The regression is guarded by `storage/tests/migration_025_non_superuser_test.rs` and `.github/workflows/non-superuser-migration.yml` (kind + CNPG integration).

Until Bundle A.3 Wave 6 flips `DATABASE_URL` to `aeterna_app`, you can leave `DATABASE_URL_ADMIN` unset — both pools will then share the current role and the helpers remain behaviorally correct (they still `BEGIN`/`COMMIT` and `SET LOCAL`, just without the RLS gate).

### Forbidden patterns

- `backend.pool()` or `backend.admin_pool()` outside the two helpers — blocked by `storage/tests/admin_pool_access_lint.rs` (and tenant-pool equivalent in Wave 6).
- `SET` / `SET SESSION` of `app.tenant_id` — session-scope is pool-leakable. Always `SET LOCAL` inside a transaction. Enforced by the grep test added in Bundle A.1.
