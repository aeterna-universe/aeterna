#!/usr/bin/env bash
# wait-for-aeterna.sh — Wait for Aeterna server to be healthy and ready
#
# Usage: ./e2e/wait-for-aeterna.sh <base-url> [timeout-secs]
#
set -euo pipefail

URL="${1:?usage: wait-for-aeterna.sh <base-url> [timeout-secs]}"
TIMEOUT="${2:-300}"

echo "[wait] Waiting for Aeterna at ${URL} (timeout: ${TIMEOUT}s)..."

deadline=$((SECONDS + TIMEOUT))
while (( SECONDS < deadline )); do
  # Health endpoint
  status=$(curl -s -o /dev/null -w "%{http_code}" "${URL}/health" 2>/dev/null || echo "000")
  if [[ "${status}" == "200" ]]; then
    # Check readiness (includes DB + vector store checks)
    ready=$(curl -s -o /dev/null -w "%{http_code}" "${URL}/ready" 2>/dev/null || echo "000")
    if [[ "${ready}" == "200" ]]; then
      echo "[wait] Aeterna is healthy and ready ✓"
      exit 0
    else
      echo "[wait] /health=200 but /ready=${ready} — waiting for backing services..."
    fi
  else
    echo "[wait] /health=${status} — server not up yet..."
  fi
  sleep 5
done

echo "[wait] ERROR: Aeterna did not become ready within ${TIMEOUT}s" >&2
echo "[wait] Last health response:"
curl -sf "${URL}/health" 2>&1 || echo "(no response)"
echo ""
echo "[wait] Last ready response:"
curl -sf "${URL}/ready" 2>&1 || echo "(no response)"
exit 1
