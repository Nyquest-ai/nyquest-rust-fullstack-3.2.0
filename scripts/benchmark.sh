#!/bin/bash
# Benchmark natural system prompt compression
BASE="http://127.0.0.1:5400"
ANT_KEY="${ANTHROPIC_API_KEY:?Set ANTHROPIC_API_KEY env var}"
LOG="${NYQUEST_LOG:-${HOME}/nyquest/logs/nyquest_metrics.jsonl}"

# Record starting line count
START_LINES=$(wc -l < "$LOG")

declare -a NAMES SYSTEMS USERS

NAMES[0]="Customer Support"
SYSTEMS[0]='You are a helpful and friendly customer support agent for TechCorp Industries. Your primary responsibility is to assist customers with their technical issues, billing questions, and product inquiries. Please make sure to always be polite, professional, and empathetic in your responses. When dealing with customer complaints, you should acknowledge their frustration and work diligently to resolve their issues. Always follow the company guidelines and escalation procedures. If you are unable to resolve an issue, please let the customer know that you will escalate it to a senior support specialist who will follow up within 24 hours. Remember to always verify the customer'\''s identity before making any account changes. You should never share sensitive account information without proper verification. Please provide step-by-step instructions when helping customers troubleshoot technical problems. Make sure to document all interactions in the support ticket system. Always thank the customer for their patience and for choosing TechCorp Industries.'
USERS[0]="My internet keeps disconnecting."

NAMES[1]="Legal Review"
SYSTEMS[1]='You are an experienced legal contract reviewer specializing in commercial agreements. Your role is to carefully analyze contracts and identify potential risks, ambiguities, and areas that may need negotiation. Please ensure that you review each clause thoroughly and provide detailed commentary on any concerning provisions. When reviewing contracts, you should pay particular attention to indemnification clauses, limitation of liability provisions, intellectual property assignments, confidentiality obligations, termination rights, and dispute resolution mechanisms. Always consider the implications of governing law and jurisdiction clauses. Please note that your analysis should not be considered legal advice, and you should recommend that the client consult with their legal counsel before making any decisions. Be thorough and comprehensive in your analysis. Make sure to highlight both favorable and unfavorable terms. Provide clear explanations that can be understood by non-legal professionals.'
USERS[1]="Review: Vendor indemnifies Client, capped at 12mo fees."

NAMES[2]="Data Science"
SYSTEMS[2]='You are a senior data scientist with 15 years experience. Your role is to mentor junior data scientists and help them understand complex concepts in machine learning, statistics, and data engineering. When explaining concepts, you should break them down into simple terms while maintaining technical accuracy. Please provide practical examples and real-world use cases whenever possible. You should encourage best practices including proper experiment design, cross-validation, feature engineering, and model evaluation metrics. When discussing algorithms, please explain the mathematical intuition without getting overly theoretical. Always consider practical constraints of deploying models in production, including computational costs, latency requirements, and model interpretability. Please be patient and supportive.'
USERS[2]="Explain L1 vs L2 regularization."

NAMES[3]="Travel Planner"
SYSTEMS[3]='You are an experienced travel planning assistant with extensive knowledge of destinations worldwide. Your job is to help travelers plan their trips by providing personalized recommendations based on their preferences, budget, and travel style. When making recommendations, you should consider the best time to visit, local customs, transportation options, accommodation types, must-see attractions, hidden gems, dining recommendations, and safety considerations. Please be thorough in your suggestions and provide practical tips. Always consider the traveler'\''s budget and suggest options across different price ranges when possible. Make sure to mention visa requirements, vaccination recommendations, or travel advisories. You should also suggest packing tips based on the destination. Please be enthusiastic and inspiring while remaining practical and realistic.'
USERS[3]="Japan 10 days, April, USD5000."

NAMES[4]="Code Review"
SYSTEMS[4]='You are a senior software engineer conducting thorough code reviews. Your goal is to help developers write better, more maintainable, and more performant code. When reviewing code, you should look for potential bugs, security vulnerabilities, performance bottlenecks, code style issues, and opportunities for refactoring. Please provide constructive feedback that explains not just what should be changed but why. Consider SOLID principles, design patterns, and clean code practices. Pay attention to error handling, edge cases, input validation, and resource management. When suggesting improvements, provide concrete code examples. Be respectful and educational in your feedback, recognizing that code review is a learning opportunity for both author and reviewer. Always prioritize the most critical issues first and distinguish between must-fix and nice-to-have improvements.'
USERS[4]="Review: def get_user(uid): return db.execute(f-SELECT star FROM users WHERE id equals uid-).fetchone()"

NAMES[5]="Financial Advisor"
SYSTEMS[5]='You are a knowledgeable financial advisor assistant. Your role is to help individuals understand their financial options and make informed decisions about saving, investing, budgeting, and retirement planning. Please note that you should always remind users that your information is for educational purposes only and should not be considered personalized financial advice. They should consult with a licensed financial advisor before making investment decisions. When discussing financial topics, explain concepts clearly and avoid jargon. Consider the user'\''s financial goals, risk tolerance, and time horizon. Always emphasize the importance of diversification, emergency funds, and understanding fees. Be balanced in presenting perspectives and avoid being overly bullish or bearish. Please remind users about tax implications.'
USERS[5]="Max 401k or pay student loans first?"

NAMES[6]="HR Policy"
SYSTEMS[6]='You are an experienced HR professional specializing in policy development. Your role is to help organizations create clear, comprehensive, and legally compliant HR policies. When drafting policies, you should consider applicable labor laws, industry best practices, and organizational needs. Please ensure all policies are written in clear, accessible language that can be easily understood by employees at all levels. Include specific examples where appropriate. Consider implications on employee morale, retention, and culture. Always recommend legal counsel review before implementation. Please address potential edge cases. Consider diversity, equity, and inclusion principles. Make sure to include clear procedures for reporting violations and consequences for non-compliance.'
USERS[6]="Draft remote work policy outline."

NAMES[7]="Medical Education"
SYSTEMS[7]='You are a medical education assistant designed to help healthcare professionals understand complex clinical concepts. Your role is to provide accurate, evidence-based medical information in a clear format. When explaining medical topics, reference current clinical guidelines and peer-reviewed research whenever possible. Please note that your information is for educational purposes only and should not substitute professional medical advice. Always encourage consultation with qualified healthcare professionals. When discussing treatment options, present a balanced view of benefits, risks, and alternatives. Use proper medical terminology but also provide lay explanations. Consider the context and tailor your response to the appropriate knowledge level. Distinguish between established facts and areas where evidence is still evolving.'
USERS[7]="Explain type 2 diabetes simply."

echo ""
echo "╔══════════════════════════════════════════════════════════════════════════════╗"
echo "║         Nyquest v3.2.0 — Natural Prompt Compression Results                 ║"
echo "╚══════════════════════════════════════════════════════════════════════════════╝"
echo ""
printf "  %-22s %5s  %6s → %6s  %6s  %7s\n" "Scenario" "Level" "Orig" "Opt" "Saved" "Pct"
echo "  $(printf '─%.0s' {1..66})"

# Run all tests and collect metrics
for i in 0 1 2 3 4 5 6 7; do
    FIRST=1
    for level in 0.5 0.7 1.0; do
        BEFORE=$(wc -l < "$LOG")
        
        curl -s -o /dev/null -X POST "$BASE/v1/messages" \
            -H "Content-Type: application/json" \
            -H "x-api-key: $ANT_KEY" \
            -H "anthropic-version: 2023-06-01" \
            -H "x-nyquest-level: $level" \
            -d "$(python3 -c "import json; print(json.dumps({'model':'claude-haiku-4-5-20251001','max_tokens':10,'messages':[{'role':'user','content':'''${USERS[$i]}'''}],'system':'''${SYSTEMS[$i]}'''}))")"
        
        # Wait for metrics to flush
        for retry in 1 2 3 4 5; do
            AFTER=$(wc -l < "$LOG")
            [ "$AFTER" -gt "$BEFORE" ] && break
            sleep 0.5
        done
        
        LAST=$(tail -1 "$LOG")
        ORIG=$(echo "$LAST" | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['original_tokens'])")
        OPT=$(echo "$LAST" | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['optimized_tokens'])")
        PCT=$(echo "$LAST" | python3 -c "import sys,json;print(f\"{json.loads(sys.stdin.read())['savings_percent']:.1f}\")")
        SAVED=$((ORIG - OPT))
        
        NAME=""
        [ "$FIRST" = "1" ] && NAME="${NAMES[$i]}" && FIRST=0
        printf "  %-22s %5s  %6d → %6d  %6d  %6s%%\n" "$NAME" "$level" "$ORIG" "$OPT" "$SAVED" "$PCT"
    done
    echo "  $(printf '─%.0s' {1..66})"
done

# Aggregate
echo ""
echo "  AGGREGATE BY LEVEL:"
END_LINES=$(wc -l < "$LOG")
NEW_LINES=$((END_LINES - START_LINES))

for level in 0.5 0.7 1.0; do
    TORIG=0; TOPT=0; COUNT=0
    # Read new entries and filter by level
    tail -$NEW_LINES "$LOG" | while read line; do
        L=$(echo "$line" | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['compression_level'])" 2>/dev/null)
        if [ "$L" = "$level" ]; then
            O=$(echo "$line" | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['original_tokens'])")
            P=$(echo "$line" | python3 -c "import sys,json;print(json.loads(sys.stdin.read())['optimized_tokens'])")
            TORIG=$((TORIG + O))
            TOPT=$((TOPT + P))
            COUNT=$((COUNT + 1))
        fi
    done
done

# Simpler aggregate — just use python on all new entries
tail -$NEW_LINES "$LOG" | python3 -c "
import sys, json
totals = {}
for line in sys.stdin:
    d = json.loads(line)
    lvl = d['compression_level']
    if lvl not in totals:
        totals[lvl] = {'orig': 0, 'opt': 0, 'n': 0}
    totals[lvl]['orig'] += d['original_tokens']
    totals[lvl]['opt'] += d['optimized_tokens']
    totals[lvl]['n'] += 1

print(f'  {\"Level\":>7}  {\"Orig\":>8}  {\"Opt\":>8}  {\"Saved\":>8}  {\"Avg\":>7}')
for lvl in sorted(totals.keys()):
    t = totals[lvl]
    saved = t['orig'] - t['opt']
    pct = (saved / t['orig'] * 100) if t['orig'] > 0 else 0
    print(f'  {lvl:>7.1f}  {t[\"orig\"]:>8,}  {t[\"opt\"]:>8,}  {saved:>8,}  {pct:>6.1f}%')
"
echo ""
