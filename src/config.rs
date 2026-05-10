//! Nyquest Configuration
//! YAML config loader with provider-specific settings.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NyquestConfig {
    pub compression_level: f64,
    pub adaptive_mode: bool,
    pub semantic_validation: bool,
    pub semantic_threshold: f64,
    pub target_api_base: String,
    pub target_api_version: String,
    pub host: String,
    pub port: u16,
    pub log_metrics: bool,
    pub log_file: String,
    pub log_level: String,
    pub token_counting: String,
    pub request_timeout: u64,
    pub allow_header_override: bool,

    // Normalization
    pub normalize: bool,
    pub inject_boundaries: bool,

    // Stability mode
    pub stability_mode: bool,
    pub stability_active: bool,
    pub stability_level_step: f64,
    pub stability_min_level: f64,
    pub stability_log_file: String,

    // Context window optimization
    pub context_optimization: bool,
    pub context_max_input_tokens: usize,
    pub context_preserve_recent_turns: usize,
    pub context_min_turns: usize,

    // Security
    pub privacy_mode: bool,
    pub encrypt_keys: bool,
    pub vault_path: String,

    // Response Compression (compress older assistant messages)
    pub compress_responses: bool,
    pub response_compression_age: usize,

    // OpenClaw Agent Mode
    pub openclaw_mode: bool,
    pub openclaw_tool_prune_turns: usize,
    pub openclaw_thought_prune_turns: usize,
    pub openclaw_schema_minimize: bool,
    pub openclaw_dedup_errors: bool,
    pub openclaw_condense_views: bool,
    pub openclaw_cache_control: bool,
    pub openclaw_sliding_window: bool,
    pub openclaw_sliding_window_threshold: f64,
    pub openclaw_sliding_window_max_tokens: usize,
    pub openclaw_sliding_window_preserve: usize,

    // Semantic Compression (local LLM co-processor)
    pub semantic_enabled: bool,
    pub semantic_endpoint: String,
    pub semantic_model: String,
    pub semantic_timeout_ms: u64,
    pub semantic_history_threshold: usize,
    pub semantic_system_threshold: usize,
    pub semantic_dedup: bool,
    pub semantic_temperature: f64,
    pub semantic_max_tokens: usize,
    pub semantic_fallback: String,

    // Per-provider settings
    #[serde(default)]
    pub providers: HashMap<String, HashMap<String, String>>,
}

impl Default for NyquestConfig {
    fn default() -> Self {
        Self {
            compression_level: 0.5,
            adaptive_mode: false,
            semantic_validation: false,
            semantic_threshold: 0.85,
            target_api_base: "https://api.anthropic.com".to_string(),
            target_api_version: "2023-06-01".to_string(),
            host: "0.0.0.0".to_string(),
            port: 5400,
            log_metrics: true,
            log_file: "logs/nyquest_metrics.jsonl".to_string(),
            log_level: "INFO".to_string(),
            token_counting: "estimate".to_string(),
            request_timeout: 120,
            allow_header_override: true,
            normalize: true,
            inject_boundaries: false,
            stability_mode: false,
            stability_active: false,
            stability_level_step: 0.1,
            stability_min_level: 0.0,
            stability_log_file: "logs/stability_log.jsonl".to_string(),
            context_optimization: true,
            context_max_input_tokens: 20000,
            context_preserve_recent_turns: 3,
            context_min_turns: 4,
            privacy_mode: false,
            encrypt_keys: false,
            vault_path: "~/.nyquest/vault.enc".to_string(),
            compress_responses: true,
            response_compression_age: 4,
            openclaw_mode: false,
            openclaw_tool_prune_turns: 2,
            openclaw_thought_prune_turns: 3,
            openclaw_schema_minimize: true,
            openclaw_dedup_errors: true,
            openclaw_condense_views: true,
            openclaw_cache_control: true,
            openclaw_sliding_window: true,
            openclaw_sliding_window_threshold: 0.80,
            openclaw_sliding_window_max_tokens: 200000,
            openclaw_sliding_window_preserve: 5,
            semantic_enabled: false,
            semantic_endpoint: "http://localhost:11434/v1/chat/completions".to_string(),
            semantic_model: "qwen2.5:1.5b-instruct".to_string(),
            semantic_timeout_ms: 3000,
            semantic_history_threshold: 8000,
            semantic_system_threshold: 4000,
            semantic_dedup: false,
            semantic_temperature: 0.0,
            semantic_max_tokens: 2048,
            semantic_fallback: "extractive".to_string(),
            providers: HashMap::new(),
        }
    }
}

impl NyquestConfig {
    pub fn effective_level(&self, header_value: Option<&str>) -> f64 {
        if self.allow_header_override {
            if let Some(val) = header_value {
                if let Ok(level) = val.parse::<f64>() {
                    return level.clamp(0.0, 1.0);
                }
            }
        }
        self.compression_level
    }

    pub fn get_provider_key(&self, provider: &str) -> Option<String> {
        self.providers.get(provider)?.get("api_key").cloned()
    }

    pub fn get_provider_base_url(&self, provider: &str) -> Option<String> {
        self.providers.get(provider)?.get("base_url").cloned()
    }

    pub fn get_provider_model(&self, provider: &str) -> Option<String> {
        self.providers.get(provider)?.get("model").cloned()
    }

    pub fn semantic_config(&self) -> crate::semantic::SemanticConfig {
        crate::semantic::SemanticConfig {
            enabled: self.semantic_enabled,
            endpoint: self.semantic_endpoint.clone(),
            model: self.semantic_model.clone(),
            timeout_ms: self.semantic_timeout_ms,
            history_threshold: self.semantic_history_threshold,
            system_threshold: self.semantic_system_threshold,
            dedup: self.semantic_dedup,
            temperature: self.semantic_temperature,
            max_tokens: self.semantic_max_tokens,
            fallback: self.semantic_fallback.clone(),
        }
    }
}

/// Load config from YAML file, env vars, or defaults
pub fn load_config(config_path: Option<&str>) -> NyquestConfig {
    let path = config_path
        .map(String::from)
        .or_else(|| env::var("NYQUEST_CONFIG").ok())
        .unwrap_or_else(|| "nyquest.yaml".to_string());

    let mut cfg = if Path::new(&path).exists() {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        serde_yaml::from_str(&content).unwrap_or_default()
    } else {
        NyquestConfig::default()
    };

    // Environment variable overrides
    if let Ok(val) = env::var("NYQUEST_COMPRESSION_LEVEL") {
        if let Ok(level) = val.parse() {
            cfg.compression_level = level;
        }
    }
    if let Ok(val) = env::var("NYQUEST_TARGET_API_BASE") {
        cfg.target_api_base = val;
    }
    if let Ok(val) = env::var("NYQUEST_HOST") {
        cfg.host = val;
    }
    if let Ok(val) = env::var("NYQUEST_PORT") {
        if let Ok(port) = val.parse() {
            cfg.port = port;
        }
    }
    if let Ok(val) = env::var("NYQUEST_LOG_LEVEL") {
        cfg.log_level = val;
    }
    if let Ok(val) = env::var("NYQUEST_TIMEOUT") {
        if let Ok(timeout) = val.parse() {
            cfg.request_timeout = timeout;
        }
    }
    if let Ok(val) = env::var("NYQUEST_OPENCLAW_MODE") {
        cfg.openclaw_mode = val.to_lowercase() == "true";
    }
    if let Ok(val) = env::var("NYQUEST_SEMANTIC_ENABLED") {
        cfg.semantic_enabled = val.to_lowercase() == "true";
    }
    if let Ok(val) = env::var("NYQUEST_SEMANTIC_ENDPOINT") {
        cfg.semantic_endpoint = val;
    }
    if let Ok(val) = env::var("NYQUEST_SEMANTIC_MODEL") {
        cfg.semantic_model = val;
    }

    cfg
}
