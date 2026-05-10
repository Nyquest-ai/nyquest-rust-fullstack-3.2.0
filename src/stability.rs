//! Nyquest Stability Validator
//! Compares baseline and compressed outputs to detect semantic drift.
//!
//! Two modes:
//! - Passive: Log divergence scores, don't modify behavior
//! - Active: Auto-adjust compression level when divergence exceeds threshold
//!
//! Dev/testing module — doubles API costs.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use tracing::{info, warn};

static WORD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\w+\b").unwrap());

#[derive(Debug, Serialize, Deserialize)]
pub struct StabilityResult {
    pub request_id: String,
    pub timestamp: f64,
    pub model: String,
    pub compression_level: f64,
    pub baseline_response: String,
    pub compressed_response: String,
    pub baseline_tokens: usize,
    pub compressed_tokens: usize,
    pub cosine_similarity: f64,
    pub jaccard_similarity: f64,
    pub length_ratio: f64,
    pub divergence_score: f64,
    pub is_stable: bool,
    pub threshold: f64,
    pub action_taken: String,
}

impl StabilityResult {
    fn to_log_json(&self) -> Value {
        let mut val = serde_json::to_value(self).unwrap_or(Value::Null);
        // Truncate responses for log
        if let Some(obj) = val.as_object_mut() {
            if let Some(Value::String(s)) = obj.get("baseline_response") {
                let truncated: String = s.chars().take(200).collect();
                obj.insert("baseline_response".into(), Value::String(truncated));
            }
            if let Some(Value::String(s)) = obj.get("compressed_response") {
                let truncated: String = s.chars().take(200).collect();
                obj.insert("compressed_response".into(), Value::String(truncated));
            }
        }
        val
    }
}

pub struct StabilityValidator {
    pub threshold: f64,
    pub active_mode: bool,
    level_step: f64,
    min_level: f64,
    log_path: PathBuf,
    adjusted_levels: Mutex<HashMap<String, f64>>,
}

impl StabilityValidator {
    pub fn new(
        threshold: f64,
        active_mode: bool,
        level_step: f64,
        min_level: f64,
        log_file: &str,
    ) -> Self {
        let log_path = PathBuf::from(log_file);
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        Self {
            threshold,
            active_mode,
            level_step,
            min_level,
            log_path,
            adjusted_levels: Mutex::new(HashMap::new()),
        }
    }

    pub fn get_adjusted_level(&self, model: &str, requested_level: f64) -> f64 {
        if !self.active_mode {
            return requested_level;
        }
        let levels = self.adjusted_levels.lock().unwrap();
        if let Some(&adjusted) = levels.get(model) {
            requested_level.min(adjusted)
        } else {
            requested_level
        }
    }

    /// Run stability validation: send both original and compressed, compare responses.
    #[allow(clippy::too_many_arguments)]
    pub async fn validate(
        &self,
        original_body: &Value,
        compressed_body: &Value,
        api_url: &str,
        headers: &HashMap<String, String>,
        http_client: &reqwest::Client,
        request_id: &str,
        compression_level: f64,
    ) -> Option<StabilityResult> {
        let model = original_body
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Force non-streaming
        let mut original_req = original_body.clone();
        let mut compressed_req = compressed_body.clone();
        original_req["stream"] = Value::Bool(false);
        compressed_req["stream"] = Value::Bool(false);

        // Build reqwest headers
        let mut req_headers = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                reqwest::header::HeaderValue::from_str(v),
            ) {
                req_headers.insert(name, val);
            }
        }

        // Send both requests concurrently
        let baseline_future = http_client
            .post(api_url)
            .headers(req_headers.clone())
            .json(&original_req)
            .timeout(std::time::Duration::from_secs(90))
            .send();

        let compressed_future = http_client
            .post(api_url)
            .headers(req_headers)
            .json(&compressed_req)
            .timeout(std::time::Duration::from_secs(90))
            .send();

        let (baseline_resp, compressed_resp) =
            match tokio::try_join!(baseline_future, compressed_future) {
                Ok(pair) => pair,
                Err(e) => {
                    warn!("[{}] Stability validation failed: {}", request_id, e);
                    return None;
                }
            };

        let baseline_status = baseline_resp.status();
        let compressed_status = compressed_resp.status();

        let baseline_json: Value = baseline_resp.json().await.ok()?;
        let compressed_json: Value = compressed_resp.json().await.ok()?;

        let baseline_text = extract_response_text(&baseline_json, baseline_status.as_u16());
        let compressed_text = extract_response_text(&compressed_json, compressed_status.as_u16());

        if baseline_text.is_empty() || compressed_text.is_empty() {
            warn!("[{}] Empty response in stability check", request_id);
            return None;
        }

        // Compute similarity metrics
        let cosine_sim = cosine_similarity_bow(&baseline_text, &compressed_text);
        let jaccard_sim = jaccard_similarity(&baseline_text, &compressed_text);
        let len_ratio = length_ratio(&baseline_text, &compressed_text);

        // Composite: cosine 50%, jaccard 30%, length 20%
        let composite = cosine_sim * 0.5 + jaccard_sim * 0.3 + len_ratio * 0.2;
        let divergence = 1.0 - composite;
        let is_stable = composite >= self.threshold;

        let action = if !is_stable && self.active_mode {
            let mut levels = self.adjusted_levels.lock().unwrap();
            let current = levels.get(&model).copied().unwrap_or(compression_level);
            let new_level = (current - self.level_step).max(self.min_level);
            levels.insert(model.clone(), new_level);
            warn!(
                "[{}] Divergence detected ({:.3} < {}). Auto-reducing {} compression: {:.2} → {:.2}",
                request_id, composite, self.threshold, model, current, new_level
            );
            format!("level_reduced:{:.2}→{:.2}", current, new_level)
        } else if !is_stable {
            warn!(
                "[{}] Divergence detected ({:.3} < {}) for {} at level {:.1}",
                request_id, composite, self.threshold, model, compression_level
            );
            "logged".to_string()
        } else {
            "logged".to_string()
        };

        let baseline_tokens = baseline_json
            .get("usage")
            .and_then(|u| u.get("output_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as usize;
        let compressed_tokens = compressed_json
            .get("usage")
            .and_then(|u| u.get("output_tokens"))
            .and_then(|t| t.as_u64())
            .unwrap_or(0) as usize;

        let result = StabilityResult {
            request_id: request_id.to_string(),
            timestamp: chrono::Utc::now().timestamp() as f64,
            model: model.clone(),
            compression_level,
            baseline_response: baseline_text,
            compressed_response: compressed_text,
            baseline_tokens,
            compressed_tokens,
            cosine_similarity: (cosine_sim * 10000.0).round() / 10000.0,
            jaccard_similarity: (jaccard_sim * 10000.0).round() / 10000.0,
            length_ratio: (len_ratio * 10000.0).round() / 10000.0,
            divergence_score: (divergence * 10000.0).round() / 10000.0,
            is_stable,
            threshold: self.threshold,
            action_taken: action,
        };

        self.log_result(&result);

        info!(
            "[{}] Stability: cosine={:.3} jaccard={:.3} length={:.3} → composite={:.3} {}",
            request_id,
            cosine_sim,
            jaccard_sim,
            len_ratio,
            composite,
            if is_stable {
                "✅ STABLE"
            } else {
                "⚠️ DIVERGENT"
            }
        );

        Some(result)
    }

    fn log_result(&self, result: &StabilityResult) {
        let json_line = result.to_log_json().to_string();
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
        {
            let _ = writeln!(f, "{}", json_line);
        }
    }

    pub fn get_summary(&self, last_n: usize) -> Value {
        if !self.log_path.exists() {
            return serde_json::json!({"count": 0});
        }

        let content = fs::read_to_string(&self.log_path).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();
        let start = lines.len().saturating_sub(last_n);
        let entries: Vec<Value> = lines[start..]
            .iter()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect();

        if entries.is_empty() {
            return serde_json::json!({"count": 0});
        }

        let stable_count = entries
            .iter()
            .filter(|e| {
                e.get("is_stable")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false)
            })
            .count();
        let divergent_count = entries.len() - stable_count;
        let avg_cosine: f64 = entries
            .iter()
            .filter_map(|e| e.get("cosine_similarity").and_then(|c| c.as_f64()))
            .sum::<f64>()
            / entries.len() as f64;
        let avg_divergence: f64 = entries
            .iter()
            .filter_map(|e| e.get("divergence_score").and_then(|d| d.as_f64()))
            .sum::<f64>()
            / entries.len() as f64;

        let levels = self.adjusted_levels.lock().unwrap();
        let adjusted: HashMap<String, f64> = levels.clone();

        serde_json::json!({
            "count": entries.len(),
            "stable": stable_count,
            "divergent": divergent_count,
            "stability_rate": (stable_count as f64 / entries.len() as f64 * 1000.0).round() / 10.0,
            "avg_cosine_similarity": (avg_cosine * 10000.0).round() / 10000.0,
            "avg_divergence_score": (avg_divergence * 10000.0).round() / 10000.0,
            "adjusted_levels": adjusted,
        })
    }
}

// ── Similarity functions ──

fn tokenize(text: &str) -> Vec<String> {
    WORD_RE
        .find_iter(&text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect()
}

fn cosine_similarity_bow(text_a: &str, text_b: &str) -> f64 {
    let tokens_a = tokenize(text_a);
    let tokens_b = tokenize(text_b);
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }

    let mut freq_a: HashMap<&str, f64> = HashMap::new();
    let mut freq_b: HashMap<&str, f64> = HashMap::new();
    for t in &tokens_a {
        *freq_a.entry(t.as_str()).or_default() += 1.0;
    }
    for t in &tokens_b {
        *freq_b.entry(t.as_str()).or_default() += 1.0;
    }

    let vocab: std::collections::HashSet<&str> =
        freq_a.keys().chain(freq_b.keys()).copied().collect();

    let dot: f64 = vocab
        .iter()
        .map(|w| freq_a.get(w).unwrap_or(&0.0) * freq_b.get(w).unwrap_or(&0.0))
        .sum();
    let mag_a: f64 = freq_a.values().map(|v| v * v).sum::<f64>().sqrt();
    let mag_b: f64 = freq_b.values().map(|v| v * v).sum::<f64>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    dot / (mag_a * mag_b)
}

fn jaccard_similarity(text_a: &str, text_b: &str) -> f64 {
    let set_a: std::collections::HashSet<String> = tokenize(text_a).into_iter().collect();
    let set_b: std::collections::HashSet<String> = tokenize(text_b).into_iter().collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;
    }
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    intersection as f64 / union as f64
}

fn length_ratio(text_a: &str, text_b: &str) -> f64 {
    let len_a = text_a.len();
    let len_b = text_b.len();
    if len_a == 0 && len_b == 0 {
        return 1.0;
    }
    if len_a == 0 || len_b == 0 {
        return 0.0;
    }
    len_a.min(len_b) as f64 / len_a.max(len_b) as f64
}

fn extract_response_text(data: &Value, status: u16) -> String {
    if status != 200 {
        return String::new();
    }
    let content = data.get("content").and_then(|c| c.as_array());
    match content {
        Some(blocks) => blocks
            .iter()
            .filter_map(|b| {
                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                    b.get("text")
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        None => String::new(),
    }
}
