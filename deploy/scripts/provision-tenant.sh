#!/usr/bin/env bash
# provision-tenant.sh — Generate a tenant manifest and optionally create via API
#
# Usage:
#   ./deploy/scripts/provision-tenant.sh <env> <slug> <name> [tenant-id]
#
# If tenant-id is omitted, creates the tenant via the Aeterna API (requires
# AETERNA_SERVER_URL + AETERNA_ACCESS_TOKEN). If provided, only generates the
# manifest file.
#
# Example:
#   ./deploy/scripts/provision-tenant.sh dev my-team "My Team"
#   ./deploy/scripts/provision-tenant.sh staging acme "Acme Corp" "$(uuidgen)"
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

ENV_NAME="${1:?usage: provision-tenant.sh <env> <slug> <name> [tenant-id]}"
TENANT_SLUG="${2:?usage: provision-tenant.sh <env> <slug> <name> [tenant-id]}"
TENANT_NAME="${3:?usage: provision-tenant.sh <env> <slug> <name> [tenant-id]}"
TENANT_ID="${4:-}"

OUTPUT_DIR="${REPO_ROOT}/deploy/environments/${ENV_NAME}/tenant-config"
OUTPUT_FILE="${OUTPUT_DIR}/${TENANT_SLUG}.yaml"

mkdir -p "${OUTPUT_DIR}"

# If no tenant ID, create via API
if [[ -z "${TENANT_ID}" ]]; then
  : "${AETERNA_SERVER_URL:?AETERNA_SERVER_URL is required when tenant-id is not provided}"
  : "${AETERNA_ACCESS_TOKEN:?AETERNA_ACCESS_TOKEN is required when tenant-id is not provided}"

  response="$({
    curl -fsSL \
      -H "Authorization: Bearer ${AETERNA_ACCESS_TOKEN}" \
      -H "Content-Type: application/json" \
      -X POST \
      -d "{\"slug\":\"${TENANT_SLUG}\",\"name\":\"${TENANT_NAME}\"}" \
      "${AETERNA_SERVER_URL%/}/api/v1/admin/tenants"
  } 2>/dev/null || true)"

  TENANT_ID="$(printf '%s' "${response}" | jq -r '.tenant.id // empty' 2>/dev/null || true)"
  if [[ -z "${TENANT_ID}" ]]; then
    echo "ERROR: tenant creation response did not contain tenant.id"
    echo "Response: ${response}"
    exit 1
  fi
fi

# Generate the tenant manifest
cat > "${OUTPUT_FILE}" <<EOF
apiVersion: aeterna.io/v1
kind: TenantManifest

tenant:
  slug: "${TENANT_SLUG}"
  name: "${TENANT_NAME}"

config:
  fields: {}

secrets: []
# Uncomment to wire the knowledge GitHub App per-tenant:
#   - logicalName: "github.app_pem"
#     ownership: platform
#     secretValue: "INJECTED_AT_PROVISION_TIME"

repository:
  kind: GitHubApp
  remoteUrl: "https://github.com/aeterna-universe/knowledge.git"
  branch: "main"
  branchPolicy: RequirePullRequest
  credentialKind: GitHubApp
  githubOwner: "aeterna-universe"
  githubRepo: "knowledge"
  gitProviderConnectionId: "aeterna-knowledge-app"

hierarchy: []

roles: []
# Example:
#   - userId: "your-github-username"
#     role: "PlatformAdmin"
EOF

echo "Tenant ID:    ${TENANT_ID}"
echo "Manifest:     ${OUTPUT_FILE}"
echo "Edit the manifest, then run deploy-fresh.sh to provision."
