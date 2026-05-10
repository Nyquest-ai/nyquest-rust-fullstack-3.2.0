//! Nyquest Model Profiles
//! Per-model-family compression tuning. Adjusts which rule categories
//! fire at which levels based on model capabilities.
//!
//! Larger, more capable models (Claude Opus, GPT-4o) handle aggressive
//! compression well. Smaller models (Haiku, Gemini Flash, Grok Mini)
//! can lose coherence with aggressive structural rewrites.

use serde::{Deserialize, Serialize};

/// Compression profile — controls per-category level thresholds.
/// Each field is the minimum compression level at which that category fires.
/// Default profile uses the standard thresholds from the engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProfile {
    pub name: &'static str,
    /// Filler phrases: "due to the fact that" → "because"
    pub filler_phrases: f64,
    /// Verbose phrases: "your primary responsibility is to" → removed
    pub verbose_phrases: f64,
    /// Imperative: "you should always" → "always"
    pub imperative_conversions: f64,
    /// Clause collapse: "in situations where" → "when"
    pub clause_collapse: f64,
    /// Dev boilerplate: TODO/FIXME strip
    pub developer_boilerplate: f64,
    /// Semantic formatting: "for example" → "e.g."
    pub semantic_formatting: f64,
    /// Credential stripping
    pub credential_strip: f64,
    /// Whitespace cleanup
    pub whitespace_cleanup: f64,
    /// Conversational strip: "As an AI language model..."
    pub conversational_strip: f64,
    /// AI output noise: "I'd be happy to help!"
    pub ai_output_noise: f64,
    /// Markdown minification
    pub markdown_minification: f64,
    /// Source code compression
    pub source_code_compression: f64,
    /// Context deduplication
    pub context_deduplication: f64,
    /// Anti-noise patterns
    pub anti_noise: f64,
    /// Disclaimer collapse
    pub disclaimer_collapse: f64,
    /// Adjective collapse
    pub adjective_collapse: f64,
    /// Clause simplify
    pub clause_simplify: f64,
    /// Adverb strip
    pub adverb_strip: f64,
    /// Telegraph sentence compression
    pub telegraph: f64,
    /// Code minification (minify.rs)
    pub code_minify: f64,
    /// Format optimization (JSON→YAML/CSV)
    pub format_optimize: f64,
    /// Telegraph intensity multiplier (0.0-1.0, applied to level)
    pub telegraph_intensity: f64,
}

/// Aggressive profile — for large, capable models.
/// Standard thresholds, full compression at configured level.
const PROFILE_AGGRESSIVE: ModelProfile = ModelProfile {
    name: "aggressive",
    filler_phrases: 0.2,
    verbose_phrases: 0.2,
    imperative_conversions: 0.5,
    clause_collapse: 0.5,
    developer_boilerplate: 0.5,
    semantic_formatting: 0.5,
    credential_strip: 0.5,
    whitespace_cleanup: 0.5,
    conversational_strip: 0.8,
    ai_output_noise: 0.8,
    markdown_minification: 0.8,
    source_code_compression: 0.8,
    context_deduplication: 0.8,
    anti_noise: 0.8,
    disclaimer_collapse: 0.8,
    adjective_collapse: 0.8,
    clause_simplify: 0.8,
    adverb_strip: 0.8,
    telegraph: 0.5,
    code_minify: 0.8,
    format_optimize: 0.8,
    telegraph_intensity: 1.0,
};

/// Balanced profile — for mid-tier models.
/// Raises thresholds on structural rewrites that can confuse smaller models.
const PROFILE_BALANCED: ModelProfile = ModelProfile {
    name: "balanced",
    filler_phrases: 0.2,
    verbose_phrases: 0.2,
    imperative_conversions: 0.5,
    clause_collapse: 0.5,
    developer_boilerplate: 0.5,
    semantic_formatting: 0.5,
    credential_strip: 0.5,
    whitespace_cleanup: 0.5,
    conversational_strip: 0.8,
    ai_output_noise: 0.8,
    markdown_minification: 0.8,
    source_code_compression: 0.8,
    context_deduplication: 0.8,
    anti_noise: 0.8,
    disclaimer_collapse: 0.8,
    adjective_collapse: 0.9, // raised — can distort meaning for mid-tier
    clause_simplify: 0.9,    // raised — structural rewrites are risky
    adverb_strip: 0.9,       // raised — adverbs carry meaning for smaller models
    telegraph: 0.6,          // raised — less aggressive sentence rewriting
    code_minify: 0.8,
    format_optimize: 0.8,
    telegraph_intensity: 0.85, // slightly reduced telegraph depth
};

/// Conservative profile — for small/weak models.
/// Only safe, non-destructive rules at normal levels.
/// Structural rewrites require near-max compression.
const PROFILE_CONSERVATIVE: ModelProfile = ModelProfile {
    name: "conservative",
    filler_phrases: 0.2,
    verbose_phrases: 0.3, // raised — some "verbose" patterns carry context
    imperative_conversions: 0.6, // raised — imperative rewrites can confuse small models
    clause_collapse: 0.7, // raised — clause structure matters more
    developer_boilerplate: 0.5,
    semantic_formatting: 0.6, // raised — abbreviations can confuse
    credential_strip: 0.5,
    whitespace_cleanup: 0.5,
    conversational_strip: 0.9, // raised — small models use conversational cues
    ai_output_noise: 0.8,
    markdown_minification: 0.9, // raised — formatting aids small model parsing
    source_code_compression: 0.9, // raised — code comments help small models
    context_deduplication: 0.8,
    anti_noise: 0.9,          // raised
    disclaimer_collapse: 0.9, // raised
    adjective_collapse: 1.1,  // effectively disabled — never fires (max level is 1.0)
    clause_simplify: 1.1,     // effectively disabled
    adverb_strip: 1.1,        // effectively disabled
    telegraph: 0.8,           // raised significantly
    code_minify: 0.9,         // raised — preserve code comments
    format_optimize: 0.9,     // raised — preserve explicit JSON structure
    telegraph_intensity: 0.6, // significantly reduced
};

/// Detect the appropriate profile for a model string.
///
/// Specific-first matching: small-model markers (mini/haiku/flash/light/nano/
/// 7b/8b/etc.) win over family-name substring matches so a model like
/// `claude-sonnet-4-6-mini` is correctly classified as conservative even
/// though its name contains "sonnet".
pub fn detect_profile(model: &str) -> &'static ModelProfile {
    let m = model.to_lowercase();

    // === Tier 1: small/mini markers — always conservative regardless of family ===
    // These suffixes/markers indicate a smaller variant. Check FIRST so
    // family-name substrings (sonnet/opus/gpt-4o) cannot upgrade them.
    let small_markers = ["mini", "haiku", "flash", "nano", "light", "-lite"];
    if small_markers.iter().any(|marker| m.contains(marker)) {
        return &PROFILE_CONSERVATIVE;
    }

    // === Tier 2: explicitly conservative model IDs ===
    if m.contains("gpt-3.5")
        || m.contains("command-r-light")
        || m.contains("llama-3.1-8b")
        || m.contains("llama-3.2")
        || m.contains("mistral-7b")
        || m.contains("mixtral-8x7b")
        || m.contains("phi-")
        || m.contains("qwen-2.5-7b")
        || m.contains("qwen-2.5-14b")
    {
        return &PROFILE_CONSERVATIVE;
    }

    // === Tier 3: aggressive premium models ===
    if m.contains("opus")
        || m.contains("sonnet")
        || m.contains("gpt-4o")
        || m.contains("gpt-4-turbo")
        || m.contains("grok-3")
        || m.contains("gemini-2.5-pro")
        || m.contains("gemini-1.5-pro")
        || m.contains("command-r-plus")
        || m.contains("llama-3.1-405b")
        || m.contains("llama-3.3-70b")
        || m.contains("deepseek-v3")
        || m.contains("qwen-2.5-72b")
    {
        return &PROFILE_AGGRESSIVE;
    }

    // === Default: balanced for everything else ===
    &PROFILE_BALANCED
}

/// Get a named profile (for config override)
pub fn get_profile(name: &str) -> &'static ModelProfile {
    match name.to_lowercase().as_str() {
        "aggressive" => &PROFILE_AGGRESSIVE,
        "conservative" => &PROFILE_CONSERVATIVE,
        "balanced" => &PROFILE_BALANCED,
        _ => &PROFILE_BALANCED,
    }
}

impl ModelProfile {
    /// Check if a category should fire at the given compression level
    pub fn should_fire(&self, category_threshold: f64, level: f64) -> bool {
        level >= category_threshold
    }

    /// Get the effective telegraph level (level * intensity)
    pub fn effective_telegraph_level(&self, level: f64) -> f64 {
        level * self.telegraph_intensity
    }
}
