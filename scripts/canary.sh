#!/usr/bin/env bash
# Budgeted canary run (PLAN.md §17/§19): create a small, budgeted agent run against a
# live control plane and fail the deploy if it doesn't complete. Intended as a
# post-deploy smoke test (CI, or `make canary`).
#
# Env:
#   CAT_HERDER_API_URL            (default http://localhost:8080)
#   CAT_HERDER_ADMIN_USER         (default admin)
#   CAT_HERDER_ADMIN_PASSWORD     (default admin)
# Requires: curl, jq.
set -euo pipefail

API="${CAT_HERDER_API_URL:-http://localhost:8080}"
USER="${CAT_HERDER_ADMIN_USER:-admin}"
PASS="${CAT_HERDER_ADMIN_PASSWORD:-admin}"
TASK="${CAT_HERDER_CANARY_TASK:-Canary: confirm the harness boots and completes. Reply with the single word OK.}"
BUDGET="${CAT_HERDER_CANARY_BUDGET:-1000}"

JAR="$(mktemp)"
trap 'rm -f "$JAR"' EXIT

# 1. Sign in (session cookie).
curl -fsS -c "$JAR" -X POST "$API/v1/auth/login" \
  -H 'Content-Type: application/json' \
  -d "{\"username\":\"$USER\",\"password\":\"$PASS\"}" >/dev/null

# 2. Create a budgeted canary run.
run_json="$(curl -fsS -b "$JAR" -X POST "$API/v1/runs" \
  -H 'Content-Type: application/json' \
  -d "$(jq -nc --arg t "$TASK" --argjson b "$BUDGET" \
        '{task_spec:$t, kind:"agent", agent_id:"agent-codeact", budget:$b}')")"

run_id="$(echo "$run_json" | jq -r '.id')"
echo "canary: run $run_id created (budget $BUDGET tokens)"

# 3. Poll until the run reaches a done state.
state=""
for _ in $(seq 1 60); do
  state="$(curl -fsS "$API/v1/runs/$run_id" | jq -r '.state')"
  case "$state" in
    succeeded|awaiting_review|reviewed|rejected|failed|timed_out|cancelled|stuck) break ;;
  esac
  sleep 5
done

# 4. The canary passes only if the run succeeded (or is awaiting review of a real diff).
case "$state" in
  succeeded|awaiting_review|reviewed)
    echo "canary: run $run_id $state — OK"
    exit 0
    ;;
  *)
    echo "canary: run $run_id failed ($state)" >&2
    exit 1
    ;;
esac
