# ===========================================================================
# Tiltfile — Aeterna local development on kind
# ===========================================================================
#
# Brings up the full Aeterna stack on a local kind cluster with live reload.
#
# Prerequisites (one-time):
#
#   kind create cluster --config deploy/tilt/kind-cluster.yaml
#   docker network connect kind kind-registry   # if using a local registry
#   kubectl config use-context kind-aeterna-dev
#
# Then:
#
#   tilt up
#
# What you get:
#   - Aeterna server (built from source, live-reloaded on Rust/admin-ui edits)
#   - PostgreSQL via CloudNativePG (with the managed.roles from PR #195)
#   - Dragonfly (Redis-compatible cache)
#   - Qdrant (vector store)
#   - http://localhost:8080  — Aeterna API + Admin UI
#   - http://localhost:9090  — Prometheus metrics
#
# Press 'q' in the Tilt UI (or Ctrl-C) to tear everything down.
# ===========================================================================

# --- Config --------------------------------------------------------------

local_registry = 'localhost:5001'
namespace      = 'aeterna'
release_prereq = 'aeterna-prereqs'
release_main   = 'aeterna'

# --- Cluster preflight ---------------------------------------------------

# Fail fast if the kind cluster / kubectl context isn't set up.
preflight_cmd = '\n'.join([
    'set -euo pipefail',
    'kubectl config current-context | grep -q kind-aeterna-dev || '
        + '{ echo "ERROR: kubeconfig context must be kind-aeterna-dev"; exit 1; }',
    'kubectl get namespace %s >/dev/null 2>&1 || kubectl create namespace %s' % (namespace, namespace),
    'kubectl get crd clusters.postgresql.cnpg.io >/dev/null 2>&1 || {'
        + '  echo "ERROR: CloudNativePG CRDs missing. Install with:";'
        + '  echo "  kubectl apply -f https://raw.githubusercontent.com/cloudnative-pg/cloudnative-pg/release-1.25/releases/cnpg-1.25.1.yaml";'
        + '  echo "  (poolers CRD needs: kubectl apply --server-side --force-conflicts -f <same-file>)";'
        + '  exit 1;'
        + '}',
    'echo "preflight OK"',
])

local_resource('cluster-preflight', ['bash', '-c', preflight_cmd], allow_parallel=True)

# --- Namespace (created synchronously before any chart deploy) -----------

k8s_yaml(blob('apiVersion: v1\nkind: Namespace\nmetadata:\n  name: %s\n' % namespace))

# --- Prerequisites chart (Postgres/CNPG + Dragonfly + Qdrant) ------------
#
# Installs the backing services. The managed.roles block in this chart's
# values.yaml provisions aeterna_app + aeterna_admin (issue #194).

prereqs_values = [
    # Keep the cluster small for local dev
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

# --- Build the Aeterna image ---------------------------------------------
#
# docker_build builds the multi-stage Dockerfile (admin-ui + Rust + runtime)
# and pushes to the local registry that kind's containerd is configured to use.

docker_build(
    'aeterna/server',
    context='.',
    dockerfile='Dockerfile',
    build_args={
        'BUILD_DATE': str(local('date -u +%Y-%m-%dT%H:%M:%SZ', quiet=True)).strip(),
        'VCS_REF':    str(local('git rev-parse --short HEAD', quiet=True)).strip(),
    },
    live_update=[
        # NOTE: full Rust recompile on every edit is slow. For true live update
        # we'd sync only changed source and recompile inside the container.
        # The default rebuild on save is still a big DX win over full CI cycles.
        sync('./cli/src',     '/app/cli/src'),
        sync('./storage/src', '/app/storage/src'),
        sync('./memory/src',  '/app/memory/src'),
    ],
)

# --- Main Aeterna chart --------------------------------------------------
#
# Points the app at the prereqs services. Uses the locally-built image.

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

# --- Port forwards -------------------------------------------------------

k8s_resource(
    workload='aeterna',
    port_forwards=['8080:8080', '9090:9090'],
    extra_pod_selectors=[{'app.kubernetes.io/component': 'aeterna'}],
)

# --- Convenience: show URLs in the Tilt UI on startup --------------------

dev_urls_msg = (
    'echo "Aeterna local dev is ready:" && '
    + 'echo "  API:      http://localhost:8080" && '
    + 'echo "  Metrics:  http://localhost:9090" && '
    + 'echo "  Health:   http://localhost:8080/health" && '
    + 'echo "  Tilt UI:  http://localhost:10350"'
)

local_resource('dev-urls', ['bash', '-c', dev_urls_msg], allow_parallel=True, resource_deps=[release_main])
