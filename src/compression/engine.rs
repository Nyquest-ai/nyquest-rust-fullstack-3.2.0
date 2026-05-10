//! Nyquest Compression Engine
//! Applies tiered compression to LLM API requests.
//! Supports per-model profiles for tuning rule aggressiveness.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::format;
use super::rules::{
    self, ADJECTIVE_COLLAPSE, ADVERB_STRIP, AI_OUTPUT_NOISE, ANTI_NOISE, CLAUSE_COLLAPSE,
    CLAUSE_SIMPLIFY, CONTEXT_DEDUPLICATION, CONVERSATIONAL_STRIP, CREDENTIAL_STRIP,
    DEVELOPER_BOILERPLATE, DISCLAIMER_COLLAPSE, FILLER_PHRASES, IMPERATIVE_CONVERSIONS,
    MARKDOWN_MINIFICATION, OPENCLAW_RULES, SEMANTIC_FORMATTING, SOURCE_CODE_COMPRESSION,
    VERBOSE_PHRASES, WHITESPACE_CLEANUP,
};
use super::telegraph;
use crate::profiles::ModelProfile;

/// Decide whether `SOURCE_CODE_COMPRESSION` rules are safe for this text.
///
/// Those rules collapse JSON/code-style whitespace and strip lines beginning
/// with `#` or `//`. Applied to prose they would corrupt markdown headings
/// (`# Heading`), URL fragments, semicolon-separated clauses, Python tuples
/// (`('foo',)`), etc. So fail closed: only allow on content we can confirm
/// is code or JSON.
fn is_code_or_json_content(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Fenced code block at start — minify-style content.
    if trimmed.starts_with("```") {
        return true;
    }

    // Confirmed JSON object or array.
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if serde_json::from_str::<Value>(trimmed).is_ok() {
            return true;
        }
    }

    // Detected source language (multi-signal heuristics in minify::detect_language).
    if super::minify::detect_language(text).is_some() {
        return true;
    }

    false
}

static RE_DOUBLE_NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{2,}").unwrap());
static RE_EMPHASIS_STRIP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\*{1,2}(?:Important|Note|Warning|Caution)\*{1,2}:?\s*").unwrap());
static RE_LIST_VERBOSITY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)(\d+\.)\s*(?:First|Second|Third|Fourth|Fifth|Next|Then|Finally|Lastly)\s*,?\s*",
    )
    .unwrap()
});

/// Roles that are safe to compress (includes "tool" for OpenAI-format tool results)
const COMPRESSIBLE_ROLES: &[&str] = &["user", "system", "tool"];

/// Per-category hit counters for compression rules
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CompressionStats {
    pub openclaw_rules: usize,
    pub filler_phrases: usize,
    pub verbose_phrases: usize,
    pub imperative_conversions: usize,
    pub clause_collapse: usize,
    pub developer_boilerplate: usize,
    pub semantic_formatting: usize,
    pub credential_strip: usize,
    pub whitespace_cleanup: usize,
    pub conversational_strip: usize,
    pub ai_output_noise: usize,
    pub markdown_minification: usize,
    pub source_code_compression: usize,
    pub context_deduplication: usize,
    pub anti_noise: usize,
    pub disclaimer_collapse: usize,
    pub adjective_collapse: usize,
    pub clause_simplify: usize,
    pub adverb_strip: usize,
    pub total_rule_hits: usize,
}

impl CompressionStats {
    /// Merge another stats instance into this one (used for multi-pass aggregation).
    pub fn merge(&mut self, other: &CompressionStats) {
        self.openclaw_rules += other.openclaw_rules;
        self.filler_phrases += other.filler_phrases;
        self.verbose_phrases += other.verbose_phrases;
        self.imperative_conversions += other.imperative_conversions;
        self.clause_collapse += other.clause_collapse;
        self.developer_boilerplate += other.developer_boilerplate;
        self.semantic_formatting += other.semantic_formatting;
        self.credential_strip += other.credential_strip;
        self.whitespace_cleanup += other.whitespace_cleanup;
        self.conversational_strip += other.conversational_strip;
        self.ai_output_noise += other.ai_output_noise;
        self.markdown_minification += other.markdown_minification;
        self.source_code_compression += other.source_code_compression;
        self.context_deduplication += other.context_deduplication;
        self.anti_noise += other.anti_noise;
        self.disclaimer_collapse += other.disclaimer_collapse;
        self.adjective_collapse += other.adjective_collapse;
        self.clause_simplify += other.clause_simplify;
        self.adverb_strip += other.adverb_strip;
        self.total_rule_hits += other.total_rule_hits;
    }

    /// Return top N categories by hit count
    pub fn top_categories(&self, n: usize) -> Vec<(String, usize)> {
        let mut cats = vec![
            ("openclaw_rules".into(), self.openclaw_rules),
            ("filler_phrases".into(), self.filler_phrases),
            ("verbose_phrases".into(), self.verbose_phrases),
            ("imperative_conversions".into(), self.imperative_conversions),
            ("clause_collapse".into(), self.clause_collapse),
            ("developer_boilerplate".into(), self.developer_boilerplate),
            ("semantic_formatting".into(), self.semantic_formatting),
            ("credential_strip".into(), self.credential_strip),
            ("whitespace_cleanup".into(), self.whitespace_cleanup),
            ("conversational_strip".into(), self.conversational_strip),
            ("ai_output_noise".into(), self.ai_output_noise),
            ("markdown_minification".into(), self.markdown_minification),
            (
                "source_code_compression".into(),
                self.source_code_compression,
            ),
            ("context_deduplication".into(), self.context_deduplication),
            ("anti_noise".into(), self.anti_noise),
            ("disclaimer_collapse".into(), self.disclaimer_collapse),
            ("adjective_collapse".into(), self.adjective_collapse),
            ("clause_simplify".into(), self.clause_simplify),
            ("adverb_strip".into(), self.adverb_strip),
        ];
        cats.sort_by(|a, b| b.1.cmp(&a.1));
        cats.into_iter().take(n).collect()
    }
}

/// Multi-level compression engine for LLM prompts.
/// Accepts an optional ModelProfile to tune rule thresholds per model family.
pub struct CompressionEngine {
    pub level: f64,
    pub stats: CompressionStats,
    pub profile: &'static ModelProfile,
}

impl CompressionEngine {
    pub fn new(level: f64) -> Self {
        Self {
            level: level.clamp(0.0, 1.0),
            stats: CompressionStats::default(),
            profile: crate::profiles::get_profile("aggressive"),
        }
    }

    pub fn with_profile(level: f64, profile: &'static ModelProfile) -> Self {
        Self {
            level: level.clamp(0.0, 1.0),
            stats: CompressionStats::default(),
            profile,
        }
    }

    /// Helper: apply a rule category with counting + RegexSet prefilter.
    /// Per-rule and per-category hit counters live on the category itself
    /// (atomic), so this just routes the *category-level* hit count into the
    /// engine's per-request `CompressionStats` for the response payload.
    fn apply_counted(&mut self, text: &str, category: &rules::RuleCategory, field: &str) -> String {
        let (result, hits) = rules::apply_category_counted(text, category);
        if hits > 0 {
            match field {
                "openclaw_rules" => self.stats.openclaw_rules += hits,
                "filler_phrases" => self.stats.filler_phrases += hits,
                "verbose_phrases" => self.stats.verbose_phrases += hits,
                "imperative_conversions" => self.stats.imperative_conversions += hits,
                "clause_collapse" => self.stats.clause_collapse += hits,
                "developer_boilerplate" => self.stats.developer_boilerplate += hits,
                "semantic_formatting" => self.stats.semantic_formatting += hits,
                "credential_strip" => self.stats.credential_strip += hits,
                "whitespace_cleanup" => self.stats.whitespace_cleanup += hits,
                "conversational_strip" => self.stats.conversational_strip += hits,
                "ai_output_noise" => self.stats.ai_output_noise += hits,
                "markdown_minification" => self.stats.markdown_minification += hits,
                "source_code_compression" => self.stats.source_code_compression += hits,
                "context_deduplication" => self.stats.context_deduplication += hits,
                "anti_noise" => self.stats.anti_noise += hits,
                "disclaimer_collapse" => self.stats.disclaimer_collapse += hits,
                "adjective_collapse" => self.stats.adjective_collapse += hits,
                "clause_simplify" => self.stats.clause_simplify += hits,
                "adverb_strip" => self.stats.adverb_strip += hits,
                _ => {}
            }
            self.stats.total_rule_hits += hits;
        }
        result.into_owned()
    }

    /// Apply compression rules to a text string based on current level and model profile.
    /// Profile thresholds control when each category fires — conservative profiles
    /// raise thresholds so risky rewrites only engage at higher levels.
    pub fn compress_text(&mut self, text: &str) -> String {
        if self.level == 0.0 {
            return text.to_string();
        }

        // Bounded LRU cache: agent conversations resend identical history every
        // turn — a hit returns the previously computed output without running
        // the rule pipeline. Key includes level + profile + mode.
        if let Some((cached, hits)) = super::cache::GLOBAL_CACHE.get(
            text,
            self.level,
            self.profile.name,
            super::cache::Mode::Request,
        ) {
            self.stats.total_rule_hits += hits;
            return cached;
        }

        let mut result = text.to_string();
        let p = self.profile;

        // Always: OpenClaw-specific metadata/boilerplate removal
        result = self.apply_counted(&result, &OPENCLAW_RULES, "openclaw_rules");
        // Per-category cleanup intentionally absent here — one final pass at end.

        // Tier 1: Filler removal + synonyms (profile-controlled thresholds)
        if self.level >= p.filler_phrases {
            result = rules::apply_whitespace_normalization(&result);
            result = self.apply_counted(&result, &FILLER_PHRASES, "filler_phrases");
        }
        if self.level >= p.verbose_phrases {
            result = self.apply_counted(&result, &VERBOSE_PHRASES, "verbose_phrases");
        }

        // Tier 2: Structural compression (profile-controlled)
        if self.level >= p.imperative_conversions {
            result = self.apply_counted(&result, &IMPERATIVE_CONVERSIONS, "imperative_conversions");
        }
        if self.level >= p.clause_collapse {
            result = self.apply_counted(&result, &CLAUSE_COLLAPSE, "clause_collapse");
        }
        if self.level >= p.developer_boilerplate {
            result = self.apply_counted(&result, &DEVELOPER_BOILERPLATE, "developer_boilerplate");
        }
        if self.level >= p.semantic_formatting {
            result = rules::compress_dates(&result);
            result = self.apply_counted(&result, &SEMANTIC_FORMATTING, "semantic_formatting");
        }
        if self.level >= p.credential_strip {
            result = self.apply_counted(&result, &CREDENTIAL_STRIP, "credential_strip");
        }
        if self.level >= p.whitespace_cleanup {
            result = self.apply_counted(&result, &WHITESPACE_CLEANUP, "whitespace_cleanup");
            result = RE_DOUBLE_NEWLINE.replace_all(&result, "\n").to_string();
        }

        // Tier 3: Aggressive canonical compression (profile-controlled)
        if self.level >= p.conversational_strip {
            result = self.apply_counted(&result, &CONVERSATIONAL_STRIP, "conversational_strip");
        }
        if self.level >= p.ai_output_noise {
            result = self.apply_counted(&result, &AI_OUTPUT_NOISE, "ai_output_noise");
        }
        if self.level >= p.markdown_minification {
            result = self.apply_counted(&result, &MARKDOWN_MINIFICATION, "markdown_minification");
        }
        if self.level >= p.source_code_compression && is_code_or_json_content(&result) {
            // Scoped: SOURCE_CODE_COMPRESSION mutates JSON/code-only whitespace
            // (`,\s*([}\]])`, `:\s+(\d)`, `;\s+`, `\{\s+`, `\s+\}`) and strips
            // `#`/`//` comment lines. On prose this corrupts markdown headings,
            // numeric prose ("Result: 42"), and semicolon-separated clauses.
            result =
                self.apply_counted(&result, &SOURCE_CODE_COMPRESSION, "source_code_compression");
        }
        if self.level >= p.context_deduplication {
            result = self.apply_counted(&result, &CONTEXT_DEDUPLICATION, "context_deduplication");
        }
        if self.level >= p.anti_noise {
            result = self.apply_counted(&result, &ANTI_NOISE, "anti_noise");
        }
        if self.level >= p.disclaimer_collapse {
            result = self.apply_counted(&result, &DISCLAIMER_COLLAPSE, "disclaimer_collapse");
        }
        if self.level >= p.adjective_collapse {
            result = self.apply_counted(&result, &ADJECTIVE_COLLAPSE, "adjective_collapse");
        }
        if self.level >= p.clause_simplify {
            result = self.apply_counted(&result, &CLAUSE_SIMPLIFY, "clause_simplify");
        }
        if self.level >= p.adverb_strip {
            result = self.apply_counted(&result, &ADVERB_STRIP, "adverb_strip");
            result = self.apply_counted(&result, &WHITESPACE_CLEANUP, "whitespace_cleanup");
        }
        if self.level >= p.code_minify {
            result = rules::minify_json_payload(&result);
            result = super::minify::minify_code_block(&result);
        }
        if self.level >= p.format_optimize {
            result = format::compact_json_blocks(&result);
            result = format::flatten_markdown_tables(&result);
        }
        // Emphasis strip and list collapse at 0.8+ (all profiles)
        if self.level >= 0.8 {
            result = RE_EMPHASIS_STRIP.replace_all(&result, "").to_string();
            result = self.collapse_numbered_lists(&result);
            result = RE_DOUBLE_NEWLINE.replace_all(&result, "\n").to_string();
        }

        // Sentence-level telegraph compression (profile-controlled level + intensity)
        if self.level >= p.telegraph {
            let effective_level = p.effective_telegraph_level(self.level);
            result = telegraph::telegraph_compress(&result, effective_level);
        }

        // Final cleanup — single end-of-pipeline pass.
        result = rules::apply_whitespace_normalization(&result);

        // Fail-closed: never return a longer string than the input.
        if result.len() > text.len() {
            return text.to_string();
        }

        // Cache the result so repeated identical text hits next time.
        super::cache::GLOBAL_CACHE.put(
            text,
            self.level,
            self.profile.name,
            super::cache::Mode::Request,
            result.clone(),
            self.stats.total_rule_hits,
        );

        result
    }

    /// Compress assistant responses in older turns.
    /// Progressive tiers: always strips AI noise, level 0.5+ adds markdown/whitespace,
    /// level 0.8+ adds fillers/adverbs/code minify/format optimization/telegraph.
    pub fn compress_response_text(&mut self, text: &str) -> String {
        if self.level == 0.0 {
            return text.to_string();
        }

        // Cache check (response mode — distinct key from request).
        if let Some((cached, hits)) = super::cache::GLOBAL_CACHE.get(
            text,
            self.level,
            self.profile.name,
            super::cache::Mode::Response,
        ) {
            self.stats.total_rule_hits += hits;
            return cached;
        }

        let mut result = text.to_string();

        // Always: Strip AI output noise ("I'd be happy to help!", "Great question!")
        result = self.apply_counted(&result, &AI_OUTPUT_NOISE, "ai_output_noise");

        // Whitespace normalization
        result = rules::apply_whitespace_normalization(&result);

        // Level 0.5+: markdown minification, whitespace cleanup, inline JSON compaction
        if self.level >= 0.5 {
            result = self.apply_counted(&result, &MARKDOWN_MINIFICATION, "markdown_minification");
            result = self.apply_counted(&result, &WHITESPACE_CLEANUP, "whitespace_cleanup");
            result = format::compact_inline_json(&result, 150);
            result = RE_DOUBLE_NEWLINE.replace_all(&result, "\n").to_string();
        }

        // Level 0.8+: filler/verbose stripping, code minification, format optimization, telegraph
        if self.level >= 0.8 {
            result = self.apply_counted(&result, &FILLER_PHRASES, "filler_phrases");
            result = self.apply_counted(&result, &VERBOSE_PHRASES, "verbose_phrases");
            result = self.apply_counted(&result, &ADVERB_STRIP, "adverb_strip");
            result = self.apply_counted(&result, &CONVERSATIONAL_STRIP, "conversational_strip");
            // Code minification in assistant code blocks
            result = super::minify::minify_code_block(&result);
            // JSON→YAML/CSV conversion in assistant responses
            result = format::compact_json_blocks(&result);
            result = format::flatten_markdown_tables(&result);
            // Sentence-level telegraph compression
            result = telegraph::telegraph_compress(&result, self.level * 0.7);
            result = self.apply_counted(&result, &WHITESPACE_CLEANUP, "whitespace_cleanup");
            result = RE_DOUBLE_NEWLINE.replace_all(&result, "\n").to_string();
        }

        // Final cleanup
        result = rules::apply_whitespace_normalization(&result);

        if result.len() > text.len() {
            return text.to_string();
        }

        super::cache::GLOBAL_CACHE.put(
            text,
            self.level,
            self.profile.name,
            super::cache::Mode::Response,
            result.clone(),
            self.stats.total_rule_hits,
        );

        result
    }

    fn collapse_numbered_lists(&self, text: &str) -> String {
        RE_LIST_VERBOSITY.replace_all(text, "$1 ").to_string()
    }

    /// Compress a single content block
    pub fn compress_content_block(&mut self, block: &Value) -> Value {
        match block {
            Value::String(s) => Value::String(self.compress_text(s)),
            Value::Object(map) => {
                let block_type = map.get("type").and_then(|t| t.as_str()).unwrap_or("");

                // tool_use blocks: never touch (they have structured input)
                if block_type == "tool_use" || block_type == "image" {
                    return block.clone();
                }

                let mut map = map.clone();

                // tool_result blocks: compress the nested text content + JSON compaction
                if block_type == "tool_result" {
                    if let Some(content) = map.get("content").cloned() {
                        let compressed_content = match content {
                            Value::String(s) => {
                                let text = self.compress_text(&s);
                                // Try inline JSON compaction for tool results (often raw JSON)
                                if self.level >= 0.5 {
                                    Value::String(format::compact_inline_json(&text, 150))
                                } else {
                                    Value::String(text)
                                }
                            }
                            Value::Array(arr) => {
                                let compressed: Vec<Value> = arr
                                    .iter()
                                    .map(|inner| self.compress_content_block(inner))
                                    .collect();
                                Value::Array(compressed)
                            }
                            other => other,
                        };
                        map.insert("content".to_string(), compressed_content);
                    }
                    return Value::Object(map);
                }

                // text blocks: compress the text field
                if block_type == "text" {
                    if let Some(Value::String(text)) = map.get("text") {
                        map.insert("text".to_string(), Value::String(self.compress_text(text)));
                    }
                }

                Value::Object(map)
            }
            _ => block.clone(),
        }
    }

    /// Compress a content block from an assistant response (lighter rules)
    fn compress_response_content_block(&mut self, block: &Value) -> Value {
        match block {
            Value::String(s) => Value::String(self.compress_response_text(s)),
            Value::Object(map) => {
                let block_type = map.get("type").and_then(|t| t.as_str()).unwrap_or("");

                // Never touch tool_use, image, or tool_result in assistant responses
                if block_type == "tool_use" || block_type == "image" || block_type == "tool_result"
                {
                    return block.clone();
                }

                let mut map = map.clone();
                if block_type == "text" {
                    if let Some(Value::String(text)) = map.get("text") {
                        map.insert(
                            "text".to_string(),
                            Value::String(self.compress_response_text(text)),
                        );
                    }
                }
                Value::Object(map)
            }
            _ => block.clone(),
        }
    }

    /// Compress a single message object
    pub fn compress_message(&mut self, message: &Value) -> Value {
        let mut msg = message.clone();
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

        if !COMPRESSIBLE_ROLES.contains(&role) {
            return msg;
        }

        if let Some(content) = msg.get("content").cloned() {
            match content {
                Value::String(s) => {
                    msg.as_object_mut()
                        .unwrap()
                        .insert("content".to_string(), Value::String(self.compress_text(&s)));
                }
                Value::Array(arr) => {
                    let compressed: Vec<Value> = arr
                        .iter()
                        .map(|block| self.compress_content_block(block))
                        .collect();
                    msg.as_object_mut()
                        .unwrap()
                        .insert("content".to_string(), Value::Array(compressed));
                }
                _ => {}
            }
        }

        msg
    }

    /// Compress an assistant message (lighter rules, for response compression)
    pub fn compress_response_message(&mut self, message: &Value) -> Value {
        let mut msg = message.clone();

        if let Some(content) = msg.get("content").cloned() {
            match content {
                Value::String(s) => {
                    msg.as_object_mut().unwrap().insert(
                        "content".to_string(),
                        Value::String(self.compress_response_text(&s)),
                    );
                }
                Value::Array(arr) => {
                    let compressed: Vec<Value> = arr
                        .iter()
                        .map(|block| self.compress_response_content_block(block))
                        .collect();
                    msg.as_object_mut()
                        .unwrap()
                        .insert("content".to_string(), Value::Array(compressed));
                }
                _ => {}
            }
        }

        msg
    }

    /// Compress the system message
    pub fn compress_system(&mut self, system: &Value) -> Value {
        match system {
            Value::String(s) => Value::String(self.compress_text(s)),
            Value::Array(arr) => {
                let compressed: Vec<Value> = arr
                    .iter()
                    .map(|block| self.compress_content_block(block))
                    .collect();
                Value::Array(compressed)
            }
            Value::Null => Value::Null,
            _ => system.clone(),
        }
    }
}

/// Result of compress_request including response compression tracking
pub struct CompressionResult {
    pub body: Value,
    pub stats: CompressionStats,
    pub response_tokens_saved: usize,
    pub responses_compressed: usize,
}

/// Compress an Anthropic Messages API request body.
///
/// Pipeline: Normalize → Compress
/// Returns CompressionResult with body, stats, and response compression tracking
pub fn compress_request(
    request_body: &Value,
    level: f64,
    normalize: bool,
    inject_boundaries: bool,
    compress_responses: bool,
    response_age: usize,
    model: &str,
) -> (Value, CompressionStats, usize, usize) {
    if level == 0.0 && !normalize {
        return (request_body.clone(), CompressionStats::default(), 0, 0);
    }

    let mut result = request_body.clone();
    let mut response_tokens_saved: usize = 0;
    let mut responses_compressed: usize = 0;

    // Phase 1: Normalize (hallucination mitigation)
    if normalize {
        result = crate::normalizer::normalize_request(&result, inject_boundaries);
    }

    // Phase 2: Compress (token reduction) with model-aware profile
    if level > 0.0 {
        let profile = crate::profiles::detect_profile(model);
        let mut engine = CompressionEngine::with_profile(level, profile);
        let tc = crate::tokens::TokenCounter::new();

        if let Some(system) = result.get("system").cloned() {
            let compressed = engine.compress_system(&system);
            result
                .as_object_mut()
                .unwrap()
                .insert("system".to_string(), compressed);
        }

        if let Some(Value::Array(messages)) = result.get("messages").cloned() {
            let total = messages.len();
            // Determine response compression cutoff: compress assistant messages
            // older than response_age turns from the end
            let response_cutoff = if compress_responses {
                total.saturating_sub(response_age * 2)
            } else {
                0
            };

            let compressed: Vec<Value> = messages
                .iter()
                .enumerate()
                .map(|(i, msg)| {
                    let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");

                    if role == "assistant" && compress_responses && i < response_cutoff {
                        // Track response compression savings
                        let before = tc.count_message_tokens(msg);
                        let compressed_msg = engine.compress_response_message(msg);
                        let after = tc.count_message_tokens(&compressed_msg);
                        let saved = before.saturating_sub(after);
                        if saved > 0 {
                            response_tokens_saved += saved;
                            responses_compressed += 1;
                        }
                        compressed_msg
                    } else {
                        engine.compress_message(msg)
                    }
                })
                .collect();
            result
                .as_object_mut()
                .unwrap()
                .insert("messages".to_string(), Value::Array(compressed));
        }

        return (
            result,
            engine.stats,
            response_tokens_saved,
            responses_compressed,
        );
    }

    (result, CompressionStats::default(), 0, 0)
}
