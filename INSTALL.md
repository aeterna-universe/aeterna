# Installation & Deployment Guide

This guide covers deploying Aeterna in production via the supported **Helm chart** path.

For interactive user access with federated authentication, also see:

- [`docs/guides/okta-auth-deployment.md`](docs/guides/okta-auth-deployment.md) — Okta-backed login, oauth2-proxy ingress, secret handling, OPAL/Cedar authorization

For high-availability infrastructure references (Patroni, Qdrant cluster, Redis Sentinel), see:

- [`docs/guides/ha-deployment.md`](docs/guides/ha-deployment.md)

---

## Server-Side LLM Provider Deployment

Aeterna can construct server-side LLM and embedding services at startup from deployment configuration. The runtime currently supports these provider values through `AETERNA_LLM_PROVIDER`:

- `openai`
- `google`
- `bedrock`
- `none`

### Common Runtime Selection

Set the provider explicitly:

```bash
export AETERNA_LLM_PROVIDER=openai
```

If you set `AETERNA_LLM_PROVIDER=none`, provider-dependent memory and reasoning operations fail closed instead of silently falling back.

### OpenAI

```bash
export AETERNA_LLM_PROVIDER=openai
export OPENAI_API_KEY=your-api-key
export AETERNA_OPENAI_MODEL=gpt-4.1-mini
export AETERNA_OPENAI_EMBEDDING_MODEL=text-embedding-3-small
```

### Google Cloud (Vertex AI / Gemini)

Required runtime settings:

```bash
export AETERNA_LLM_PROVIDER=google
export AETERNA_GOOGLE_PROJECT_ID=my-gcp-project
export AETERNA_GOOGLE_LOCATION=global
export AETERNA_GOOGLE_MODEL=gemini-2.5-flash
export AETERNA_GOOGLE_EMBEDDING_MODEL=text-embedding-005
```

Authentication is resolved in this order:

1. `GOOGLE_ACCESS_TOKEN`
2. Application Default Credentials via `GOOGLE_APPLICATION_CREDENTIALS`
3. Ambient ADC in GCP runtimes such as GKE Workload Identity

Example with a service-account key file:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/var/run/secrets/google/service-account.json
```

Operational notes:

- the configured service account needs Vertex AI access in the configured project
- the selected location and model identifiers must match enabled Vertex AI resources
- missing project, location, or model configuration fails closed during provider construction

### AWS Bedrock

Required runtime settings:

```bash
export AETERNA_LLM_PROVIDER=bedrock
export AETERNA_BEDROCK_REGION=eu-west-1
export AETERNA_BEDROCK_MODEL=anthropic.claude-3-5-sonnet-20241022-v2:0
export AETERNA_BEDROCK_EMBEDDING_MODEL=amazon.titan-embed-text-v2:0
```

Authentication uses the normal AWS credential chain, including:

- `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`
- `AWS_SESSION_TOKEN`
- IAM roles for service accounts or instance profiles
- shared AWS config/credential files

Operational notes:

- the runtime region must be a Bedrock-enabled region for the selected models
- the workload identity must have permission to invoke the selected Bedrock models
- missing region or model configuration fails closed during provider construction

### Helm / Setup CLI Alignment

The setup wizard and Helm chart emit the exact environment variables consumed by the runtime:

- `AETERNA_LLM_PROVIDER`
- `AETERNA_GOOGLE_PROJECT_ID`
- `AETERNA_GOOGLE_LOCATION`
- `AETERNA_GOOGLE_MODEL`
- `AETERNA_GOOGLE_EMBEDDING_MODEL`
- `AETERNA_BEDROCK_REGION`
- `AETERNA_BEDROCK_MODEL`
- `AETERNA_BEDROCK_EMBEDDING_MODEL`

Use the chart examples for cloud deployments:

- `charts/aeterna/examples/values-gke.yaml`
- `charts/aeterna/examples/values-aws.yaml`

---

## Native CLI Installation (macOS and Linux)

The supported `aeterna` CLI distribution is published through GitHub Releases and the repository installer script.

### Supported release assets

The CLI release workflow publishes these archives for every tagged release:

- `aeterna-x86_64-linux.tar.gz`
- `aeterna-aarch64-linux.tar.gz`
- `aeterna-x86_64-macos.tar.gz`
- `aeterna-aarch64-macos.tar.gz`
- matching `*.sha256` checksum files
- combined `checksums.sha256`

These assets are produced by `.github/workflows/cli-release.yml`.

### Quick install via installer script

```bash
curl -fsSL https://raw.githubusercontent.com/kikokikok/aeterna/main/install.sh | sh
```

The installer:
- detects your OS/architecture
- downloads the matching GitHub Release archive
- installs `aeterna` into `/usr/local/bin` when writable, otherwise `~/.local/bin`

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/kikokikok/aeterna/main/install.sh | sh -s -- --version v0.6.0
```

Install to a custom directory:

```bash
curl -fsSL https://raw.githubusercontent.com/kikokikok/aeterna/main/install.sh | sh -s -- --install-dir "$HOME/bin"
```

### Manual install from release asset

```bash
# Example: macOS Apple Silicon
VERSION=v0.6.0
ARCHIVE=aeterna-aarch64-macos.tar.gz

curl -fsSLO "https://github.com/kikokikok/aeterna/releases/download/${VERSION}/${ARCHIVE}"
curl -fsSLO "https://github.com/kikokikok/aeterna/releases/download/${VERSION}/${ARCHIVE}.sha256"
shasum -a 256 -c "${ARCHIVE}.sha256"
tar -xzf "${ARCHIVE}"
install -m 0755 aeterna "$HOME/.local/bin/aeterna"
```

### Verify installation

```bash
aeterna --version
aeterna auth status --json
```

### First-time onboarding

```bash
# 1. log in to a target
aeterna auth login --profile dev --server-url https://aeterna.example.com

# 2. inspect effective config and precedence
aeterna config show --profile dev

# 3. validate configuration
aeterna config validate --profile dev

# 4. inspect resolved runtime context
aeterna status --verbose

# 5. verify authentication
aeterna auth status --profile dev
```

### Daily authenticated usage

```bash
# switch target/profile
aeterna auth status --profile dev
aeterna config show --profile prod

# memory / knowledge / governance
aeterna memory search "database preferences"
aeterna knowledge search "oauth callback"
aeterna govern status

# operator runtime check
aeterna admin health --verbose
```

### Operator/admin flow

```bash
# install
curl -fsSL https://raw.githubusercontent.com/kikokikok/aeterna/main/install.sh | sh

# log in against the operator target
aeterna auth login --profile ops --server-url https://aeterna.example.com

# validate config and runtime
aeterna config validate --profile ops
aeterna admin health --verbose
aeterna admin validate --target all
```

---

## Helm Chart Deployment

### Public Artifacts

The Aeterna container image and Helm chart are publicly available:

| Artifact | Location |
|----------|----------|
| Container image | `ghcr.io/kikokikok/aeterna` |
| Helm chart (OCI) | `oci://ghcr.io/kikokikok/charts/aeterna` |

No authentication is required to pull either artifact.

### Quick Install

```bash
helm install aeterna oci://ghcr.io/kikokikok/charts/aeterna \
  --version 0.1.0 \
  -n aeterna --create-namespace \
  -f your-values.yaml
```

### Example Values

See `charts/aeterna/examples/` for cloud-specific examples:

- `values-gke.yaml` — Google Kubernetes Engine with Vertex AI
- `values-aws.yaml` — AWS EKS with Bedrock
- `values-aks.yaml` — Azure AKS
- `values-production.yaml` — Production-hardened HA deployment

For sizing presets:

- `values-small.yaml` — Single replicas, minimal resources
- `values-medium.yaml` — 2 replicas, moderate resources
- `values-large.yaml` — 3+ replicas, full HA

### Environment-Specific Values

Keep environment-specific deployment values (project IDs, secret references, cluster-specific config) in a **separate private repository** to avoid leaking internal infrastructure details. The public chart and examples provide the structure; your private repo provides the overrides.

---

## Secret Management

### Kubernetes Secrets

The Helm chart generates secrets on first install for internal credentials (PostgreSQL password, Redis password, API keys). To manage secrets externally:

1. **Pre-create secrets** before `helm install`:
   ```bash
   kubectl create secret generic aeterna-secrets -n aeterna \
     --from-literal=AETERNA_ADMIN_API_KEY=your-key \
     --from-literal=DATABASE_URL=postgres://user:pass@host:5432/aeterna
   ```

2. **Reference in values.yaml**:
   ```yaml
   aeterna:
     existingSecret: aeterna-secrets
   ```

3. **External secret managers** (recommended for production):
   - Use [External Secrets Operator](https://external-secrets.io/) to sync from AWS Secrets Manager, GCP Secret Manager, or HashiCorp Vault
   - See `charts/aeterna/examples/values-sops.yaml` for SOPS-encrypted values example

### GitHub App Authentication

For GitHub organization sync, Aeterna uses GitHub App certificate-based authentication (not personal access tokens):

```bash
kubectl create secret generic aeterna-github-app-pem -n aeterna \
  --from-file=private-key.pem=/path/to/your-github-app.pem
```

Configure in values.yaml:
```yaml
aeterna:
  github:
    appId: "your-app-id"
    installationId: "your-installation-id"
    privateKeySecret: aeterna-github-app-pem
```

### Tenant-scoped config and secrets

For multi-tenant deployments, Aeterna materializes one Kubernetes ConfigMap and one paired Secret per tenant:

- `aeterna-tenant-<tenant-id>`
- `aeterna-tenant-<tenant-id>-secret`

The Helm chart can seed empty tenant containers via `tenantConfigProvider.seedTenants`, but the canonical tenant config content is managed through the control plane.

```yaml
tenantConfigProvider:
  enabled: true
  seedTenants:
    - "11111111-1111-1111-1111-111111111111"
```

### Tenant bootstrap workflow

Use the control plane to bootstrap tenants, then keep environment-specific overlays in a **private deployment repository**.

```bash
# authenticate as PlatformAdmin
aeterna auth login

# create tenant record
aeterna tenant create --slug acme --name "Acme Corp"

# configure the tenant repository binding
aeterna tenant repo-binding set acme \
  --kind github \
  --remote-url https://github.com/acme/knowledge.git \
  --branch main \
  --credential-kind githubApp \
  --github-owner acme \
  --github-repo knowledge

# inspect and validate tenant config
aeterna tenant config inspect --tenant acme
aeterna tenant config validate --tenant acme --file tenant-config.json
```

For the full ownership model, shared Git provider connections, and tenant-admin workflows, see:

- [`docs/guides/tenant-admin-control-plane.md`](docs/guides/tenant-admin-control-plane.md)

---

## Ingress & TLS

### TLS Termination

The chart supports TLS via cert-manager or pre-provisioned certificates:

```yaml
ingress:
  enabled: true
  className: nginx
  hosts:
    - host: aeterna.example.com
      paths:
        - path: /
          pathType: Prefix
  tls:
    - secretName: aeterna-tls
      hosts:
        - aeterna.example.com
```

**With cert-manager** (recommended):
```yaml
ingress:
  annotations:
    cert-manager.io/cluster-issuer: letsencrypt-prod
  tls:
    - secretName: aeterna-tls
      hosts:
        - aeterna.example.com
```

**Pre-provisioned certificate**:
```bash
kubectl create secret tls aeterna-tls -n aeterna \
  --cert=/path/to/tls.crt \
  --key=/path/to/tls.key
```

---

## Database Migration

Aeterna initializes its schema automatically on startup. The Helm chart includes a migration Job that runs before the main deployment:

- Migrations are idempotent — safe to re-run on upgrade
- The migration job uses `initContainers` to wait for PostgreSQL readiness
- Schema includes: `organizational_units`, `users`, `memberships`, `governance_roles`, `user_roles`, `agents`, `tenants`, plus OPAL authorization views

### Manual Migration

If you need to run migrations manually:

```bash
# Port-forward to PostgreSQL (or use your DB endpoint directly)
kubectl port-forward svc/aeterna-postgresql 5432:5432 -n aeterna

# Run migrations
DATABASE_URL="postgres://aeterna:password@localhost:5432/aeterna" \
  cargo run -p cli -- migrate
```

---

## Upgrade Procedures

### Helm Upgrade

```bash
# Standard upgrade (recommended: no --wait flag for large deployments)
helm upgrade aeterna oci://ghcr.io/kikokikok/charts/aeterna \
  --version <new-version> \
  -n aeterna \
  -f your-values.yaml \
  --no-hooks --server-side=false

# Or from local chart during development
helm upgrade aeterna ./charts/aeterna \
  -n aeterna \
  -f your-values.yaml \
  --no-hooks --server-side=false
```

### Pre-Upgrade Checklist

1. Review the [CHANGELOG](CHANGELOG.md) for breaking changes
2. Back up PostgreSQL data (`pg_dump`)
3. Ensure new image is available (`ghcr.io/kikokikok/aeterna:<tag>`)
4. Test with `helm diff` if available:
   ```bash
   helm diff upgrade aeterna ./charts/aeterna -n aeterna -f your-values.yaml
   ```

### Rollback

```bash
helm rollback aeterna <revision> -n aeterna
```

---

## GitHub Organization Sync

Aeterna can sync users, teams, and memberships from a GitHub organization:

- **Scheduled sync**: Kubernetes CronJob runs every 15 minutes by default
- **Webhook sync**: GitHub Organization Webhooks push real-time updates (subscribe to: `team`, `membership`, `organization`, `member`)
- **Idempotent**: Re-syncs are safe; keyed on user email uniqueness

Configure in values.yaml:
```yaml
aeterna:
  github:
    enabled: true
    orgName: your-org
    syncSchedule: "*/15 * * * *"
```

---

## Supported Deployment Paths

| Path | Status | Documentation |
|------|--------|---------------|
| **Helm chart** (Kubernetes) | **Supported** | This file + `charts/aeterna/examples/` |
| **Okta-backed interactive auth** | **Supported** | `docs/guides/okta-auth-deployment.md` |
| **HA infrastructure references** | **Reference only** | `docs/guides/ha-deployment.md` |

> **Note:** Code search is not a built-in Aeterna component. It integrates as an external skill via pluggable MCP backends (e.g., JetBrains Code Intelligence MCP). No sidecar indexer, StatefulSet, or ShardRouter deployment is required.

---

## Local-First Memory (OpenCode Plugin)

The OpenCode plugin includes a **local-first memory store** that keeps personal layers (agent, user, session) in an embedded SQLite database on the developer's machine. This enables offline-first operation with automatic sync when connected to an Aeterna server.

### Architecture

```
Developer Machine                          Aeterna Server
┌──────────────────────┐                  ┌────────────────────┐
│  OpenCode Plugin     │                  │  Axum HTTP API     │
│                      │                  │                    │
│  ┌────────────────┐  │   push (30s)     │  POST /sync/push   │
│  │ SQLite Store   │──┼────────────────► │  (upsert + embed)  │
│  │ ~/.aeterna/    │  │                  │                    │
│  │  local.db      │◄─┼────────────────  │  GET /sync/pull    │
│  └────────────────┘  │   pull (60s)     │  (cursor paginate) │
│                      │                  │                    │
│  Personal layers:    │                  │  Shared layers:    │
│  agent, user,        │                  │  project, team,    │
│  session             │                  │  org, tenant      │
└──────────────────────┘                  └────────────────────┘
```

### Configuration

The local store is enabled by default. Configure via `.aeterna/config.toml` or environment variables:

```toml
[local]
enabled = true                      # Enable/disable local store
db_path = "~/.aeterna/local.db"     # SQLite database path
sync_push_interval_ms = 30000       # Push cycle interval (default: 30s)
sync_pull_interval_ms = 60000       # Pull cycle interval (default: 60s)
max_cached_entries = 50000          # Max cached shared-layer entries
session_storage_ttl_hours = 24      # Session memory retention
```

Environment variable overrides (take precedence over config file):

| Variable | Description |
|----------|-------------|
| `AETERNA_LOCAL_ENABLED` | Enable/disable local store |
| `AETERNA_LOCAL_DB_PATH` | SQLite database path |
| `AETERNA_LOCAL_SYNC_PUSH_INTERVAL_MS` | Push sync interval |
| `AETERNA_LOCAL_SYNC_PULL_INTERVAL_MS` | Pull sync interval |
| `AETERNA_LOCAL_MAX_CACHED_ENTRIES` | Max cached entries |
| `AETERNA_LOCAL_SESSION_STORAGE_TTL_HOURS` | Session TTL |

### Offline Behavior

- **Personal layers** (agent/user/session) work fully offline — all reads and writes go to the local SQLite database
- **Shared layers** (project/team/org/tenant) require server connectivity for writes; reads use the local cache with staleness warnings after 10 minutes
- When connectivity is restored, the sync engine automatically pushes queued changes and pulls remote updates
- Conflict resolution is server-wins: if the same memory ID has a newer `updated_at` on the server, the server version takes precedence

### Server-Side Requirements

The sync endpoints require the Aeterna server (v0.3.0+) with:
- `POST /api/v1/sync/push` — accepts batched memory entries with device ID
- `GET /api/v1/sync/pull` — cursor-based pagination with layer filtering
- Authentication via Bearer token (same `AETERNA_TOKEN` used for all API calls)
- PostgreSQL with `device_id` and `importance_score` columns on `memory_entries` table (applied automatically by schema initialization)
