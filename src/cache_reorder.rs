//! Nyquest Prompt-Cache Reordering
//! Maximizes provider cache hit rates by deterministically ordering
//! static system content at consistent message indices.
//!
//! Anthropic and OpenAI cache based on exact token prefix matching.
//! If the system prompt is identical across requests, it gets cached.
//! This module ensures:
//!   1. System prompt text blocks are sorted deterministically
//!   2. Static boilerplate is separated from dynamic instructions
//!   3. Tool definitions maintain stable ordering

use serde_json::Value;
use tracing::debug;

/// Reorder system prompt blocks for cache optimization.
/// Static content (role definitions, rules, guidelines) goes first,
/// dynamic content (date-dependent, context-dependent) goes last.
pub fn reorder_for_cache(request: &Value) -> Value {
    let mut result = request.clone();

    // 1. Sort tool definitions by name (consistent order = cacheable prefix)
    if let Some(tools) = result.get("tools").and_then(|t| t.as_array()).cloned() {
        if tools.len() > 1 {
            let mut sorted = tools;
            sorted.sort_by(|a, b| {
                let name_a = a.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let name_b = b.get("name").and_then(|n| n.as_str()).unwrap_or("");
                name_a.cmp(name_b)
            });
            result["tools"] = Value::Array(sorted);
            debug!(
                "Cache reorder: sorted {} tools by name",
                result["tools"].as_array().map(|a| a.len()).unwrap_or(0)
            );
        }
    }

    // 2. Sort system prompt blocks: cache_control blocks first, then by stability
    if let Some(Value::Array(blocks)) = result.get("system").cloned() {
        if blocks.len() > 1 {
            let mut sorted = blocks;
            sorted.sort_by(|a, b| {
                let a_score = block_stability_score(a);
                let b_score = block_stability_score(b);
                // Higher stability = earlier position (more cacheable)
                b_score.cmp(&a_score)
            });
            result["system"] = Value::Array(sorted);
            debug!(
                "Cache reorder: sorted {} system blocks by stability",
                result["system"].as_array().map(|a| a.len()).unwrap_or(0)
            );
        }
    }

    result
}

/// Score how "stable" (cacheable) a system block is.
/// Higher = more stable = should come first in the array.
fn block_stability_score(block: &Value) -> u32 {
    let text = block
        .get("text")
        .and_then(|t| t.as_str())
        .or_else(|| block.as_str())
        .unwrap_or("");

    let mut score: u32 = 50; // baseline

    // Blocks with cache_control are explicitly meant to be cached → highest priority
    if block.get("cache_control").is_some() {
        score += 100;
    }

    // Dynamic content markers → lower stability
    let dynamic_markers = [
        "today",
        "current date",
        "current time",
        "right now",
        "as of",
        "recently",
        "latest",
        "updated",
    ];
    for marker in &dynamic_markers {
        if text.to_lowercase().contains(marker) {
            score = score.saturating_sub(20);
        }
    }

    // Role definitions are highly stable
    if text.to_lowercase().starts_with("you are") || text.to_lowercase().contains("your role") {
        score += 30;
    }

    // Long blocks are more likely to be static templates
    if text.len() > 500 {
        score += 10;
    }
    if text.len() > 2000 {
        score += 10;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_sorting() {
        let request = json!({
            "model": "claude-haiku-4-5-20251001",
            "tools": [
                {"name": "bash", "description": "Run commands"},
                {"name": "analyze", "description": "Analyze data"},
            ],
            "messages": [{"role": "user", "content": "hello"}]
        });

        let result = reorder_for_cache(&request);
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools[0]["name"], "analyze"); // alphabetical
        assert_eq!(tools[1]["name"], "bash");
    }

    #[test]
    fn test_system_block_ordering() {
        let request = json!({
            "model": "claude-haiku-4-5-20251001",
            "system": [
                {"type": "text", "text": "Today is February 28. Current news..."},
                {"type": "text", "text": "You are a helpful assistant. Your role is to...", "cache_control": {"type": "ephemeral"}},
            ],
            "messages": [{"role": "user", "content": "hello"}]
        });

        let result = reorder_for_cache(&request);
        let system = result["system"].as_array().unwrap();
        // Cached block with role definition should come first
        assert!(system[0].get("cache_control").is_some());
    }
}
