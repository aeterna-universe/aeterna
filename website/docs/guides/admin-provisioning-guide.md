# Admin Provisioning Guide

End-to-end walkthrough for bootstrapping a PlatformAdmin, authenticating, creating tenants, managing users and roles, and building organizational hierarchy — using both the CLI and REST API.

**Prerequisites:** A running Aeterna deployment (see [Helm Quickstart](../helm/quickstart-local.md)).

> **Cross-references**
>
> - [CLI Quick Reference](cli-quick-reference.md) — concise command cheat sheet
> - [Tenant Admin Control Plane](tenant-admin-control-plane.md) — deep-dive on tenant ownership, config/secrets, shared Git connections

---

## Contents

1. [Overview](#1-overview)
2. [PlatformAdmin Bootstrap](#2-platformadmin-bootstrap)
3. [CLI Profile Setup](#3-cli-profile-setup)
4. [Authentication](#4-authentication)
5. [Tenant Creation](#5-tenant-creation)
6. [User Management](#6-user-management)
7. [Role Assignment](#7-role-assignment)
8. [Organizational Hierarchy](#8-organizational-hierarchy)
9. [Tenant Configuration and Secrets](#9-tenant-configuration-and-secrets)
10. [Repository Binding](#10-repository-binding)
11. [Shared Git Provider Connections](#11-shared-git-provider-connections)
12. [Manifest-Based Provisioning](#12-manifest-based-provisioning)
13. [Permission Inspection](#13-permission-inspection)
14. [API Reference](#14-api-reference)
15. [Troubleshooting](#15-troubleshooting)

---

## 1. Overview

Provisioning an Aeterna instance follows this sequence:

```
Deploy Aeterna (Helm) ──► Bootstrap PlatformAdmin ──► Authenticate CLI
        │
        ▼
  Create Tenant ──► Register Users ──► Assign Roles ──► Build Hierarchy
        │
        ▼
  Configure Tenant ──► Set Secrets ──► Bind Repository ──► Done
```

| Step | Who | How |
|------|-----|-----|
| Deploy | Ops / Infra | Helm chart |
| Bootstrap PlatformAdmin | Ops / Infra | Environment variables at startup |
| Authenticate | PlatformAdmin | CLI device-code flow |
| Create tenants | PlatformAdmin | CLI or API |
| Register users | PlatformAdmin / TenantAdmin | CLI or API |
| Assign roles | PlatformAdmin / TenantAdmin | CLI or API |
| Build hierarchy | PlatformAdmin / TenantAdmin | CLI or API |
| Configure tenant | PlatformAdmin / TenantAdmin | CLI or API |

---

## 2. PlatformAdmin Bootstrap

The first PlatformAdmin is created automatically at server startup via environment variables. No manual database intervention is needed.

### Required Environment Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `AETERNA_ADMIN_BOOTSTRAP_EMAIL` | Admin user's email | `admin@acme-corp.com` |
| `AETERNA_ADMIN_BOOTSTRAP_PROVIDER` | Identity provider type (default: `github`) | `github` |
| `AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT` | Provider-specific subject (e.g. GitHub login) | `admin-user` |

### Helm Values

In your `values.yaml`:

```yaml
server:
  env:
    AETERNA_ADMIN_BOOTSTRAP_EMAIL: "admin@acme-corp.com"
    AETERNA_ADMIN_BOOTSTRAP_PROVIDER: "github"
    AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT: "admin-user"
```

### What Happens at Startup

When the server starts with valid bootstrap configuration:

1. **Organizational unit** — creates the instance-scope organizational unit (`__root__`)
2. **Tenant** — creates the root tenant record
3. **Organization** — creates a `platform` organization under the tenant
4. **Team** — creates an `admins` team within the platform organization
5. **User** — upserts a user row with the configured email, provider, and subject
6. **Membership** — adds the user to the `admins` team with `admin` role
7. **Role grant** — grants `PlatformAdmin` role at instance scope (`__root__`)

All operations are **idempotent** — restarting with the same configuration is safe and creates no duplicates.

### Verification

```bash
# Check server health
aeterna admin health

# After authenticating (see next sections), verify your identity
aeterna user whoami
```

### Bootstrap Disabled

If `AETERNA_ADMIN_BOOTSTRAP_EMAIL` is not set, the server skips bootstrap entirely and starts normally. If partial configuration is provided (e.g. email set but no provider subject), the server logs a warning and skips bootstrap.

---

## 3. CLI Profile Setup

Before authenticating, create a CLI profile that points to your Aeterna instance.

### Create a Profile

```bash
aeterna profile add \
  --name production \
  --server-url https://aeterna.example.com \
  --auth-method device-code \
  --github-client-id <GITHUB_APP_CLIENT_ID>
```

| Flag | Description |
|------|-------------|
| `--name` | Local profile name (you pick this) |
| `--server-url` | URL of your Aeterna server |
| `--auth-method` | Authentication method (`device-code`, `pat`, `api-key`) |
| `--github-client-id` | GitHub App client ID for device-code flow |

### Profile Management

```bash
# List all profiles
aeterna profile list

# Set default profile
aeterna profile default production

# Update a profile
aeterna profile update production --server-url https://new-url.example.com

# Remove a profile
aeterna profile remove old-profile
```

### Environment Variable Override

The GitHub client ID can also be set via environment variable:

```bash
export AETERNA_GITHUB_CLIENT_ID=your_github_app_client_id
```

The environment variable takes precedence over the profile configuration.

---

## 4. Authentication

### GitHub Device-Code Flow (Interactive)

The recommended method for interactive users:

```bash
aeterna profile login
```

This will:

1. Request a device code from GitHub
2. Display a verification URL and user code in your terminal
3. Open your browser (or prompt you to navigate to `https://github.com/login/device`)
4. Poll for authorization completion
5. Exchange the GitHub token with the Aeterna server for Aeterna-issued credentials
6. Store credentials in your local credential store

**Example output:**

```
To authenticate, visit: https://github.com/login/device
Enter code: ABCD-1234
Waiting for authorization...
✓ Authenticated as admin-user (admin@acme-corp.com)
```

### PAT Fallback

If device-code flow is not available, use a GitHub Personal Access Token:

```bash
aeterna auth login --github-token ghp_xxxxxxxxxxxx
```

### Token Refresh

The CLI automatically refreshes expired tokens when a valid refresh token is available. If the refresh token itself has expired, you'll be prompted to re-authenticate:

```bash
aeterna auth login
```

### Verify Authentication

```bash
# Check auth status
aeterna auth status

# Show current identity
aeterna user whoami
```

---

## 5. Tenant Creation

Only PlatformAdmin can create tenants.

### CLI

```bash
aeterna tenant create --slug acme --name "Acme Corp"
```

| Flag | Description |
|------|-------------|
| `--slug` | URL-friendly identifier (kebab-case, immutable) |
| `--name` | Human-readable display name |

### API

```bash
curl -X POST https://aeterna.example.com/api/v1/admin/tenants \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "slug": "acme",
    "name": "Acme Corp"
  }'
```

### Set Default Tenant

After creating a tenant, set it as your active context:

```bash
aeterna tenant use acme
```

### Tenant Lifecycle

```bash
# List all tenants
aeterna tenant list

# Show tenant details
aeterna tenant show acme

# Update tenant metadata
aeterna tenant update acme --name "Acme Corp (EU)"

# Add domain mapping
aeterna tenant domain-map acme --domain acme.example.com

# Deactivate (soft-delete)
aeterna tenant deactivate acme --yes
```

---

## 6. User Management

### Register a User

Register a user within the current tenant context:

```bash
aeterna user register \
  --email alice@acme-corp.com \
  --provider github \
  --provider-subject alice-github
```

### API

```bash
curl -X POST https://aeterna.example.com/api/v1/users \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "email": "alice@acme-corp.com",
    "idp_provider": "github",
    "idp_subject": "alice-github"
  }'
```

### Invite a User

Send an invitation email:

```bash
aeterna user invite --email bob@acme-corp.com
```

### List and Inspect Users

```bash
# List all users in the current tenant
aeterna user list

# Show user details
aeterna user show alice@acme-corp.com

# Show a user's roles
aeterna user roles alice@acme-corp.com

# Show your own identity
aeterna user whoami
```

---

## 7. Role Assignment

### Role Hierarchy

| Role | Precedence | Scope | Description |
|------|------------|-------|-------------|
| PlatformAdmin | 6 | Instance (`__root__`) | Cross-tenant administration |
| TenantAdmin | 5 | Single tenant | Tenant-scoped administration |
| Admin | 4 | Single tenant | Full tenant access |
| Architect | 3 | Single tenant | Design policies, manage knowledge |
| TechLead | 2 | Single tenant | Manage team resources |
| Developer | 1 | Single tenant | Standard development access |
| Agent | 0 | Delegated | AI agent with delegated permissions |

### Assign a Role

```bash
aeterna govern roles assign \
  --user alice@acme-corp.com \
  --role TenantAdmin \
  --tenant acme
```

### API

```bash
curl -X POST https://aeterna.example.com/api/v1/govern/roles/assign \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "user_email": "alice@acme-corp.com",
    "role": "TenantAdmin",
    "tenant_id": "acme"
  }'
```

### Revoke a Role

```bash
aeterna govern roles revoke \
  --user alice@acme-corp.com \
  --role TenantAdmin \
  --tenant acme
```

### List Role Assignments

```bash
# View roles for a specific user
aeterna user roles alice@acme-corp.com

# View the full role-permission matrix
aeterna permissions matrix
```

---

## 8. Organizational Hierarchy

Aeterna uses a four-level hierarchy: **Tenant → Organization → Team → Project**.

### Create an Organization

```bash
aeterna org create \
  --slug engineering \
  --name "Engineering"
```

### Create a Team

```bash
aeterna team create \
  --slug api-team \
  --name "API Team" \
  --org engineering
```

### Create a Project

```bash
aeterna project init \
  --slug payments-service \
  --name "Payments Service" \
  --team api-team
```

### Example: Full Hierarchy

```bash
# Create organizations
aeterna org create --slug platform-eng --name "Platform Engineering"
aeterna org create --slug product-eng --name "Product Engineering"

# Create teams under platform-eng
aeterna team create --slug api-team --name "API Team" --org platform-eng
aeterna team create --slug data-team --name "Data Platform" --org platform-eng

# Create teams under product-eng
aeterna team create --slug web-team --name "Web Team" --org product-eng
aeterna team create --slug mobile-team --name "Mobile Team" --org product-eng

# Create projects under teams
aeterna project init --slug auth-service --name "Auth Service" --team api-team
aeterna project init --slug payments-api --name "Payments API" --team api-team
aeterna project init --slug dashboard-ui --name "Dashboard UI" --team web-team
```

This produces:

```
Acme Corp (tenant)
├── Platform Engineering (org)
│   ├── API Team (team)
│   │   ├── Auth Service (project)
│   │   └── Payments API (project)
│   └── Data Platform (team)
└── Product Engineering (org)
    ├── Web Team (team)
    │   └── Dashboard UI (project)
    └── Mobile Team (team)
```

---

## 9. Tenant Configuration and Secrets

### Inspect Configuration

```bash
# View current tenant config
aeterna tenant config inspect

# Validate configuration against schema
aeterna tenant config validate
```

### Set Configuration Fields

```bash
aeterna tenant config upsert \
  --key default_embedding_model \
  --value text-embedding-3-small
```

### Manage Secrets

Secrets are write-only — they are never returned in API responses.

```bash
# Set a secret
aeterna tenant secret set \
  --key webhook_secret \
  --value "whsec_xxxxxxxxxxx"

# Delete a secret
aeterna tenant secret delete --key webhook_secret
```

### Ownership Model

| Owner | Who can modify | Examples |
|-------|---------------|----------|
| **tenant** | TenantAdmin, PlatformAdmin | Runtime settings, integration credentials |
| **platform** | PlatformAdmin only | Deployment fields, bootstrap secrets |

---

## 10. Repository Binding

Each tenant can bind to a source code repository for code-aware features.

```bash
# Show current binding
aeterna tenant repo-binding show

# Set repository binding
aeterna tenant repo-binding set \
  --provider github \
  --owner acme-corp \
  --repo main-monorepo

# Validate binding connectivity
aeterna tenant repo-binding validate
```

If a shared Git provider connection is configured (see next section), reference it:

```bash
aeterna tenant repo-binding set \
  --provider github \
  --owner acme-corp \
  --repo main-monorepo \
  --connection-id conn_abc123
```

---

## 11. Shared Git Provider Connections

PlatformAdmin can create shared GitHub App connections that multiple tenants reference, avoiding per-tenant PEM key distribution.

### Create a Connection

```bash
# Only PlatformAdmin can create shared connections
aeterna tenant connection create \
  --name "GitHub Enterprise" \
  --provider github \
  --app-id 12345 \
  --installation-id 67890 \
  --private-key-file /path/to/private-key.pem
```

### Grant/Revoke Tenant Access

```bash
# Grant a tenant access to a shared connection
aeterna tenant connection grant \
  --connection-id conn_abc123 \
  --tenant acme

# Revoke access
aeterna tenant connection revoke \
  --connection-id conn_abc123 \
  --tenant acme
```

### List Connections

```bash
aeterna tenant connection list
```

> **Security note:** Tenants see only redacted metadata and the connection ID — never raw PEM keys or webhook secrets.

---

## 12. Manifest-Based Provisioning

For automated or repeatable provisioning, use a YAML manifest to create and configure a tenant in one shot.

### Manifest Schema

```yaml
apiVersion: aeterna.io/v1
kind: TenantManifest

tenant:
  slug: acme
  name: "Acme Corp"

config:
  fields:
    default_embedding_model: "text-embedding-3-small"
    memory_ttl_days: "90"

secrets:
  webhook_secret: "whsec_xxxxxxxxxxx"
  api_key: "sk-xxxxxxxxxxxx"

repository:
  provider: github
  owner: acme-corp
  repo: main-monorepo
  git_provider_connection_id: conn_abc123  # optional

hierarchy:
  organizations:
    - slug: engineering
      name: "Engineering"
      teams:
        - slug: api-team
          name: "API Team"
          projects:
            - slug: auth-service
              name: "Auth Service"
            - slug: payments-api
              name: "Payments API"
        - slug: data-team
          name: "Data Platform"

roles:
  - user_email: alice@acme-corp.com
    role: TenantAdmin
  - user_email: bob@acme-corp.com
    role: Architect
    scope: engineering/api-team
```

### Apply the Manifest

**CLI:**

```bash
aeterna admin tenant provision --file manifest.yaml
```

**API:**

```bash
curl -X POST https://aeterna.example.com/api/v1/admin/tenants/provision \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/yaml" \
  --data-binary @manifest.yaml
```

### Idempotent Re-application

Manifests are idempotent — re-submitting the same manifest updates configuration without creating duplicates. This enables GitOps-style tenant management.

---

## 13. Permission Inspection

### Role-Permission Matrix

View the complete mapping of roles to permissions:

```bash
aeterna permissions matrix
```

### Effective Permissions

Check what a specific user can actually do:

```bash
aeterna permissions effective --user alice@acme-corp.com
```

### Cross-Tenant Inspection (PlatformAdmin)

PlatformAdmin can inspect permissions in any tenant:

```bash
aeterna permissions effective \
  --user alice@acme-corp.com \
  --target-tenant acme
```

---

## 14. API Reference

All CLI operations have REST API equivalents. The base URL is your Aeterna server address.

### Authentication

All API calls require a Bearer token:

```
Authorization: Bearer <aeterna-jwt>
```

### Endpoint Summary

| Operation | Method | Endpoint |
|-----------|--------|----------|
| **Tenants** | | |
| Create tenant | `POST` | `/api/v1/admin/tenants` |
| List tenants | `GET` | `/api/v1/admin/tenants` |
| Show tenant | `GET` | `/api/v1/admin/tenants/:slug` |
| Update tenant | `PATCH` | `/api/v1/admin/tenants/:slug` |
| Deactivate tenant | `POST` | `/api/v1/admin/tenants/:slug/deactivate` |
| Provision from manifest | `POST` | `/api/v1/admin/tenants/provision` |
| **Users** | | |
| Register user | `POST` | `/api/v1/users` |
| List users | `GET` | `/api/v1/users` |
| Show user | `GET` | `/api/v1/users/:id` |
| Invite user | `POST` | `/api/v1/users/invite` |
| **Roles** | | |
| Assign role | `POST` | `/api/v1/govern/roles/assign` |
| Revoke role | `POST` | `/api/v1/govern/roles/revoke` |
| **Hierarchy** | | |
| Create org | `POST` | `/api/v1/orgs` |
| Create team | `POST` | `/api/v1/teams` |
| Create project | `POST` | `/api/v1/projects` |
| **Config** | | |
| Inspect config | `GET` | `/api/v1/tenants/:slug/config` |
| Upsert config | `PUT` | `/api/v1/tenants/:slug/config` |
| Set secret | `PUT` | `/api/v1/tenants/:slug/secrets/:key` |
| Delete secret | `DELETE` | `/api/v1/tenants/:slug/secrets/:key` |
| **Repo Binding** | | |
| Show binding | `GET` | `/api/v1/tenants/:slug/repo-binding` |
| Set binding | `PUT` | `/api/v1/tenants/:slug/repo-binding` |
| Validate binding | `POST` | `/api/v1/tenants/:slug/repo-binding/validate` |
| **Permissions** | | |
| Permission matrix | `GET` | `/api/v1/permissions/matrix` |
| Effective permissions | `GET` | `/api/v1/permissions/effective` |
| **Connections** | | |
| List connections | `GET` | `/api/v1/admin/connections` |
| Create connection | `POST` | `/api/v1/admin/connections` |
| Grant tenant access | `POST` | `/api/v1/admin/connections/:id/grant` |
| Revoke tenant access | `POST` | `/api/v1/admin/connections/:id/revoke` |
| **Admin** | | |
| Health check | `GET` | `/api/v1/admin/health` |
| Validate config | `POST` | `/api/v1/admin/validate` |
| Run migrations | `POST` | `/api/v1/admin/migrate` |
| Drift detection | `GET` | `/api/v1/admin/drift` |
| Export data | `GET` | `/api/v1/admin/export` |
| Import data | `POST` | `/api/v1/admin/import` |
| Sync | `POST` | `/api/v1/admin/sync` |

---

## 15. Troubleshooting

### "403 Forbidden" on Tenant Operations

**Symptom:** PlatformAdmin user receives `403 Forbidden` when creating tenants or performing admin operations.

**Cause:** The server may be reading roles from the request header (`X-User-Role`) instead of the database. If no role header is present, the server defaults to `Developer` role.

**Fix:** Ensure the server is configured to resolve roles from the `user_roles` database table. See the `fix-role-resolution-from-db` change for the full fix.

**Workaround (temporary):** If using a reverse proxy, ensure it sets the `X-User-Role: PlatformAdmin` header for the bootstrapped admin user.

### "Bootstrap Skipped" in Server Logs

**Symptom:** Server starts but PlatformAdmin user is not created.

**Check:**
- `AETERNA_ADMIN_BOOTSTRAP_EMAIL` is set and non-empty
- `AETERNA_ADMIN_BOOTSTRAP_PROVIDER_SUBJECT` is set (otherwise bootstrap logs a warning and skips)
- Check server logs for `bootstrap` messages

### "Device Code Expired"

**Symptom:** CLI shows "device code expired" during login.

**Fix:** Re-run `aeterna profile login`. The GitHub device code is valid for a limited time window (typically 15 minutes).

### "No Tenant Context"

**Symptom:** CLI commands fail with "no tenant context".

**Fix:** Set your active tenant:

```bash
aeterna tenant use <tenant-slug>
```

### Connection Refused

**Symptom:** CLI cannot connect to the server.

**Check:**
- Server URL in your profile is correct: `aeterna profile list`
- Server is running: `curl https://aeterna.example.com/api/v1/admin/health`
- TLS certificates are valid (if using HTTPS)

---

## Quick Start Checklist

For a fresh deployment, run these commands in order:

```bash
# 1. Set bootstrap env vars in Helm values and deploy
#    (AETERNA_ADMIN_BOOTSTRAP_EMAIL, _PROVIDER, _PROVIDER_SUBJECT)

# 2. Create CLI profile
aeterna profile add \
  --name prod \
  --server-url https://aeterna.example.com \
  --auth-method device-code \
  --github-client-id <CLIENT_ID>

# 3. Authenticate
aeterna profile login

# 4. Verify identity
aeterna user whoami

# 5. Create first tenant
aeterna tenant create --slug acme --name "Acme Corp"

# 6. Set default tenant
aeterna tenant use acme

# 7. Register users
aeterna user register --email alice@acme-corp.com --provider github --provider-subject alice
aeterna user register --email bob@acme-corp.com --provider github --provider-subject bob

# 8. Assign roles
aeterna govern roles assign --user alice@acme-corp.com --role TenantAdmin --tenant acme
aeterna govern roles assign --user bob@acme-corp.com --role Architect --tenant acme

# 9. Build hierarchy
aeterna org create --slug engineering --name "Engineering"
aeterna team create --slug api-team --name "API Team" --org engineering
aeterna project init --slug payments-api --name "Payments API" --team api-team

# 10. Verify
aeterna permissions effective --user alice@acme-corp.com
```
