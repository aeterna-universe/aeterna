# Tenant Admin Control Plane

End-to-end guide for account-owned tenant lifecycle boundaries, tenant bootstrap, tenant configuration/secrets, shared Git provider connections, and scoped administrative workflows.

---

## Contents

1. [Authority model](#1-authority-model)
2. [Ownership model](#2-ownership-model)
3. [Platform-admin workflows](#3-platform-admin-workflows)
4. [Tenant bootstrap](#4-tenant-bootstrap)
5. [Tenant repository configuration](#5-tenant-repository-configuration)
6. [Tenant config and secrets](#6-tenant-config-and-secrets)
7. [Shared Git provider connections](#7-shared-git-provider-connections)
8. [Tenant-admin workflows](#8-tenant-admin-workflows)
9. [Cross-tenant targeting (--target-tenant)](#9-cross-tenant-targeting---target-tenant)
10. [Permission inspection](#10-permission-inspection)
11. [Audit trail](#11-audit-trail)
12. [Error reference](#12-error-reference)

---

## 1. Authority Model

Aeterna uses two administrative roles that sit above the standard tenant-scoped
role hierarchy:

| Role              | Rust variant              | Cedar value        | Scope                                                                   |
| ----------------- | ------------------------- | ------------------ | ----------------------------------------------------------------------- |
| **PlatformAdmin** | `RoleKind::PlatformAdmin` | `"platform_admin"` | Cross-tenant — tenant lifecycle, repo bindings, cross-tenant inspection |
| **TenantAdmin**   | `RoleKind::TenantAdmin`   | `"tenant_admin"`   | Within a single tenant — same permissions as `admin`                    |

The existing five tenant-scoped roles (`admin`, `architect`, `tech_lead`,
`developer`, `viewer`) continue to operate exactly as before.

### What PlatformAdmin can do

- Create, list, show, update, and deactivate tenants.
- Show, set, and validate tenant repository bindings for any tenant.
- Inspect, upsert, and validate tenant configuration for any tenant.
- Set or delete tenant secret entries across tenants.
- Create shared Git provider connections and grant/revoke tenant visibility.
- Inspect the role-permission matrix and effective permissions for any principal.
- Target any tenant for org/team/user/role/governance operations using
  `--target-tenant <tenant-id>`.
- View audit logs and export data across tenants.

### What PlatformAdmin cannot do (boundary rules)

- PlatformAdmin does **not** grant implicit read/write access to tenant memory
  or knowledge content without an explicit `--target-tenant` context.
- Content mutations (memory, knowledge, policy) within a tenant still require
  normal per-tenant policy evaluation after the tenant context is resolved.
- Cedar forbid policies and the server middleware layer
  (`X-Admin-Target-Tenant` header check) enforce this boundary at both the
  policy level and the request level.

---

## 2. Ownership Model

The control plane uses one ownership vocabulary everywhere:

| Surface                  | Tenant-owned                                                | Platform-owned                                       |
| ------------------------ | ----------------------------------------------------------- | ---------------------------------------------------- |
| Tenant config fields     | Runtime settings owned by the tenant                        | Deployment or platform-enforced fields               |
| Tenant secret entries    | Tenant integration credentials, webhook secrets, API tokens | Shared/platform bootstrap secrets                    |
| Repository binding       | Canonical repo binding for a tenant                         | Validation and cross-tenant control by PlatformAdmin |
| Git provider connections | Never directly owned by a tenant                            | Shared GitHub App connectivity and secret refs       |

Rules:

- `TenantAdmin` may mutate only `tenant`-owned config fields and secret entries
- `PlatformAdmin` may manage both ownership classes
- raw secret values are never returned from API or CLI responses
- shared Git provider connections expose only redacted metadata and connection IDs to tenants

---

## 3. Platform-Admin Workflows

### 2.1 Create a tenant

```bash
aeterna tenant create \
  --slug acme-prod \
  --name "Acme Prod" \
  --account acme \
  --environment prod
```

The server creates a tenant record, assigns a tenant UUID, persists the optional account/environment metadata, and emits audit/governance events.

### 2.2 List all tenants

```bash
aeterna tenant list
aeterna tenant list --json        # machine-readable
```

Requires `PlatformAdmin` — returns 403 for any other role.

### 2.3 Show a specific tenant

```bash
aeterna tenant show acme
```

### 2.4 Update a tenant

```bash
aeterna tenant update acme --name "Acme Corp (EU)"
```

### 2.5 Deactivate a tenant

```bash
aeterna tenant deactivate acme --yes
```

Sets `status: "deactivated"`. All API calls that resolve to this tenant will
begin returning `tenant_deactivated` errors. The record is retained for audit.

---

## 4. Tenant Bootstrap

### Bootstrapping from the CLI

```bash
# Authenticate as platform-admin
aeterna auth login

# Create the tenant record
aeterna tenant create --slug acme-prod --name "Acme Prod" --account acme --environment prod

# Bootstrap the repository binding
aeterna tenant repo-binding set \
  acme \
  --kind github \
  --remote-url https://github.com/acme/knowledge.git \
  --branch main \
  --credential-kind githubApp \
  --github-owner acme \
  --github-repo knowledge

# Inspect and validate tenant configuration
aeterna tenant config inspect --tenant acme
aeterna tenant config validate --tenant acme --file tenant-config.json
```

---

## 5. Tenant Repository Configuration

Each tenant has exactly one canonical knowledge repository binding. It describes
how knowledge reads and writes resolve storage for that tenant.

### Binding model

| Field                     | Required                   | Notes                                                        |
| ------------------------- | -------------------------- | ------------------------------------------------------------ |
| `kind`                    | Yes                        | `local`, `gitRemote`, or `github`                            |
| `localPath`               | For `local`                | Filesystem path                                              |
| `remoteUrl`               | For `gitRemote` / `github` | Remote repository URL                                        |
| `branch`                  | Usually yes                | Default branch for reads/writes                              |
| `credentialRef`           | Optional                   | Secret reference only — never raw secret material            |
| `gitProviderConnectionId` | Optional                   | Reference to a shared platform-owned Git provider connection |
| `sourceOwner`             | Auto                       | Record ownership metadata                                    |

### Show the current binding

```bash
aeterna tenant repo-binding show acme
aeterna tenant repo-binding show acme --json
```

### Set or update the binding

```bash
aeterna tenant repo-binding set \
  acme \
  --kind github \
  --remote-url https://github.com/acme/knowledge.git \
  --branch main
```

### Validate the binding

```bash
aeterna tenant repo-binding validate acme --kind github --remote-url https://github.com/acme/knowledge.git --branch main --credential-kind githubApp --github-owner acme --github-repo knowledge
```

Validation checks include structural validity, acceptable credential reference format, and for shared connections, explicit tenant visibility.

### Credential references

Bindings store **references**, never raw secrets:

```bash
# Local logical secret source
--credential-ref "local/acme-github-app"

# Kubernetes secret/key reference
--credential-ref "secret/aeterna-github-app-pem/pem-key"

# AWS secret manager ARN
--credential-ref "arn:aws:secretsmanager:..."
```

---

## 6. Tenant Config and Secrets

The tenant config provider separates non-secret config from secret references.

### Inspect tenant config

```bash
aeterna tenant config inspect --tenant acme
```

### Upsert tenant config

```bash
aeterna tenant config upsert --tenant acme --file tenant-config.json
```

Example payload:

```json
{
    "fields": {
        "runtime.logLevel": {
            "ownership": "tenant",
            "value": "info"
        }
    },
    "secretReferences": {}
}
```

### Manage tenant secrets

```bash
aeterna tenant secret set --tenant acme repo.token --value 'super-secret'
aeterna tenant secret delete --tenant acme repo.token
```

Important rules:

- raw secret values are write-only
- API and CLI responses return logical references only
- `TenantAdmin` cannot mutate `platform`-owned config or secrets

---

## 7. Shared Git provider connections

Shared Git provider connections let PlatformAdmin reuse one GitHub App integration across one or more tenants without copying PEM material into tenant-owned config.

### Grant and revoke visibility

```bash
aeterna tenant connection list acme
aeterna tenant connection grant acme --connection <connection-id>
aeterna tenant connection revoke acme --connection <connection-id>
```

### Isolation guarantees

- the connection stores secret references, not raw PEM/webhook values
- a tenant can reference only connections explicitly granted to it
- repository binding validation fails closed when the connection is not visible

---

## 8. Tenant-Admin Workflows

A `TenantAdmin` has the same permissions as `admin` within their own tenant but
cannot cross tenant boundaries.

### Typical tenant-admin tasks

```bash
# Manage users in their tenant
aeterna user list
aeterna user show --user-id <id>
aeterna user invite --email alice@acme.com --role developer

# Manage org structure
aeterna org create --name "Platform Engineering"
aeterna org list
aeterna team create --org-id <id> --name "API Team"

# Configure governance within their tenant
aeterna govern configure --template standard
aeterna govern status
```

None of these commands require `--target-tenant` when the operator's profile is
already resolved to the correct tenant.

---

## 9. Cross-Tenant Targeting (`--target-tenant`)

All CLI command groups that modify or inspect tenant-scoped state accept an optional `--target-tenant <tenant-id>` flag. This flag is the **only** supported way for a `PlatformAdmin` to operate in a tenant other than their own ambient profile tenant.

### How it works

1. The CLI sends `x-target-tenant-id: <tenant-id>` with each request.
2. The server checks that the caller holds `PlatformAdmin` for cross-tenant targeting.
3. On success the server resolves storage, policy evaluation, and audit events in the target tenant context.

`TenantAdmin` cannot use `--target-tenant` to escape its tenant boundary.

---

## 10. Permission Inspection

Roles can be assigned at four scopes: `tenant` (root), `org`, `team`, and
`project`. The `govern roles` command is the primary interface.

### List role assignments

```bash
aeterna govern roles list
aeterna govern roles list --json
```

### Assign a role

```bash
# Assign within the current tenant
aeterna govern roles assign \
  --principal alice@acme.com \
  --role developer \
  --scope "tenant"

# Platform-admin assigning a role in another tenant
aeterna govern roles assign \
  --principal bob@other.com \
  --role tenant_admin \
  --scope "tenant" \
  --target-tenant <other-tenant-id>
```

### Revoke a role

```bash
aeterna govern roles revoke \
  --principal alice@acme.com \
  --role developer \
  --scope "tenant"
```

### Available role values

| Value            | Maps to                   |
| ---------------- | ------------------------- |
| `platform_admin` | `RoleKind::PlatformAdmin` |
| `tenant_admin`   | `RoleKind::TenantAdmin`   |
| `admin`          | `RoleKind::Admin`         |
| `architect`      | `RoleKind::Architect`     |
| `tech_lead`      | `RoleKind::TechLead`      |
| `developer`      | `RoleKind::Developer`     |
| `viewer`         | `RoleKind::Viewer`        |

---

### Role-permission matrix

```bash
aeterna permissions matrix
aeterna permissions matrix --json
aeterna permissions effective --user-id <id>
```

---

## 11. Audit Trail

The control-plane authority model is expressed in three Cedar files:

| File                                 | Contains                                                                                                    |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------- |
| `policies/cedar/aeterna.cedarschema` | Entity types (`TenantRecord`, `TenantRepoBinding`, extended `User` with `platform_admin` flag), new actions |
| `policies/cedar/rbac.cedar`          | `platform_admin` and `tenant_admin` role permit policies                                                    |
| `policies/cedar/forbid.cedar`        | Cross-tenant boundary forbid rules; `tenant_admin` lifecycle restrictions                                   |

The canonical role catalog in Rust is `RoleKind` (defined in `server/`). Cedar
role strings (`"platform_admin"`, `"tenant_admin"`, etc.) must match the
`serde`-serialised snake_case values of `RoleKind` variants.

To inspect the current role-permission matrix as derived from the live Cedar
bundle:

```bash
aeterna permissions matrix
```

---

All tenant-lifecycle and role-mutation operations emit durable audit events. The
event schema follows the governance audit model:

| Operation              | Event kind                      |
| ---------------------- | ------------------------------- |
| Tenant create          | `tenant.created`                |
| Tenant update          | `tenant.updated`                |
| Tenant deactivate      | `tenant.deactivated`            |
| Repo binding set       | `tenant_repo_binding.set`       |
| Repo binding validated | `tenant_repo_binding.validated` |
| Role assigned          | `role.assigned`                 |
| Role revoked           | `role.revoked`                  |

View the audit trail:

```bash
aeterna govern audit --action all --since 30d
aeterna govern audit --action all --since 7d --export json --output audit.json
```

Platform-admins can view audit events across tenants using `--target-tenant`.

---

## 12. Error Reference

| Error key                   | HTTP status | Meaning                                                                                         |
| --------------------------- | ----------- | ----------------------------------------------------------------------------------------------- |
| `forbidden`                 | 403         | Caller lacks the required control-plane role or targets a forbidden tenant/config surface       |
| `tenant_not_found`          | 404         | Tenant ID does not exist                                                                        |
| `tenant_deactivated`        | 403         | Target tenant is deactivated                                                                    |
| `invalid_credential_ref`    | 422         | Repository binding contains raw or unsupported credential material                              |
| `git_provider_connection_*` | 404 / 422   | Shared Git provider connection not found or invalid                                             |
| `tenant_config_invalid`     | 422         | Tenant config fails validation (raw secret material, wrong ownership, cross-tenant secret refs) |
