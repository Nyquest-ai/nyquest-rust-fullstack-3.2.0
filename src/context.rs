//! Nyquest Context Window Optimizer
//! Summarizes older conversation turns to reduce token accumulation
//! while preserving key facts, constraints, and intent continuity.
//!
//! Pipeline position: Before Normalize → Before Compress

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashSet;
use tracing::{info, warn};

use crate::tokens::TokenCounter;

// ── High-value patterns to preserve ──

static HIGH_VALUE_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)\b(?:decided|agreed|confirmed|conclusion|result|answer|solution)\b")
            .unwrap(),
        Regex::new(r"(?i)\b(?:error|bug|fix|issue|problem|warning)\b").unwrap(),
        Regex::new(r"(?i)\b(?:must|shall|require|critical|important)\b").unwrap(),
        Regex::new(r"(?i)https?://\S+").unwrap(),
        Regex::new(r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}").unwrap(),
        Regex::new(r"(?s)```[\s\S]*?```").unwrap(),
        Regex::new(r"\b\d{4}[-/]\d{2}[-/]\d{2}\b").unwrap(),
        Regex::new(r"\$[\d,.]+\b").unwrap(),
    ]
});

static LOW_VALUE_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)^(?:hi|hello|hey|thanks|thank you|ok|okay|sure|great|perfect|got it|sounds good)[\s.!]*$").unwrap(),
        Regex::new(r"(?i)^(?:can you|could you|would you|please)\s").unwrap(),
    ]
});

// Split text on sentence boundaries (manual, since regex crate lacks lookbehind)
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let chars: Vec<char> = text.chars().collect();
    let byte_indices: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let len = chars.len();
    for ci in 0..len {
        if (chars[ci] == '.' || chars[ci] == '!' || chars[ci] == '?')
            && ci + 1 < len
            && chars[ci + 1].is_whitespace()
        {
            let end_byte = if ci + 1 < byte_indices.len() {
                byte_indices[ci + 1]
            } else {
                text.len()
            };
            sentences.push(&text[start..end_byte]);
            // Advance start past the whitespace
            if ci + 2 < byte_indices.len() {
                start = byte_indices[ci + 2];
            } else {
                start = text.len();
            }
        }
    }
    if start < text.len() {
        sentences.push(&text[start..]);
    }
    sentences
}

static CODE_BLOCK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)```[\s\S]*?```").unwrap());

// ── Config ──

pub struct ContextConfig {
    pub enabled: bool,
    pub max_input_tokens: usize,
    pub preserve_recent_turns: usize,
    pub min_turns_for_summary: usize,
    pub max_summary_chars: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_input_tokens: 50000,
            preserve_recent_turns: 4,
            min_turns_for_summary: 6,
            max_summary_chars: 2000,
        }
    }
}

// ── Optimizer ──

pub struct ContextOptimizer {
    config: ContextConfig,
}

impl ContextOptimizer {
    pub fn new(config: ContextConfig) -> Self {
        Self { config }
    }

    pub fn should_optimize(&self, request_body: &Value, token_counter: &TokenCounter) -> bool {
        if !self.config.enabled {
            return false;
        }
        let messages = request_body.get("messages").and_then(|m| m.as_array());
        let msg_count = messages.map(|m| m.len()).unwrap_or(0);
        if msg_count < self.config.min_turns_for_summary {
            return false;
        }
        let total_tokens = token_counter.count_request_tokens(request_body);
        total_tokens > self.config.max_input_tokens
    }

    pub fn optimize(&self, request_body: &Value, token_counter: &TokenCounter) -> Value {
        if !self.should_optimize(request_body, token_counter) {
            return request_body.clone();
        }

        let mut result = request_body.clone();
        let messages = match result.get("messages").and_then(|m| m.as_array()) {
            Some(m) if m.len() >= self.config.min_turns_for_summary => m.clone(),
            _ => return result,
        };

        let total = messages.len();

        // Determine split point
        let preserve_count = (self.config.preserve_recent_turns * 2).min(total.saturating_sub(2));
        if preserve_count >= total {
            return result;
        }

        let split = total - preserve_count;
        let older = &messages[..split];
        let recent = &messages[split..];

        // Extract key facts from older messages
        let summary = self.summarize_messages(older);
        if summary.trim().is_empty() {
            return result;
        }

        let summary_msg = json!({
            "role": "user",
            "content": format!("[Context Summary of {} earlier messages]\n{}", older.len(), summary)
        });
        let ack_msg = json!({
            "role": "assistant",
            "content": "Understood. I have the context from our earlier conversation."
        });

        // Repair tool pairs
        let repaired_recent = self.repair_tool_pairs(older, recent);

        let mut new_messages = vec![summary_msg, ack_msg];
        new_messages.extend(repaired_recent);

        result["messages"] = Value::Array(new_messages);

        // Log optimization
        let original_tokens = token_counter.count_request_tokens(request_body);
        let optimized_tokens = token_counter.count_request_tokens(&result);
        let savings = original_tokens.saturating_sub(optimized_tokens);

        info!(
            "Context optimized: {} msgs → {} msgs | {} → {} tokens (saved {})",
            total,
            result["messages"].as_array().map(|a| a.len()).unwrap_or(0),
            original_tokens,
            optimized_tokens,
            savings
        );

        result
    }

    /// Ensure every tool_result in recent has a matching tool_use
    fn repair_tool_pairs(&self, older: &[Value], recent: &[Value]) -> Vec<Value> {
        // Index tool_use blocks in older messages
        let mut older_tool_uses: std::collections::HashMap<String, Value> =
            std::collections::HashMap::new();
        for msg in older {
            if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for block in arr {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                            older_tool_uses.insert(id.to_string(), msg.clone());
                        }
                    }
                }
            }
        }

        // Collect tool_use_ids and tool_result_ids in recent
        let mut recent_tool_use_ids = HashSet::new();
        let mut recent_tool_result_ids = HashSet::new();
        for msg in recent {
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for block in arr {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("tool_use") => {
                            if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                                recent_tool_use_ids.insert(id.to_string());
                            }
                        }
                        Some("tool_result") => {
                            if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                                recent_tool_result_ids.insert(id.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        let orphaned: HashSet<_> = recent_tool_result_ids
            .difference(&recent_tool_use_ids)
            .cloned()
            .collect();

        if orphaned.is_empty() {
            return recent.to_vec();
        }

        // Recover matching tool_use messages from older set
        let mut recovered = Vec::new();
        let mut still_orphaned = HashSet::new();
        for tid in &orphaned {
            if let Some(msg) = older_tool_uses.get(tid) {
                recovered.push(msg.clone());
                info!(
                    "Context repair: recovered tool_use {}... from pruned messages",
                    &tid[..tid.len().min(12)]
                );
            } else {
                still_orphaned.insert(tid.clone());
            }
        }

        // Drop orphaned tool_result blocks
        let mut repaired_recent: Vec<Value> = if still_orphaned.is_empty() {
            recent.to_vec()
        } else {
            recent.iter().filter_map(|msg| {
                if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                    let filtered: Vec<Value> = arr.iter().filter(|block| {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            if let Some(tid) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                                if still_orphaned.contains(tid) {
                                    warn!("Context repair: dropping orphaned tool_result {}...", &tid[..tid.len().min(12)]);
                                    return false;
                                }
                            }
                        }
                        true
                    }).cloned().collect();

                    if filtered.is_empty() {
                        None
                    } else {
                        let mut msg = msg.clone();
                        msg["content"] = Value::Array(filtered);
                        Some(msg)
                    }
                } else {
                    Some(msg.clone())
                }
            }).collect()
        };

        // Prepend recovered tool_use messages
        if !recovered.is_empty() {
            // Deduplicate
            let mut seen = HashSet::new();
            let unique: Vec<Value> = recovered
                .into_iter()
                .filter(|msg| {
                    let key = msg.to_string();
                    seen.insert(key)
                })
                .collect();

            let mut result = unique;
            result.append(&mut repaired_recent);
            return result;
        }

        repaired_recent
    }

    /// Extractive summarization — no LLM call needed
    fn summarize_messages(&self, messages: &[Value]) -> String {
        let mut facts = Vec::new();
        let mut tool_results = Vec::new();

        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = msg.get("content").cloned().unwrap_or(Value::Null);

            match &content {
                Value::Array(arr) => {
                    for block in arr {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    self.extract_from_text(text, role, &mut facts);
                                }
                            }
                            Some("tool_result") => {
                                let tc = block.get("content").cloned().unwrap_or(Value::Null);
                                let text = extract_text(&tc);
                                if !text.trim().is_empty() {
                                    let truncated: String = text.chars().take(200).collect();
                                    tool_results.push(format!("Tool result: {}", truncated));
                                }
                            }
                            Some("tool_use") => {
                                let name =
                                    block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                                tool_results.push(format!("Called tool: {}", name));
                            }
                            _ => {}
                        }
                    }
                }
                Value::String(s) if !s.trim().is_empty() => {
                    self.extract_from_text(s, role, &mut facts);
                }
                _ => {}
            }
        }

        let mut parts = Vec::new();

        if !facts.is_empty() {
            // Deduplicate
            let mut seen = HashSet::new();
            let unique: Vec<&String> = facts
                .iter()
                .filter(|f| {
                    let key: String = f.to_lowercase().chars().take(80).collect();
                    seen.insert(key)
                })
                .collect();

            let limited: Vec<_> = unique.into_iter().take(30).collect();
            let lines: Vec<String> = limited.iter().map(|f| format!("- {}", f)).collect();
            parts.push(format!("Key points:\n{}", lines.join("\n")));
        }

        if !tool_results.is_empty() {
            let limited: Vec<_> = tool_results.iter().take(10).collect();
            let lines: Vec<String> = limited.iter().map(|t| format!("- {}", t)).collect();
            parts.push(format!("Tool interactions:\n{}", lines.join("\n")));
        }

        let mut summary = parts.join("\n\n");

        if summary.len() > self.config.max_summary_chars {
            summary.truncate(self.config.max_summary_chars);
            summary.push_str("\n[...truncated]");
        }

        summary
    }

    fn extract_from_text(&self, text: &str, role: &str, facts: &mut Vec<String>) {
        let sentences: Vec<&str> = split_sentences(text);

        for sentence in sentences {
            let sentence = sentence.trim();
            if sentence.is_empty() || sentence.len() < 10 {
                continue;
            }

            // Skip low-value
            if LOW_VALUE_PATTERNS.iter().any(|p| p.is_match(sentence)) {
                continue;
            }

            // Check high-value
            let is_high_value = HIGH_VALUE_PATTERNS.iter().any(|p| p.is_match(sentence));

            if is_high_value {
                let truncated: String = sentence.chars().take(200).collect();
                match role {
                    "assistant" => facts.push(truncated),
                    "user" => facts.push(format!("User stated: {}", truncated)),
                    _ => facts.push(truncated),
                }
            }

            // Keep small code blocks
            for block in CODE_BLOCK_RE.find_iter(sentence) {
                if block.as_str().len() < 500 {
                    facts.push(block.as_str().to_string());
                }
            }
        }
    }
}

fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block
                        .get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    block.as_str().map(|s| s.to_string())
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
