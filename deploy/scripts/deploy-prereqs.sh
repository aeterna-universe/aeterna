#!/usr/bin/env bash
# deploy-prereqs.sh — Deploy Aeterna prerequisites (Postgres + Redis + Qdrant)
#
# Usage:
#   ./deploy/scripts/deploy-prereqs.sh <env> [chart-version]
#
# Examples:
#   ./deploy/scripts/deploy-prereqs.sh dev
#   ./deploy/scripts/deploy-prereqs.sh staging 0.8.0-rc.12
#
# Reads overlay from deploy/environments/<env>/prereqs-values.yaml if present.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

ENV="${1:-dev}"
CHART_VERSION="${2:-}"
NAMESPACE="${AETERNA_NAMESPACE:-aeterna}"

# Use local chart source for dev, OCI for remote envs
if [[ "${ENV}" == "dev" && -z "${CHART_VERSION}" ]]; then
  CHART_REF="${REPO_ROOT}/charts/aeterna-prereqs"
  echo "Using local chart source: ${CHART_REF}"
else
  CHART_REF="oci://ghcr.io/aeterna-universe/charts/aeterna-prereqs"
  CHART_VERSION="${CHART_VERSION:-0.8.0-rc.12}"
  echo "Using OCI chart: ${CHART_REF}:${CHART_VERSION}"
fi

echo "=== Aeterna Prerequisites Deploy ==="
echo "Environment: ${ENV}"
echo "Chart:       ${CHART_REF}${CHART_VERSION:+:$CHART_VERSION}"
echo "Namespace:   ${NAMESPACE}"
echo ""

PREREQS_VALUES="${REPO_ROOT}/deploy/environments/${ENV}/prereqs-values.yaml"
if [[ ! -f "${PREREQS_VALUES}" ]]; then
  echo "No prereqs-values.yaml at ${PREREQS_VALUES}, using chart defaults."
  PREREQS_VALUES=""
fi

# Ensure namespace
kubectl create namespace "${NAMESPACE}" 2>/dev/null || true

# Build helm args
HELM_ARGS=("${CHART_REF}" -n "${NAMESPACE}")
if [[ -n "${CHART_VERSION}" ]]; then
  HELM_ARGS+=(--version "${CHART_VERSION}")
fi
if [[ -n "${PREREQS_VALUES}" ]]; then
  HELM_ARGS+=(-f "${PREREQS_VALUES}")
fi

# Deploy
if helm status aeterna-prereqs -n "${NAMESPACE}" &>/dev/null; then
  echo "Upgrading existing prereqs release..."
  helm upgrade aeterna-prereqs "${HELM_ARGS[@]}"
else
  echo "Installing prereqs..."
  helm install aeterna-prereqs "${HELM_ARGS[@]}"
fi

echo ""
echo "=== Waiting for pods ==="
echo "PostgreSQL..."
kubectl wait --for=condition=ready pod -l cnpg.io/cluster=aeterna-prereqs-cnpg -n "${NAMESPACE}" --timeout=300s 2>/dev/null || true
echo "Qdrant..."
kubectl wait --for=condition=ready pod -l app.kubernetes.io/name=qdrant -n "${NAMESPACE}" --timeout=180s 2>/dev/null || true
echo "Dragonfly..."
kubectl wait --for=condition=ready pod -l app=dragonfly -n "${NAMESPACE}" --timeout=120s 2>/dev/null || true

echo ""
echo "=== Prerequisites deployed ==="
kubectl get pods -n "${NAMESPACE}" -l app.kubernetes.io/instance=aeterna-prereqs
echo ""
echo "Connection info for ${ENV} values.yaml:"
echo "  postgresql.host: aeterna-prereqs-cnpg-rw"
echo "  redis.host:      aeterna-prereqs-dragonfly"
echo "  vectorStore.host: aeterna-prereqs-qdrant"
