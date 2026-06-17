# Aeterna Agent Integration Guide

How AI agents interact with the Aeterna memory and knowledge framework.

---

## 🔴 HARD CONSTRAINT — Public vs Internal Repository Split

**This repository is a PUBLIC OSS codebase.** A **separate internal repository**
owns everything related to deploying it on internal infrastructure.

Agents MUST respect this split at all times — in code, configuration, commit
messages, PR titles, PR bodies, issue bodies, review comments, filenames,
and inline code comments and docstrings.

### Categories that MUST NOT appear in this public repo

No specific forbidden strings are enumerated here on purpose — listing them
would itself be a leak. The **authoritative list** is maintained in the
internal repo and enforced mechanically by the leak-guard (see below).
Categorically, nothing from any of these classes belongs here:

- Any identifier naming a specific internally operated environment, cluster,
  namespace, tenant, customer, region, or availability zone
- Any internally owned domain, hostname, subdomain, or private DNS zone
- Any internally owned IP range, CIDR block, VPC/subnet identifier, or VPN
  endpoint
- Cloud account identifiers, resource ARNs/URNs, bucket names, database
  identifiers, managed-cluster names
- Secret-management paths, key-management identifiers, IAM role/policy
  identifiers
- GitOps/CD application names, release names, or pipeline identifiers tied
  to specific environments
- Internal ticket/incident identifiers, runbook names, internal URLs to
  wikis/dashboards/observability tools
- Production data of any kind, even if it looks anonymised

### What belongs in the INTERNAL repo instead

- Environment-specific Helm/Kustomize overlays
- IaC (Terraform / Pulumi / CDK) describing real cloud resources
- GitOps application manifests targeting real clusters
- Secret references (SealedSecrets, ExternalSecrets, SOPS-encrypted files)
- Runbooks, playbooks, on-call docs, incident post-mortems
- Anything mentioning a real environment by name

### What IS OK in this public repo

- Generic, environment-agnostic code and configuration
- Sample values files using placeholder hostnames drawn from IANA-reserved
  examples (see RFC 2606 / RFC 5737)
- Abstract wording ("a development environment", "the staging cluster",
  "an internal deployment") — never bound to a specific real target
- Fictional test fixtures that don't resemble anything real

### Before any commit or PR

Agents MUST, before `git commit` / `gh pr create`:

1. Run the leak-guard (see `docs/leak-guard.md`) against the staged diff
   AND the commit message AND any PR/issue body drafted. This checks both
   generic shape-based rules (committed in-repo) and a project-specific
   denylist loaded from outside the repo.
2. If anything matches, stop. Either scrub the draft or move the topic to
   the internal repo. Do NOT commit first and "clean up later" — rewriting
   public history is a partial mitigation, not a fix.

### Remediation if a leak does ship

- PR bodies/titles: `gh pr edit <n> --body-file <scrubbed.md>` (immediate,
  no history rewrite).
- Commit messages on an unmerged branch: `git rebase -i` + reword,
  force-push.
- Commit messages already on `master`: `git filter-branch --msg-filter`
  scoped to the affected range, then `git push --force-with-lease` after
  explicit human approval. This is loud and partial — GitHub retains
  old SHAs via the events/timeline API, and third-party mirrors/archives
  may have already captured the content. Prevention > cure.
- **Credentials** (tokens, keys, passwords, session cookies): history
  rewrites do not erase them from third-party caches. Rotate the credential
  at its source system immediately; clean up the repo as a secondary step.

### Code comments and docstrings

Inline comments and doc strings are part of the public surface and subject
to every rule above. No "note to self" references to internal systems in
source files.

See `docs/leak-guard.md` for the enforcement mechanism (CI + local hook).

---

## MCP Tools

Aeterna exposes tools via the Model Context Protocol (MCP), defined in `tools/src/server.rs` and implemented across `tools/src/`. Tools are registered in the `ToolRegistry` and served over HTTP at `/mcp/*`.

### Memory Tools

| Tool                          | Cedar Action   | Description                                    |
| ----------------------------- | -------------- | ---------------------------------------------- |
| `memory_add`                  | AddMemory      | Store a new memory entry with embedding        |
| `memory_search`               | SearchMemory   | Semantic search across memory layers           |
| `memory_delete`               | DeleteMemory   | Remove a memory entry                          |
| `memory_feedback`             | FeedbackMemory | Provide relevance feedback on search results   |
| `memory_optimize`             | OptimizeMemory | Trigger memory optimization (dedup, promotion) |
| `memory_reason`               | ReasonMemory   | R1-style reflective reasoning over memories    |
| `memory_close`                | CloseMemory    | Close a working memory session                 |
| `aeterna_memory_promote`      | AddMemory      | Promote memory to a higher layer               |
| `aeterna_memory_auto_promote` | OptimizeMemory | Auto-promote based on importance scoring       |

### Knowledge Tools

| Tool                       | Cedar Action     | Description                          |
| -------------------------- | ---------------- | ------------------------------------ |
| `knowledge_query`          | QueryKnowledge   | Search knowledge entries             |
| `knowledge_get`            | QueryKnowledge   | Retrieve a specific knowledge entry  |
| `knowledge_list`           | QueryKnowledge   | List knowledge entries with filters  |
| `knowledge_propose`        | ProposeKnowledge | Submit a knowledge proposal          |
| `knowledge_promote`        | PromoteKnowledge | Promote knowledge through governance |
| `knowledge_approve`        | ApproveKnowledge | Approve a pending knowledge proposal |
| `knowledge_reject`         | RejectKnowledge  | Reject a pending knowledge proposal  |
| `knowledge_link`           | ModifyKnowledge  | Link knowledge entries together      |
| `knowledge_review_pending` | QueryKnowledge   | List pending proposals for review    |

### Graph Tools

| Tool                    | Cedar Action | Description                               |
| ----------------------- | ------------ | ----------------------------------------- |
| `graph_query`           | QueryGraph   | Query the memory relationship graph       |
| `graph_neighbors`       | QueryGraph   | Find neighbors of a graph node            |
| `graph_path`            | QueryGraph   | Find shortest path between nodes          |
| `graph_link`            | ModifyGraph  | Create a relationship between nodes       |
| `graph_unlink`          | ModifyGraph  | Remove a relationship                     |
| `graph_traverse`        | QueryGraph   | Traverse the graph from a starting node   |
| `graph_find_path`       | QueryGraph   | Find paths with constraints               |
| `graph_violations`      | QueryGraph   | Detect constraint violations in the graph |
| `graph_implementations` | QueryGraph   | Find implementation nodes                 |

### Governance Tools

| Tool                        | Description                    |
| --------------------------- | ------------------------------ |
| `governance_request_create` | Create a governance request    |
| `governance_request_get`    | Get governance request details |
| `governance_request_list`   | List governance requests       |
| `governance_approve`        | Approve a governance request   |
| `governance_reject`         | Reject a governance request    |
| `governance_configure`      | Configure governance settings  |
| `governance_config_get`     | Get governance configuration   |
| `governance_role_assign`    | Assign a governance role       |
| `governance_role_revoke`    | Revoke a governance role       |
| `governance_role_list`      | List governance roles          |
| `governance_audit_list`     | List audit events              |

### Sync and Context Tools

| Tool                          | Description                                  |
| ----------------------------- | -------------------------------------------- |
| `sync_now`                    | Trigger immediate memory-knowledge sync      |
| `sync_status`                 | Check sync status                            |
| `resolve_federation_conflict` | Resolve conflicts during sync                |
| `context_assemble`            | Assemble context from multiple memory layers |
| `hindsight_query`             | Query past decisions and reasoning           |
| `meta_loop_status`            | Check meta-reasoning loop status             |
| `note_capture`                | Capture a note for later processing          |

### Tool Interface

All tools implement the `Tool` trait (`tools/src/tools.rs`):

```rust
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;       // JSON Schema
    async fn call(&self, params: Value) -> Result<Value, Box<dyn Error + Send + Sync>>;
}
```

Tools are registered in the `ToolRegistry` which provides:

- JSON Schema validation of inputs
- Cedar authorization check before execution
- Timeout enforcement
- Error classification (InvalidInput, NotFound, ProviderError, RateLimited, InternalError)

---

## A2A Protocol

Aeterna supports agent-to-agent communication via the A2A protocol, implemented in the `agent-a2a/` crate using the Radkit SDK.

### Architecture

- **Route**: `/a2a/*` on the Aeterna server
- **SDK**: `radkit = "0.0.4"` with `a2a-types = "0.1"`
- **Auth**: Separate `A2aAuthState` for agent authentication

### Capabilities

- **Agent cards**: Agents can discover each other's capabilities
- **Task lifecycle**: Create, update, and complete tasks across agents
- **Message passing**: Structured message exchange between agents

### Configuration

The `A2aConfig` is initialized during server bootstrap and shared via `AppState`. Agent-to-agent routes are mounted at `/a2a/*` with their own authentication layer separate from the user-facing JWT auth.

---

## OpenCode Plugin

Published as `@aeterna-org/opencode-plugin`, this provides the primary integration point for OpenCode-based AI agents.

### Features

- **MCP tools**: All 11+ core tools available via MCP transport
- **Hooks**: Lifecycle hooks for session start/end, memory capture
- **Context injection**: Automatic context assembly from memory layers
- **Session management**: WebSocket-based real-time sync

### Plugin Auth Flow

1. Plugin initiates GitHub OAuth via `POST /api/v1/auth/plugin/bootstrap`
2. User authenticates with GitHub
3. Server issues JWT (access + refresh tokens)
4. Plugin includes JWT in all subsequent MCP requests
5. Tokens refresh automatically via `POST /api/v1/auth/plugin/refresh`

---

## Agent Memory Hierarchy

Agents interact with a 7-layer memory system that spans from volatile agent-local context to durable tenant-wide knowledge.

### Layers (lowest to highest precedence)

```
Agent (1)    -- Volatile, per-agent working context
User (2)     -- Per-user preferences, history
Session (3)  -- Session-scoped working memory
Project (4)  -- Project-level shared knowledge
Team (5)     -- Team-wide conventions
Org (6)      -- Organization policies
Tenant (7)   -- Tenant-wide standards
```

### Promotion Rules

Memories can be promoted from lower layers to higher layers through:

1. **Manual promotion**: Via `aeterna_memory_promote` tool with governance approval
2. **Auto-promotion**: Via `aeterna_memory_auto_promote` based on importance scoring
3. **Governance gates**: Promotions to Team/Org/Tenant layers require approval from users with appropriate roles

### Importance Scoring

The `memory_optimize` tool triggers importance scoring that considers:

- Access frequency
- Relevance feedback (from `memory_feedback`)
- Age decay
- Cross-reference count (graph degree)

### Per-Tenant Embedding

Each tenant can configure its own embedding provider. When an agent stores or searches memory, the `TenantProviderRegistry` resolves the correct embedding service:

1. Check tenant config for `embedding_provider` / `embedding_model` / `embedding_api_key`
2. If configured, use tenant-specific provider (OpenAI, Google Vertex, AWS Bedrock)
3. Otherwise fall back to platform default

---

## ReplicaSet-Aware Agent Design

Aeterna runs as a Kubernetes ReplicaSet. Agents connecting via MCP or A2A must account for:

- **Request routing is non-sticky** -- consecutive requests from the same agent may hit different replicas. Do not assume in-process state persists between requests.
- **Memory operations are immediately consistent** -- writes to PostgreSQL/Qdrant are visible to all replicas immediately.
- **Cache staleness window** -- provider registry and quota caches have TTL (1h and 5min respectively). A config change may take up to TTL to propagate to all replicas.
- **Export/import jobs** -- job state is stored in Redis, so polling from any replica works. Download requests are served from S3, not local filesystem.
- **WebSocket connections are per-replica** -- if a replica restarts, WebSocket clients must reconnect (potentially to a different replica).

---

## Integration Patterns

### LangChain Adapter

The `adapters/src/langchain.rs` module provides a LangChain-compatible interface:

- Memory backend adapter for LangChain's memory abstraction
- Tool wrappers for LangChain's tool interface
- Chain-of-thought integration with the reasoning engine

### OpenCode Adapter

The `adapters/src/opencode.rs` module provides:

- Plugin configuration and lifecycle management
- Hook registration for memory capture
- Context window optimization via CCA (Context Compression Architecture)

### Custom Agent Integration via MCP

Any agent that speaks MCP can connect to Aeterna:

1. Connect to `/mcp/*` endpoint with a valid JWT
2. List available tools via MCP tool listing
3. Call tools with JSON parameters matching the tool's input schema
4. Handle responses (success with JSON result, or error with classification)

### Custom Agent Integration via REST API

Agents can also use the REST API directly:

1. Authenticate via GitHub OAuth to obtain JWT
2. Set `X-Tenant-ID` and Authorization headers on all requests
3. Use `/api/v1/memories/*` for memory operations
4. Use `/api/v1/knowledge/*` for knowledge queries
5. Use `/api/v1/governance/*` for governance operations

---

## Authorization for Agents

All tool invocations go through Cedar policy evaluation:

1. Tool name is mapped to a Cedar action (e.g., `memory_add` -> `AddMemory`)
2. The agent's `TenantContext` (tenant, user/agent ID, roles) is extracted
3. Cedar policy is evaluated against the action + principal + resource
4. If denied, the tool returns an authorization error

Agents with the `Agent` role have a restricted permission set focused on memory and knowledge read/write operations. Governance and admin operations require higher roles.

---

## OpenSpec Workflow for Contributors

For AI coding assistants contributing to Aeterna, see the OpenSpec workflow below. All non-trivial changes require a validated proposal before implementation.

### Quick Reference

```bash
openspec list              # Active changes
openspec list --specs      # Existing capabilities
openspec show <change>     # View details
openspec validate <change> --strict  # Validate
openspec archive <change-id> --yes   # Archive after deployment
```

### Three-Stage Workflow

1. **Create**: Proposal + tasks + delta specs in `openspec/changes/<change-id>/`
2. **Implement**: Follow tasks.md sequentially, TDD with 80% coverage minimum
3. **Archive**: Move to `changes/archive/` after deployment

See `openspec/` directory and the OpenSpec CLI for full workflow documentation.

---

## RLS Enforcement (issue #58)

Row-Level Security is authoritative tenant isolation at runtime, not a paper artifact. Every query against an RLS-protected table flows through one of two helpers on `PostgresBackend`:

- **`with_tenant_context(&ctx, |tx| …)`** — the default. Opens a transaction on the tenant pool (`aeterna_app`, NOBYPASSRLS), issues `SET LOCAL app.tenant_id = $1`, runs the body, commits. Every tenant-scoped handler MUST use this helper.
- **`with_admin_context(&ctx, action, |tx| …)`** — narrow. Opens a transaction on the admin pool (`aeterna_admin`, BYPASSRLS), runs the body, writes an `admin_scope = TRUE` audit row, commits. Used ONLY for PlatformAdmin cross-tenant endpoints (`?tenant=*`), scheduled cross-tenant jobs, and the migration runner.

### Rules

1. **Direct pool access is forbidden.** `backend.pool()` and `backend.admin_pool()` MUST NOT be used outside the two helpers. `cli/tests/admin_pool_access_lint.rs` + `storage/tests/admin_pool_access_lint.rs` enforce this (warn-level today; deny-level once Bundle A.3 Wave 6 lands).
2. **`WHERE tenant_id = ?` stays.** The explicit app-layer tenant filter is required defense in depth on top of RLS. RLS is the floor; the `WHERE` clause is the ceiling. Any query where they disagree is a bug surfaced by `storage/tests/rls_enforcement_test.rs`.
3. **Scheduled jobs pick explicitly.** Scheduled cross-tenant work uses `with_admin_context(&TenantContext::system_ctx(), …)`. Scheduled per-tenant work enumerates tenants via admin, then dispatches each tenant through `with_tenant_context(&TenantContext::from_scheduled_job(id, job), …)`.

See `openspec/changes/decide-rls-enforcement-model/design.md` for the full rationale.

---

## Graph Storage: Event-Sourced WAL Coordination

Phase 2 of `optimize-duckdb-graph-store` adds event-sourced write coordination across pods.

### Feature Flags

| Flag                                     | Default | Description                                               |
| ---------------------------------------- | ------- | --------------------------------------------------------- |
| `providers.graph.event_sourcing_enabled` | `true`  | Dual-write mutations to Postgres event log + local DuckDB |

### Metrics (Prometheus)

| Metric                      | Type      | Description                       |
| --------------------------- | --------- | --------------------------------- |
| `graph_nodes_total`         | Gauge     | Total live nodes per tenant       |
| `graph_edges_total`         | Gauge     | Total live edges per tenant       |
| `graph_traversal_depth`     | Histogram | Traversal depth per query         |
| `graph_duckdb_lock_wait_ms` | Histogram | Writer lock acquisition latency   |
| `graph_partition_load_ms`   | Histogram | Cold-start partition load latency |
| `graph_snapshot_bytes`      | Counter   | Bytes written in snapshot uploads |

### Key Modules

| Module                     | Purpose                                                       |
| -------------------------- | ------------------------------------------------------------- |
| `storage::graph_event_log` | Postgres event log: `append()`, `tail()`, `head_seq()`        |
| `storage::graph_projector` | Per-tenant tokio task that tails events and applies to DuckDB |
| `storage::graph_verify`    | SHA-256 digest computation for divergence detection           |

### Server Endpoints

| Endpoint                        | Method | Auth          | Description                                                       |
| ------------------------------- | ------ | ------------- | ----------------------------------------------------------------- |
| `/api/v1/internal/graph/digest` | GET    | PlatformAdmin | Returns SHA-256 digest of a tenant's graph state (`?tenant_id=X`) |

### Cron Jobs

| Job                        | Schedule | Description                                                                                                                               |
| -------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `verify_graph_consistency` | Hourly   | Iterates all active tenants, computes SHA-256 digests via `graph_verify`, emits `graph_consistency_divergences_total` counter on mismatch |

### Environment Variables

| Variable                    | Default | Description                     |
| --------------------------- | ------- | ------------------------------- |
| `GR_EVENT_SOURCING_ENABLED` | `true`  | Enable event-sourced dual-write |
