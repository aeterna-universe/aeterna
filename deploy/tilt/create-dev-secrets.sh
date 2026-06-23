#!/usr/bin/env bash
#
# create-dev-secrets.sh — Provision Kubernetes secrets for local Aeterna dev
#
# Creates the secrets the Helm chart needs for GitHub App authentication:
#   - aeterna-knowledge-pem      (knowledge repo + PR governance)
#   - aeterna-plugin-auth        (JWT secret for plugin auth — generated)
#
# These secrets are consumed by the aeterna Helm chart's knowledgeRepo
# and pluginAuth values. Run once after creating the kind cluster.
#
# Usage:
#   ./deploy/tilt/create-dev-secrets.sh
#
set -euo pipefail

NAMESPACE="${AETERNA_NAMESPACE:-aeterna}"
PEM_POLICY="${AETERNA_POLICY_PEM:-/mnt/c/Users/kikok/Downloads/aeterna-policy.2026-06-22.private-key.pem}"
PEM_KNOWLEDGE="${AETERNA_KNOWLEDGE_PEM:-/mnt/c/Users/kikok/Downloads/aeterna-knowledge.2026-06-22.private-key.pem}"

echo "=== Creating Aeterna dev secrets in namespace: ${NAMESPACE} ==="

# Ensure namespace exists
kubectl get namespace "${NAMESPACE}" >/dev/null 2>&1 || kubectl create namespace "${NAMESPACE}"

# --- Knowledge repo GitHub App PEM ---
if [ -f "${PEM_KNOWLEDGE}" ]; then
    kubectl -n "${NAMESPACE}" create secret generic aeterna-knowledge-pem \
        --from-file=pem-key="${PEM_KNOWLEDGE}" \
        --dry-run=client -o yaml | kubectl apply -f -
    echo "  [OK] aeterna-knowledge-pem"
else
    echo "  [SKIP] aeterna-knowledge-pem — PEM file not found: ${PEM_KNOWLEDGE}"
fi

# --- Policy repo GitHub App PEM ---
if [ -f "${PEM_POLICY}" ]; then
    kubectl -n "${NAMESPACE}" create secret generic aeterna-policy-pem \
        --from-file=pem-key="${PEM_POLICY}" \
        --dry-run=client -o yaml | kubectl apply -f -
    echo "  [OK] aeterna-policy-pem"
else
    echo "  [SKIP] aeterna-policy-pem — PEM file not found: ${PEM_POLICY}"
fi

# --- Plugin auth JWT secret (random, for local dev only) ---
kubectl -n "${NAMESPACE}" create secret generic aeterna-plugin-auth \
    --from-literal=jwt-secret="$(openssl rand -hex 32)" \
    --from-literal=github-client-secret="dev-dummy-not-real" \
    --dry-run=client -o yaml | kubectl apply -f -
echo "  [OK] aeterna-plugin-auth (generated JWT + dummy GitHub OAuth)"

echo ""
echo "=== Secrets created ==="
kubectl -n "${NAMESPACE}" get secrets | grep aeterna
