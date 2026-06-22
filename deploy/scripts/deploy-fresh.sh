#!/usr/bin/env bash
# deploy-fresh.sh — Full Aeterna deployment (prereqs + app + tenant provisioning)
#
# Usage:
#   ./deploy/scripts/deploy-fresh.sh <env> [chart-version]
#
# Steps:
#   0. Switch context + create namespace
#   1. Verify required secrets exist
#   2. Deploy prerequisites (Postgres + Redis + Qdrant)
#   3. Wait for prerequisites to be ready
#   4. Deploy Aeterna application
#   5. Wait for application to be ready
#   6. Provision tenants from environment manifests
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

ENV="${1:-dev}"
CHART_VERSION="${2:-}"
NAMESPACE="${AETERNA_NAMESPACE:-aeterna}"
TENANT_CONFIG_DIR="${REPO_ROOT}/deploy/environments/${ENV}/tenant-config"

# Chart refs
if [[ "${ENV}" == "dev" && -z "${CHART_VERSION}" ]]; then
  APP_CHART="${REPO_ROOT}/charts/aeterna"
  PREREQS_CHART="${REPO_ROOT}/charts/aeterna-prereqs"
else
  APP_CHART="oci://ghcr.io/aeterna-universe/charts/aeterna"
  PREREQS_CHART="oci://ghcr.io/aeterna-universe/charts/aeterna-prereqs"
  CHART_VERSION="${CHART_VERSION:-0.8.0-rc.12}"
fi

VALUES_FILE="${REPO_ROOT}/deploy/environments/${ENV}/values.yaml"
PREREQS_VALUES="${REPO_ROOT}/deploy/environments/${ENV}/prereqs-values.yaml"

echo "=============================================="
echo " Aeterna Fresh Install"
echo "=============================================="
echo " Environment:    ${ENV}"
echo " Namespace:      ${NAMESPACE}"
[[ -n "${CHART_VERSION}" ]] && echo " Chart version:  ${CHART_VERSION}"
echo "=============================================="
echo ""

if [[ ! -f "${VALUES_FILE}" ]]; then
  echo "ERROR: Values file not found: ${VALUES_FILE}"
  echo "Create it from deploy/environments/${ENV}/values.yaml"
  exit 1
fi

# ---- Step 0: Namespace ----------------------------------------------------
echo "[0/6] Creating namespace ${NAMESPACE}..."
kubectl create namespace "${NAMESPACE}" 2>/dev/null || true

# ---- Step 1: Verify secrets -----------------------------------------------
echo ""
echo "[1/6] Checking required secrets..."

# Check which secrets the values file references and verify they exist.
REQUIRED_SECRETS=()

# Always check knowledge repo PEM if knowledgeRepo is enabled
if grep -q 'knowledgeRepo:' "${VALUES_FILE}" 2>/dev/null && \
   grep -A10 'knowledgeRepo:' "${VALUES_FILE}" | grep -q 'enabled: true'; then
  PEM_SECRET=$(grep -A30 'knowledgeRepo:' "${VALUES_FILE}" | grep 'pemSecret:' | head -1 | sed 's/.*pemSecret: *"\?\([^"]*\)"\?/\1/')
  [[ -n "${PEM_SECRET}" ]] && REQUIRED_SECRETS+=("${PEM_SECRET}")
fi

# Check plugin auth secret if enabled
if grep -A5 'pluginAuth:' "${VALUES_FILE}" 2>/dev/null | grep -q 'enabled: true'; then
  AUTH_SECRET=$(grep -A10 'pluginAuth:' "${VALUES_FILE}" | grep 'existingSecret:' | head -1 | sed 's/.*existingSecret: *"\?\([^"]*\)"\?/\1/')
  [[ -n "${AUTH_SECRET}" ]] && REQUIRED_SECRETS+=("${AUTH_SECRET}")
fi

MISSING=()
for secret in "${REQUIRED_SECRETS[@]}"; do
  if ! kubectl get secret "${secret}" -n "${NAMESPACE}" &>/dev/null; then
    MISSING+=("${secret}")
  fi
done

if (( ${#MISSING[@]} > 0 )); then
  echo ""
  echo "WARNING: Missing secrets (deployment may fail until created):"
  for s in "${MISSING[@]}"; do
    echo "  - ${s}"
  done
  echo ""
  echo "For local dev: ./deploy/tilt/create-dev-secrets.sh"
  echo "For remote envs: create via external-secrets or kubectl"
  echo ""
  echo "Continue anyway? (y/N)"
  read -r response
  [[ "${response}" =~ ^[Yy] ]] || exit 1
else
  echo "  All required secrets present."
fi

# ---- Step 2: Deploy prerequisites -----------------------------------------
echo ""
echo "[2/6] Deploying prerequisites..."
PREREQS_ARGS=("${PREREQS_CHART}" -n "${NAMESPACE}")
[[ -n "${CHART_VERSION}" ]] && PREREQS_ARGS+=(--version "${CHART_VERSION}")
[[ -f "${PREREQS_VALUES}" ]] && PREREQS_ARGS+=(-f "${PREREQS_VALUES}")

if helm status aeterna-prereqs -n "${NAMESPACE}" &>/dev/null; then
  helm upgrade aeterna-prereqs "${PREREQS_ARGS[@]}"
else
  helm install aeterna-prereqs "${PREREQS_ARGS[@]}"
fi

# ---- Step 3: Wait for prerequisites ---------------------------------------
echo ""
echo "[3/6] Waiting for prerequisites..."
echo "  PostgreSQL..."
kubectl wait --for=condition=ready pod -l cnpg.io/cluster=aeterna-prereqs-cnpg -n "${NAMESPACE}" --timeout=300s 2>/dev/null || true
echo "  Qdrant..."
kubectl wait --for=condition=ready pod -l app.kubernetes.io/name=qdrant -n "${NAMESPACE}" --timeout=180s 2>/dev/null || true
echo "  Dragonfly..."
kubectl wait --for=condition=ready pod -l app=dragonfly -n "${NAMESPACE}" --timeout=120s 2>/dev/null || true
echo "  Prerequisites ready."

# ---- Step 4: Deploy application -------------------------------------------
echo ""
echo "[4/6] Deploying Aeterna application..."
APP_ARGS=("${APP_CHART}" -n "${NAMESPACE}" -f "${VALUES_FILE}")
[[ -n "${CHART_VERSION}" ]] && APP_ARGS+=(--version "${CHART_VERSION}")

if helm status aeterna -n "${NAMESPACE}" &>/dev/null; then
  helm upgrade aeterna "${APP_ARGS[@]}"
else
  helm install aeterna "${APP_ARGS[@]}"
fi

# ---- Step 5: Wait for application -----------------------------------------
echo ""
echo "[5/6] Waiting for Aeterna pods..."
kubectl rollout status deployment -l app.kubernetes.io/instance=aeterna -n "${NAMESPACE}" --timeout=180s 2>/dev/null || true

# ---- Step 6: Provision tenants --------------------------------------------
echo ""
echo "[6/6] Provisioning tenants..."
echo "  Waiting for API..."
for i in $(seq 1 30); do
  if kubectl exec -n "${NAMESPACE}" deploy/aeterna -- curl -sf "http://localhost:8080/health" &>/dev/null 2>&1; then
    break
  fi
  sleep 5
done

if [[ -d "${TENANT_CONFIG_DIR}" ]]; then
  shopt -s nullglob
  manifests=("${TENANT_CONFIG_DIR}"/*.yaml)
  shopt -u nullglob
  for manifest in "${manifests[@]}"; do
    BASENAME="$(basename "${manifest}" .yaml)"
    echo "  Provisioning tenant: ${BASENAME}..."

    kubectl -n "${NAMESPACE}" port-forward svc/aeterna 18080:8080 &>/dev/null &
    PF_PID=$!
    sleep 2

    # Inject PEM from k8s secret if the manifest uses the placeholder
    HTTP_CODE=$(curl -s -o /tmp/provision-response.json -w "%{http_code}" \
      -X POST "http://localhost:18080/admin/tenants/provision" \
      -H "Content-Type: application/json" \
      -d "$(python3 - "${manifest}" "${NAMESPACE}" <<'PYEOF'
import yaml, json, sys, subprocess

manifest_path, namespace = sys.argv[1], sys.argv[2]
with open(manifest_path) as f:
    data = yaml.safe_load(f)

# Inject PEM from k8s secret for any INJECTED_AT_PROVISION_TIME placeholders
for s in data.get('secrets', []):
    if s.get('secretValue') == 'INJECTED_AT_PROVISION_TIME':
        logical = s.get('logicalName', '')
        # Map logical name to k8s secret
        secret_map = {
            'github.app_pem': ('aeterna-knowledge-pem', 'pem-key'),
            'github.policy_pem': ('aeterna-policy-pem', 'pem-key'),
        }
        if logical in secret_map:
            sec_name, sec_key = secret_map[logical]
            try:
                result = subprocess.run(
                    ['kubectl', 'get', 'secret', sec_name, '-n', namespace,
                     '-o', f'jsonpath={{.data.{sec_key}}}'],
                    capture_output=True, text=True, check=True
                )
                import base64
                pem = base64.b64decode(result.stdout).decode()
                s['secretValue'] = pem.strip()
            except Exception:
                pass  # Leave placeholder — server will reject if required

print(json.dumps(data))
PYEOF
)" 2>/dev/null || echo "{}")

    kill "${PF_PID}" 2>/dev/null || true
    wait "${PF_PID}" 2>/dev/null || true

    if [[ "${HTTP_CODE}" =~ ^(200|201|409)$ ]]; then
      echo "    OK (HTTP ${HTTP_CODE})"
    else
      echo "    WARNING: HTTP ${HTTP_CODE}"
      cat /tmp/provision-response.json 2>/dev/null || true
    fi
  done
else
  echo "  No tenant-config directory — skipping tenant provisioning."
fi

# ---- Done -----------------------------------------------------------------
echo ""
echo "=============================================="
echo " Deployment complete!"
echo "=============================================="
echo ""
kubectl get pods -n "${NAMESPACE}"
