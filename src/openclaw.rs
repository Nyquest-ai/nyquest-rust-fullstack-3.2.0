//! Nyquest OpenClaw Agent Mode
//! Specialized optimization pipeline for autonomous agentic systems.
//!
//! Implements 7 strategies:
//!   1. Tool Result Pruning ("Ack & Drop")
//!   2. Dynamic Schema Minimization
//!   3. Thought Block Compression
//!   4. Error/Log Deduplication
//!   5. System Prompt Cache Optimization
//!   6. Terminal/File View Condensation
//!   7. Infinite Context Sliding Window
//!
//! Pipeline position: After Context Optimization, Before Compression

use md5::{Digest, Md5};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::compression::format;
use crate::tokens::TokenCounter;

// ── Compiled regexes ──

static THOUGHT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?s)<(?:thought|thinking|inner_monologue)>.*?</(?:thought|thinking|inner_monologue)>",
    )
    .unwrap()
});

static TRACEBACK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)Traceback \(most recent call last\):.*?(?:\n\S|\z)").unwrap());

static SYS_PATH_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?m)^\s*File "(?:/usr/lib/|/usr/local/lib/|/lib/python|<frozen |/home/\w+/\.local/lib/).*$"#).unwrap()
});

static DOCSTRING_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?s)"""[\s\S]*?"""|'''[\s\S]*?'''"#).unwrap());

static TRAILING_COMMENT_RE: Lazy<fancy_regex::Regex> = Lazy::new(|| {
    fancy_regex::Regex::new(r"(?m)(?<=\S)\s+#\s+(?!type:|noqa|pylint|pragma).*$").unwrap()
});

static MULTI_BLANK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

static JSON_BLOCK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)```json\s*\n([\s\S]*?)\n\s*```").unwrap());

// ── Config ──

pub struct OpenClawConfig {
    pub enabled: bool,
    // Strategy 1
    pub tool_prune_after_turns: usize,
    pub tool_prune_placeholder: String,
    // Strategy 2
    pub schema_strip_descriptions: bool,
    pub schema_first_call_full: bool,
    // Strategy 3
    pub thought_prune_after_turns: usize,
    pub thought_placeholder: String,
    // Strategy 4
    pub dedup_errors: bool,
    pub dedup_placeholder: String,
    // Strategy 5
    pub inject_cache_control: bool,
    // Strategy 6
    pub condense_file_views: bool,
    pub strip_docstrings: bool,
    pub strip_blank_lines: bool,
    pub minify_json_in_results: bool,
    // Strategy 7
    pub sliding_window_enabled: bool,
    pub sliding_window_threshold: f64,
    pub sliding_window_max_tokens: usize,
    pub sliding_window_preserve_turns: usize,
    pub sliding_window_tool_prune_age: usize,
    pub sliding_window_thought_prune_age: usize,
}

impl Default for OpenClawConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tool_prune_after_turns: 2,
            tool_prune_placeholder: "[Nyquest: tool output processed. {n} chars truncated.]".into(),
            schema_strip_descriptions: true,
            schema_first_call_full: true,
            thought_prune_after_turns: 3,
            thought_placeholder: "[Nyquest: historical reasoning truncated]".into(),
            dedup_errors: true,
            dedup_placeholder: "[Nyquest: identical error trace as previous attempt.]".into(),
            inject_cache_control: true,
            condense_file_views: true,
            strip_docstrings: true,
            strip_blank_lines: true,
            minify_json_in_results: true,
            sliding_window_enabled: true,
            sliding_window_threshold: 0.80,
            sliding_window_max_tokens: 200000,
            sliding_window_preserve_turns: 5,
            sliding_window_tool_prune_age: 10,
            sliding_window_thought_prune_age: 5,
        }
    }
}

// ── Optimizer ──

pub struct OpenClawOptimizer {
    config: OpenClawConfig,
    schema_seen: bool,
}

impl OpenClawOptimizer {
    pub fn new(config: OpenClawConfig) -> Self {
        Self {
            config,
            schema_seen: false,
        }
    }

    pub fn optimize(
        &mut self,
        request_body: &Value,
        token_counter: Option<&TokenCounter>,
    ) -> Value {
        if !self.config.enabled {
            return request_body.clone();
        }

        let mut result = request_body.clone();
        let messages = match result.get("messages").and_then(|m| m.as_array()) {
            Some(m) if m.len() >= 2 => m.clone(),
            _ => return result,
        };

        // Auto-detect if tools have already been used in this conversation
        // (each request creates a new optimizer, so we infer from history)
        if !self.schema_seen {
            for msg in &messages {
                if let Some(content) = msg.get("content").and_then(|c| c.as_array()) {
                    for block in content {
                        let btype = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if btype == "tool_use" || btype == "tool_result" {
                            self.schema_seen = true;
                            break;
                        }
                    }
                }
                if self.schema_seen {
                    break;
                }
            }
        }

        let original_chars = count_chars(&messages);
        let mut messages = messages;

        let mut stats = Stats::default();

        // ── Strategy 7: Sliding Window (check first) ──
        if self.config.sliding_window_enabled {
            if let Some(tc) = token_counter {
                let total_tokens = tc.count_request_tokens(&result);
                let threshold = (self.config.sliding_window_max_tokens as f64
                    * self.config.sliding_window_threshold)
                    as usize;
                if total_tokens > threshold {
                    messages = self.apply_sliding_window(messages, total_tokens, threshold);
                    warn!(
                        "OpenClaw sliding window activated: {} tokens exceeded {:.0}% threshold",
                        total_tokens,
                        self.config.sliding_window_threshold * 100.0
                    );
                }
            }
        }

        // ── Strategy 1: Tool Result Pruning ──
        let (msgs, n) = self.prune_tool_results(messages);
        messages = msgs;
        stats.tool_results_pruned = n;

        // ── Strategy 3: Thought Block Compression ──
        let (msgs, n) = self.prune_thought_blocks(messages);
        messages = msgs;
        stats.thoughts_pruned = n;

        // ── Strategy 4: Error Deduplication ──
        if self.config.dedup_errors {
            let (msgs, n) = self.dedup_errors(messages);
            messages = msgs;
            stats.errors_deduped = n;
        }

        // ── Strategy 6: File View Condensation ──
        if self.config.condense_file_views {
            let (msgs, n) = self.condense_file_views(messages);
            messages = msgs;
            stats.file_views_condensed = n;
        }

        // ── Strategy 2: Schema Minimization ──
        if self.config.schema_strip_descriptions {
            if let Some(tools) = result.get("tools").and_then(|t| t.as_array()) {
                let minimized = self.minimize_schemas(tools);
                result["tools"] = Value::Array(minimized);
            }
        }

        // ── Strategy 5: Cache Control Injection ──
        if self.config.inject_cache_control {
            if let Some(system) = result.get("system") {
                let injected = self.inject_cache_headers(system);
                result["system"] = injected;
            }
        }

        result["messages"] = Value::Array(messages.clone());

        let optimized_chars = count_chars(&messages);
        stats.chars_saved = original_chars.saturating_sub(optimized_chars);

        if stats.any_work() {
            info!(
                "OpenClaw mode: pruned {} tool results, {} thought blocks, \
                 deduped {} errors, condensed {} file views, saved {} chars",
                stats.tool_results_pruned,
                stats.thoughts_pruned,
                stats.errors_deduped,
                stats.file_views_condensed,
                stats.chars_saved
            );
        }

        result
    }

    // ── Strategy 1: Tool Result Pruning ──

    #[allow(clippy::needless_range_loop)]
    fn prune_tool_results(&self, mut messages: Vec<Value>) -> (Vec<Value>, usize) {
        let total = messages.len();
        // Preserve the last N message pairs; prune tool results in everything older.
        // Use preserve_turns * 2 to account for user/assistant pairs, but always
        // allow pruning to start from index 0 if there are enough messages.
        let preserve_count = self.config.tool_prune_after_turns * 2;
        let cutoff = total.saturating_sub(preserve_count);
        let mut pruned = 0;

        // Also prune tool results that are large even in the preserved window
        // but with a higher threshold (500 chars vs 200 for old ones)
        for i in 0..total {
            let role = messages[i]
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let content = messages[i].get("content");
            let in_old_zone = i < cutoff;
            let size_threshold: usize = if in_old_zone { 200 } else { 2000 };

            if let Some(arr) = content.and_then(|c| c.as_array()) {
                let mut arr = arr.clone();
                for j in 0..arr.len() {
                    if arr[j].get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let raw_text = extract_text_from_content(
                            &arr[j].get("content").cloned().unwrap_or(Value::Null),
                        );
                        if raw_text.len() > size_threshold {
                            let placeholder = self
                                .config
                                .tool_prune_placeholder
                                .replace("{n}", &raw_text.len().to_string());
                            arr[j]["content"] = Value::String(placeholder);
                            pruned += 1;
                        }
                    }
                }
                messages[i]["content"] = Value::Array(arr);
            } else if role == "tool" {
                if let Some(s) = content.and_then(|c| c.as_str()) {
                    if s.len() > size_threshold {
                        let placeholder = self
                            .config
                            .tool_prune_placeholder
                            .replace("{n}", &s.len().to_string());
                        messages[i]["content"] = Value::String(placeholder);
                        pruned += 1;
                    }
                }
            }
        }

        (messages, pruned)
    }

    // ── Strategy 3: Thought Block Compression ──

    #[allow(clippy::needless_range_loop)]
    fn prune_thought_blocks(&self, mut messages: Vec<Value>) -> (Vec<Value>, usize) {
        let total = messages.len();
        let cutoff = total.saturating_sub(self.config.thought_prune_after_turns * 2);
        let mut pruned = 0;

        let replacement = format!("<thought>{}</thought>", self.config.thought_placeholder);

        for i in 0..cutoff.min(total) {
            if messages[i].get("role").and_then(|r| r.as_str()) != Some("assistant") {
                continue;
            }

            let content = messages[i].get("content").cloned();

            match content {
                Some(Value::String(s)) if THOUGHT_RE.is_match(&s) => {
                    let original_len = s.len();
                    let replaced = THOUGHT_RE.replace_all(&s, replacement.as_str()).to_string();
                    if replaced.len() < original_len {
                        messages[i]["content"] = Value::String(replaced);
                        pruned += 1;
                    }
                }
                Some(Value::Array(arr)) => {
                    let mut arr = arr;
                    for j in 0..arr.len() {
                        if arr[j].get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(text) = arr[j].get("text").and_then(|t| t.as_str()) {
                                if THOUGHT_RE.is_match(text) {
                                    let original_len = text.len();
                                    let replaced = THOUGHT_RE
                                        .replace_all(text, replacement.as_str())
                                        .to_string();
                                    if replaced.len() < original_len {
                                        arr[j]["text"] = Value::String(replaced);
                                        pruned += 1;
                                    }
                                }
                            }
                        }
                    }
                    messages[i]["content"] = Value::Array(arr);
                }
                _ => {}
            }
        }

        (messages, pruned)
    }

    // ── Strategy 4: Error Deduplication ──

    #[allow(clippy::needless_range_loop, clippy::map_entry)]
    fn dedup_errors(&self, mut messages: Vec<Value>) -> (Vec<Value>, usize) {
        let mut deduped = 0;
        let mut seen_hashes: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for i in 0..messages.len() {
            let content = messages[i].get("content").cloned();
            let text = extract_text_from_value(&content.unwrap_or(Value::Null));

            if !TRACEBACK_RE.is_match(&text) {
                continue;
            }

            let traces: Vec<&str> = TRACEBACK_RE.find_iter(&text).map(|m| m.as_str()).collect();

            for trace in &traces {
                let mut hasher = Md5::new();
                hasher.update(trace.trim().as_bytes());
                let hash = format!("{:x}", hasher.finalize())[..12].to_string();

                if seen_hashes.contains_key(&hash) {
                    // Replace with dedup placeholder
                    let replacement = &self.config.dedup_placeholder;
                    match messages[i].get("content") {
                        Some(Value::String(s)) => {
                            messages[i]["content"] = Value::String(
                                TRACEBACK_RE.replace(s, replacement.as_str()).to_string(),
                            );
                        }
                        Some(Value::Array(arr)) => {
                            let mut arr = arr.clone();
                            for j in 0..arr.len() {
                                if arr[j].get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(t) = arr[j].get("text").and_then(|t| t.as_str()) {
                                        arr[j]["text"] = Value::String(
                                            TRACEBACK_RE
                                                .replace(t, replacement.as_str())
                                                .to_string(),
                                        );
                                    }
                                }
                            }
                            messages[i]["content"] = Value::Array(arr);
                        }
                        _ => {}
                    }
                    deduped += 1;
                } else {
                    seen_hashes.insert(hash, i);
                    // Strip system paths even on first occurrence
                    match messages[i].get("content") {
                        Some(Value::String(s)) => {
                            messages[i]["content"] =
                                Value::String(SYS_PATH_RE.replace_all(s, "").to_string());
                        }
                        Some(Value::Array(arr)) => {
                            let mut arr = arr.clone();
                            for j in 0..arr.len() {
                                if arr[j].get("type").and_then(|t| t.as_str()) == Some("text") {
                                    if let Some(t) = arr[j].get("text").and_then(|t| t.as_str()) {
                                        arr[j]["text"] = Value::String(
                                            SYS_PATH_RE.replace_all(t, "").to_string(),
                                        );
                                    }
                                }
                            }
                            messages[i]["content"] = Value::Array(arr);
                        }
                        _ => {}
                    }
                }
            }
        }

        (messages, deduped)
    }

    // ── Strategy 6: File View Condensation ──

    #[allow(clippy::needless_range_loop)]
    fn condense_file_views(&self, mut messages: Vec<Value>) -> (Vec<Value>, usize) {
        let mut condensed = 0;

        for i in 0..messages.len() {
            let content = messages[i].get("content").cloned();
            if let Some(Value::Array(arr)) = content {
                let mut arr = arr;
                for j in 0..arr.len() {
                    let is_file_result = arr[j].get("type").and_then(|t| t.as_str())
                        == Some("tool_result")
                        || (arr[j].get("type").and_then(|t| t.as_str()) == Some("text")
                            && messages[i].get("role").and_then(|r| r.as_str()) == Some("tool"));

                    if !is_file_result {
                        continue;
                    }

                    let raw = arr[j].get("content").cloned().unwrap_or(Value::Null);
                    let mut text = extract_text_from_content(&raw);
                    if text.len() < 100 {
                        continue;
                    }

                    let original_len = text.len();

                    // Strip docstrings
                    if self.config.strip_docstrings {
                        text = DOCSTRING_RE.replace_all(&text, r#""""""#).to_string();
                    }

                    // Strip trailing comments
                    text = TRAILING_COMMENT_RE.replace_all(&text, "").to_string();

                    // Collapse multiple blank lines
                    if self.config.strip_blank_lines {
                        text = MULTI_BLANK_RE.replace_all(&text, "\n\n").to_string();
                    }

                    // Minify embedded JSON
                    if self.config.minify_json_in_results {
                        text = minify_embedded_json(&text);
                    }

                    if text.len() < original_len {
                        match &raw {
                            Value::String(_) => {
                                arr[j]["content"] = Value::String(text);
                            }
                            Value::Array(sub_arr) => {
                                let mut sub = sub_arr.clone();
                                for k in 0..sub.len() {
                                    if sub[k].get("type").and_then(|t| t.as_str()) == Some("text") {
                                        sub[k]["text"] = Value::String(text.clone());
                                        break;
                                    }
                                }
                                arr[j]["content"] = Value::Array(sub);
                            }
                            _ => {
                                arr[j]["content"] = Value::String(text);
                            }
                        }
                        condensed += 1;
                    }
                }
                messages[i]["content"] = Value::Array(arr);
            }
        }

        (messages, condensed)
    }

    // ── Strategy 2: Schema Minimization ──

    fn minimize_schemas(&mut self, tools: &[Value]) -> Vec<Value> {
        if self.config.schema_first_call_full && !self.schema_seen {
            self.schema_seen = true;
            return tools.to_vec();
        }

        tools
            .iter()
            .map(|tool| {
                let mut tool = tool.clone();
                let name = tool
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("fn")
                    .to_string();

                // Keep first sentence of description only
                if let Some(desc) = tool.get("description").and_then(|d| d.as_str()) {
                    let first_sentence = if let Some(pos) = desc.find(". ") {
                        &desc[..pos + 1]
                    } else if desc.len() > 80 {
                        &desc[..80]
                    } else {
                        desc
                    };
                    tool["description"] = Value::String(first_sentence.to_string());
                }

                // Convert schema to compact TypeScript-style signature
                let schema_key = if tool.get("input_schema").is_some() {
                    "input_schema"
                } else {
                    "parameters"
                };

                if let Some(schema) = tool.get(schema_key) {
                    let ts_sig = format::schema_to_typescript(&name, schema);
                    // Replace full schema with compact signature as description suffix
                    let desc = tool
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    tool["description"] = Value::String(format!("{} Sig: {}", desc, ts_sig));

                    // Minimize the schema: strip descriptions but keep structure
                    // (provider still needs valid JSON Schema for tool_use validation)
                    if let Some(props) = tool
                        .get_mut(schema_key)
                        .and_then(|s| s.get_mut("properties"))
                        .and_then(|p| p.as_object_mut())
                    {
                        for (_name, prop_def) in props.iter_mut() {
                            if let Some(obj) = prop_def.as_object_mut() {
                                obj.remove("description");
                                obj.remove("examples");
                                obj.remove("default");
                            }
                        }
                    }
                }

                tool
            })
            .collect()
    }

    // ── Strategy 5: Cache Control Injection ──

    fn inject_cache_headers(&self, system: &Value) -> Value {
        match system {
            Value::String(s) => {
                json!([{
                    "type": "text",
                    "text": s,
                    "cache_control": {"type": "ephemeral"}
                }])
            }
            Value::Array(arr) if !arr.is_empty() => {
                let mut arr = arr.clone();
                if let Some(last) = arr.last_mut() {
                    if let Some(obj) = last.as_object_mut() {
                        obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                    }
                }
                Value::Array(arr)
            }
            other => other.clone(),
        }
    }

    // ── Strategy 7: Sliding Window ──

    #[allow(clippy::needless_range_loop)]
    fn apply_sliding_window(
        &self,
        mut messages: Vec<Value>,
        _current_tokens: usize,
        _threshold: usize,
    ) -> Vec<Value> {
        let total = messages.len();
        let preserve = (self.config.sliding_window_preserve_turns * 2).min(total);

        // Phase 1: Nuke all tool results older than N turns
        let tool_cutoff = total.saturating_sub(self.config.sliding_window_tool_prune_age * 2);
        for i in 0..tool_cutoff {
            let role = messages[i]
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("");

            if let Some(arr) = messages[i].get("content").and_then(|c| c.as_array()) {
                let mut arr = arr.clone();
                for j in 0..arr.len() {
                    if arr[j].get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        arr[j]["content"] = Value::String(
                            "[Nyquest: old tool output purged — sliding window]".into(),
                        );
                    }
                }
                messages[i]["content"] = Value::Array(arr);
            } else if role == "tool" {
                messages[i]["content"] =
                    Value::String("[Nyquest: old tool output purged — sliding window]".into());
            }
        }

        // Phase 2: Nuke all thought blocks older than N turns
        let thought_cutoff = total.saturating_sub(self.config.sliding_window_thought_prune_age * 2);
        for i in 0..thought_cutoff {
            if messages[i].get("role").and_then(|r| r.as_str()) != Some("assistant") {
                continue;
            }
            if let Some(Value::String(s)) = messages[i].get("content").cloned() {
                messages[i]["content"] = Value::String(THOUGHT_RE.replace_all(&s, "").to_string());
            }
        }

        // Phase 3: If still large, drop oldest messages keeping preserved window
        if total > preserve + 4 {
            let split = total - preserve;
            let older = &messages[..split];
            let recent = messages[split..].to_vec();

            // Build emergency summary from last 6 of the old messages
            let summary_start = older.len().saturating_sub(6);
            let mut summary_parts = Vec::new();
            for msg in &older[summary_start..] {
                let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                let text = extract_text_from_value(msg.get("content").unwrap_or(&Value::Null));
                let truncated: String = text.chars().take(100).collect();
                if !truncated.trim().is_empty() {
                    summary_parts.push(format!("{}: {}", role, truncated));
                }
            }

            if !summary_parts.is_empty() {
                let summary_msg = json!({
                    "role": "user",
                    "content": format!(
                        "[Nyquest: Sliding window active — older messages truncated]\nRecent context summary:\n{}",
                        summary_parts.join("\n")
                    )
                });
                let ack_msg = json!({
                    "role": "assistant",
                    "content": "Understood. Continuing with current task context."
                });

                let mut result = vec![summary_msg, ack_msg];
                result.extend(recent);
                return result;
            } else {
                return recent;
            }
        }

        messages
    }
}

// ── Helper functions ──

#[derive(Default)]
struct Stats {
    tool_results_pruned: usize,
    thoughts_pruned: usize,
    errors_deduped: usize,
    file_views_condensed: usize,
    chars_saved: usize,
}

impl Stats {
    fn any_work(&self) -> bool {
        self.tool_results_pruned > 0
            || self.thoughts_pruned > 0
            || self.errors_deduped > 0
            || self.file_views_condensed > 0
            || self.chars_saved > 0
    }
}

fn extract_text_from_content(content: &Value) -> String {
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
                } else if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    Some(extract_text_from_content(
                        block.get("content").unwrap_or(&Value::Null),
                    ))
                } else {
                    block.as_str().map(|s| s.to_string())
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn extract_text_from_value(value: &Value) -> String {
    extract_text_from_content(value)
}

fn count_chars(messages: &[Value]) -> usize {
    messages
        .iter()
        .map(|msg| extract_text_from_value(msg.get("content").unwrap_or(&Value::Null)).len())
        .sum()
}

fn minify_embedded_json(text: &str) -> String {
    JSON_BLOCK_RE
        .replace_all(text, |caps: &regex::Captures| {
            match serde_json::from_str::<Value>(&caps[1]) {
                Ok(parsed) => {
                    format!(
                        "```json\n{}\n```",
                        serde_json::to_string(&parsed).unwrap_or_else(|_| caps[0].to_string())
                    )
                }
                Err(_) => caps[0].to_string(),
            }
        })
        .to_string()
}
