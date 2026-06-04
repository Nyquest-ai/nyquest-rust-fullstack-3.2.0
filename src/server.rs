//! Nyquest HTTP Server
//! Axum-based proxy server with SSE streaming, multi-provider routing,
//! and both Anthropic and OpenAI-compatible endpoints.

use axum::body::Body;
use axum::{
    extract::{Json, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use bytes::Bytes;
use futures::stream::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, info};

use crate::analytics::RuleAnalytics;
use crate::cache_reorder;
use crate::compression::{compress_request, CompressionStats};
use crate::config::NyquestConfig;
use crate::context::{ContextConfig, ContextOptimizer};
use crate::openclaw::{OpenClawConfig, OpenClawOptimizer};
use crate::providers::{
    anthropic_to_openai_response, build_upstream_headers, detect_provider, get_provider_config,
    openai_to_anthropic,
};
use crate::semantic::SemanticEngine;
use crate::tokens::{MetricsLogger, TokenCounter, TokenMetrics};

pub struct AppState {
    pub config: NyquestConfig,
    pub token_counter: TokenCounter,
    pub metrics_logger: MetricsLogger,
    pub http_client: reqwest::Client,
    pub analytics: RuleAnalytics,
    pub semantic: tokio::sync::Mutex<SemanticEngine>,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .route("/analytics", get(analytics))
        .route("/admin/v2/compression/rules", get(rules_telemetry))
        .route("/admin/v2/compression/cache", get(cache_snapshot))
        .route("/dashboard", get(dashboard))
        .route("/v1/messages", post(proxy_messages))
        .route("/v1/chat/completions", post(proxy_chat_completions))
        .route("/v2/compress", post(compress_only))
        .with_state(state)
}

// ─── Per-rule telemetry ────────────────────────────────

/// Per-rule hit counts for every category. Useful to identify dead rules
/// (zero hits across many requests) and over-firing rules.
/// No prompt content is exposed — only the stable rule id and the regex source.
async fn rules_telemetry(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    use crate::compression::rules;
    let categories: Vec<&'static rules::RuleCategory> = vec![
        &rules::OPENCLAW_RULES,
        &rules::FILLER_PHRASES,
        &rules::VERBOSE_PHRASES,
        &rules::IMPERATIVE_CONVERSIONS,
        &rules::CLAUSE_COLLAPSE,
        &rules::DEVELOPER_BOILERPLATE,
        &rules::CONVERSATIONAL_STRIP,
        &rules::AI_OUTPUT_NOISE,
        &rules::MARKDOWN_MINIFICATION,
        &rules::SOURCE_CODE_COMPRESSION,
        &rules::CONTEXT_DEDUPLICATION,
        &rules::SEMANTIC_FORMATTING,
        &rules::ANTI_NOISE,
        &rules::CREDENTIAL_STRIP,
        &rules::DISCLAIMER_COLLAPSE,
        &rules::ADJECTIVE_COLLAPSE,
        &rules::CLAUSE_SIMPLIFY,
        &rules::ADVERB_STRIP,
        &rules::WHITESPACE_CLEANUP,
    ];

    let cats: Vec<Value> = categories
        .iter()
        .map(|cat| {
            let rules_json: Vec<Value> = cat
                .rules
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "id": r.id,
                        "pattern": r.pattern_str,
                        "hits": r.hits(),
                    })
                })
                .collect();
            serde_json::json!({
                "name": cat.name,
                "category_hits": cat.category_hits(),
                "rules": rules_json,
            })
        })
        .collect();

    Json(serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "categories": cats,
    }))
}

/// Snapshot of the bounded compression cache.
async fn cache_snapshot(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    let snap = crate::compression::GLOBAL_CACHE.snapshot();
    Json(serde_json::json!({
        "status": "ok",
        "cache": snap,
    }))
}

// ─── Health ──────────────────────────────────────────────────

async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "compression_level": state.config.compression_level,
        "normalize": state.config.normalize,
        "stability_guard": state.config.stability_mode,
        "context_optimizer": state.config.context_optimization,
        "openclaw_mode": state.config.openclaw_mode,
        "semantic_enabled": state.config.semantic_enabled,
        "semantic_model": state.config.semantic_model,
        "rust_engine": true,
    }))
}

// ─── Metrics ─────────────────────────────────────────────────

async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let summary = state.metrics_logger.get_summary(5000);
    let analytics = state.analytics.snapshot();
    Json(serde_json::json!({
        "status": "ok",
        "compression_level": state.config.compression_level,
        "metrics": summary,
        "rule_analytics": analytics,
    }))
}

// ─── Analytics ───────────────────────────────────────────────

async fn analytics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snapshot = state.analytics.snapshot();
    Json(serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "analytics": snapshot,
    }))
}

// ─── Dashboard ───────────────────────────────────────────────

async fn dashboard(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let summary = state.metrics_logger.get_summary(5000);
    let analytics_snapshot = state.analytics.snapshot();
    let html = crate::dashboard::render_dashboard_html_with_analytics(
        &summary,
        state.config.compression_level,
        Some(&analytics_snapshot),
    );
    Response::builder()
        .status(200)
        .header("content-type", "text/html; charset=utf-8")
        .body(Body::from(html))
        .unwrap()
}

// ─── Shared: extract headers into HashMap ────────────────────

fn headers_to_map(headers: &HeaderMap) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            map.insert(name.as_str().to_lowercase(), v.to_string());
        }
    }
    map
}

// ─── Shared: compression + optimization pipeline ─────────────

/// Auto-scale compression level based on token count and model context window.
/// Small prompts get lighter compression for maximum fidelity.
/// Large prompts get heavier compression to stay within context limits.
fn auto_scale_level(base_level: f64, token_count: usize, body: &Value) -> f64 {
    // If explicitly set to 0.0 (passthrough), respect it
    if base_level == 0.0 {
        return 0.0;
    }

    // Detect model context window (default 128K)
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
    let max_context: usize = if model.contains("haiku") {
        200_000
    } else if model.contains("sonnet") || model.contains("opus") {
        200_000
    } else if model.contains("grok") {
        131_072
    } else if model.contains("gemini") {
        1_000_000
    } else if model.contains("gpt-4o") {
        128_000
    } else {
        128_000
    };

    let usage_ratio = token_count as f64 / max_context as f64;

    let scaled = if usage_ratio > 0.8 {
        // >80% of context: force maximum compression + enable sliding window territory
        1.0_f64.max(base_level)
    } else if usage_ratio > 0.5 {
        // 50-80%: ramp up from base toward 0.9
        let ramp = (usage_ratio - 0.5) / 0.3; // 0.0 at 50%, 1.0 at 80%
        base_level + (0.9 - base_level) * ramp
    } else if token_count < 100 {
        // Truly tiny prompts (<100 tok): reduce compression for fidelity
        (base_level * 0.5).max(0.2)
    } else if token_count < 200 {
        // Small prompts: use slightly lower compression
        base_level * 0.85
    } else {
        // Normal range: use configured level
        base_level
    };

    let final_level = scaled.clamp(0.0, 1.0);

    if (final_level - base_level).abs() > 0.05 {
        tracing::info!(
            "Auto-scaled compression: {:.2} → {:.2} ({}tok, {:.1}% of {}K context)",
            base_level,
            final_level,
            token_count,
            usage_ratio * 100.0,
            max_context / 1000
        );
    }

    final_level
}

async fn run_pipeline(
    body: &Value,
    state: &AppState,
    level: f64,
    openclaw_enabled: bool,
    response_age_override: Option<usize>,
    cache_workspace: Option<&str>,
    cache_user: Option<&str>,
) -> (Value, usize, usize, CompressionStats, usize, usize) {
    let original_tokens = state.token_counter.count_request_tokens(body);

    // Step 0: Auto-scale compression level based on token budget
    let level = auto_scale_level(level, original_tokens, body);

    // Step 1: Context optimization
    let mut working = body.clone();
    if state.config.context_optimization {
        let ctx_config = ContextConfig {
            enabled: true,
            max_input_tokens: 50_000,
            preserve_recent_turns: 4,
            min_turns_for_summary: 6,
            max_summary_chars: 2000,
        };
        let optimizer = ContextOptimizer::new(ctx_config);
        if optimizer.should_optimize(&working, &state.token_counter) {
            working = optimizer.optimize(&working, &state.token_counter);
        }
    }

    // Step 2: OpenClaw agent optimization
    if openclaw_enabled {
        let oc_config = OpenClawConfig {
            enabled: true,
            ..Default::default()
        };
        let mut optimizer = OpenClawOptimizer::new(oc_config);
        working = optimizer.optimize(&working, Some(&state.token_counter));
    }

    // Step 2.5: Cache optimization (reorder for provider prefix caching)
    working = cache_reorder::reorder_for_cache(&working);

    // Step 2.7: Semantic compression (Qwen 2.5 1.5B via Ollama)
    if state.config.semantic_enabled {
        let mut sem = state.semantic.lock().await;
        if sem.is_available() {
            // --- System prompt condensation ---
            // Handle both Anthropic format (top-level "system" string)
            // and OpenAI format (messages array with role=system)
            let mut system_text = String::new();
            let mut system_in_messages_idx: Option<usize> = None;

            // Check Anthropic-style top-level "system" field
            if let Some(s) = working.get("system").and_then(|s| s.as_str()) {
                if !s.is_empty() {
                    system_text = s.to_string();
                }
            }

            // Check OpenAI-style system message in messages array
            if system_text.is_empty() {
                if let Some(messages) = working.get("messages").and_then(|m| m.as_array()) {
                    for (i, msg) in messages.iter().enumerate() {
                        if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                                system_text = content.to_string();
                                system_in_messages_idx = Some(i);
                            }
                            break;
                        }
                    }
                }
            }

            let sys_tokens = state.token_counter.count_text_tokens(&system_text);
            if sys_tokens >= state.config.semantic_system_threshold {
                match sem
                    .condense_system(&system_text, &state.token_counter, cache_workspace, cache_user)
                    .await
                {
                    Ok((condensed, saved)) => {
                        if let Some(idx) = system_in_messages_idx {
                            // OpenAI format: replace content in messages array
                            if let Some(messages) =
                                working.get_mut("messages").and_then(|m| m.as_array_mut())
                            {
                                if let Some(msg) = messages.get_mut(idx) {
                                    msg["content"] = Value::String(condensed);
                                }
                            }
                        } else {
                            // Anthropic format: replace top-level system
                            working["system"] = Value::String(condensed);
                        }
                        info!(
                            "Semantic: system prompt condensed, {} -> {} tokens (saved {})",
                            sys_tokens,
                            sys_tokens - saved,
                            saved
                        );
                    }
                    Err(e) => {
                        info!("Semantic system fallback: {}", e);
                    }
                }
            }

            // --- History condensation ---
            // Gather non-system messages for token counting
            if let Some(messages) = working.get("messages").and_then(|m| m.as_array()).cloned() {
                let non_system: Vec<(usize, &Value)> = messages
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) != Some("system"))
                    .collect();
                let hist_tokens: usize = non_system
                    .iter()
                    .map(|(_, m)| {
                        state.token_counter.count_text_tokens(
                            &m.get("content").and_then(|c| c.as_str()).unwrap_or(""),
                        )
                    })
                    .sum();
                if hist_tokens >= state.config.semantic_history_threshold && non_system.len() > 6 {
                    // Keep last 4 non-system turns, condense the rest
                    let preserve = 4.min(non_system.len());
                    let condense_end = non_system.len() - preserve;
                    let to_condense: Vec<Value> = non_system[..condense_end]
                        .iter()
                        .map(|(_, m)| (*m).clone())
                        .collect();
                    let preserved: Vec<Value> = non_system[condense_end..]
                        .iter()
                        .map(|(_, m)| (*m).clone())
                        .collect();

                    match sem
                        .condense_history(&to_condense, &state.token_counter, cache_workspace, cache_user)
                        .await
                    {
                        Ok((condensed_msgs, saved)) => {
                            let mut new_msgs: Vec<Value> = Vec::new();
                            // Keep any system message at the front
                            if let Some(idx) = system_in_messages_idx {
                                new_msgs.push(messages[idx].clone());
                            }
                            new_msgs.extend(condensed_msgs);
                            new_msgs.extend(preserved);
                            let new_len = new_msgs.len();
                            working["messages"] = Value::Array(new_msgs);
                            info!(
                                "Semantic: history condensed, {} msgs -> {} msgs (saved {} tokens)",
                                non_system.len(),
                                new_len,
                                saved
                            );
                        }
                        Err(e) => {
                            info!("Semantic history fallback: {}", e);
                        }
                    }
                }
            }
        }
    }

    // Step 3: Core compression (normalize + rules + response compression + model profile)
    let compress_responses = state.config.compress_responses;
    let response_age = response_age_override.unwrap_or(state.config.response_compression_age);
    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    let (compressed, stats, resp_tokens_saved, resp_count) = compress_request(
        &working,
        level,
        state.config.normalize,
        state.config.inject_boundaries,
        compress_responses,
        response_age,
        model,
    );

    // Step 4: Stability guard (semantic validation)
    let final_result = compressed;

    let optimized_tokens = state.token_counter.count_request_tokens(&final_result);
    (
        final_result,
        original_tokens,
        optimized_tokens,
        stats,
        resp_tokens_saved,
        resp_count,
    )
}

// ─── Shared: log metrics ─────────────────────────────────────

fn log_metrics(
    state: &AppState,
    original_tokens: usize,
    optimized_tokens: usize,
    level: f64,
    latency_ms: f64,
    model: &str,
    request_id: &str,
    stats: &CompressionStats,
) {
    // Record into global analytics (atomic, lock-free)
    state
        .analytics
        .record_request(stats, original_tokens, optimized_tokens);

    let savings = original_tokens.saturating_sub(optimized_tokens);
    let savings_pct = if original_tokens > 0 {
        savings as f64 / original_tokens as f64 * 100.0
    } else {
        0.0
    };

    if state.config.log_metrics {
        let top_cats = stats.top_categories(5);
        let m = TokenMetrics {
            timestamp: chrono::Utc::now().timestamp() as f64,
            original_tokens,
            optimized_tokens,
            compression_ratio: optimized_tokens as f64 / original_tokens.max(1) as f64,
            token_savings: savings,
            savings_percent: savings_pct,
            compression_level: level,
            latency_ms,
            model: model.to_string(),
            request_id: request_id.to_string(),
            total_rule_hits: stats.total_rule_hits,
            top_categories: if top_cats.is_empty() {
                None
            } else {
                Some(top_cats)
            },
        };
        state.metrics_logger.log(&m);
    }
}

// ─── /v2/compress (compress-only; no upstream forward) ───────

/// Run the full compression pipeline and return the compressed request's
/// `messages` + token counts, WITHOUT calling any model. This is what makes
/// the Splicer cost-moat possible: compress the prompt once, then fan the
/// optimized prompt out to N models. Honors the same `x-nyquest-*` tuning
/// headers as the proxy endpoints so behaviour matches the inline path.
///
/// Input is OpenAI chat-completions shape (`{ model?, messages: [...] }`);
/// the response is `{ messages, model, original_tokens, optimized_tokens }`.
async fn compress_only(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let start = Instant::now();
    let header_map = headers_to_map(&headers);
    let cache_workspace = header_map.get("x-nyquest-workspace-id").map(|s| s.as_str());
    let cache_user = header_map.get("x-nyquest-user-id").map(|s| s.as_str());

    let header_level = header_map.get("x-nyquest-level").map(|s| s.as_str());
    let level = state.config.effective_level(header_level);

    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown")
        .to_string();

    let openclaw_enabled = state.config.openclaw_mode
        || header_map
            .get("x-nyquest-openclaw")
            .map(|v| v == "true")
            .unwrap_or(false);
    let response_age_override = header_map
        .get("x-nyquest-response-age")
        .and_then(|v| v.parse::<usize>().ok());

    let (compressed, original_tokens, optimized_tokens, comp_stats, _r_saved, _r_count) =
        run_pipeline(
            &body,
            &state,
            level,
            openclaw_enabled,
            response_age_override,
            cache_workspace,
            cache_user,
        )
        .await;

    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
    log_metrics(
        &state,
        original_tokens,
        optimized_tokens,
        level,
        latency_ms,
        &model,
        &request_id,
        &comp_stats,
    );

    let savings = original_tokens.saturating_sub(optimized_tokens);
    let savings_pct = if original_tokens > 0 {
        savings as f64 / original_tokens as f64 * 100.0
    } else {
        0.0
    };
    info!(
        "[{}] compress-only | level={:.1} | {}→{} tokens | saved {} ({:.1}%)",
        request_id, level, original_tokens, optimized_tokens, savings, savings_pct
    );

    // Prefer the compressed messages; fall back to the original on the
    // (shouldn't-happen) case the pipeline dropped the field.
    let messages = compressed
        .get("messages")
        .or_else(|| body.get("messages"))
        .cloned()
        .unwrap_or(Value::Null);

    Json(serde_json::json!({
        "messages": messages,
        "model": model,
        "original_tokens": original_tokens,
        "optimized_tokens": optimized_tokens,
    }))
    .into_response()
}

// ─── /v1/messages (Anthropic native) ─────────────────────────

async fn proxy_messages(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let start = Instant::now();
    let header_map = headers_to_map(&headers);

    let header_level = header_map.get("x-nyquest-level").map(|s| s.as_str());
    // Hardening 1: workspace + user for semantic-cache isolation. Both
    // optional — managed/anon traffic without them just shares the
    // global cache as before.
    let cache_workspace = header_map.get("x-nyquest-workspace-id").map(|s| s.as_str());
    let cache_user = header_map.get("x-nyquest-user-id").map(|s| s.as_str());
    let level = state.config.effective_level(header_level);

    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown");
    let is_streaming = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let openclaw_enabled = state.config.openclaw_mode
        || header_map
            .get("x-nyquest-openclaw")
            .map(|v| v == "true")
            .unwrap_or(false);
    let response_age_override = header_map
        .get("x-nyquest-response-age")
        .and_then(|v| v.parse::<usize>().ok());

    let (compressed, original_tokens, optimized_tokens, comp_stats, resp_saved, resp_count) =
        run_pipeline(
            &body,
            &state,
            level,
            openclaw_enabled,
            response_age_override,
            cache_workspace,
            cache_user,
        )
        .await;

    // Record response compression into analytics
    if resp_count > 0 {
        state.analytics.record_response_compression(resp_saved);
    }

    let savings = original_tokens.saturating_sub(optimized_tokens);
    let savings_pct = if original_tokens > 0 {
        savings as f64 / original_tokens as f64 * 100.0
    } else {
        0.0
    };

    let profile = crate::profiles::detect_profile(model);
    info!(
        "[{}] {} | level={:.1} | profile={} | {}→{} tokens | saved {} ({:.1}%)",
        request_id,
        model,
        level,
        profile.name,
        original_tokens,
        optimized_tokens,
        savings,
        savings_pct
    );

    // Build upstream request
    let provider_cfg = get_provider_config("anthropic", None);
    let upstream_headers = build_upstream_headers(
        "anthropic",
        &provider_cfg,
        &header_map,
        state.config.get_provider_key("anthropic").as_deref(),
    );

    let target_url = format!("{}/v1/messages", provider_cfg.base_url);

    let mut req_builder = state.http_client.post(&target_url).json(&compressed);
    for (k, v) in &upstream_headers {
        req_builder = req_builder.header(k.as_str(), v.as_str());
    }

    let nyquest_headers = |builder: axum::http::response::Builder| {
        builder
            .header("x-nyquest-original-tokens", original_tokens.to_string())
            .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
            .header("x-nyquest-savings-percent", format!("{:.1}", savings_pct))
            .header("x-nyquest-profile", profile.name)
            .header("x-nyquest-request-id", &request_id)
    };

    match req_builder.send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as f64;
            log_metrics(
                &state,
                original_tokens,
                optimized_tokens,
                level,
                latency_ms,
                model,
                &request_id,
                &comp_stats,
            );

            let status = resp.status();

            if is_streaming {
                // Stream SSE chunks through directly
                let byte_stream = resp
                    .bytes_stream()
                    .map(|chunk| chunk.map_err(|e| std::io::Error::other(e.to_string())));

                nyquest_headers(Response::builder().status(status.as_u16()))
                    .header("content-type", "text/event-stream")
                    .header("cache-control", "no-cache")
                    .body(Body::from_stream(byte_stream))
                    .unwrap()
            } else {
                let body_bytes = resp.bytes().await.unwrap_or_default();

                nyquest_headers(Response::builder().status(status.as_u16()))
                    .header("content-type", "application/json")
                    .body(Body::from(body_bytes))
                    .unwrap()
            }
        }
        Err(e) => {
            error!("[{}] Upstream error: {}", request_id, e);
            Response::builder()
                .status(502)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "error": {
                            "type": "connection_error",
                            "message": e.to_string()
                        }
                    })
                    .to_string(),
                ))
                .unwrap()
        }
    }
}

// ─── /v1/chat/completions (OpenAI-compat) ────────────────────

async fn proxy_chat_completions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let request_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    let start = Instant::now();
    let header_map = headers_to_map(&headers);
    let cache_workspace = header_map.get("x-nyquest-workspace-id").map(|s| s.as_str());
    let cache_user = header_map.get("x-nyquest-user-id").map(|s| s.as_str());

    let header_level = header_map.get("x-nyquest-level").map(|s| s.as_str());
    let level = state.config.effective_level(header_level);

    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .unwrap_or("gpt-4")
        .to_string();
    let is_streaming = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    let openclaw_enabled = state.config.openclaw_mode
        || header_map
            .get("x-nyquest-openclaw")
            .map(|v| v == "true")
            .unwrap_or(false);
    let response_age_override = header_map
        .get("x-nyquest-response-age")
        .and_then(|v| v.parse::<usize>().ok());

    // Detect target provider from model name
    let provider_name = detect_provider(&model, &header_map);
    let custom_base = header_map.get("x-nyquest-base-url").map(|s| s.as_str());
    let provider_cfg = get_provider_config(&provider_name, custom_base);

    let needs_conversion = provider_cfg.format == "anthropic";

    // If going to Anthropic, convert OpenAI → Anthropic wire format
    let (upstream_body, original_tokens, optimized_tokens, comp_stats) = if needs_conversion {
        let anthropic_body = openai_to_anthropic(&body);

        // Run compression on Anthropic format
        let (compressed, orig, opt, cstats, r_saved, r_count) = run_pipeline(
            &anthropic_body,
            &state,
            level,
            openclaw_enabled,
            response_age_override,
            cache_workspace,
            cache_user,
        )
        .await;
        if r_count > 0 {
            state.analytics.record_response_compression(r_saved);
        }

        let savings = orig.saturating_sub(opt);
        let savings_pct = if orig > 0 {
            savings as f64 / orig as f64 * 100.0
        } else {
            0.0
        };

        info!(
            "[{}] {} → {} | level={:.1} | {}→{} tokens | saved {} ({:.1}%) | {} rule hits",
            request_id,
            model,
            provider_name,
            level,
            orig,
            opt,
            savings,
            savings_pct,
            cstats.total_rule_hits
        );

        (compressed, orig, opt, cstats)
    } else {
        // OpenAI-format providers: compress messages in-place
        let (compressed, orig, opt, cstats, r_saved, r_count) = run_pipeline(
            &body,
            &state,
            level,
            openclaw_enabled,
            response_age_override,
            cache_workspace,
            cache_user,
        )
        .await;
        if r_count > 0 {
            state.analytics.record_response_compression(r_saved);
        }

        let savings = orig.saturating_sub(opt);
        let savings_pct = if orig > 0 {
            savings as f64 / orig as f64 * 100.0
        } else {
            0.0
        };

        info!(
            "[{}] {} → {} | level={:.1} | {}→{} tokens | saved {} ({:.1}%) | {} rule hits",
            request_id,
            model,
            provider_name,
            level,
            orig,
            opt,
            savings,
            savings_pct,
            cstats.total_rule_hits
        );

        (compressed, orig, opt, cstats)
    };

    // FIX 1: Enforce minimum max_tokens for thinking-model providers (Gemini, xAI Grok)
    // These providers use thinking tokens that consume from the max_tokens budget,
    // so a low value can result in zero content tokens and empty responses.
    let upstream_body = {
        let mut body = upstream_body;
        let is_thinking_provider = matches!(provider_name.as_str(), "gemini" | "xai");
        if is_thinking_provider {
            if let Some(max_tok) = body.get("max_tokens").and_then(|v| v.as_u64()) {
                let floor = 256_u64;
                if max_tok < floor {
                    info!(
                        "[{}] Raising max_tokens {} → {} for thinking-model provider {}",
                        request_id, max_tok, floor, provider_name
                    );
                    body["max_tokens"] = Value::Number(floor.into());
                }
            }
        }
        body
    };

    // Build upstream headers
    let upstream_headers = build_upstream_headers(
        &provider_name,
        &provider_cfg,
        &header_map,
        state.config.get_provider_key(&provider_name).as_deref(),
    );

    // Determine upstream URL
    let target_url = if needs_conversion {
        format!("{}/v1/messages", provider_cfg.base_url)
    } else {
        format!("{}/chat/completions", provider_cfg.base_url)
    };

    let mut req_builder = state.http_client.post(&target_url).json(&upstream_body);
    for (k, v) in &upstream_headers {
        req_builder = req_builder.header(k.as_str(), v.as_str());
    }

    match req_builder.send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as f64;
            log_metrics(
                &state,
                original_tokens,
                optimized_tokens,
                level,
                latency_ms,
                &model,
                &request_id,
                &comp_stats,
            );
            let status = resp.status();

            if is_streaming {
                if needs_conversion {
                    let model_for_stream = model.clone();
                    // Anthropic SSE → OpenAI SSE conversion for streaming
                    let byte_stream = resp.bytes_stream().map(move |chunk| {
                        match chunk {
                            Ok(bytes) => {
                                let text = String::from_utf8_lossy(&bytes);
                                // Pass through SSE events — convert on the fly
                                // Each SSE line that starts with "data: " may contain
                                // Anthropic event JSON that should be converted
                                let converted =
                                    convert_sse_anthropic_to_openai(&text, &model_for_stream);
                                Ok::<Bytes, std::io::Error>(Bytes::from(converted))
                            }
                            Err(e) => Err(std::io::Error::other(e.to_string())),
                        }
                    });

                    Response::builder()
                        .status(status.as_u16())
                        .header("content-type", "text/event-stream")
                        .header("cache-control", "no-cache")
                        .header("x-nyquest-original-tokens", original_tokens.to_string())
                        .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
                        .header("x-nyquest-savings-percent", format!("{:.1}",
                            if original_tokens > 0 { (original_tokens.saturating_sub(optimized_tokens)) as f64 / original_tokens as f64 * 100.0 } else { 0.0 }))
                        .header("x-nyquest-request-id", &request_id)
                        .body(Body::from_stream(byte_stream))
                        .unwrap()
                } else {
                    // Same-format streaming: pass through directly
                    let byte_stream = resp
                        .bytes_stream()
                        .map(|chunk| chunk.map_err(|e| std::io::Error::other(e.to_string())));

                    Response::builder()
                        .status(status.as_u16())
                        .header("content-type", "text/event-stream")
                        .header("cache-control", "no-cache")
                        .header("x-nyquest-original-tokens", original_tokens.to_string())
                        .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
                        .header("x-nyquest-savings-percent", format!("{:.1}",
                            if original_tokens > 0 { (original_tokens.saturating_sub(optimized_tokens)) as f64 / original_tokens as f64 * 100.0 } else { 0.0 }))
                        .header("x-nyquest-request-id", &request_id)
                        .body(Body::from_stream(byte_stream))
                        .unwrap()
                }
            } else {
                // Non-streaming
                let body_bytes = resp.bytes().await.unwrap_or_default();

                if needs_conversion {
                    // Convert Anthropic response → OpenAI format
                    let response_value: Value =
                        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);

                    if response_value.get("error").is_some() {
                        // Pass through errors
                        Response::builder()
                            .status(status.as_u16())
                            .header("content-type", "application/json")
                            .header("x-nyquest-original-tokens", original_tokens.to_string())
                        .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
                        .header("x-nyquest-savings-percent", format!("{:.1}",
                            if original_tokens > 0 { (original_tokens.saturating_sub(optimized_tokens)) as f64 / original_tokens as f64 * 100.0 } else { 0.0 }))
                        .header("x-nyquest-request-id", &request_id)
                            .body(Body::from(body_bytes))
                            .unwrap()
                    } else {
                        let openai_response = anthropic_to_openai_response(&response_value);
                        Response::builder()
                            .status(200)
                            .header("content-type", "application/json")
                            .header("x-nyquest-original-tokens", original_tokens.to_string())
                        .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
                        .header("x-nyquest-savings-percent", format!("{:.1}",
                            if original_tokens > 0 { (original_tokens.saturating_sub(optimized_tokens)) as f64 / original_tokens as f64 * 100.0 } else { 0.0 }))
                        .header("x-nyquest-request-id", &request_id)
                            .body(Body::from(
                                serde_json::to_string(&openai_response).unwrap_or_default(),
                            ))
                            .unwrap()
                    }
                } else {
                    Response::builder()
                        .status(status.as_u16())
                        .header("content-type", "application/json")
                        .header("x-nyquest-original-tokens", original_tokens.to_string())
                        .header("x-nyquest-optimized-tokens", optimized_tokens.to_string())
                        .header("x-nyquest-savings-percent", format!("{:.1}",
                            if original_tokens > 0 { (original_tokens.saturating_sub(optimized_tokens)) as f64 / original_tokens as f64 * 100.0 } else { 0.0 }))
                        .header("x-nyquest-request-id", &request_id)
                        .body(Body::from(body_bytes))
                        .unwrap()
                }
            }
        }
        Err(e) => {
            error!(
                "[{}] Upstream error for {}: {}",
                request_id, provider_name, e
            );
            Response::builder()
                .status(502)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "error": {
                            "type": "connection_error",
                            "message": e.to_string(),
                            "provider": provider_name,
                        }
                    })
                    .to_string(),
                ))
                .unwrap()
        }
    }
}

// ─── SSE Stream Conversion (Anthropic → OpenAI) ─────────────

fn convert_sse_anthropic_to_openai(chunk: &str, model: &str) -> String {
    let mut output = String::new();

    for line in chunk.lines() {
        if let Some(data) = line.strip_prefix("data: ") {
            if data.trim() == "[DONE]" {
                output.push_str("data: [DONE]\n\n");
                continue;
            }

            if let Ok(event) = serde_json::from_str::<Value>(data) {
                let event_type = event.get("type").and_then(|t| t.as_str()).unwrap_or("");

                match event_type {
                    "content_block_delta" => {
                        let delta = event.get("delta").unwrap_or(&Value::Null);
                        let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");

                        if delta_type == "text_delta" {
                            let text = delta.get("text").and_then(|t| t.as_str()).unwrap_or("");
                            let openai_chunk = serde_json::json!({
                                "id": format!("chatcmpl-nyq-stream"),
                                "object": "chat.completion.chunk",
                                "model": model,
                                "choices": [{
                                    "index": 0,
                                    "delta": { "content": text },
                                    "finish_reason": null,
                                }]
                            });
                            output.push_str(&format!("data: {}\n\n", openai_chunk));
                        }
                    }
                    "message_stop" => {
                        let openai_chunk = serde_json::json!({
                            "id": format!("chatcmpl-nyq-stream"),
                            "object": "chat.completion.chunk",
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "delta": {},
                                "finish_reason": "stop",
                            }]
                        });
                        output.push_str(&format!("data: {}\n\n", openai_chunk));
                        output.push_str("data: [DONE]\n\n");
                    }
                    "message_delta" => {
                        // Usage info in stream — could forward as x-headers but skip for now
                    }
                    _ => {
                        // message_start, content_block_start, ping — skip
                    }
                }
            } else {
                // Unrecognized data line — pass through
                output.push_str(line);
                output.push('\n');
            }
        } else if line.starts_with("event:") || line.is_empty() {
            // Skip Anthropic event: lines and blank separators
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}
