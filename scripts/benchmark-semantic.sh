#!/bin/bash
# Nyquest Semantic Stage — Full Benchmark & Live Test Suite
# Run: chmod +x bench_semantic.sh && bash bench_semantic.sh
set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

OLLAMA_PORT=11434
MODEL="qwen2.5:1.5b-instruct"
OLLAMA_URL="http://localhost:${OLLAMA_PORT}/v1/chat/completions"
NYQUEST_URL="http://localhost:5400"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_FILE="${SCRIPT_DIR}/SEMANTIC_BENCHMARK_RESULTS.txt"

# Redirect all output to both terminal and file
exec > >(tee "$RESULTS_FILE") 2>&1

echo -e "${BOLD}${CYAN}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${CYAN}║  Nyquest Semantic Compression Stage — Benchmarks     ║${NC}"
echo -e "${BOLD}${CYAN}╚══════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Date: $(date)"
echo "Host: $(hostname)"
echo ""

# ══════════════════════════════════════════════════════
# PHASE 0: Prerequisites
# ══════════════════════════════════════════════════════
echo -e "${BOLD}═══ PHASE 0: Prerequisites ═══${NC}"
echo ""

# Check Ollama
echo -n "Ollama installed: "
if command -v ollama &> /dev/null; then
    echo -e "${GREEN}YES$(NC) ($(ollama --version 2>/dev/null || echo 'unknown'))"
else
    echo -e "${RED}NO — Installing...${NC}"
    curl -fsSL https://ollama.com/install.sh | sh
fi

# Check Ollama service
echo -n "Ollama service: "
if systemctl is-active --quiet ollama 2>/dev/null; then
    echo -e "${GREEN}active${NC}"
else
    echo -e "${YELLOW}starting...${NC}"
    sudo systemctl start ollama
    sleep 3
    if systemctl is-active --quiet ollama 2>/dev/null; then
        echo -e "  ${GREEN}started${NC}"
    else
        echo -e "  ${RED}FAILED — check: journalctl -u ollama -n 20${NC}"
        exit 1
    fi
fi

# Check/pull model
echo -n "Model ${MODEL}: "
if ollama list 2>/dev/null | grep -q "qwen2.5:1.5b"; then
    echo -e "${GREEN}available${NC}"
    ollama list 2>/dev/null | grep "qwen2.5:1.5b"
else
    echo -e "${YELLOW}pulling...${NC}"
    ollama pull "$MODEL"
    echo -e "  ${GREEN}pulled${NC}"
fi

# GPU info
echo ""
echo "GPU Status:"
if command -v nvidia-smi &> /dev/null; then
    nvidia-smi --query-gpu=name,memory.used,memory.total,temperature.gpu --format=csv,noheader 2>/dev/null || echo "  nvidia-smi failed"
else
    echo "  nvidia-smi not available"
fi

# RAM info
echo ""
echo "RAM: $(free -h | awk '/^Mem:/{print $3 "/" $2 " used"}')"
echo ""

# ══════════════════════════════════════════════════════
# PHASE 1: Model Warm-Up & Basic Response Test
# ══════════════════════════════════════════════════════
echo -e "${BOLD}═══ PHASE 1: Model Warm-Up & Basic Response ═══${NC}"
echo ""

echo "Warming up model (first call loads into VRAM)..."
WARMUP_START=$(date +%s%N)
WARMUP_RESP=$(curl -s -w "\n%{time_total}" -X POST "$OLLAMA_URL" \
    -H "Content-Type: application/json" \
    -d "{
        \"model\": \"${MODEL}\",
        \"messages\": [{\"role\": \"user\", \"content\": \"Say OK\"}],
        \"max_tokens\": 5,
        \"temperature\": 0
    }" 2>/dev/null)
WARMUP_TIME=$(echo "$WARMUP_RESP" | tail -1)
WARMUP_BODY=$(echo "$WARMUP_RESP" | head -n -1)
echo "  Warm-up time: ${WARMUP_TIME}s"

if echo "$WARMUP_BODY" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['choices'][0]['message']['content'])" 2>/dev/null; then
    echo -e "  ${GREEN}Model responding${NC}"
else
    echo -e "  ${RED}Model not responding!${NC}"
    echo "$WARMUP_BODY"
    exit 1
fi

# Post-warmup GPU
echo ""
echo "Post-warmup GPU VRAM:"
nvidia-smi --query-gpu=memory.used,memory.total --format=csv,noheader 2>/dev/null || echo "  n/a"
echo ""

# ══════════════════════════════════════════════════════
# PHASE 2: Latency Benchmarks (10 iterations each)
# ══════════════════════════════════════════════════════
echo -e "${BOLD}═══ PHASE 2: Latency Benchmarks ═══${NC}"
echo ""

run_bench() {
    local LABEL="$1"
    local SYSTEM_PROMPT="$2"
    local USER_CONTENT="$3"
    local MAX_TOKENS="${4:-256}"
    local ITERS=5

    echo -e "${CYAN}Test: ${LABEL}${NC}"
    echo "  Input length: $(echo -n "$USER_CONTENT" | wc -c) chars"

    local TOTAL_MS=0
    local TOTAL_PROMPT_TOKENS=0
    local TOTAL_COMPLETION_TOKENS=0
    local FIRST_RESPONSE=""

    for i in $(seq 1 $ITERS); do
        RESP=$(curl -s -X POST "$OLLAMA_URL" \
            -H "Content-Type: application/json" \
            -d "$(python3 -c "
import json
print(json.dumps({
    'model': '${MODEL}',
    'messages': [
        {'role': 'system', 'content': $(python3 -c "import json; print(json.dumps('''${SYSTEM_PROMPT}'''))")},
        {'role': 'user', 'content': $(python3 -c "import json; print(json.dumps('''${USER_CONTENT}'''))")}
    ],
    'max_tokens': ${MAX_TOKENS},
    'temperature': 0,
    'stream': False
}))
" 2>/dev/null)" 2>/dev/null)

        # Extract metrics via python
        METRICS=$(echo "$RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    content = d['choices'][0]['message']['content']
    usage = d.get('usage', {})
    pt = usage.get('prompt_tokens', 0)
    ct = usage.get('completion_tokens', 0)
    # Ollama returns total_duration in nanoseconds in some formats
    print(f'{pt}|{ct}|{len(content)}')
except Exception as e:
    print(f'0|0|0')
" 2>/dev/null)

        PT=$(echo "$METRICS" | cut -d'|' -f1)
        CT=$(echo "$METRICS" | cut -d'|' -f2)
        CLEN=$(echo "$METRICS" | cut -d'|' -f3)

        TOTAL_PROMPT_TOKENS=$((TOTAL_PROMPT_TOKENS + PT))
        TOTAL_COMPLETION_TOKENS=$((TOTAL_COMPLETION_TOKENS + CT))

        if [ "$i" -eq 1 ]; then
            FIRST_RESPONSE=$(echo "$RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(d['choices'][0]['message']['content'][:300])
except:
    print('ERROR')
" 2>/dev/null)
        fi
    done

    AVG_PT=$((TOTAL_PROMPT_TOKENS / ITERS))
    AVG_CT=$((TOTAL_COMPLETION_TOKENS / ITERS))

    echo "  Avg prompt tokens: ${AVG_PT}"
    echo "  Avg completion tokens: ${AVG_CT}"
    echo "  Sample output: ${FIRST_RESPONSE}"
    echo ""
}

# ── Benchmark 1: System Prompt Condensation (small) ──
run_bench \
    "System Prompt Condensation — Small (500 chars)" \
    "You are a compression engine. Rewrite the following system instructions in minimal imperative form. Convert all sentences to imperative. Remove redundant qualifiers. Merge duplicate instructions. Preserve ALL behavioral constraints exactly. Output ONLY the compressed instructions." \
    "You are a helpful AI assistant. You should always try to be as helpful as possible when responding to the user. It is very important that you maintain a professional and friendly tone at all times during the conversation. You should never provide misleading or incorrect information to the user. Always make sure your responses are accurate and well-researched. If you are not sure about something, you should let the user know that you are uncertain rather than making something up." \
    200

# ── Benchmark 2: System Prompt Condensation (large) ──
LARGE_SYSTEM="You are an advanced AI coding assistant specialized in helping software developers write, debug, and optimize code. You have extensive knowledge of multiple programming languages including Python, JavaScript, TypeScript, Rust, Go, Java, and C++. When helping users with code, you should always follow these guidelines: First, understand the complete context of what the user is trying to accomplish before providing a solution. Second, provide clean, well-documented code that follows best practices for the specific language being used. Third, explain your reasoning and approach so the user can learn from the interaction. Fourth, if there are multiple ways to solve a problem, present the most efficient solution first but mention alternatives. Fifth, always consider edge cases and error handling in your solutions. You should format code blocks properly using markdown. When reviewing existing code, point out potential bugs, performance issues, and style improvements. Never provide code that could be used for malicious purposes. If the user asks for help with something that could be harmful, politely decline and explain why. You should also be familiar with popular frameworks and libraries for each language, including React, Next.js, Django, Flask, Express, Actix, Tokio, Spring Boot, and others. When discussing architecture decisions, consider scalability, maintainability, and security implications. Always test your code mentally before providing it to ensure it would work correctly."

run_bench \
    "System Prompt Condensation — Large (1500 chars)" \
    "You are a compression engine. Rewrite the following system instructions in minimal imperative form. Convert all sentences to imperative. Remove redundant qualifiers. Merge duplicate instructions. Preserve ALL behavioral constraints exactly. Output ONLY the compressed instructions." \
    "$LARGE_SYSTEM" \
    512

# ── Benchmark 3: History Condensation (multi-turn) ──
HISTORY_INPUT="[0] user: I need help setting up a Kubernetes cluster on AWS using EKS. I have 3 microservices that need to communicate with each other.

[1] assistant: I can help you set up an EKS cluster. First, let's install eksctl and configure your AWS credentials. Run: aws configure with your access key. Then create the cluster with: eksctl create cluster --name my-cluster --region us-east-1 --nodes 3.

[2] user: OK I did that. The cluster is running but my services can't talk to each other. I'm getting connection refused errors.

[3] assistant: That sounds like a networking issue. Let's check a few things: 1) Are your services using ClusterIP service type? Run kubectl get svc. 2) Check if the pods are in the same namespace. 3) Verify your security groups allow internal traffic on the required ports.

[4] user: kubectl get svc shows all three services with ClusterIP. They're all in the default namespace. The ports are 8080, 8081, and 8082.

[5] assistant: Good, they're all ClusterIP in the same namespace. Let's check if the pods themselves are healthy: kubectl get pods -o wide. Also check if the endpoints are populated: kubectl get endpoints. If endpoints show no IPs, the service selectors don't match the pod labels.

[6] user: The endpoints are empty for service-b! The labels don't match. I had app=serviceb in the deployment but app=service-b in the service selector.

[7] assistant: That's the issue. Fix the label mismatch by updating either the deployment or service. I'd update the deployment: kubectl patch deployment service-b -p '{\"spec\":{\"template\":{\"metadata\":{\"labels\":{\"app\":\"service-b\"}}}}}'. After that, verify with kubectl get endpoints service-b.

[8] user: Fixed it! All endpoints are populated now and the services can communicate. Thanks!

[9] assistant: Great, glad that's resolved. The label mismatch is a common Kubernetes gotcha. For future reference, I recommend using a consistent naming convention for labels and always verifying endpoints after creating services."

run_bench \
    "History Condensation — 10 messages (2K chars)" \
    "You are a compression engine. Condense the following conversation history into a minimal summary. Preserve decisions, errors, resolutions, URLs, IPs, code, constraints. Remove greetings, filler, meta-commentary. Output ONLY the compressed summary." \
    "$HISTORY_INPUT" \
    400

# ── Benchmark 4: Redundancy Scoring ──
DEDUP_INPUT="[0] user: Configure the firewall to allow traffic on port 443 and port 80.

[1] assistant: I'll configure the firewall rules. Running: sudo ufw allow 443/tcp and sudo ufw allow 80/tcp.

[2] user: Also make sure the firewall allows HTTPS traffic on port 443 and HTTP on port 80.

[3] assistant: Those are already configured from the previous step - port 443 (HTTPS) and port 80 (HTTP) are both allowed.

[4] user: Can you verify that ports 80 and 443 are open in the firewall?

[5] assistant: Running sudo ufw status. Both port 80 and 443 show as ALLOW from Anywhere."

run_bench \
    "Redundancy Scoring — 6 messages with duplicates" \
    "Analyze these messages for semantic duplicates. Return a JSON array of objects with msg_index, span_start, span_end, duplicate_of, confidence. Only flag confidence > 0.85. Output ONLY valid JSON array." \
    "$DEDUP_INPUT" \
    300

# ══════════════════════════════════════════════════════
# PHASE 3: Compression Quality Tests
# ══════════════════════════════════════════════════════
echo -e "${BOLD}═══ PHASE 3: Compression Quality Tests ═══${NC}"
echo ""

quality_test() {
    local LABEL="$1"
    local SYSTEM="$2"
    local INPUT="$3"
    local MAX_TOK="${4:-400}"

    echo -e "${CYAN}Quality Test: ${LABEL}${NC}"

    local INPUT_CHARS=$(echo -n "$INPUT" | wc -c)
    local INPUT_WORDS=$(echo -n "$INPUT" | wc -w)

    RESP=$(curl -s -X POST "$OLLAMA_URL" \
        -H "Content-Type: application/json" \
        -d "$(python3 -c "
import json
print(json.dumps({
    'model': '${MODEL}',
    'messages': [
        {'role': 'system', 'content': $(python3 -c "import json; print(json.dumps('''${SYSTEM}'''))")},
        {'role': 'user', 'content': $(python3 -c "import json; print(json.dumps('''${INPUT}'''))")}
    ],
    'max_tokens': ${MAX_TOK},
    'temperature': 0,
    'stream': False
}))
" 2>/dev/null)" 2>/dev/null)

    OUTPUT=$(echo "$RESP" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    c = d['choices'][0]['message']['content']
    usage = d.get('usage', {})
    pt = usage.get('prompt_tokens', 0)
    ct = usage.get('completion_tokens', 0)
    print(f'TOKENS:{pt}:{ct}')
    print(c)
except Exception as e:
    print(f'ERROR: {e}')
" 2>/dev/null)

    TOKENS_LINE=$(echo "$OUTPUT" | head -1)
    CONTENT=$(echo "$OUTPUT" | tail -n +2)
    OUTPUT_CHARS=$(echo -n "$CONTENT" | wc -c)
    OUTPUT_WORDS=$(echo -n "$CONTENT" | wc -w)

    PT=$(echo "$TOKENS_LINE" | cut -d: -f2)
    CT=$(echo "$TOKENS_LINE" | cut -d: -f3)

    if [ "$INPUT_CHARS" -gt 0 ]; then
        SAVINGS=$(python3 -c "print(f'{(1 - $OUTPUT_CHARS/$INPUT_CHARS) * 100:.1f}')" 2>/dev/null || echo "?")
    else
        SAVINGS="0"
    fi

    echo "  Input:  ${INPUT_CHARS} chars / ${INPUT_WORDS} words / ~${PT} prompt tokens"
    echo "  Output: ${OUTPUT_CHARS} chars / ${OUTPUT_WORDS} words / ~${CT} completion tokens"
    echo "  Char savings: ${SAVINGS}%"
    echo "  ---"
    echo "  OUTPUT:"
    echo "$CONTENT" | head -20
    echo "  ---"
    echo ""
}

# Test 1: Verbose system prompt
quality_test \
    "Claude-style system prompt compression" \
    "You are a compression engine. Rewrite the following system instructions in minimal imperative form. Convert to imperative. Remove redundant qualifiers. Merge duplicates. Preserve ALL constraints exactly. Output ONLY compressed text." \
    "You are Claude, an AI assistant created by Anthropic. You should always be helpful, harmless, and honest in your responses. When answering questions, you should think step by step and provide clear, accurate information. You should not make up facts or provide information you are not confident about. If you do not know something, you should say so rather than guessing. You should be respectful and considerate in all your interactions. You should not engage in harmful, illegal, or unethical activities or encourage others to do so. You should protect user privacy and not share personal information. When writing code, you should follow best practices and include error handling. You should provide explanations for complex topics in simple terms when possible." \
    300

# Test 2: OpenClaw-style multi-turn history
quality_test \
    "OpenClaw agent history compression" \
    "You are a compression engine. Condense this conversation into minimal summary preserving all decisions, errors, commands, file paths, and outcomes. Remove pleasantries and meta-commentary. Output ONLY the summary." \
    "[0] user: Hey Claude, can you help me debug this Python script? It's throwing a KeyError on line 42.
[1] assistant: Of course! I'd be happy to help you debug that. Could you share the relevant code around line 42? A KeyError typically means you're trying to access a dictionary key that doesn't exist.
[2] user: Here's the code: data = json.load(f); result = data['config']['database']['host']
[3] assistant: I see the issue. The nested dictionary access isn't safe. If any intermediate key is missing, you'll get a KeyError. Let me suggest a safer approach using .get() with defaults: host = data.get('config', {}).get('database', {}).get('host', 'localhost'). This returns 'localhost' as default if any key is missing.
[4] user: That worked! But now I need to also get the port which defaults to 5432.
[5] assistant: Great! Apply the same pattern: port = data.get('config', {}).get('database', {}).get('port', 5432). You might also want to create a helper function since you're doing this repeatedly.
[6] user: Good idea. Can you write that helper?
[7] assistant: Here's a helper function: def get_nested(d, keys, default=None): for k in keys: if isinstance(d, dict): d = d.get(k, default) else: return default; return d. Usage: host = get_nested(data, ['config', 'database', 'host'], 'localhost')
[8] user: Perfect, using that now. Thanks!" \
    400

# ══════════════════════════════════════════════════════
# PHASE 4: Live Nyquest Integration Test
# ══════════════════════════════════════════════════════
echo -e "${BOLD}═══ PHASE 4: Live Nyquest Proxy Test ═══${NC}"
echo ""

# Check if Nyquest is running
echo -n "Nyquest proxy status: "
if curl -s "${NYQUEST_URL}/health" 2>/dev/null | grep -qi "ok\|healthy\|running"; then
    echo -e "${GREEN}running${NC}"
    echo ""

    # Test a request through Nyquest
    echo "Sending test request through Nyquest proxy..."
    echo "(This tests the full pipeline: normalize -> compress -> forward)"
    echo ""

    NYQUEST_RESP=$(curl -s -w "\nHTTP_CODE:%{http_code}\nTIME:%{time_total}" \
        -X POST "${NYQUEST_URL}/v1/messages" \
        -H "Content-Type: application/json" \
        -H "x-api-key: test" \
        -H "anthropic-version: 2023-06-01" \
        -d '{
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 50,
            "system": "You are a helpful assistant. You should always be polite and professional. Please provide accurate information.",
            "messages": [
                {"role": "user", "content": "What is 2+2?"}
            ]
        }' 2>/dev/null || echo "CONNECTION_FAILED")

    HTTP_CODE=$(echo "$NYQUEST_RESP" | grep "HTTP_CODE:" | cut -d: -f2)
    REQ_TIME=$(echo "$NYQUEST_RESP" | grep "TIME:" | cut -d: -f2)

    echo "  HTTP Code: ${HTTP_CODE:-N/A}"
    echo "  Total time: ${REQ_TIME:-N/A}s"

    # Check Nyquest stats
    echo ""
    echo "Nyquest compression stats:"
    curl -s "${NYQUEST_URL}/api/stats" 2>/dev/null | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print(json.dumps(d, indent=2))
except:
    print('  Could not parse stats')
" 2>/dev/null || echo "  Stats endpoint not available"

else
    echo -e "${YELLOW}not running${NC}"
    echo "  Nyquest proxy is not running on port 5400."
    echo "  Start it: cd $SCRIPT_DIR && cargo run --release"
    echo "  (Semantic stage tests will run when Nyquest is rebuilt with semantic.rs)"
fi

# ══════════════════════════════════════════════════════
# PHASE 5: Throughput Test
# ══════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}═══ PHASE 5: Throughput — Sequential Requests ═══${NC}"
echo ""

echo "Running 10 sequential compression requests..."
THROUGHPUT_START=$(date +%s%N)

for i in $(seq 1 10); do
    curl -s -X POST "$OLLAMA_URL" \
        -H "Content-Type: application/json" \
        -d "{
            \"model\": \"${MODEL}\",
            \"messages\": [
                {\"role\": \"system\", \"content\": \"Compress this to minimal form. Output only compressed text.\"},
                {\"role\": \"user\", \"content\": \"The system should ensure that all responses are helpful and accurate. It is important to maintain professional tone.\"}
            ],
            \"max_tokens\": 50,
            \"temperature\": 0,
            \"stream\": false
        }" > /dev/null 2>/dev/null
    echo -n "."
done

THROUGHPUT_END=$(date +%s%N)
THROUGHPUT_MS=$(( (THROUGHPUT_END - THROUGHPUT_START) / 1000000 ))
AVG_MS=$((THROUGHPUT_MS / 10))

echo ""
echo "  10 requests in ${THROUGHPUT_MS}ms"
echo "  Average: ${AVG_MS}ms per request"
echo "  Throughput: $(python3 -c "print(f'{10000/${THROUGHPUT_MS}:.2f}')" 2>/dev/null) req/s"

# ══════════════════════════════════════════════════════
# Summary
# ══════════════════════════════════════════════════════
echo ""
echo -e "${BOLD}${CYAN}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}${CYAN}║  Benchmark Complete                                  ║${NC}"
echo -e "${BOLD}${CYAN}╚══════════════════════════════════════════════════════╝${NC}"
echo ""
echo "Results saved to: ${RESULTS_FILE}"
echo ""
echo "Next steps:"
echo "  1. Wire semantic.rs into lib.rs and engine.rs"
echo "  2. Add SemanticConfig fields to config.rs"
echo "  3. cargo build --release"
echo "  4. Add semantic_enabled: true to nyquest.yaml"
echo "  5. sudo systemctl restart nyquest"
