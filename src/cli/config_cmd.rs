//! Nyquest Config — show/get/set subcommands

use console::style;
use std::fs;
use std::path::Path;

pub fn run_show(config_path: &str) {
    let path = Path::new(config_path);
    if !path.exists() {
        eprintln!("  {} No config found at {}", style("✗").red(), config_path);
        std::process::exit(1);
    }

    let content = fs::read_to_string(path).unwrap_or_default();
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&content).unwrap_or_default();

    let flat = flatten_yaml(&yaml_val, "");
    let mut keys: Vec<&String> = flat.keys().collect();
    keys.sort();

    println!();
    println!("  {}", style("Nyquest Configuration").magenta().bold());
    println!("  {}", style("─".repeat(60)).dim());

    for key in keys {
        let val = &flat[key];
        let display = mask_secret(key, val);
        println!("  {:40} {}", style(key).white(), style(&display).magenta());
    }
    println!();
}

pub fn run_get(config_path: &str, key: &str) {
    let path = Path::new(config_path);
    if !path.exists() {
        eprintln!("  {} No config found at {}", style("✗").red(), config_path);
        std::process::exit(1);
    }

    let content = fs::read_to_string(path).unwrap_or_default();
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&content).unwrap_or_default();

    match yaml_navigate(&yaml_val, key) {
        Some(val) => {
            let s = yaml_to_string(&val);
            println!("{}", mask_secret(key, &s));
        }
        None => {
            eprintln!("  {} Key not found: {}", style("✗").red(), key);
            std::process::exit(1);
        }
    }
}

pub fn run_set(config_path: &str, key: &str, value: &str) {
    let path = Path::new(config_path);
    let content = if path.exists() {
        fs::read_to_string(path).unwrap_or_default()
    } else {
        String::new()
    };

    let mut yaml_val: serde_yaml::Value = serde_yaml::from_str(&content)
        .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

    let typed_val = auto_type(value);
    yaml_set_nested(&mut yaml_val, key, typed_val);

    // Preserve header
    let header = format!(
        "# ──────────────────────────────────────────────\n\
         # Nyquest Configuration\n\
         # Semantic Compression Proxy for LLMs — v{}\n\
         # ──────────────────────────────────────────────\n\n",
        crate::VERSION,
    );
    let yaml_str = serde_yaml::to_string(&yaml_val).unwrap_or_default();
    fs::write(path, format!("{}{}", header, yaml_str)).unwrap_or_else(|e| {
        eprintln!("  {} Failed to write: {}", style("✗").red(), e);
        std::process::exit(1);
    });

    println!("  {} Set {} = {}", style("✓").green(), key, value);
}

// ── Helpers ──

fn yaml_navigate(val: &serde_yaml::Value, dotkey: &str) -> Option<serde_yaml::Value> {
    let parts: Vec<&str> = dotkey.split('.').collect();
    let mut current = val.clone();
    for p in &parts {
        current = current.get(*p)?.clone();
    }
    Some(current)
}

fn yaml_set_nested(val: &mut serde_yaml::Value, dotkey: &str, new_val: serde_yaml::Value) {
    let parts: Vec<&str> = dotkey.split('.').collect();
    let mut current = val;
    for p in &parts[..parts.len() - 1] {
        if current.get(p).is_none() {
            current[*p] = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        current = &mut current[*p];
    }
    current[*parts.last().unwrap()] = new_val;
}

fn yaml_to_string(val: &serde_yaml::Value) -> String {
    match val {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "null".to_string(),
        _ => format!("{:?}", val),
    }
}

fn auto_type(s: &str) -> serde_yaml::Value {
    if s.eq_ignore_ascii_case("true") {
        return serde_yaml::Value::Bool(true);
    }
    if s.eq_ignore_ascii_case("false") {
        return serde_yaml::Value::Bool(false);
    }
    if let Ok(n) = s.parse::<i64>() {
        return serde_yaml::Value::Number(n.into());
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_yaml::Number::from(f).into() {
            return serde_yaml::Value::Number(n);
        }
    }
    serde_yaml::Value::String(s.to_string())
}

fn flatten_yaml(
    val: &serde_yaml::Value,
    prefix: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut result = std::collections::BTreeMap::new();
    if let serde_yaml::Value::Mapping(map) = val {
        for (k, v) in map {
            let key_str = k.as_str().unwrap_or("?");
            let full_key = if prefix.is_empty() {
                key_str.to_string()
            } else {
                format!("{}.{}", prefix, key_str)
            };
            if let serde_yaml::Value::Mapping(_) = v {
                result.extend(flatten_yaml(v, &full_key));
            } else {
                result.insert(full_key, yaml_to_string(v));
            }
        }
    }
    result
}

fn mask_secret(key: &str, val: &str) -> String {
    if key.contains("api_key") && val.len() > 12 {
        format!("{}••••{}", &val[..8], &val[val.len() - 4..])
    } else {
        val.to_string()
    }
}
