# Aeterna Deployment

This directory contains environment-specific deployment configuration for Aeterna.

## Structure

```
deploy/
├── environments/
│   ├── dev/
│   │   ├── values.yaml              # Dev values (local kind / CI)
│   │   ├── prereqs-values.yaml      # Dev prereqs (small Postgres/Redis/Qdrant)
│   │   └── tenant-config/           # Tenant manifests (auto-generated)
│   └── staging/
│       └── values.yaml              # Staging values (production-like)
├── scripts/
│   ├── deploy-prereqs.sh            # Deploy just the backing services
│   ├── deploy-fresh.sh              # Full deploy (prereqs + app + tenants)
│   └── provision-tenant.sh          # Generate tenant manifest
└── tilt/
    ├── kind-cluster.yaml            # kind cluster config
    └── create-dev-secrets.sh        # Create K8s secrets from PEM files
```

## Quick start

### Local dev (kind + Tilt)
```bash
kind create cluster --config deploy/tilt/kind-cluster.yaml
tilt up
```

### Local dev (kind + Helm, no Tilt)
```bash
kind create cluster --config deploy/tilt/kind-cluster.yaml
./deploy/tilt/create-dev-secrets.sh
./deploy/scripts/deploy-fresh.sh dev
```

### Remote environment (staging/prod)
```bash
# 1. Create required secrets (via external-secrets, SOPS, or kubectl)
kubectl -n aeterna create secret generic aeterna-knowledge-pem \
  --from-file=pem-key=/path/to/aeterna-knowledge.pem

kubectl -n aeterna create secret generic aeterna-plugin-auth \
  --from-literal=jwt-secret="$(openssl rand -hex 32)" \
  --from-literal=github-client-secret="your-oauth-secret"

# 2. Deploy
./deploy/scripts/deploy-fresh.sh staging 0.8.0-rc.12
```

## How policies work (NO inline YAML needed)

Cedar policies live in the `aeterna-universe/policies` repo. The OPAL fetcher
clones them at runtime — you never need to inline 2000+ lines of Cedar in
values.yaml. The `opal.server.policyRepoUrl` value points at the repo.

To modify policies: edit files in `aeterna-universe/policies`, merge via PR.
The OPAL fetcher picks up changes automatically.

## How knowledge sync works

The knowledge repo (`aeterna-universe/knowledge`) is the shared store.
The `aeterna-knowledge` GitHub App (App ID 4119573) provides read/write access.
Changes go through PR-based governance (propose → approve → merge).

## Adding a new tenant

```bash
./deploy/scripts/provision-tenant.sh dev my-team "My Team"
# Edit deploy/environments/dev/tenant-config/my-team.yaml
# Run deploy-fresh.sh again to provision
```
