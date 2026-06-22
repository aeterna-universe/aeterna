# ===========================================================================
# Tiltfile — Aeterna local development on kind
# ===========================================================================
#
# One-command full-stack local dev. Installs required operators, deploys all
# backing services + the Aeterna server, with live reload on source changes.
#
# Prerequisites (one-time):
#   kind create cluster --config deploy/tilt/kind-cluster.yaml
#
# Then just:
#   tilt up
#
# What you get:
#   - CloudNativePG operator (auto-installed)
#   - DragonflyDB operator   (auto-installed)
#   - PostgreSQL via CloudNativePG (with managed.roles from PR #195)
#   - Dragonfly (Redis-compatible cache)
#   - Qdrant (vector store)
#   - OPAL server + Cedar agent (governance)
#   - Aeterna server (built from source, live-reloaded)
#   - http://localhost:8080  — Aeterna API + Admin UI
#   - http://localhost:9090  — Prometheus metrics
#   - http://localhost:10350 — Tilt UI
#
# Press 'space' in the Tilt UI to open the browser, 'q' or Ctrl-C to stop.
# ===========================================================================

# --- Config --------------------------------------------------------------

local_registry = 'localhost:5001'
namespace      = 'aeterna'
release_prereq = 'aeterna-prereqs'
release_main   = 'aeterna'

# Pinned operator versions (bump here when upgrading).
CNPG_VERSION        = '1.25.1'
CNPG_MANIFEST_URL   = 'https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.25/releases/cnpg-%s.yaml' % CNPG_VERSION
DRAGONFLY_MANIFEST_URL = 'https://raw.githubusercontent.com/dragonflydb/dragonfly-operator/main/manifests/dragonfly-operator.yaml'

# ===========================================================================
# Operator bootstrap (runs before any chart deploy)
# ===========================================================================
#
# Idempotently installs the CloudNativePG and DragonflyDB operators.
# The CNPG `poolers` CRD exceeds Kubernetes' 262KB annotation limit, so it
# requires server-side apply. We handle that with a targeted retry.
# This resource blocks all downstream k8s deploys via resource_deps.

bootstrap_cmd = '\n'.join([
    'set -euo pipefail',
    'echo "=== Operator bootstrap ==="',

    # --- CloudNativePG ---
    'if ! kubectl get crd clusters.postgresql.cnpg.io >/dev/null 2>&1; then',
    '  echo "[CNPG] Installing CloudNativePG %s..."' % CNPG_VERSION,
    '  curl -fsSL "%s" -o /tmp/cnpg-operator.yaml' % CNPG_MANIFEST_URL,
    '  kubectl apply -f /tmp/cnpg-operator.yaml 2>&1 || true',
    '  # The poolers CRD is too large for client-side apply (>262KB).',
    '  kubectl apply --server-side --force-conflicts -f /tmp/cnpg-operator.yaml 2>&1 || true',
    '  kubectl wait --for=condition=Available deployment/cnpg-controller-manager -n cnpg-system --timeout=180s',
    '  echo "[CNPG] Operator ready"',
    'else',
    '  echo "[CNPG] Already installed"',
    'fi',

    # --- DragonflyDB ---
    'if ! kubectl get crd dragonflies.dragonflydb.io >/dev/null 2>&1; then',
    '  echo "[Dragonfly] Installing DragonflyDB operator..."',
    '  curl -fsSL "%s" -o /tmp/dragonfly-operator.yaml' % DRAGONFLY_MANIFEST_URL,
    '  kubectl apply -f /tmp/dragonfly-operator.yaml',
    '  kubectl rollout status deployment/dragonfly-operator-controller-manager -n dragonfly-operator-system --timeout=120s',
    '  echo "[Dragonfly] Operator ready"',
    'else',
    '  echo "[Dragonfly] Already installed"',
    'fi',

    # --- Namespace ---
    'kubectl get namespace %s >/dev/null 2>&1 || kubectl create namespace %s' % (namespace, namespace),

    'echo "=== Bootstrap complete ==="',
])

local_resource(
    'bootstrap-operators',
    ['bash', '-c', bootstrap_cmd],
    trigger_mode=TRIGGER_MODE_MANUAL,
)

# ===========================================================================
# Backing services — aeterna-prereqs chart
# ===========================================================================

prereqs_values = [
    'postgresql.instances=1',
    'postgresql.storage.size=2Gi',
    'postgresql.resources.requests.cpu=100m',
    'postgresql.resources.requests.memory=256Mi',
    'postgresql.resources.limits.cpu=1000m',
    'postgresql.resources.limits.memory=1Gi',
]

k8s_yaml(helm(
    './charts/aeterna-prereqs',
    name=release_prereq,
    namespace=namespace,
    values=['./charts/aeterna-prereqs/values.yaml'],
    set=prereqs_values,
))

# The Dragonfly operator creates child resources (Service, StatefulSet) from
# the Dragonfly CR that Tilt would otherwise try to reconcile and conflict
# with. Tell Tilt to only track resources we explicitly select, ignoring
# operator-managed child objects.
k8s_resource(
    workload='aeterna-prereqs-dragonfly',
    discovery_strategy='selectors-only',
)

# ===========================================================================
# Build the Aeterna image (multi-stage: admin-ui + Rust + runtime)
# ===========================================================================

docker_build(
    'aeterna/server',
    context='.',
    dockerfile='Dockerfile',
    build_args={
        'BUILD_DATE': str(local('date -u +%Y-%m-%dT%H:%M:%SZ', quiet=True)).strip(),
        'VCS_REF':    str(local('git rev-parse --short HEAD', quiet=True)).strip(),
    },
    live_update=[
        # Sync source dirs; Tilt rebuilds the binary inside the container.
        sync('./cli/src',     '/app/cli/src'),
        sync('./storage/src', '/app/storage/src'),
        sync('./memory/src',  '/app/memory/src'),
    ],
)

# ===========================================================================
# Aeterna server — main chart
# ===========================================================================

main_values = [
    'aeterna.image.repository=aeterna/server',
    'aeterna.image.tag=tilt',
    'aeterna.image.pullPolicy=Always',

    # Connect to prereqs services via in-cluster DNS
    'postgresql.host=aeterna-prereqs-cnpg-rw',
    'postgresql.port=5432',
    'postgresql.database=aeterna',
    'postgresql.username=aeterna',

    'redis.host=aeterna-prereqs-dragonfly',
    'redis.port=6379',

    'vectorStore.host=aeterna-prereqs-qdrant',
    'vectorStore.port=6333',

    # Local dev: disable external integrations that need secrets
    'pluginAuth.enabled=false',
    'githubOrgSync.enabled=false',
    'knowledgeRepo.enabled=false',
    'codesearch.enabled=false',
    'adminUi.enabled=true',
    'adminUi.path=/app/admin-ui/dist',

    # Local KMS (no cloud KMS needed for dev)
    'kms.provider=local',
    'kms.local.generate=true',

    # Lighter resources for local dev
    'aeterna.resources.requests.cpu=100m',
    'aeterna.resources.requests.memory=256Mi',
    'aeterna.resources.limits.cpu=2000m',
    'aeterna.resources.limits.memory=2Gi',

    # Verbose logging for dev
    'observability.logging.level=debug',
    'observability.logging.format=pretty',
]

k8s_yaml(helm(
    './charts/aeterna',
    name=release_main,
    namespace=namespace,
    set=main_values,
))

# ===========================================================================
# Port forwards + dev UX
# ===========================================================================

k8s_resource(
    workload='aeterna',
    port_forwards=['8080:8080', '9090:9090'],
    extra_pod_selectors=[{'app.kubernetes.io/component': 'aeterna'}],
)

# Show URLs in the Tilt UI once the server is up.
dev_urls_msg = (
    'echo "Aeterna local dev is ready:" && '
    + 'echo "  API:      http://localhost:8080" && '
    + 'echo "  Metrics:  http://localhost:9090" && '
    + 'echo "  Health:   http://localhost:8080/health" && '
    + 'echo "  Tilt UI:  http://localhost:10350"'
)

local_resource(
    'dev-urls',
    ['bash', '-c', dev_urls_msg],
    allow_parallel=True,
    resource_deps=['aeterna'],
)
