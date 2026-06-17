#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${SCRIPT_DIR}/.."

cd "$REPO_ROOT"
cargo test -p adapters --test rbac_matrix_doc_test -- --ignored update_rbac_doc

echo "Generated RBAC matrix: docs/security/rbac-matrix.md"
