//! Nyquest Token Measurement Module

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

static RE_CODE_BLOCK: Lazy<Regex> = Lazy::new(|| Regex::new(r"```[\s\S]*?```").unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenMetrics {
    pub timestamp: f64,
    pub original_tokens: usize,
    pub optimized_tokens: usize,
    pub compression_ratio: f64,
    pub token_savings: usize,
    pub savings_percent: f64,
    pub compression_level: f64,
    #[serde(default)]
    pub latency_ms: f64,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub total_rule_hits: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_categories: Option<Vec<(String, usize)>>,
}

/// Estimates token counts for LLM models.
/// Uses calibrated heuristic: ~3.5 characters per token for English.
pub struct TokenCounter;

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenCounter {
    #[allow(dead_code)]
    const CHARS_PER_TOKEN: f64 = 3.5;
    const MESSAGE_OVERHEAD: usize = 4;
    const SYSTEM_OVERHEAD: usize = 6;

    pub fn new() -> Self {
        Self
    }

    pub fn count_text_tokens(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let char_count = text.len();
        let code_chars: usize = RE_CODE_BLOCK
            .find_iter(text)
            .map(|m| m.as_str().len())
            .sum();
        let non_code_chars = char_count - code_chars;
        let tokens = (non_code_chars as f64 / 3.5) + (code_chars as f64 / 3.0);
        std::cmp::max(1, tokens.round() as usize)
    }

    pub fn count_message_tokens(&self, message: &Value) -> usize {
        let mut tokens = Self::MESSAGE_OVERHEAD;
        if let Some(content) = message.get("content") {
            match content {
                Value::String(s) => tokens += self.count_text_tokens(s),
                Value::Array(arr) => {
                    for block in arr {
                        if let Some(btype) = block.get("type").and_then(|t| t.as_str()) {
                            match btype {
                                "text" => {
                                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                        tokens += self.count_text_tokens(t);
                                    }
                                }
                                "tool_use" => {
                                    if let Some(n) = block.get("name").and_then(|n| n.as_str()) {
                                        tokens += self.count_text_tokens(n);
                                    }
                                    if let Some(input) = block.get("input") {
                                        tokens += self.count_text_tokens(&input.to_string());
                                    }
                                    tokens += 10;
                                }
                                "tool_result" => {
                                    if let Some(c) = block.get("content") {
                                        match c {
                                            Value::String(s) => tokens += self.count_text_tokens(s),
                                            Value::Array(sub) => {
                                                for item in sub {
                                                    if item.get("type").and_then(|t| t.as_str())
                                                        == Some("text")
                                                    {
                                                        if let Some(t) = item
                                                            .get("text")
                                                            .and_then(|t| t.as_str())
                                                        {
                                                            tokens += self.count_text_tokens(t);
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    tokens += 8;
                                }
                                "image" => tokens += 1600,
                                _ => tokens += self.count_text_tokens(&block.to_string()),
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        tokens
    }

    pub fn count_request_tokens(&self, request_body: &Value) -> usize {
        let mut total = 0;

        // System message
        if let Some(system) = request_body.get("system") {
            match system {
                Value::String(s) if !s.is_empty() => {
                    total += Self::SYSTEM_OVERHEAD + self.count_text_tokens(s);
                }
                Value::Array(blocks) => {
                    total += Self::SYSTEM_OVERHEAD;
                    for block in blocks {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                total += self.count_text_tokens(t);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Messages
        if let Some(Value::Array(messages)) = request_body.get("messages") {
            for msg in messages {
                total += self.count_message_tokens(msg);
            }
        }

        // Tools
        if let Some(Value::Array(tools)) = request_body.get("tools") {
            let tools_text = serde_json::to_string(tools).unwrap_or_default();
            total += self.count_text_tokens(&tools_text) + (tools.len() * 8);
        }

        total
    }
}

/// Logs token metrics to JSONL file
pub struct MetricsLogger {
    log_path: PathBuf,
}

impl MetricsLogger {
    pub fn new(log_file: &str) -> Self {
        let path = PathBuf::from(log_file);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        Self { log_path: path }
    }

    pub fn log(&self, metrics: &TokenMetrics) {
        if let Ok(json) = serde_json::to_string(metrics) {
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = writeln!(file, "{}", json);
            }
        }
    }

    pub fn get_summary(&self, last_n: usize) -> Value {
        let (entries, total_lines) = self.read_entries(last_n);
        if entries.is_empty() {
            return serde_json::json!({"count": 0, "total_count": 0, "total_requests": 0});
        }

        let savings: Vec<f64> = entries.iter().map(|e| e.savings_percent).collect();
        let latencies: Vec<f64> = entries
            .iter()
            .filter(|e| e.latency_ms > 0.0)
            .map(|e| e.latency_ms)
            .collect();
        let total_saved: usize = entries.iter().map(|e| e.token_savings).sum();
        let total_original: usize = entries.iter().map(|e| e.original_tokens).sum();

        // Build recent requests array (last 20) for dashboard table
        let recent_count = 20_usize.min(entries.len());
        let recent: Vec<Value> = entries[entries.len() - recent_count..]
            .iter()
            .rev()
            .map(|e| {
                serde_json::json!({
                    "request_id": e.request_id,
                    "model": e.model,
                    "compression_level": e.compression_level,
                    "original_tokens": e.original_tokens,
                    "optimized_tokens": e.optimized_tokens,
                    "savings_percent": e.savings_percent,
                    "latency_ms": e.latency_ms,
                    "timestamp": e.timestamp,
                })
            })
            .collect();

        // Avg compression ratio
        let avg_ratio = if total_original == 0 {
            1.0
        } else {
            entries.iter().map(|e| e.compression_ratio).sum::<f64>() / entries.len() as f64
        };

        serde_json::json!({
            "count": total_lines,
            "total_requests": total_lines,
            "avg_savings_percent": savings.iter().sum::<f64>() / savings.len() as f64,
            "avg_compression_ratio": avg_ratio,
            "max_savings_percent": savings.iter().cloned().fold(f64::MIN, f64::max),
            "min_savings_percent": savings.iter().cloned().fold(f64::MAX, f64::min),
            "total_tokens_saved": total_saved,
            "total_tokens_processed": total_original,
            "avg_latency_ms": if latencies.is_empty() { 0.0 } else { latencies.iter().sum::<f64>() / latencies.len() as f64 },
            "window_size": entries.len(),
            "recent": recent,
        })
    }

    fn read_entries(&self, last_n: usize) -> (Vec<TokenMetrics>, usize) {
        let content = fs::read_to_string(&self.log_path).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let start = if lines.len() > last_n {
            lines.len() - last_n
        } else {
            0
        };
        let entries = lines[start..]
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();
        (entries, total_lines)
    }
}
