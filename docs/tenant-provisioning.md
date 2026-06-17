# Tenant provisioning (canonical reference)

**Audience:** platform operators and tenant admins.
**Status:** canonical reference for the v0.9 apply-based pipeline.
**Related:** [`tenant-cli-migration-v0.9.md`](guides/tenant-cli-migration-v0.9.md) · [`security/tenant-provisioning-security.md`](security/tenant-provisioning-security.md) · spec: `openspec/changes/harden-tenant-provisioning/`.

---

## 1. The lifecycle

Every tenant transition the CLI or UI performs goes through the same four steps:

```
render → edit → diff → apply
   │        │       │      │
   ▼        ▼       ▼      ▼
 server   local   server  server
 current  file    dry-run  write
 state    edit            transaction
```

1. **`render`** — ask the server for its current state of a tenant, serialised as a `TenantManifest`. This is the *only* authoritative source of the current state; there is no client-side cache.
2. **`edit`** — modify the manifest locally in whatever tool you prefer (editor, `jq`, `sed`, a Helm values template, a Terraform resource).
3. **`diff`** — ask the server what it would change if the edited manifest were applied. No writes. Safe to run from CI or pre-commit hooks.
4. **`apply`** — submit the edited manifest to `POST /api/v1/admin/tenants/provision`. The server writes the full tenant state in a single transaction. One audit row. One generation bump. No partial states.

This is a **GitOps-style** pipeline. The manifest is the authoritative declaration; everything in the database is a materialisation of the most recent `apply`.

## 2. Bootstrap — creating a tenant from scratch

```bash
# 1. Author the manifest by hand, or start from a template:
cat > acme.manifest.json <<'EOF'
{
  "apiVersion": "aeterna/v1",
  "kind": "Tenant",
  "metadata": {
    "labels":      { "env": "prod", "region": "eu-west-1" },
    "annotations": { "owner": "platform-team@acme.example" }
  },
  "tenant": {
    "slug": "acme",
    "name": "Acme Corp",
    "domainMappings": ["acme.example.com"]
  },
  "config": {
    "fields": {
      "llm.default.model":       { "value": "claude-sonnet-4.7" },
      "embedding.default.model": { "value": "text-embedding-3-large" }
    },
    "secretReferences": {
      "openai.api_key":    { "kind": "file",      "path": "/run/secrets/openai" },
      "anthropic.api_key": { "kind": "k8sSecret", "namespace": "aeterna", "name": "anthropic", "key": "key" }
    }
  },
  "secrets": [
    {
      "logicalName": "repo.token",
      "ownership":   "tenant",
      "secretValue": "<literal token — prefer --secret-file or K8sSecretRef in production>"
    }
  ],
  "repository": {
    "kind":           "github",
    "github":         { "owner": "acme", "repo": "knowledge" },
    "branch":         "main",
    "branchPolicy":   "directCommit",
    "credentialKind": "githubApp",
    "credentialRef":  "acme-gh-app"
  },
  "providers": {
    "llm":       { "kind": "anthropic", "model": "claude-sonnet-4.7", "secretRef": "anthropic.api_key" },
    "embedding": { "kind": "openai",    "model": "text-embedding-3-large", "secretRef": "openai.api_key" }
  }
}
EOF

# 2. Dry-run validate:
aeterna tenant validate -f acme.manifest.json

# 3. Apply:
aeterna tenant apply -f acme.manifest.json --watch
```

### Worked example: the `secretRef` indirection

**Dev (local file):**

```json
"openai.api_key": { "kind": "file", "path": "/home/dev/.secrets/openai" }
```

At apply time, the server reads the file via the `FileRef` resolver into a `SecretBytes`, and stores only the reference string (`file:/home/dev/.secrets/openai`) in `tenant_config_secrets.reference`. The plaintext bytes never touch the database.

**Production (Kubernetes):**

```json
"openai.api_key": {
  "kind":      "k8sSecret",
  "namespace": "aeterna",
  "name":      "openai",
  "key":       "api-key"
}
```

Same mechanism, different resolver. The tenant manifest itself is identical; only the `secretReferences` entry changes between environments — making the manifest promotable across envs with nothing more than a `secretReferences` overlay.

## 3. Day-2 edits — the render → diff → apply loop

```bash
# 1. Pull current state:
aeterna tenant render --slug acme > acme.manifest.json

# 2. Edit locally. Example: change the LLM model.
jq '.providers.llm.model = "claude-opus-4.8"' acme.manifest.json > tmp && mv tmp acme.manifest.json

# 3. Preview the delta:
aeterna tenant diff --slug acme -f acme.manifest.json

# 4. Apply:
aeterna tenant apply -f acme.manifest.json
```

`aeterna tenant apply` is **interactive by default** — it prints the diff and prompts before proceeding. Pass `--yes` to skip the prompt for CI usage. Pass `--watch` to stream per-step progress via SSE.

## 4. Manifest reference

All shapes come from `cli/src/server/tenant_api.rs`. Field names in JSON use `camelCase`; Rust structs are `snake_case` with `#[serde(rename_all = "camelCase")]`.

### 4.1 Top-level shape

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `apiVersion`    | string | ✓ | `"aeterna/v1"` |
| `kind`          | string | ✓ | `"Tenant"` |
| `metadata`      | [Metadata](#42-metadata) | — | Generation, labels, annotations |
| `tenant`        | [Tenant](#43-tenant)     | ✓ | Slug, name, domain mappings |
| `config`        | [Config](#44-config)     | — | Non-sensitive fields + secret references |
| `secrets[]`     | [Secret](#45-secrets)    | — | Secret entries bound to this tenant |
| `providers`     | [Providers](#46-providers) | — | Declarative LLM / embedding / memory-layer providers |
| `repository`    | [Repository](#47-repository) | — | Tenant knowledge-repo binding (GitHub / Local) |
| `hierarchy[]`   | Tenant        | — | Initial org hierarchy (root orgs / orgs / teams / members) |
| `roles[]`       | RoleAssignment | — | Role grants bundled with this tenant |

### 4.2 `metadata`

```json
{
  "metadata": {
    "generation": 17,
    "labels":      { "env": "prod" },
    "annotations": { "doc-url": "https://wiki.example.com/tenants/acme" }
  }
}
```

- `generation` — caller-owned monotonic counter. `/provision` **rejects** an apply where the incoming `generation` ≤ server's last. Omit and the server auto-bumps. Use it to enforce "this manifest is based on render output N; reject if someone else applied in between."
- `labels` / `annotations` — free-form k/v pairs. Not interpreted; part of the canonical hash, so label drift shows up in `diff`.

### 4.3 `tenant`

```json
{
  "tenant": {
    "slug": "acme",
    "name": "Acme Corp",
    "domainMappings": ["acme.example.com", "acme.internal"]
  }
}
```

`slug` is the immutable URL-safe identifier (B2 §1.3 — slug changes require the rename workflow, not a manifest edit). `name` is display-only. `domainMappings` are verified-domain entries for auth/SSO auto-assignment.

### 4.4 `config`

```json
{
  "config": {
    "fields": {
      "llm.default.model":         { "value": "claude-sonnet-4.7" },
      "index.embedding.dimension": { "value": "1536" }
    },
    "secretReferences": {
      "openai.api_key": { "kind": "file", "path": "/run/secrets/openai" }
    }
  }
}
```

`fields` — non-sensitive config. Values are strings by convention (numbers / booleans stringified) so the canonical hash is stable across JSON-number drift.

`secretReferences` — **where sensitive config lives**. Never put a secret value in `fields`. A `secretReferences` entry is addressed by its logical name (e.g. `openai.api_key`) and resolved at apply time by one of the resolvers below.

#### Supported secret reference kinds

| Kind         | Shape                                                        | Where resolved |
|--------------|--------------------------------------------------------------|----------------|
| `file`       | `{ kind: "file", path: "/abs/path" }`                        | dev / local; reads the file via `FileRef` |
| `k8sSecret`  | `{ kind: "k8sSecret", namespace, name, key }`                | in-cluster; reads via `K8sSecretRef` |
| `env`        | `{ kind: "env", var: "OPENAI_KEY" }`                         | dev / single-node only |
| `inline`     | `{ kind: "inline", value: "…" }`                             | **discouraged** — only for tests and bootstrap |

See [`security/tenant-provisioning-security.md`](security/tenant-provisioning-security.md) for which kinds are permitted in which environments (cluster config gates `file` and `inline`).

### 4.5 `secrets`

```json
{
  "secrets": [
    { "logicalName": "repo.token",      "ownership": "tenant",   "secretValue": "<bytes>" },
    { "logicalName": "crypto.root_key", "ownership": "platform", "secretValue": "<bytes>" }
  ]
}
```

Same `logicalName` namespace as `config.secretReferences`, but these are **bound** secret bytes (stored encrypted in `tenant_config_secrets.ciphertext`). The `ownership` field distinguishes secrets the tenant admin can rotate (`tenant`) from secrets only platform admins can touch (`platform`) — see B2 §3 for the ownership model.

`secretValue` is a `mk_core::SecretBytes` — redacted from `Debug`, not re-serialised on error paths. Prefer `--secret-file` or `--secret-stdin` on the CLI over inlining in the JSON.

### 4.6 `providers`

```json
{
  "providers": {
    "llm":       { "kind": "anthropic", "model": "claude-sonnet-4.7", "secretRef": "anthropic.api_key" },
    "embedding": { "kind": "openai",    "model": "text-embedding-3-large", "secretRef": "openai.api_key" },
    "memoryLayers": {
      "episodic": { "kind": "qdrant",    "config": { "url": "https://qdrant.example" } },
      "semantic": { "kind": "memory",    "config": { } }
    }
  }
}
```

Every `secretRef` MUST be a logical name declared in `config.secretReferences`. `validate` catches unknown refs at dry-run time.

### 4.7 `repository`

Shape from `SetTenantRepositoryBindingRequest`:

```json
{
  "repository": {
    "kind":           "github",
    "github":         { "owner": "acme", "repo": "knowledge" },
    "branch":         "main",
    "branchPolicy":   "directCommit",
    "credentialKind": "githubApp",
    "credentialRef":  "acme-gh-app"
  }
}
```

`kind` is one of `github`, `local`. Local binds to a filesystem path for dev/test:

```json
{ "kind": "local", "local": { "path": "/repos/acme" }, "branch": "main" }
```

## 5. Subcommand reference

Mutations all go through `apply`; reads are separate.

### Reads

- **`aeterna tenant list`** — list tenants (PlatformAdmin).
- **`aeterna tenant show <slug>`** — show tenant row + summary.
- **`aeterna tenant render --slug <slug>`** — emit the canonical `TenantManifest` for the current state.
- **`aeterna tenant repo-binding show <slug>`** — inspect the binding.
- **`aeterna tenant config inspect <slug>`** — inspect config fields + secret references.
- **`aeterna tenant connection list <slug>`** — list Git provider connections visible to the tenant.

### Manifest pipeline

- **`aeterna tenant validate -f manifest.json`** — dry-run. No writes. Returns the canonical hash, the diff if a `--slug` is supplied, and any validation errors.
- **`aeterna tenant diff --slug <slug> -f manifest.json`** — unified (`-o unified`) or JSON (`-o json`) diff of the incoming manifest vs server state.
- **`aeterna tenant apply -f manifest.json [--yes] [--watch]`** — real apply. Prompts interactively unless `--yes`.
- **`aeterna tenant watch --slug <slug>`** — SSE stream of the per-step provisioning events for the latest apply (§7.5).

### Lifecycle

- **`aeterna tenant deactivate <slug>`** — soft-delete with tombstone; separate from the manifest model.

### User context (client-side)

- **`aeterna tenant use <slug>`** — write default tenant into `.aeterna/context.toml`.
- **`aeterna tenant switch <slug>`** — write server-side preference (persists across devices).
- **`aeterna tenant current`** — show effective tenant.

### Connection visibility (PlatformAdmin)

Not part of the manifest model; has its own standalone surface:

- **`aeterna tenant connection grant <slug> --connection <id>`**
- **`aeterna tenant connection revoke <slug> --connection <id>`**

A future manifest revision may absorb these under a top-level `connections[]` field; for now they are standalone operations on the `git_provider_connections_tenants` junction.

## 6. Under the hood

Every `tenant apply` resolves to a single HTTP call:

```
POST /api/v1/admin/tenants/provision
Content-Type: application/json
X-Aeterna-Client-Kind: cli
Authorization: Bearer <token with tenants:provision scope OR PlatformAdmin role>

<TenantManifest body>
```

The server runs `provision_tenant` — one Postgres transaction that:

1. Resolves every `secretReferences` entry through its resolver.
2. Validates the manifest shape and cross-references (unknown `secretRef` in providers, duplicate logical names, etc.).
3. Checks the `generation` precondition.
4. Writes the tenant, config, secrets, repo binding, hierarchy, and role assignments atomically.
5. Emits a single `TenantAuditEvent::Provisioned` row with the canonical hash and generation.
6. Publishes per-step SSE events for `watch` consumers.

Either the whole apply succeeds and the tenant reaches the declared state, or it fails and nothing is written. No half-provisioned states.

## 7. Migrating from the pre-0.9 CLI

See [`guides/tenant-cli-migration-v0.9.md`](guides/tenant-cli-migration-v0.9.md) for the full breaking-change table and pin-to-0.8 rollback instructions.
