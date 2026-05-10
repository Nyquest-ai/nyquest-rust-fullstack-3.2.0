//! Nyquest Prompt Normalizer
//! Structured prompt normalization for hallucination mitigation.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

// ──────────────────────────────────────────
// Pattern sets
// ──────────────────────────────────────────

#[allow(dead_code)]
static FORMAT_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
    Regex::new(r"(?i)(?:respond|reply|answer|output|return|format)\s+(?:in|as|with|using)\s+(\w[\w\s]*?)(?:\.|,|$)").unwrap(),
    Regex::new(r"(?i)(?:use|follow|adhere\s+to)\s+(?:this|the\s+following)\s+(?:format|schema|structure|template)").unwrap(),
    Regex::new(r"(?i)(?:output|response)\s+(?:must|should)\s+be\s+(?:in\s+)?(\w[\w\s]*?)(?:\.|,|$)").unwrap(),
    Regex::new(r"(?i)```(?:json|xml|yaml|csv|markdown)").unwrap(),
    Regex::new(r"(?i)(?:JSON|XML|YAML|CSV|markdown|HTML)\s+(?:format|output|response)").unwrap(),
]
});

static SPECULATION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
    Regex::new(r"(?i)(?:do\s+not|don't|never)\s+(?:guess|speculate|assume|invent|fabricate|make\s+up|hallucinate)").unwrap(),
    Regex::new(r"(?i)(?:only|stick\s+to)\s+(?:facts|verified|known|confirmed|provided)\s+(?:information|data)").unwrap(),
    Regex::new(r"(?i)(?:if|when)\s+(?:unsure|uncertain|not\s+sure|don't\s+know)").unwrap(),
]
});

struct ConflictPair {
    pattern_a: Regex,
    pattern_b: Regex,
}

static CONFLICT_PAIRS: Lazy<Vec<ConflictPair>> = Lazy::new(|| {
    vec![
        ConflictPair {
            pattern_a: Regex::new(r"(?i)be\s+(?:very\s+)?(?:concise|brief|short)").unwrap(),
            pattern_b: Regex::new(
                r"(?i)be\s+(?:very\s+)?(?:detailed|thorough|comprehensive|verbose)",
            )
            .unwrap(),
        },
        ConflictPair {
            pattern_a: Regex::new(r"(?i)(?:always|must)\s+(?:include|provide)\s+(?:examples|code)")
                .unwrap(),
            pattern_b: Regex::new(
                r"(?i)(?:do\s+not|don't|never)\s+(?:include|provide)\s+(?:examples|code)",
            )
            .unwrap(),
        },
        ConflictPair {
            pattern_a: Regex::new(r"(?i)(?:use|respond\s+in)\s+(?:formal|professional)").unwrap(),
            pattern_b: Regex::new(r"(?i)(?:use|respond\s+in)\s+(?:casual|informal|friendly)")
                .unwrap(),
        },
    ]
});

static RE_FILLER_DEDUP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(?:please|always|make sure to|ensure that|remember to)\b").unwrap()
});
static RE_MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

// ──────────────────────────────────────────
// Core functions
// ──────────────────────────────────────────

/// Normalize for deduplication comparison
fn normalize_for_dedup(text: &str) -> String {
    let t = text.to_lowercase();
    let t = RE_FILLER_DEDUP.replace_all(&t, "");
    let t = RE_MULTI_SPACE.replace_all(&t, " ");
    t.trim()
        .trim_end_matches(|c: char| ".,;:!".contains(c))
        .to_string()
}

/// Remove semantically redundant instructions
fn remove_redundant_instructions(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();

    for line in lines {
        let stripped = line.trim();
        if stripped.is_empty() {
            result.push(line.to_string());
            continue;
        }
        let normalized = normalize_for_dedup(stripped);
        if seen.contains(&normalized) {
            continue;
        }
        seen.insert(normalized);
        result.push(line.to_string());
    }

    result.join("\n")
}

/// Remove sentences containing a pattern
fn remove_sentence_containing(text: &str, pattern: &Regex) -> String {
    // Manual sentence splitting (regex crate doesn't support lookbehind)
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    for i in 0..len {
        if (chars[i] == '.' || chars[i] == '!' || chars[i] == '?')
            && i + 1 < len
            && chars[i + 1].is_whitespace()
        {
            let end = i + 1; // include the punctuation
            sentences.push(text[start..end].trim());
            start = end;
        }
    }
    if start < text.len() {
        let remaining = text[start..].trim();
        if !remaining.is_empty() {
            sentences.push(remaining);
        }
    }
    let kept: Vec<&str> = sentences
        .into_iter()
        .filter(|s| !s.is_empty() && !pattern.is_match(s))
        .collect();
    kept.join(" ")
}
/// Inject speculation boundaries if none exist
fn inject_speculation_boundaries(text: &str) -> String {
    let has = SPECULATION_PATTERNS.iter().any(|p| p.is_match(text));
    if has {
        return text.to_string();
    }
    format!(
        "{}\nIf information is unavailable or uncertain, state that explicitly. \
         Do not speculate, infer, or generate unverified claims.",
        text
    )
}

/// Full normalization pipeline for a system message
fn normalize_system_message(system: &str, inject_boundaries: bool) -> String {
    let mut result = remove_redundant_instructions(system);

    // Detect and resolve conflicts (keep first occurrence)
    for pair in CONFLICT_PAIRS.iter() {
        let match_a = pair.pattern_a.is_match(&result);
        let match_b = pair.pattern_b.is_match(&result);
        if match_a && match_b {
            result = remove_sentence_containing(&result, &pair.pattern_b);
        }
    }

    if inject_boundaries {
        result = inject_speculation_boundaries(&result);
    }

    result
}

/// Optimize prompt structure for provider cache hits.
/// Ensures static system instructions are isolated from dynamic content
/// so that the provider's prompt caching can reuse them across requests.
fn optimize_for_caching(result: &mut Value) {
    // Strategy 1: Convert string system prompts to array format with cache_control
    // Array format allows the provider to cache each block independently
    if let Some(Value::String(sys)) = result.get("system").cloned() {
        if !sys.is_empty() {
            // Split system prompt into static instructions vs dynamic context
            // Static: role definitions, behavioral rules (doesn't change between requests)
            // Dynamic: conversation-specific context, injected variables
            let parts: Vec<&str> = sys.splitn(2, "\n\n---\n\n").collect();

            if parts.len() == 2 {
                // Explicit separator found: first part is static, second is dynamic
                let blocks = serde_json::json!([
                    {"type": "text", "text": parts[0], "cache_control": {"type": "ephemeral"}},
                    {"type": "text", "text": parts[1]}
                ]);
                result
                    .as_object_mut()
                    .unwrap()
                    .insert("system".to_string(), blocks);
            }
            // If no separator, leave as-is — user hasn't structured for caching
        }
    }

    // Strategy 2: Sort tool definitions deterministically
    // Tools in the same order = same cache prefix = cache hit
    if let Some(Value::Array(tools)) = result.get_mut("tools") {
        tools.sort_by(|a, b| {
            let name_a = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let name_b = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
            name_a.cmp(name_b)
        });
    }
}

/// Normalize an entire Anthropic Messages API request
pub fn normalize_request(request_body: &Value, inject_boundaries: bool) -> Value {
    let mut result = request_body.clone();

    // Normalize system message
    if let Some(system) = result.get("system").cloned() {
        match system {
            Value::String(s) if !s.is_empty() => {
                let normalized = normalize_system_message(&s, inject_boundaries);
                result
                    .as_object_mut()
                    .unwrap()
                    .insert("system".to_string(), Value::String(normalized));
            }
            Value::Array(blocks) => {
                let normalized: Vec<Value> = blocks
                    .iter()
                    .map(|block| {
                        if let Some(map) = block.as_object() {
                            if map.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(Value::String(text)) = map.get("text") {
                                    let mut new_block = block.clone();
                                    new_block.as_object_mut().unwrap().insert(
                                        "text".to_string(),
                                        Value::String(normalize_system_message(
                                            text,
                                            inject_boundaries,
                                        )),
                                    );
                                    return new_block;
                                }
                            }
                        }
                        block.clone()
                    })
                    .collect();
                result
                    .as_object_mut()
                    .unwrap()
                    .insert("system".to_string(), Value::Array(normalized));
            }
            _ => {}
        }
    }

    // Normalize user messages (light touch — only dedup)
    if let Some(Value::Array(messages)) = result.get("messages").cloned() {
        let normalized: Vec<Value> = messages
            .iter()
            .map(|msg| {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
                if role != "user" {
                    return msg.clone();
                }
                let mut msg = msg.clone();
                if let Some(content) = msg.get("content").cloned() {
                    match content {
                        Value::String(s) => {
                            msg.as_object_mut().unwrap().insert(
                                "content".to_string(),
                                Value::String(remove_redundant_instructions(&s)),
                            );
                        }
                        Value::Array(blocks) => {
                            let new_blocks: Vec<Value> = blocks
                                .iter()
                                .map(|block| {
                                    if let Some(map) = block.as_object() {
                                        if map.get("type").and_then(|t| t.as_str()) == Some("text")
                                        {
                                            if let Some(Value::String(text)) = map.get("text") {
                                                let mut new_block = block.clone();
                                                new_block.as_object_mut().unwrap().insert(
                                                    "text".to_string(),
                                                    Value::String(remove_redundant_instructions(
                                                        text,
                                                    )),
                                                );
                                                return new_block;
                                            }
                                        }
                                    }
                                    block.clone()
                                })
                                .collect();
                            msg.as_object_mut()
                                .unwrap()
                                .insert("content".to_string(), Value::Array(new_blocks));
                        }
                        _ => {}
                    }
                }
                msg
            })
            .collect();
        result
            .as_object_mut()
            .unwrap()
            .insert("messages".to_string(), Value::Array(normalized));
    }

    // Phase 3: Cache optimization (structural)
    optimize_for_caching(&mut result);

    result
}
