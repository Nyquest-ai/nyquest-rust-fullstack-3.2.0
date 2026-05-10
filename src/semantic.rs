//! Nyquest Semantic Compression Stage
//!
//! Uses a local LLM (Qwen 2.5 1.5B via Ollama) as a semantic co-processor
//! for compression tasks that regex rules cannot handle:
//! - History condensation (neural summarization)
//! - System prompt condensation (imperative rewriting)
//! - Cross-message redundancy scoring
//!
//! All calls are async with hard timeouts and automatic fallback
//! to extractive summarization when the model is unavailable.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use md5::{Digest, Md5};

use crate::tokens::TokenCounter;

// ── Configuration ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub model: String,
    pub timeout_ms: u64,
    pub history_threshold: usize,
    pub system_threshold: usize,
    pub dedup: bool,
    pub temperature: f64,
    pub max_tokens: usize,
    pub fallback: String,
}

impl Default for SemanticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://127.0.0.1:11434/v1/chat/completions".into(),
            model: "qwen2.5:1.5b-instruct".into(),
            timeout_ms: 3000,
            history_threshold: 8000,
            system_threshold: 4000,
            dedup: false,
            temperature: 0.0,
            max_tokens: 2048,
            fallback: "extractive".into(),
        }
    }
}

// ── Stats ──

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SemanticStats {
    pub history_condensations: usize,
    pub system_condensations: usize,
    pub dedup_hits: usize,
    pub tokens_saved: usize,
    pub fallbacks: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub total_latency_ms: f64,
    pub call_count: usize,
}

impl SemanticStats {
    pub fn avg_latency_ms(&self) -> f64 {
        if self.call_count == 0 {
            0.0
        } else {
            self.total_latency_ms / self.call_count as f64
        }
    }
}

// ── Errors ──

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("Semantic model unavailable: {0}")]
    Unavailable(String),
    #[error("Semantic model timeout after {0}ms")]
    Timeout(u64),
    #[error("Invalid model response: {0}")]
    InvalidResponse(String),
    #[error("Output longer than input - discarded")]
    OutputLonger,
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

// ── Cache ──

#[derive(Debug, Clone)]
struct CacheEntry {
    result: Value,
    tokens_saved: usize,
    created_at: Instant,
}

static CACHE: Lazy<Arc<RwLock<HashMap<String, CacheEntry>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

const CACHE_MAX_ENTRIES: usize = 64;
const CACHE_TTL_SECS: u64 = 300;

// ── Prompt Templates ──

const HISTORY_CONDENSATION_PROMPT: &str = r#"You are a compression engine. Condense the following conversation history into a minimal summary. Preserve ALL of:
- Decisions, agreements, conclusions
- Error states and their resolutions
- URLs, IPs, file paths, code snippets
- User-stated preferences and constraints
- Numerical values, dates, amounts

Remove ALL of:
- Greetings, pleasantries, filler
- Repeated information
- Meta-commentary ("I'll help you with that")
- Formatting that doesn't carry meaning

Output ONLY the compressed summary as plain text. No preamble."#;

const SYSTEM_CONDENSATION_PROMPT: &str = r#"You are a compression engine. Rewrite the following system instructions in minimal imperative form. Rules:
1. Convert all sentences to imperative ("Do X" not "You should do X")
2. Remove redundant qualifiers (very, really, extremely, etc.)
3. Merge duplicate instructions into single statements
4. Preserve ALL behavioral constraints and output format requirements exactly
5. Preserve ALL tool/function definitions exactly - do not modify JSON schemas
6. Remove meta-instructions about "being helpful" unless they specify HOW

Output ONLY the compressed instructions. No preamble."#;

const REDUNDANCY_SCORING_PROMPT: &str = r#"Analyze these messages for semantic duplicates. Return a JSON array of objects:
{"msg_index": <int>, "span_start": <int>, "span_end": <int>, "duplicate_of": <int>, "confidence": <float>}

Only flag content with confidence > 0.85. Ignore greetings and acknowledgments.
Output ONLY valid JSON array. No preamble."#;

// ── Core Engine ──

pub struct SemanticEngine {
    client: Client,
    config: SemanticConfig,
    pub stats: SemanticStats,
    available: bool,
}

impl SemanticEngine {
    pub fn new(config: SemanticConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms + 500))
            .build()
            .unwrap_or_default();

        Self {
            client,
            config,
            stats: SemanticStats::default(),
            available: false,
        }
    }

    /// Check if the Ollama endpoint is reachable. Call on startup.
    pub async fn health_check(&mut self) -> bool {
        if !self.config.enabled {
            info!("Semantic compression disabled by config");
            return false;
        }

        let url = self
            .config
            .endpoint
            .replace("/v1/chat/completions", "/v1/models");
        match timeout(Duration::from_secs(5), self.client.get(&url).send()).await {
            Ok(Ok(resp)) if resp.status().is_success() => {
                info!(
                    "Semantic engine connected to {} (model: {})",
                    self.config.endpoint, self.config.model
                );
                self.available = true;
                true
            }
            Ok(Ok(resp)) => {
                warn!("Semantic engine endpoint returned {}", resp.status());
                self.available = false;
                false
            }
            Ok(Err(e)) => {
                warn!(
                    "Semantic engine unavailable: {} - fallback: {}",
                    e, self.config.fallback
                );
                self.available = false;
                false
            }
            Err(_) => {
                warn!("Semantic engine health check timed out");
                self.available = false;
                false
            }
        }
    }

    pub fn is_available(&self) -> bool {
        self.available && self.config.enabled
    }

    // ── History Condensation ──

    pub async fn condense_history(
        &mut self,
        messages: &[Value],
        tc: &TokenCounter,
    ) -> Result<(Vec<Value>, usize), SemanticError> {
        let start = Instant::now();
        let cache_key = self.cache_key(messages);

        // Check cache
        {
            let cache = CACHE.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                if entry.created_at.elapsed().as_secs() < CACHE_TTL_SECS {
                    self.stats.cache_hits += 1;
                    debug!(
                        "Semantic history cache HIT (saved {} tokens)",
                        entry.tokens_saved
                    );
                    let result = serde_json::from_value(entry.result.clone()).unwrap_or_default();
                    return Ok((result, entry.tokens_saved));
                }
            }
        }
        self.stats.cache_misses += 1;

        let history_text = self.format_messages_for_model(messages);
        let input_tokens = tc.count_text_tokens(&history_text);

        let response = self
            .call_model(HISTORY_CONDENSATION_PROMPT, &history_text)
            .await?;

        let output_tokens = tc.count_text_tokens(&response);
        if output_tokens >= input_tokens {
            return Err(SemanticError::OutputLonger);
        }

        let tokens_saved = input_tokens - output_tokens;

        let summary_msg = json!({
            "role": "user",
            "content": format!("[Semantic Summary of {} messages]\n{}", messages.len(), response)
        });
        let ack_msg = json!({
            "role": "assistant",
            "content": "Understood. I have the context from our earlier conversation."
        });
        let result = vec![summary_msg, ack_msg];

        self.update_cache(&cache_key, json!(result.clone()), tokens_saved)
            .await;

        let elapsed = start.elapsed().as_millis() as f64;
        self.stats.history_condensations += 1;
        self.stats.tokens_saved += tokens_saved;
        self.stats.total_latency_ms += elapsed;
        self.stats.call_count += 1;

        info!(
            "Semantic history condensation: {} msgs, {} -> {} tokens (saved {}), {:.0}ms",
            messages.len(),
            input_tokens,
            output_tokens,
            tokens_saved,
            elapsed
        );

        Ok((result, tokens_saved))
    }

    // ── System Prompt Condensation ──

    pub async fn condense_system(
        &mut self,
        system_text: &str,
        tc: &TokenCounter,
    ) -> Result<(String, usize), SemanticError> {
        let start = Instant::now();
        let input_tokens = tc.count_text_tokens(system_text);

        let cache_key = {
            let mut hasher = Md5::new();
            hasher.update(system_text.as_bytes());
            format!("sys:{:x}", hasher.finalize())
        };

        {
            let cache = CACHE.read().await;
            if let Some(entry) = cache.get(&cache_key) {
                if entry.created_at.elapsed().as_secs() < CACHE_TTL_SECS {
                    self.stats.cache_hits += 1;
                    let text = entry.result.as_str().unwrap_or("").to_string();
                    return Ok((text, entry.tokens_saved));
                }
            }
        }
        self.stats.cache_misses += 1;

        let response = self
            .call_model(SYSTEM_CONDENSATION_PROMPT, system_text)
            .await?;

        let output_tokens = tc.count_text_tokens(&response);
        if output_tokens >= input_tokens {
            return Err(SemanticError::OutputLonger);
        }

        let tokens_saved = input_tokens - output_tokens;

        self.update_cache(&cache_key, json!(response.clone()), tokens_saved)
            .await;

        let elapsed = start.elapsed().as_millis() as f64;
        self.stats.system_condensations += 1;
        self.stats.tokens_saved += tokens_saved;
        self.stats.total_latency_ms += elapsed;
        self.stats.call_count += 1;

        info!(
            "Semantic system condensation: {} -> {} tokens (saved {}), {:.0}ms",
            input_tokens, output_tokens, tokens_saved, elapsed
        );

        Ok((response, tokens_saved))
    }

    // ── Redundancy Scoring ──

    pub async fn score_redundancy(
        &mut self,
        messages: &[Value],
    ) -> Result<Vec<RedundancyHit>, SemanticError> {
        let start = Instant::now();
        let formatted = self.format_messages_for_model(messages);

        let response = self
            .call_model(REDUNDANCY_SCORING_PROMPT, &formatted)
            .await?;

        let hits: Vec<RedundancyHit> = serde_json::from_str(&response).map_err(|e| {
            SemanticError::InvalidResponse(format!("Failed to parse redundancy JSON: {}", e))
        })?;

        let elapsed = start.elapsed().as_millis() as f64;
        self.stats.dedup_hits += hits.len();
        self.stats.total_latency_ms += elapsed;
        self.stats.call_count += 1;

        info!(
            "Semantic redundancy scoring: {} messages, {} hits, {:.0}ms",
            messages.len(),
            hits.len(),
            elapsed
        );

        Ok(hits)
    }

    // ── Internal Helpers ──

    async fn call_model(
        &self,
        system_prompt: &str,
        content: &str,
    ) -> Result<String, SemanticError> {
        if !self.available {
            return Err(SemanticError::Unavailable("Model not available".into()));
        }

        let body = json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": content}
            ],
            "temperature": self.config.temperature,
            "max_tokens": self.config.max_tokens,
            "stream": false
        });

        let resp = timeout(
            Duration::from_millis(self.config.timeout_ms),
            self.client.post(&self.config.endpoint).json(&body).send(),
        )
        .await
        .map_err(|_| SemanticError::Timeout(self.config.timeout_ms))?
        .map_err(SemanticError::Http)?;

        if !resp.status().is_success() {
            return Err(SemanticError::Unavailable(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        let data: Value = resp.json().await.map_err(SemanticError::Http)?;

        let text = data
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .ok_or_else(|| SemanticError::InvalidResponse("No content in model response".into()))?
            .trim()
            .to_string();

        if text.is_empty() {
            return Err(SemanticError::InvalidResponse("Empty response".into()));
        }

        Ok(text)
    }

    fn format_messages_for_model(&self, messages: &[Value]) -> String {
        let mut parts = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            let role = msg
                .get("role")
                .and_then(|r| r.as_str())
                .unwrap_or("unknown");
            let content = extract_text_from_message(msg);
            if !content.trim().is_empty() {
                parts.push(format!("[{}] {}: {}", i, role, content));
            }
        }
        parts.join("\n\n")
    }

    fn cache_key(&self, messages: &[Value]) -> String {
        let text = messages
            .iter()
            .map(|m| m.to_string())
            .collect::<Vec<_>>()
            .join("|");
        let mut hasher = Md5::new();
        hasher.update(text.as_bytes());
        format!("hist:{:x}", hasher.finalize())
    }

    async fn update_cache(&self, key: &str, result: Value, tokens_saved: usize) {
        let mut cache = CACHE.write().await;
        cache.retain(|_, v| v.created_at.elapsed().as_secs() < CACHE_TTL_SECS);

        if cache.len() >= CACHE_MAX_ENTRIES {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, v)| v.created_at)
                .map(|(k, _)| k.clone())
            {
                cache.remove(&oldest_key);
            }
        }

        cache.insert(
            key.to_string(),
            CacheEntry {
                result,
                tokens_saved,
                created_at: Instant::now(),
            },
        );
    }
}

// ── Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedundancyHit {
    pub msg_index: usize,
    pub span_start: usize,
    pub span_end: usize,
    pub duplicate_of: usize,
    pub confidence: f64,
}

// ── Helpers ──

fn extract_text_from_message(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|block| match block.get("type").and_then(|t| t.as_str()) {
                Some("text") => block.get("text").and_then(|t| t.as_str()).map(String::from),
                Some("tool_result") => {
                    let content = block.get("content").cloned().unwrap_or(Value::Null);
                    match content {
                        Value::String(s) => Some(format!(
                            "[tool_result: {}]",
                            s.chars().take(100).collect::<String>()
                        )),
                        _ => Some("[tool_result]".into()),
                    }
                }
                Some("tool_use") => {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    Some(format!("[called: {}]", name))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

// ── Background Pre-computation ──

pub fn spawn_precompute(messages: Vec<Value>, config: SemanticConfig) {
    tokio::spawn(async move {
        let tc = TokenCounter::new();
        let mut engine = SemanticEngine::new(config);

        if !engine.health_check().await {
            return;
        }

        match engine.condense_history(&messages, &tc).await {
            Ok((_, saved)) => {
                debug!("Background semantic precompute: saved {} tokens", saved);
            }
            Err(e) => {
                debug!(
                    "Background semantic precompute failed (non-critical): {}",
                    e
                );
            }
        }
    });
}
