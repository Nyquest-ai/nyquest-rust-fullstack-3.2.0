//! Nyquest Multi-Provider Router
//! Translates between OpenAI-compatible and Anthropic wire formats.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    pub format: String,     // "anthropic" or "openai"
    pub auth_style: String, // "x-api-key", "bearer", "none"
    pub api_version: String,
}

/// Detect provider from model name or headers
pub fn detect_provider(model: &str, headers: &std::collections::HashMap<String, String>) -> String {
    // Explicit header override
    if let Some(provider) = headers.get("x-nyquest-provider") {
        let p = provider.to_lowercase();
        if [
            "anthropic",
            "openai",
            "gemini",
            "openrouter",
            "hpc",
            "xai",
            "local",
        ]
        .contains(&p.as_str())
        {
            return p;
        }
    }

    let model_lower = model.to_lowercase();

    // HPC check
    if model.starts_with("azure_ai/") || model.starts_with("vertex_ai/") {
        return "hpc".to_string();
    }

    if model_lower.contains("grok") {
        return "xai".to_string();
    }
    if model_lower.contains("claude")
        || model_lower.contains("haiku")
        || model_lower.contains("sonnet")
        || model_lower.contains("opus")
    {
        return "anthropic".to_string();
    }
    if model_lower.contains("gemini") {
        return "gemini".to_string();
    }
    if model_lower.contains("gpt-") || model_lower.contains("o1") || model_lower.contains("o3") {
        return "openai".to_string();
    }
    if model.contains('/') {
        return "openrouter".to_string();
    }

    "openai".to_string()
}

/// Get provider config with optional custom base URL
pub fn get_provider_config(provider: &str, custom_base_url: Option<&str>) -> ProviderConfig {
    let mut cfg = match provider {
        "anthropic" => ProviderConfig {
            name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            format: "anthropic".into(),
            auth_style: "x-api-key".into(),
            api_version: "2023-06-01".into(),
        },
        "openai" => ProviderConfig {
            name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
        "gemini" => ProviderConfig {
            name: "Google Gemini".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
        "openrouter" => ProviderConfig {
            name: "OpenRouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
        "hpc" => ProviderConfig {
            name: "FAU HPC".into(),
            base_url: "http://127.0.0.1:18800/v1".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
        "xai" => ProviderConfig {
            name: "xAI (Grok)".into(),
            base_url: "https://api.x.ai/v1".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
        "local" => ProviderConfig {
            name: "Local (Ollama)".into(),
            base_url: "http://localhost:11434/v1".into(),
            format: "openai".into(),
            auth_style: "none".into(),
            api_version: String::new(),
        },
        _ => ProviderConfig {
            name: "OpenAI-Compatible".into(),
            base_url: "https://api.openai.com/v1".into(),
            format: "openai".into(),
            auth_style: "bearer".into(),
            api_version: String::new(),
        },
    };

    if let Some(url) = custom_base_url {
        if !url.is_empty() {
            cfg.base_url = url.trim_end_matches('/').to_string();
        }
    }
    cfg
}

/// Convert OpenAI chat/completions format to Anthropic Messages format
pub fn openai_to_anthropic(body: &Value) -> Value {
    let mut result = serde_json::Map::new();
    result.insert(
        "model".into(),
        body.get("model")
            .cloned()
            .unwrap_or(Value::String("claude-haiku-4-5-20251001".into())),
    );
    result.insert(
        "max_tokens".into(),
        body.get("max_tokens")
            .or(body.get("max_completion_tokens"))
            .cloned()
            .unwrap_or(Value::Number(4096.into())),
    );

    let mut system_parts = Vec::new();
    let mut conversation = Vec::new();

    if let Some(Value::Array(messages)) = body.get("messages") {
        for msg in messages {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
            let content = msg
                .get("content")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            if role == "system" {
                match &content {
                    Value::String(s) => system_parts.push(s.clone()),
                    Value::Array(arr) => {
                        for block in arr {
                            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                system_parts.push(t.to_string());
                            } else if let Some(s) = block.as_str() {
                                system_parts.push(s.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                conversation.push(serde_json::json!({"role": role, "content": content}));
            }
        }
    }

    if !system_parts.is_empty() {
        result.insert("system".into(), Value::String(system_parts.join("\n\n")));
    }
    result.insert("messages".into(), Value::Array(conversation));

    for key in ["temperature", "top_p", "stop", "stream"] {
        if let Some(val) = body.get(key) {
            result.insert(key.into(), val.clone());
        }
    }

    Value::Object(result)
}

/// Convert Anthropic Messages response to OpenAI chat/completions format
pub fn anthropic_to_openai_response(response_data: &Value) -> Value {
    let content_blocks = response_data
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in &content_blocks {
        if let Some(btype) = block.get("type").and_then(|t| t.as_str()) {
            match btype {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                        text_parts.push(t.to_string());
                    }
                }
                "tool_use" => {
                    tool_calls.push(serde_json::json!({
                        "id": block.get("id").and_then(|i| i.as_str()).unwrap_or(""),
                        "type": "function",
                        "function": {
                            "name": block.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                            "arguments": serde_json::to_string(&block.get("input").unwrap_or(&Value::Null)).unwrap_or_default(),
                        }
                    }));
                }
                _ => {}
            }
        }
    }

    let mut message = serde_json::json!({"role": "assistant"});
    if !text_parts.is_empty() {
        message["content"] = Value::String(text_parts.join("\n"));
    } else {
        message["content"] = Value::Null;
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = Value::Array(tool_calls);
    }

    let usage = response_data
        .get("usage")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let input_tokens = usage
        .get("input_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0);

    let stop_reason = response_data
        .get("stop_reason")
        .and_then(|s| s.as_str())
        .unwrap_or("end_turn");
    let finish_reason = match stop_reason {
        "end_turn" => "stop",
        "max_tokens" => "length",
        "tool_use" => "tool_calls",
        _ => "stop",
    };

    serde_json::json!({
        "id": format!("chatcmpl-nyq-{}", response_data.get("id").and_then(|i| i.as_str()).unwrap_or("unknown")),
        "object": "chat.completion",
        "model": response_data.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens,
        }
    })
}

/// Build upstream headers for provider
pub fn build_upstream_headers(
    _provider: &str,
    config: &ProviderConfig,
    request_headers: &std::collections::HashMap<String, String>,
    config_api_key: Option<&str>,
) -> std::collections::HashMap<String, String> {
    let mut headers = std::collections::HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());

    match config.auth_style.as_str() {
        "x-api-key" => {
            // Priority: request header → auth bearer → config key
            if let Some(k) = request_headers.get("x-api-key") {
                headers.insert("x-api-key".to_string(), k.clone());
            } else if let Some(auth) = request_headers.get("authorization") {
                if let Some(key) = auth.strip_prefix("Bearer ") {
                    headers.insert("x-api-key".to_string(), key.to_string());
                }
            } else if let Some(k) = config_api_key {
                headers.insert("x-api-key".to_string(), k.to_string());
            }
            if !config.api_version.is_empty() {
                headers.insert(
                    "anthropic-version".to_string(),
                    request_headers
                        .get("anthropic-version")
                        .cloned()
                        .unwrap_or(config.api_version.clone()),
                );
            }
            if let Some(beta) = request_headers.get("anthropic-beta") {
                headers.insert("anthropic-beta".to_string(), beta.clone());
            }
        }
        "bearer" => {
            if let Some(auth) = request_headers.get("authorization") {
                headers.insert("authorization".to_string(), auth.clone());
            } else if let Some(key) = request_headers.get("x-api-key") {
                headers.insert("authorization".to_string(), format!("Bearer {}", key));
            } else if let Some(key) = config_api_key {
                headers.insert("authorization".to_string(), format!("Bearer {}", key));
            }
        }
        _ => {} // "none"
    }

    headers
}
