# Aeterna — Claude Code Memory

> **🔴 PUBLIC REPO — READ FIRST:** This is a public OSS codebase.
> Everything related to deploying on real internal infrastructure lives
> in a separate internal repository. **Never** introduce identifiers
> naming real environments, hostnames, cloud resources, tenants,
> credentials, or any similar operational artefacts into commits,
> PR/issue bodies, or source-file comments here. The full rulebook,
> enforcement, and remediation recipe are in
> [`AGENTS.md § Public vs Internal Repository Split`](./AGENTS.md#-hard-constraint--public-vs-internal-repository-split)
> and the mechanical guard is documented in [`docs/leak-guard.md`](./docs/leak-guard.md).

## Project Purpose

Universal Memory & Knowledge Framework for Enterprise AI Agent Systems. Provides hierarchical memory storage, governed organizational knowledge, and a pluggable adapter architecture for AI agents at scale (LangChain, AutoGen, CrewAI, OpenCode).

---

## Tech Stack

| Layer               | Technology                                                                                                              |
| ------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **Language**        | Rust (Edition 2024 — NEVER 2021)                                                                                        |
| **Async runtime**   | Tokio (full features)                                                                                                   |
| **Memory storage**  | Redis 7+ (Working/Session), PostgreSQL 16+ (Episodic/Procedural/User/Org metadata), Qdrant 1.12+ (all semantic vectors) |
| **Graph layer**     | DuckDB 0.9+                                                                                                             |
| **Embeddings**      | rust-genai 0.4+ (OpenAI, Anthropic, Gemini, Z.AI, xAI, Ollama, Groq, Cohere…)                                           |
| **Authorization**   | Cedar policies + Permit.io + OPAL                                                                                       |
| **MCP interface**   | 11 unified tools (memory, knowledge, graph)                                                                             |
| **A2A protocol**    | Radkit SDK 0.0.4                                                                                                        |
| **Error handling**  | `anyhow` (apps), `thiserror` (libs)                                                                                     |
| **Serialisation**   | serde / serde_json / serde_yaml / toml                                                                                  |
| **HTTP**            | reqwest 0.13                                                                                                            |
| **DB (relational)** | sqlx 0.9 (PostgreSQL + Tokio)                                                                                           |
| **Testing**         | cargo-tarpaulin, proptest, cargo-mutants, testcontainers, wiremock                                                      |

---

## Workspace Layout

```
aeterna/
├── mk_core/        # Shared types, traits, domain primitives
├── memory/         # Memory system (7 layers, R1 optimization, DuckDB graph)
├── knowledge/      # Knowledge repository (Git-backed, constraint DSL)
├── sync/           # Memory ↔ Knowledge sync bridge
├── tools/          # MCP tool interface (11 tools)
├── adapters/       # Ecosystem adapters (OpenCode, LangChain, Radkit)
├── storage/        # Storage layer (Postgres, Qdrant, Redis, DuckDB)
├── config/         # Configuration management, hot-reload
├── errors/         # Error handling framework
├── utils/          # Common utilities
├── context/        # Context compression (CCA)
├── cli/            # CLI binary + Axum HTTP server
├── agent-a2a/      # Agent-to-Agent protocol (Radkit)
├── opal-fetcher/   # OPAL policy sync
├── observability/  # OpenTelemetry + Prometheus metrics
├── testing/        # Shared test fixtures and helpers
├── idp-sync/       # Identity provider sync (Okta)
├── cross-tests/    # Integration tests (require Docker)
├── backup/         # Backup/restore system (archive, NDJSON, S3)
├── admin-ui/       # Admin web UI (React 19, Vite, TypeScript, Tailwind)
├── openspec/       # Change proposals and versioned specs
└── specs/          # Legacy spec documents
```

---

## Key Architectural Concepts

### Memory Layers (8 levels, ordered fastest → slowest)

```
Working       µs  — Redis in-memory
Session       ms  — Redis with TTL
Episodic       h  — PostgreSQL + Qdrant
Semantic       d  — Qdrant vector search
Procedural     w  — PostgreSQL facts
User          mo  — PostgreSQL + Qdrant
Organization  mo  — PostgreSQL + Qdrant
Archival      yr  — Qdrant long-term
```

### Knowledge Hierarchy (4 levels, high → low precedence)

```
Tenant → Organization → Team → Project
```

Policies flow **down**; projects can override with `Merge` or `Override` strategy.

### Multi-Tenant Context

Every operation scoped to: `tenant → org → team → project → user → agent`

### Authorization (Cedar + OPAL)

- **Cedar files** hold authorization rules (not OPAL)
- **OPAL + Cedar Agent** synchronize and evaluate at runtime
- **Postgres-backed Aeterna** stores memberships, assignments, hierarchy
- **Okta** is the identity authority; Google/GitHub only via Okta federation

---

## Essential Commands

```bash
# Build
cargo build --release

# Run all tests
cargo test --all

# Run tests for a specific crate
cargo test -p aeterna-memory

# Coverage report (requires cargo-tarpaulin)
cargo tarpaulin --out Html --all

# Integration tests (requires Docker services running)
docker-compose up -d
cargo test --all -- --include-ignored

# Format
cargo fmt

# Lint
cargo clippy --all -- -D warnings

# Mutation testing
cargo mutants --jobs 4
```

---

## Testing Requirements (NON-NEGOTIABLE)

- **Minimum 80% test coverage** enforced in CI via tarpaulin (`fail-under = 80`)
- **TDD/BDD**: Write failing test first, then implementation (RED → GREEN → REFACTOR)
- **Property-based tests** (proptest) for all critical algorithms (promotion score, similarity, confidence)
- **Mutation testing** — 90%+ mutants killed for critical paths
- **All external deps behind trait abstractions** — mock everything with wiremock / testcontainers
- **Deterministic fixtures** for all external API responses (versioned)
- **BDD scenarios** (Gherkin-style) for all critical workflows

> **Violation**: Do NOT commit code without tests. Do NOT skip coverage checks.

---

## Code Conventions

### Rust Style

- Edition **2024** (workspace enforces this — never `edition = "2021"`)
- `cargo fmt` before every commit
- Public APIs must have `///` doc comments with examples
- Avoid `unsafe` unless absolutely necessary with a safety comment
- Use `Result<T, E>` — no panics in library code
- Never suppress type errors with `as any` patterns or `#[allow(unused)]` without justification
- `anyhow` for binary/app error chains; `thiserror` for library error types

### Clippy Config

Enabled: `all`, `pedantic` (as warnings).  
Allowed exceptions are listed in `Cargo.toml` — don't suppress new ones without discussion.

### Error Handling

- Return `Result` from all fallible functions
- Never swallow errors with empty catch blocks
- Propagate with `?`, wrap with `.context("…")` from anyhow

---

## OpenSpec Workflow (MANDATORY for features)

All non-trivial changes go through OpenSpec:

```
openspec list               # See active changes
openspec list --specs       # See existing capabilities
openspec show <change>      # View change details
openspec validate <change> --strict   # Validate before implementing
openspec archive <change-id> --yes    # Archive after deployment
```

**Stage 1 — Create proposal** (`openspec/changes/<change-id>/`):

- `proposal.md` — Why + What + Impact
- `tasks.md` — Implementation checklist
- `design.md` — (optional) Technical decisions
- `specs/<capability>/spec.md` — Delta specs (`## ADDED|MODIFIED|REMOVED Requirements`)

**Stage 2 — Implement** (follow `tasks.md` sequentially, mark `[x]` when done)

**Stage 3 — Archive** (after deployment, move to `changes/archive/YYYY-MM-DD-<id>/`)

> Skip proposal only for: bug fixes, typos, formatting, comment-only changes, non-breaking dep bumps.

### Active Changes (as of 2026-04)

| Change                          | Status                   | Remaining    |
| ------------------------------- | ------------------------ | ------------ |
| `add-cloud-llm-providers`       | in-progress              | 1 task       |
| `fix-production-readiness-gaps` | in-progress              | 4 tasks      |
| `add-okta-federated-auth`       | complete (needs archive) | --           |
| `add-admin-web-ui`              | in-progress              | see tasks.md |
| `add-backup-restore`            | in-progress              | see tasks.md |
| `add-day2-operations`           | in-progress              | see tasks.md |
| `add-tenant-provider-config`    | in-progress              | see tasks.md |
| `refactor-binary-split`         | in-progress              | see tasks.md |

---

## Performance Targets

| Tier            | Latency | Throughput |
| --------------- | ------- | ---------- |
| Working memory  | < 10ms  | —          |
| Session memory  | < 50ms  | —          |
| Semantic search | < 200ms | —          |
| API             | —       | > 100 QPS  |
| Creates         | —       | > 50 CPS   |
| CPU             | —       | < 70%      |
| Memory          | —       | < 80%      |
| DB connections  | —       | < 80% pool |

---

## MCP Tools (11 total)

**Memory**: `memory_add`, `memory_search`, `memory_delete`, `memory_feedback`, `memory_optimize`  
**Knowledge**: `knowledge_query`, `knowledge_check`, `knowledge_show`  
**Graph**: `graph_query`, `graph_neighbors`, `graph_path`

---

## Git Workflow

- Feature branches from `main`
- PR reviews required before merge
- **Conventional commits**: `feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`
- CI enforces: coverage ≥ 80%, clippy clean, fmt clean

---

## OpenCode Plugin

Published as `@aeterna-org/opencode-plugin` — MCP tools + hooks + automatic context injection.

---

## Key Specs to Read Before Working

| Spec                    | Location                                  |
| ----------------------- | ----------------------------------------- |
| Memory system           | `openspec/specs/memory-system/`           |
| Knowledge repository    | `openspec/specs/knowledge-repository/`    |
| OpenCode integration    | `openspec/specs/opencode-integration/`    |
| Multi-tenant governance | `openspec/specs/multi-tenant-governance/` |
| Storage layer           | `openspec/specs/storage/`                 |
| Codesearch integration  | `openspec/specs/codesearch-integration/`  |

---

## Server Modules (`cli/src/server/`)

The CLI crate hosts an Axum HTTP server with these key modules:

| Module                                                          | Purpose                                     |
| --------------------------------------------------------------- | ------------------------------------------- |
| `router.rs`                                                     | Route tree assembly, middleware stack       |
| `bootstrap.rs`                                                  | Server initialization, service wiring       |
| `plugin_auth.rs`                                                | GitHub OAuth + JWT issuance/refresh         |
| `auth_middleware.rs`                                            | JWT validation middleware layer             |
| `backup_api.rs`                                                 | Export/import job management                |
| `memory_api.rs`                                                 | Memory CRUD and search endpoints            |
| `knowledge_api.rs`                                              | Knowledge operations                        |
| `govern_api.rs`                                                 | Governance dashboard API                    |
| `tenant_api.rs`                                                 | Tenant CRUD                                 |
| `org_api.rs` / `team_api.rs` / `project_api.rs` / `user_api.rs` | Entity management                           |
| `role_grants.rs`                                                | Role administration (nested under `/admin`) |
| `mcp_transport.rs`                                              | MCP protocol transport                      |
| `health.rs`                                                     | Health, liveness, readiness probes          |

---

## Backup/Restore (`backup/` crate)

Core archive format for tenant data export/import:

- **Archive format**: tar.gz with `manifest.json` + NDJSON data files + `checksums.sha256`
- **API endpoints**: `cli/src/server/backup_api.rs` -- async export jobs, import with merge/replace/skip modes
- **S3 support**: Platform default or per-tenant S3 destination
- **Modules**: `archive.rs`, `manifest.rs`, `ndjson.rs`, `checksum.rs`, `s3.rs`, `validate.rs`, `destination.rs`

---

## Admin UI (`admin-ui/`)

React 19 SPA served at `/admin/*` when built. Tech stack: Vite, TypeScript, Tailwind CSS 4, TanStack Query, React Router 7.

- **Build**: `cd admin-ui && npm install && npm run build`
- **Dev**: `cd admin-ui && npm run dev` (proxies `/api` to localhost:8080)
- **Auth**: GitHub OAuth via `AuthContext`, JWT token management
- **Served by**: `cli/src/server/router.rs` using `tower_http::services::ServeDir`
- **Override path**: `AETERNA_ADMIN_UI_PATH` env var (default: `./admin-ui/dist`)

---

## ReplicaSet Deployment (NON-NEGOTIABLE)

Aeterna runs as a Kubernetes ReplicaSet with N replicas. Every feature MUST work correctly with multiple instances:

- **Shared state** -> Redis or PostgreSQL (never in-process HashMap/RwLock)
- **Singleton tasks** -> Redis distributed lock before execution
- **Caches** -> DashMap with TTL only (read-through, tolerate staleness)
- **File output** -> S3/object storage (never local filesystem as final destination)
- **Tokens** -> Redis with TTL (never in-process store)

Before implementing any new feature, ask: "Does this work with 3 replicas behind a load balancer?"

---

## Per-Tenant Provider Configuration

Managed by `TenantProviderRegistry` in `memory/src/provider_registry.rs`:

| Config Key           | Purpose                                      |
| -------------------- | -------------------------------------------- |
| `llm_provider`       | Provider type: `openai`, `google`, `bedrock` |
| `llm_model`          | Model identifier                             |
| `llm_api_key`        | Secret logical name for API key              |
| `embedding_provider` | Embedding provider type                      |
| `embedding_model`    | Embedding model identifier                   |
| `embedding_api_key`  | Secret logical name                          |

Cloud-specific keys: `llm_google_project_id`, `llm_google_location`, `llm_bedrock_region` (and `embedding_*` equivalents).

Platform defaults from env vars; tenant overrides from `TenantConfigDocument`; `DashMap` cache with TTL.

---

## Do NOT

- Use `edition = "2021"` -- always `2024`
- Write code without a failing test first
- Suppress clippy warnings without justification in `Cargo.toml`
- Use `unwrap()` in library code -- use `?` or explicit error handling
- Skip `cargo fmt` before committing
- Create new Cargo crates without adding them to the workspace `members` list
- Commit without running `cargo test --all` locally
- Start implementing a feature change without a validated OpenSpec proposal
- Store mutable shared state in process memory (use Redis or PostgreSQL -- Aeterna runs as a ReplicaSet)
- Use `LazyLock<RwLock<HashMap>>` for any state that must be visible across replicas
- Assume a background task runs on only one instance without a distributed lock
- Write export/import archives to local filesystem as the final destination (use S3)
- Store authentication tokens in process memory (use Redis)
