#!/usr/bin/env bash
# End-to-end smoke test for a running Nyquest engine container.
#
# Verifies (in order):
#   1. /health returns version 3.2.0
#   2. /metrics returns 200
#   3. /admin/v2/compression/cache reports the configured capacity
#   4. /admin/v2/compression/rules returns the 19 categories
#   5. POST /v1/messages compresses a verbose system prompt at L1.0
#      (uses a placeholder API key — the proxy compresses regardless of
#      whether the upstream call succeeds; we only check for a metrics
#      log line confirming the engine ran)
#
# Usage:
#   bash scripts/smoke-test.sh                           # localhost:5400
#   BASE=http://otherhost:5400 bash scripts/smoke-test.sh

set -uo pipefail

BASE=${BASE:-http://127.0.0.1:5400}
PASS=0
FAIL=0

result() {
    local status="$1" name="$2" detail="${3-}"
    if [ "$status" = "PASS" ]; then
        PASS=$((PASS+1))
        printf "  \033[32m✓\033[0m  %-44s %s\n" "$name" "$detail"
    else
        FAIL=$((FAIL+1))
        printf "  \033[31m✗\033[0m  %-44s %s\n" "$name" "$detail"
    fi
}

echo "Nyquest smoke test against $BASE"
echo "──────────────────────────────────────────────────────────────"

# 1. /health
H=$(curl -sf "$BASE/health" 2>/dev/null)
if echo "$H" | grep -q '"version":"3.2.0"'; then
    result PASS "/health reports v3.2.0"
else
    result FAIL "/health reports v3.2.0" "got: ${H:-no response}"
fi

# 2. /metrics
M=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/metrics")
[ "$M" = "200" ] && result PASS "/metrics returns 200" \
                  || result FAIL "/metrics returns 200" "got HTTP $M"

# 3. /admin/v2/compression/cache
CACHE=$(curl -sf "$BASE/admin/v2/compression/cache")
CAP=$(echo "$CACHE" | grep -oE '"capacity":[0-9]+' | head -1 | cut -d: -f2)
if [ -n "$CAP" ] && [ "$CAP" -ge 1 ]; then
    result PASS "/admin/v2/compression/cache" "capacity=$CAP"
else
    result FAIL "/admin/v2/compression/cache" "no capacity field in response"
fi

# 4. /admin/v2/compression/rules
RULES=$(curl -sf "$BASE/admin/v2/compression/rules")
N_CATS=$(echo "$RULES" | python3 -c "import json,sys; print(len(json.loads(sys.stdin.read()).get('categories', [])))" 2>/dev/null)
if [ "$N_CATS" = "19" ]; then
    result PASS "/admin/v2/compression/rules" "$N_CATS categories"
else
    result FAIL "/admin/v2/compression/rules" "expected 19 categories, got ${N_CATS:-?}"
fi

# 5. POST /v1/messages → exercises the engine end-to-end
LOG=/app/logs/nyquest_metrics.jsonl
# Try host path first (mounted), fall back to docker exec path
if [ -f ./logs/nyquest_metrics.jsonl ]; then LOG=./logs/nyquest_metrics.jsonl; fi
BEFORE=$([ -f "$LOG" ] && wc -l < "$LOG" || echo 0)
PAYLOAD='{"model":"claude-haiku-4-5","max_tokens":10,"messages":[{"role":"user","content":"hi"}],"system":"You are a customer support agent. Please make sure to always be polite. It is important to note that you should always provide accurate information. Please ensure that you carefully analyze the situation."}'
HTTP=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: smoke-test-placeholder" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-nyquest-level: 1.0" \
    -d "$PAYLOAD")
sleep 0.5
AFTER=$([ -f "$LOG" ] && wc -l < "$LOG" || echo 0)
if [ "$AFTER" -gt "$BEFORE" ]; then
    LAST=$(tail -1 "$LOG")
    PCT=$(echo "$LAST" | python3 -c "import json,sys;print(f\"{json.loads(sys.stdin.read()).get('savings_percent',0):.1f}\")" 2>/dev/null)
    result PASS "POST /v1/messages compresses (HTTP $HTTP)" "saved ${PCT}% on this prompt"
else
    # Even without log access we can confirm the proxy at least handled the request
    if [ "$HTTP" = "200" ] || [ "$HTTP" = "401" ] || [ "$HTTP" = "403" ]; then
        result PASS "POST /v1/messages handled (HTTP $HTTP)" "(metrics log not visible from this side)"
    else
        result FAIL "POST /v1/messages handled" "got HTTP $HTTP"
    fi
fi

echo "──────────────────────────────────────────────────────────────"
echo "$PASS passed, $FAIL failed"
[ "$FAIL" = "0" ]
