//! Nyquest Format Optimizer
//! Converts verbose data formats into token-efficient alternatives.
//!
//! Key transforms:
//!   - JSON blocks → YAML (removes braces, quotes, colons overhead)
//!   - Markdown tables → pipe-delimited compact format
//!   - Repeated JSON arrays → CSV-style tabular format

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

static JSON_BLOCK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)```json\s*\n([\s\S]*?)\n\s*```").unwrap());

#[allow(dead_code)]
static INLINE_JSON_RE: Lazy<Regex> = Lazy::new(|| {
    // Match standalone JSON objects/arrays that are >= 100 chars
    Regex::new(r"(?s)(\{[^{}]{100,}\}|\[[^\[\]]{100,}\])").unwrap()
});

static MD_TABLE_RE: Lazy<Regex> = Lazy::new(|| {
    // Markdown table: header row | separator row | data rows
    Regex::new(r"(?m)(^\|[^\n]+\|\s*\n\|[-\s|:]+\|\s*\n(?:\|[^\n]+\|\s*\n?)+)").unwrap()
});

/// Convert a JSON value to compact YAML-style text.
/// For arrays of objects with identical keys, use CSV format instead.
fn json_to_compact(value: &Value) -> String {
    match value {
        // Array of objects with identical keys → CSV
        Value::Array(arr) if arr.len() >= 2 => {
            if let Some(keys) = uniform_object_keys(arr) {
                return array_to_csv(arr, &keys);
            }
            // Non-uniform array → YAML list
            let items: Vec<String> = arr
                .iter()
                .map(|v| format!("- {}", value_to_yaml_inline(v)))
                .collect();
            items.join("\n")
        }
        // Single object → YAML
        Value::Object(map) => {
            let lines: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let val = value_to_yaml_inline(v);
                    format!("{}: {}", k, val)
                })
                .collect();
            lines.join("\n")
        }
        _ => value.to_string(),
    }
}

/// Convert a value to inline YAML representation
fn value_to_yaml_inline(value: &Value) -> String {
    match value {
        Value::Null => "~".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            // Only quote if contains special chars
            if s.contains(':')
                || s.contains('#')
                || s.contains('\n')
                || s.starts_with(' ')
                || s.ends_with(' ')
                || s.is_empty()
            {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(value_to_yaml_inline).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(map) => {
            let items: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", k, value_to_yaml_inline(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// Check if all array elements are objects with the same keys
fn uniform_object_keys(arr: &[Value]) -> Option<Vec<String>> {
    let first = arr.first()?.as_object()?;
    let keys: Vec<String> = first.keys().cloned().collect();
    if keys.is_empty() {
        return None;
    }

    for item in &arr[1..] {
        let obj = item.as_object()?;
        if obj.len() != keys.len() {
            return None;
        }
        for k in &keys {
            if !obj.contains_key(k) {
                return None;
            }
        }
    }
    Some(keys)
}

/// Convert array of uniform objects to CSV format
fn array_to_csv(arr: &[Value], keys: &[String]) -> String {
    let mut lines = Vec::with_capacity(arr.len() + 1);
    lines.push(keys.join(","));

    for item in arr {
        if let Some(obj) = item.as_object() {
            let vals: Vec<String> = keys
                .iter()
                .map(|k| {
                    let v = obj.get(k).unwrap_or(&Value::Null);
                    match v {
                        Value::String(s) => {
                            if s.contains(',') || s.contains('"') || s.contains('\n') {
                                format!("\"{}\"", s.replace('"', "\"\""))
                            } else {
                                s.clone()
                            }
                        }
                        Value::Null => String::new(),
                        other => other.to_string(),
                    }
                })
                .collect();
            lines.push(vals.join(","));
        }
    }
    lines.join("\n")
}

/// Compact JSON code blocks in text to YAML/CSV format
pub fn compact_json_blocks(text: &str) -> String {
    let mut result = text.to_string();

    // Convert ```json ... ``` blocks
    result = JSON_BLOCK_RE
        .replace_all(&result, |caps: &regex::Captures| {
            let json_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            match serde_json::from_str::<Value>(json_str) {
                Ok(value) => {
                    let compact = json_to_compact(&value);
                    // Only use compact form if it's actually shorter
                    if compact.len() < json_str.len() {
                        compact
                    } else {
                        caps[0].to_string()
                    }
                }
                Err(_) => caps[0].to_string(),
            }
        })
        .to_string();

    result
}

/// Flatten markdown tables into pipe-delimited compact format
pub fn flatten_markdown_tables(text: &str) -> String {
    MD_TABLE_RE
        .replace_all(text, |caps: &regex::Captures| {
            let table_text = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let lines: Vec<&str> = table_text.lines().collect();

            if lines.len() < 3 {
                return table_text.to_string();
            }

            // Parse header
            let header = parse_table_row(lines[0]);
            // Skip separator (line 1)
            let mut rows = Vec::new();
            for line in &lines[2..] {
                let row = parse_table_row(line);
                if !row.is_empty() {
                    rows.push(row);
                }
            }

            if header.is_empty() || rows.is_empty() {
                return table_text.to_string();
            }

            // Build compact format: header|header|header\nval|val|val
            let mut output = header.join("|");
            for row in &rows {
                output.push('\n');
                // Pad or truncate row to match header length
                let vals: Vec<String> = (0..header.len())
                    .map(|i| row.get(i).cloned().unwrap_or_default())
                    .collect();
                output.push_str(&vals.join("|"));
            }

            // Only use if shorter
            if output.len() < table_text.len() {
                output
            } else {
                table_text.to_string()
            }
        })
        .to_string()
}

/// Parse a markdown table row into cells
fn parse_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return Vec::new();
    }

    trimmed
        .split('|')
        .skip(1) // leading empty
        .filter(|s| {
            !s.trim().is_empty() && !s.trim().chars().all(|c| c == '-' || c == ':' || c == ' ')
        })
        .map(|s| s.trim().to_string())
        .collect()
}

/// Convert a JSON tool schema to compact TypeScript-style interface
pub fn schema_to_typescript(name: &str, schema: &Value) -> String {
    let mut parts = Vec::new();

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        let required: Vec<String> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        for (key, prop_def) in props {
            let ts_type = json_type_to_ts(prop_def);
            let optional = if required.contains(key) { "" } else { "?" };
            parts.push(format!("  {}{}: {}", key, optional, ts_type));
        }
    }

    format!("{}({{{}}})", name, parts.join(", "))
}

/// Convert a JSON Schema type to TypeScript type string
fn json_type_to_ts(schema: &Value) -> String {
    let type_str = schema.get("type").and_then(|t| t.as_str()).unwrap_or("any");

    match type_str {
        "string" => {
            if let Some(enum_vals) = schema.get("enum").and_then(|e| e.as_array()) {
                let vals: Vec<String> = enum_vals
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| format!("\"{}\"", s)))
                    .collect();
                return vals.join("|");
            }
            "string".to_string()
        }
        "integer" | "number" => "number".to_string(),
        "boolean" => "boolean".to_string(),
        "array" => {
            if let Some(items) = schema.get("items") {
                format!("{}[]", json_type_to_ts(items))
            } else {
                "any[]".to_string()
            }
        }
        "object" => {
            if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                let fields: Vec<String> = props
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, json_type_to_ts(v)))
                    .collect();
                format!("{{{}}}", fields.join(", "))
            } else {
                "object".to_string()
            }
        }
        _ => "any".to_string(),
    }
}

/// Compact raw JSON strings (tool results often contain unfenced JSON)
/// Only converts JSON objects/arrays that are >= min_size characters.
pub fn compact_inline_json(text: &str, min_size: usize) -> String {
    // Quick check: does this look like it contains substantial JSON?
    let trimmed = text.trim();
    if trimmed.len() < min_size {
        return text.to_string();
    }

    // Try parsing the entire text as JSON
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            let compact = json_to_compact(&value);
            if compact.len() < trimmed.len() {
                return compact;
            }
        }
    }

    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_array_to_csv() {
        let json = r#"[{"name":"Alice","age":30},{"name":"Bob","age":25}]"#;
        let value: Value = serde_json::from_str(json).unwrap();
        let result = json_to_compact(&value);
        assert!(result.contains("age,name"));
        assert!(result.contains("30,Alice"));
    }

    #[test]
    fn test_schema_to_ts() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
                "timeout": {"type": "integer"}
            },
            "required": ["command"]
        });
        let result = schema_to_typescript("bash", &schema);
        assert!(result.contains("command: string"));
        assert!(result.contains("timeout?: number"));
    }
}
