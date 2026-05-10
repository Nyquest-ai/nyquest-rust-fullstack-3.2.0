#!/bin/bash
# Nyquest v3.2.0 Full Rust Stack — Comprehensive Live Test Suite
set -o pipefail

BASE="http://127.0.0.1:5400"
ANT_KEY="${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY env var}"
XAI_KEY="${XAI_API_KEY:?Set XAI_API_KEY env var}"
GEM_KEY="${GEMINI_API_KEY:?Set GEMINI_API_KEY env var}"
PASS=0; FAIL=0

result() {
    local status="$1" name="$2" extra="$3"
    if [ "$status" = "PASS" ]; then
        PASS=$((PASS + 1))
    else
        FAIL=$((FAIL + 1))
    fi
    printf "  %-4s %-52s %s\n" "$status" "$name" "$extra"
}

check_json_field() {
    python3 -c "
import sys, json
try:
    d = json.loads(sys.stdin.read())
    fields = '$1'.split('.')
    v = d
    for f in fields:
        if f.startswith('[') and f.endswith(']'):
            v = v[int(f[1:-1])]
        else:
            v = v.get(f) if isinstance(v, dict) else None
    if v and str(v).strip():
        print('OK')
    else:
        print('EMPTY')
except Exception as e:
    print(f'ERR:{e}')
" 2>/dev/null
}

echo "╔══════════════════════════════════════════════════════════════════════╗"
echo "║        Nyquest v3.2.0 Full Rust Stack — Live Test Suite            ║"
echo "║        $(date '+%Y-%m-%d %H:%M:%S %Z')                                    ║"
echo "╚══════════════════════════════════════════════════════════════════════╝"
echo ""

# ─── Infrastructure ──────────────────────────────────────────
echo "── Infrastructure ────────────────────────────────────────────────────"
H=$(curl -s "$BASE/health" | python3 -c "import sys,json;print(json.loads(sys.stdin.read()).get('status',''))" 2>/dev/null)
[ "$H" = "ok" ] && result PASS "/health" || result FAIL "/health"

D=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/dashboard")
[ "$D" = "200" ] && result PASS "/dashboard" || result FAIL "/dashboard → HTTP $D"

M=$(curl -s -o /dev/null -w "%{http_code}" "$BASE/metrics")
[ "$M" = "200" ] && result PASS "/metrics" || result FAIL "/metrics → HTTP $M"

echo ""

# ─── Anthropic /v1/messages ──────────────────────────────────
echo "── Anthropic /v1/messages ────────────────────────────────────────────"

# Test 1: Simple non-streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"messages":[{"role":"user","content":"Say hello in one word."}],"system":"You are a helpful assistant. Please be very careful to follow all instructions precisely and accurately. Think step by step."}')
R=$(cat "$TMP" | check_json_field "content.[0].text")
[ "$R" = "OK" ] && result PASS "Haiku non-streaming (simple)" "${T}s" || result FAIL "Haiku non-streaming ($R)" "${T}s"
rm -f "$TMP"

# Test 2: Streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"stream":true,"messages":[{"role":"user","content":"Count 1 to 3."}],"system":"You are helpful. Always respond concisely."}')
STREAM_OK=$(grep -c "content_block_delta" "$TMP" 2>/dev/null)
[ "$STREAM_OK" -gt 0 ] && result PASS "Haiku streaming" "${T}s" || result FAIL "Haiku streaming (no deltas)" "${T}s"
rm -f "$TMP"

# Test 3: Multi-turn
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"messages":[{"role":"user","content":"My name is Mike."},{"role":"assistant","content":"Nice to meet you, Mike!"},{"role":"user","content":"What is my name?"}],"system":"You are a helpful assistant."}')
R=$(cat "$TMP" | check_json_field "content.[0].text")
[ "$R" = "OK" ] && result PASS "Haiku multi-turn context" "${T}s" || result FAIL "Haiku multi-turn ($R)" "${T}s"
rm -f "$TMP"

# Test 4: Passthrough (level=0)
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-nyquest-level: 0.0" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"messages":[{"role":"user","content":"Say hi."}],"system":"You are a helpful assistant."}')
R=$(cat "$TMP" | check_json_field "content.[0].text")
[ "$R" = "OK" ] && result PASS "Haiku level=0.0 (passthrough)" "${T}s" || result FAIL "Haiku passthrough ($R)" "${T}s"
rm -f "$TMP"

# Test 5: Max compression
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-nyquest-level: 1.0" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"messages":[{"role":"user","content":"Say hi."}],"system":"You are a helpful assistant. Please be very careful to follow all instructions precisely and accurately. Make sure to think step by step before responding. Always be thorough and comprehensive."}')
R=$(cat "$TMP" | check_json_field "content.[0].text")
[ "$R" = "OK" ] && result PASS "Haiku level=1.0 (max compression)" "${T}s" || result FAIL "Haiku max ($R)" "${T}s"
rm -f "$TMP"

echo ""

# ─── OpenAI-compat ───────────────────────────────────────────
echo "── OpenAI /v1/chat/completions ───────────────────────────────────────"

# Test 6: xAI Grok non-streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $XAI_KEY" \
    -H "x-nyquest-base-url: https://api.x.ai/v1" \
    -d '{"model":"grok-3-mini-fast","max_tokens":100,"messages":[{"role":"system","content":"You are a helpful assistant. Think carefully."},{"role":"user","content":"What is 2+2? One word."}]}')
R=$(cat "$TMP" | check_json_field "choices.[0].message.content")
[ "$R" = "OK" ] && result PASS "xAI Grok non-streaming" "${T}s" || result FAIL "xAI Grok ($R)" "${T}s"
rm -f "$TMP"

# Test 7: xAI Grok streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $XAI_KEY" \
    -H "x-nyquest-base-url: https://api.x.ai/v1" \
    -d '{"model":"grok-3-mini-fast","max_tokens":100,"stream":true,"messages":[{"role":"user","content":"Count 1 to 3."}]}')
STREAM_OK=$(grep -c "chat.completion.chunk" "$TMP" 2>/dev/null)
[ "$STREAM_OK" -gt 0 ] && result PASS "xAI Grok streaming" "${T}s" || result FAIL "xAI Grok streaming (no chunks)" "${T}s"
rm -f "$TMP"

# Test 8: xAI Grok low max_tokens (thinking-model floor test)
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $XAI_KEY" \
    -H "x-nyquest-base-url: https://api.x.ai/v1" \
    -d '{"model":"grok-3-mini-fast","max_tokens":10,"messages":[{"role":"user","content":"Say hi."}]}')
R=$(cat "$TMP" | check_json_field "choices.[0].message.content")
[ "$R" = "OK" ] && result PASS "xAI Grok low max_tokens (floor fix)" "${T}s" || result FAIL "xAI Grok low tok ($R)" "${T}s"
rm -f "$TMP"

# Test 9: Gemini non-streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $GEM_KEY" \
    -H "x-nyquest-base-url: https://generativelanguage.googleapis.com/v1beta/openai" \
    -d '{"model":"gemini-2.5-flash","max_tokens":256,"messages":[{"role":"system","content":"You are a helpful assistant. Always be thorough."},{"role":"user","content":"Capital of France? One word."}]}')
R=$(cat "$TMP" | check_json_field "choices.[0].message.content")
[ "$R" = "OK" ] && result PASS "Gemini non-streaming" "${T}s" || result FAIL "Gemini ($R)" "${T}s"
rm -f "$TMP"

# Test 10: Gemini low max_tokens (was empty before fix)
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $GEM_KEY" \
    -H "x-nyquest-base-url: https://generativelanguage.googleapis.com/v1beta/openai" \
    -d '{"model":"gemini-2.5-flash","max_tokens":50,"messages":[{"role":"system","content":"You are helpful. Think carefully."},{"role":"user","content":"What is 2+2? One word."}]}')
R=$(cat "$TMP" | check_json_field "choices.[0].message.content")
[ "$R" = "OK" ] && result PASS "Gemini low max_tokens (floor fix)" "${T}s" || result FAIL "Gemini low tok ($R)" "${T}s"
rm -f "$TMP"

# Test 11: Gemini streaming
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $GEM_KEY" \
    -H "x-nyquest-base-url: https://generativelanguage.googleapis.com/v1beta/openai" \
    -d '{"model":"gemini-2.5-flash","max_tokens":256,"stream":true,"messages":[{"role":"user","content":"Count 1 to 3."}]}')
STREAM_OK=$(grep -c '"delta"' "$TMP" 2>/dev/null)
[ "$STREAM_OK" -gt 0 ] && result PASS "Gemini streaming" "${T}s" || result FAIL "Gemini streaming (no chunks)" "${T}s"
rm -f "$TMP"

echo ""

# ─── OpenClaw Agent Mode ─────────────────────────────────────
echo "── OpenClaw Agent Mode ───────────────────────────────────────────────"

# Test 12: Tool result compression
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-nyquest-openclaw: true" \
    -H "x-nyquest-level: 0.8" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":100,"messages":[{"role":"user","content":"List the files."},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"bash","input":{"command":"ls -la"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"total 48\ndrwxr-xr-x 5 user user 4096 Feb 28 10:00 .\ndrwxr-xr-x 3 user user 4096 Feb 28 09:00 ..\n-rw-r--r-- 1 user user 1234 Feb 28 10:00 README.md\n-rw-r--r-- 1 user user 5678 Feb 28 10:00 main.py\ndrwxr-xr-x 2 user user 4096 Feb 28 10:00 src"}]},{"role":"user","content":"What files are here?"}],"system":"You are an autonomous coding agent called OpenClaw. You have access to a variety of tools including bash, file editors, and web search. You should always think step by step before taking actions. When you encounter errors, you should carefully analyze them and try alternative approaches. Always be thorough and comprehensive in your analysis."}')
R=$(cat "$TMP" | check_json_field "content.[0].text")
[ "$R" = "OK" ] && result PASS "OpenClaw: tool_result compression" "${T}s" || result FAIL "OpenClaw tool ($R)" "${T}s"
rm -f "$TMP"

# Test 13: Multi-turn with code + traceback (includes tool defs)
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" \
    -H "x-api-key: $ANT_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "x-nyquest-openclaw: true" \
    -H "x-nyquest-level: 0.8" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":300,"tools":[{"name":"bash","description":"Run a bash command in the terminal. Use this to execute shell commands, run scripts, and interact with the filesystem.","input_schema":{"type":"object","properties":{"command":{"type":"string","description":"The bash command to execute"}},"required":["command"]}},{"name":"edit_file","description":"Edit a file by replacing text. Use this to make changes to source code and configuration files.","input_schema":{"type":"object","properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}},"required":["path","old_text","new_text"]}}],"messages":[{"role":"user","content":"Debug this Python app."},{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"bash","input":{"command":"cat main.py"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"import sys, json\nfrom pathlib import Path\n\ndef transform(item):\n    result = item.copy()\n    result[\"timestamp\"] = str(datetime.now())\n    return result\n\ndef main():\n    data = json.load(open(\"input.json\"))\n    results = [transform(x) for x in data]\n    json.dump(results, open(\"output.json\",\"w\"))\n\nif __name__ == \"__main__\":\n    main()"}]},{"role":"assistant","content":[{"type":"text","text":"I see datetime is not imported. Let me run to confirm."},{"type":"tool_use","id":"t2","name":"bash","input":{"command":"python3 main.py"}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"Traceback (most recent call last):\n  File \"/home/user/main.py\", line 12\n    main()\n  File \"/home/user/main.py\", line 10\n    results = [transform(x) for x in data]\n  File \"/home/user/main.py\", line 6\nNameError: name datetime is not defined"}]},{"role":"user","content":"Fix it please."}],"system":"You are OpenClaw, an autonomous coding agent with access to bash and file editing tools. Think step by step before taking actions. When you encounter errors, carefully analyze them and try alternative approaches. Be thorough and comprehensive. Follow all instructions precisely."}')
# Check for either text or tool_use response (both are valid)
R=$(cat "$TMP" | python3 -c "
import sys, json
try:
    d = json.loads(sys.stdin.read())
    blocks = d.get('content', [])
    has_content = any(
        (b.get('type') == 'text' and b.get('text','').strip()) or
        b.get('type') == 'tool_use'
        for b in blocks
    )
    print('OK' if has_content else 'EMPTY')
except: print('ERR')
" 2>/dev/null)
[ "$R" = "OK" ] && result PASS "OpenClaw: code + traceback multi-turn" "${T}s" || result FAIL "OpenClaw multi ($R)" "${T}s"
rm -f "$TMP"

# Test 14: OpenClaw via OpenAI-compat route
TMP=$(mktemp)
T=$(curl -s -o "$TMP" -w "%{time_total}" -X POST "$BASE/v1/chat/completions" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $XAI_KEY" \
    -H "x-nyquest-base-url: https://api.x.ai/v1" \
    -H "x-nyquest-openclaw: true" \
    -H "x-nyquest-level: 0.8" \
    -d '{"model":"grok-3-mini-fast","max_tokens":200,"messages":[{"role":"system","content":"You are an autonomous coding agent. Always think step by step. Be thorough and comprehensive. Follow all instructions precisely."},{"role":"user","content":"What is Python?"}]}')
R=$(cat "$TMP" | check_json_field "choices.[0].message.content")
[ "$R" = "OK" ] && result PASS "OpenClaw via xAI (OpenAI-compat route)" "${T}s" || result FAIL "OpenClaw xAI ($R)" "${T}s"
rm -f "$TMP"

echo ""

# ─── Error Handling ──────────────────────────────────────────
echo "── Error Handling ────────────────────────────────────────────────────"

C=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" -H "x-api-key: sk-invalid" -H "anthropic-version: 2023-06-01" \
    -d '{"model":"claude-haiku-4-5-20251001","max_tokens":50,"messages":[{"role":"user","content":"Hi"}]}')
[ "$C" = "401" ] && result PASS "Invalid API key → 401" || result FAIL "Invalid key → HTTP $C"

C=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$BASE/v1/messages" \
    -H "Content-Type: application/json" -H "x-api-key: $ANT_KEY" -H "anthropic-version: 2023-06-01" \
    -d '{"model":"nonexistent-xyz","max_tokens":50,"messages":[{"role":"user","content":"Hi"}]}')
[[ "$C" =~ ^4 ]] && result PASS "Invalid model → $C" || result FAIL "Invalid model → HTTP $C"

echo ""

# ─── Metrics Verification ────────────────────────────────────
echo "── Metrics Verification ──────────────────────────────────────────────"
sleep 1
echo "  Last 12 entries:"
tail -12 "${HOME}/nyquest/logs/nyquest_metrics.jsonl" | python3 -c "
import sys, json
for line in sys.stdin:
    d = json.loads(line)
    lat = d['latency_ms']
    ico = '✅' if lat > 0 else '❌'
    print(f'    {ico} {d[\"model\"]:30s} | {d[\"original_tokens\"]:5d}→{d[\"optimized_tokens\"]:5d} | {d[\"savings_percent\"]:5.1f}% | {lat:7.0f}ms')
"

echo ""
echo "══════════════════════════════════════════════════════════════════════"
printf "  RESULTS:  %d passed  /  %d failed  /  %d total\n" "$PASS" "$FAIL" "$((PASS + FAIL))"
echo "══════════════════════════════════════════════════════════════════════"
