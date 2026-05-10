//! Nyquest Interactive Setup Wizard
//!
//! Drives both `nyquest install` and `nyquest configure`.
//! Walks through config sections with skip-friendly prompts.

use crate::config::NyquestConfig;
use console::style;
use dialoguer::{Confirm, Input, Password, Select};

use std::fs;

use std::net::TcpListener;
use std::path::{Path, PathBuf};

// ── Section & Prompt Definitions ──

#[derive(Clone)]
struct PromptDef {
    key: &'static str,
    prompt: &'static str,
    help: Option<&'static str>,
    ptype: PType,
    required: bool,
    env_var: Option<&'static str>,
    condition: Option<fn(&serde_yaml::Value) -> bool>,
    choices: Option<Vec<&'static str>>,
}

#[derive(Clone)]
enum PType {
    Text,
    Int,
    Float,
    Bool,
    Secret,
    Select,
}

struct Section {
    key: &'static str,
    title: &'static str,
    icon: &'static str,
    preamble: Option<&'static str>,
    prompts: Vec<PromptDef>,
}

fn sections() -> Vec<Section> {
    vec![
        Section {
            key: "server",
            title: "Server Settings",
            icon: "🌐",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "host", prompt: "Bind address",
                    help: Some("0.0.0.0 = LAN accessible, 127.0.0.1 = localhost only"),
                    ptype: PType::Select, required: true, env_var: Some("NYQUEST_HOST"),
                    condition: None, choices: Some(vec!["0.0.0.0", "127.0.0.1"]),
                },
                PromptDef {
                    key: "port", prompt: "Port",
                    help: Some("Must be available. Common alternatives: 8080, 3000"),
                    ptype: PType::Int, required: true, env_var: Some("NYQUEST_PORT"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "request_timeout", prompt: "Request timeout (seconds)",
                    help: Some("Increase for long generations (300+ for large context)"),
                    ptype: PType::Int, required: true, env_var: Some("NYQUEST_TIMEOUT"),
                    condition: None, choices: None,
                },
            ],
        },
        Section {
            key: "providers",
            title: "Provider API Keys",
            icon: "🔑",
            preamble: Some("Press Enter to skip any key you don't have yet.\nEnv vars also work (shown after each prompt)."),
            prompts: vec![
                PromptDef {
                    key: "providers.anthropic.api_key", prompt: "Anthropic API key",
                    help: Some("From console.anthropic.com → API Keys"),
                    ptype: PType::Secret, required: false, env_var: Some("ANTHROPIC_API_KEY"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.anthropic.base_url", prompt: "Anthropic base URL",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.openai.api_key", prompt: "OpenAI API key",
                    help: Some("From platform.openai.com → API keys"),
                    ptype: PType::Secret, required: false, env_var: Some("OPENAI_API_KEY"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.openai.base_url", prompt: "OpenAI base URL",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.gemini.api_key", prompt: "Gemini API key",
                    help: Some("From aistudio.google.com → API keys"),
                    ptype: PType::Secret, required: false, env_var: Some("GEMINI_API_KEY"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.gemini.base_url", prompt: "Gemini base URL",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.xai.api_key", prompt: "xAI (Grok) API key",
                    help: Some("From console.x.ai → API keys"),
                    ptype: PType::Secret, required: false, env_var: Some("XAI_API_KEY"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.xai.base_url", prompt: "xAI base URL",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.openrouter.api_key", prompt: "OpenRouter API key",
                    help: Some("From openrouter.ai → Keys"),
                    ptype: PType::Secret, required: false, env_var: Some("OPENROUTER_API_KEY"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.openrouter.base_url", prompt: "OpenRouter base URL",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "providers.local.base_url", prompt: "Local model base URL (Ollama, etc.)",
                    help: Some("No API key needed for local models"),
                    ptype: PType::Text, required: false, env_var: None,
                    condition: None, choices: None,
                },
            ],
        },
        Section {
            key: "compression",
            title: "Compression",
            icon: "📦",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "compression_level", prompt: "Compression level (0.0–1.0)",
                    help: Some("0.0 = pass-through, 1.0 = max compression. 0.7 recommended."),
                    ptype: PType::Float, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "adaptive_mode", prompt: "Adaptive mode (auto-adjust per prompt type)",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "allow_header_override", prompt: "Allow per-request level override via header",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
            ],
        },
        Section {
            key: "normalization",
            title: "Normalization (Hallucination Mitigation)",
            icon: "🛡️",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "normalize", prompt: "Enable normalization (dedup, conflict resolution)",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "inject_boundaries", prompt: "Inject anti-speculation guardrails",
                    help: Some("Adds boundary markers to reduce hallucination"),
                    ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
            ],
        },
        Section {
            key: "context",
            title: "Context Window Optimization",
            icon: "🧠",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "context_optimization", prompt: "Enable context optimization",
                    help: Some("Summarizes old turns to save tokens"),
                    ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "context_max_input_tokens", prompt: "Max input tokens before optimization triggers",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "context_optimization", true)),
                    choices: None,
                },
                PromptDef {
                    key: "context_preserve_recent_turns", prompt: "Recent turns to always preserve",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "context_optimization", true)),
                    choices: None,
                },
                PromptDef {
                    key: "context_min_turns", prompt: "Minimum turns before optimization activates",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "context_optimization", true)),
                    choices: None,
                },
            ],
        },
        Section {
            key: "openclaw",
            title: "OpenClaw Agent Mode",
            icon: "🤖",
            preamble: Some("7-strategy optimization for autonomous AI agents.\nOnly relevant if routing agent traffic through Nyquest."),
            prompts: vec![
                PromptDef {
                    key: "openclaw_mode", prompt: "Enable OpenClaw agent mode",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "openclaw_tool_prune_turns", prompt: "Prune tool results after N turns",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_thought_prune_turns", prompt: "Prune thought blocks after N turns",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_schema_minimize", prompt: "Minimize tool schemas",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_dedup_errors", prompt: "Deduplicate stack traces",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_condense_views", prompt: "Condense file/terminal output",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_cache_control", prompt: "Inject Anthropic cache breakpoints",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_sliding_window", prompt: "Enable sliding window for infinite context",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_sliding_window_threshold", prompt: "Sliding window threshold (0.0–1.0)",
                    help: None, ptype: PType::Float, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false) && yaml_bool(v, "openclaw_sliding_window", true)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_sliding_window_max_tokens", prompt: "Sliding window max tokens",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false) && yaml_bool(v, "openclaw_sliding_window", true)),
                    choices: None,
                },
                PromptDef {
                    key: "openclaw_sliding_window_preserve", prompt: "Turns to preserve at window start",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "openclaw_mode", false) && yaml_bool(v, "openclaw_sliding_window", true)),
                    choices: None,
                },
            ],
        },
        Section {
            key: "response_compression",
            title: "Response Compression",
            icon: "💬",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "compress_responses", prompt: "Compress older assistant messages",
                    help: Some("Reduces noise in multi-turn conversations"),
                    ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "response_compression_age", prompt: "Compress responses older than N turns",
                    help: None, ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "compress_responses", true)),
                    choices: None,
                },
            ],
        },
        Section {
            key: "security",
            title: "Security",
            icon: "🔒",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "privacy_mode", prompt: "Privacy mode (zero prompt logging)",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "encrypt_keys", prompt: "Encrypt API keys with Fernet vault",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "vault_path", prompt: "Vault file path",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "encrypt_keys", false)),
                    choices: None,
                },
            ],
        },
        Section {
            key: "logging",
            title: "Logging",
            icon: "📊",
            preamble: None,
            prompts: vec![
                PromptDef {
                    key: "log_metrics", prompt: "Log per-request metrics to JSONL",
                    help: None, ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "log_file", prompt: "Metrics log file path",
                    help: None, ptype: PType::Text, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "log_metrics", true)),
                    choices: None,
                },
                PromptDef {
                    key: "log_level", prompt: "Log level",
                    help: None, ptype: PType::Select, required: true, env_var: Some("NYQUEST_LOG_LEVEL"),
                    condition: None, choices: Some(vec!["DEBUG", "INFO", "WARNING", "ERROR"]),
                },
            ],
        },
        Section {
            key: "semantic",
            title: "Semantic LLM Stage",
            icon: "🧬",
            preamble: Some("Local LLM semantic compression using Qwen 2.5 1.5B via Ollama.\nCondenses system prompts (56%) and conversation history (75%).\nRequires: Ollama installed, 2+ GB VRAM (GPU) or 8+ GB RAM (CPU)."),
            prompts: vec![
                PromptDef {
                    key: "semantic_enabled", prompt: "Enable semantic LLM compression",
                    help: Some("Requires Ollama + Qwen 2.5 1.5B. Disable for rules-only mode."),
                    ptype: PType::Bool, required: true, env_var: Some("NYQUEST_SEMANTIC_ENABLED"),
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "semantic_endpoint", prompt: "Ollama endpoint URL",
                    help: Some("Default Ollama API: http://localhost:11434/v1/chat/completions"),
                    ptype: PType::Text, required: true, env_var: Some("NYQUEST_SEMANTIC_ENDPOINT"),
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: None,
                },
                PromptDef {
                    key: "semantic_model", prompt: "Semantic model name",
                    help: Some("Qwen 2.5 1.5B recommended. Larger models improve quality but increase latency."),
                    ptype: PType::Text, required: true, env_var: Some("NYQUEST_SEMANTIC_MODEL"),
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: None,
                },
                PromptDef {
                    key: "semantic_timeout_ms", prompt: "Semantic timeout (ms)",
                    help: Some("Max wait for LLM response. Falls back to extractive on timeout. 3000ms for GPU, 10000ms for CPU."),
                    ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: None,
                },
                PromptDef {
                    key: "semantic_system_threshold", prompt: "System prompt threshold (tokens)",
                    help: Some("Semantic condensation fires on system prompts above this size. Default: 4000."),
                    ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: None,
                },
                PromptDef {
                    key: "semantic_history_threshold", prompt: "History threshold (tokens)",
                    help: Some("Semantic condensation fires on conversation history above this size. Default: 8000."),
                    ptype: PType::Int, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: None,
                },
                PromptDef {
                    key: "semantic_fallback", prompt: "Fallback mode on timeout",
                    help: Some("'extractive' = first/last sentence extraction. 'none' = skip semantic stage."),
                    ptype: PType::Select, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "semantic_enabled", false)),
                    choices: Some(vec!["extractive", "none"]),
                },
            ],
        },
        Section {
            key: "stability",
            title: "Stability Mode (Dev/Testing)",
            icon: "⚗️",
            preamble: Some("Dual-send validation mode. Doubles API costs. For development only."),
            prompts: vec![
                PromptDef {
                    key: "stability_mode", prompt: "Enable stability mode",
                    help: Some("Sends both compressed & original prompts to compare"),
                    ptype: PType::Bool, required: true, env_var: None,
                    condition: None, choices: None,
                },
                PromptDef {
                    key: "semantic_threshold", prompt: "Semantic similarity threshold (0.0–1.0)",
                    help: None, ptype: PType::Float, required: true, env_var: None,
                    condition: Some(|v| yaml_bool(v, "stability_mode", false)),
                    choices: None,
                },
            ],
        },
    ]
}

// ── YAML Helpers ──

fn yaml_bool(val: &serde_yaml::Value, key: &str, default: bool) -> bool {
    val.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

fn yaml_get(val: &serde_yaml::Value, dotkey: &str) -> Option<serde_yaml::Value> {
    let parts: Vec<&str> = dotkey.split('.').collect();
    let mut current = val.clone();
    for p in &parts {
        current = current.get(*p)?.clone();
    }
    Some(current)
}

fn yaml_set(val: &mut serde_yaml::Value, dotkey: &str, new_val: serde_yaml::Value) {
    let parts: Vec<&str> = dotkey.split('.').collect();
    let mut current = val;
    for p in &parts[..parts.len() - 1] {
        if !current.get(p).is_some() {
            current[*p] = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        }
        current = &mut current[*p];
    }
    current[*parts.last().unwrap()] = new_val;
}

fn yaml_display(val: &serde_yaml::Value) -> String {
    match val {
        serde_yaml::Value::String(s) => {
            if (s.starts_with("sk-") || s.starts_with("AIza") || s.starts_with("xai-"))
                && s.len() > 12
            {
                format!("{}••••{}", &s[..8], &s[s.len() - 4..])
            } else {
                s.clone()
            }
        }
        serde_yaml::Value::Bool(b) => {
            if *b {
                "yes".into()
            } else {
                "no".into()
            }
        }
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => "not set".into(),
        _ => format!("{:?}", val),
    }
}

// ── Banner & Rendering ──

fn print_banner(mode: &str) {
    let term = console::Term::stdout();
    let _ = term.clear_screen();
    let mode_label = if mode == "install" {
        "Interactive Setup"
    } else {
        "Reconfigure"
    };
    println!();
    println!(
        "  {}",
        style("╔══════════════════════════════════════════════════╗").magenta()
    );
    println!(
        "  {}  {} {} {} {}  {}",
        style("║").magenta(),
        style("NYQUEST").magenta().bold(),
        style(crate::VERSION).white(),
        style("—").white(),
        style(mode_label).white(),
        style("║").magenta(),
    );
    println!(
        "  {}",
        style("╚══════════════════════════════════════════════════╝").magenta()
    );
    println!("  {}", style("  AI Prompt Compression Engine").dim());
    println!();
}

fn print_section(title: &str, icon: &str) {
    println!();
    println!("  {}  {}", icon, style(title).magenta().bold());
    let bar_len = title.len() + 4;
    println!("  {}", style("─".repeat(bar_len)).dim());
}

fn print_success(msg: &str) {
    println!("  {} {}", style("✓").green(), msg);
}

fn print_warn(msg: &str) {
    println!("  {} {}", style("⚠").yellow(), msg);
}

fn print_error(msg: &str) {
    println!("  {} {}", style("✗").red(), msg);
}

fn print_hint(msg: &str) {
    println!("    {}", style(msg).dim());
}

fn print_skip_hint(key: &str, env_var: Option<&str>) {
    println!(
        "    {} {}",
        style("→ Skipped.").green(),
        style("Add later:").dim()
    );
    println!(
        "      {}",
        style(format!("nyquest config set {} <value>", key)).green()
    );
    if let Some(ev) = env_var {
        println!(
            "      {}",
            style(format!("or export {}=<value>", ev)).green()
        );
    }
    println!();
}

// ── Wizard Engine ──

pub fn run_install(config_path: &str, defaults: bool, overrides: &[String]) {
    let path = Path::new(config_path);
    let mut yaml_val = load_yaml(path);
    let mut skipped: Vec<String> = Vec::new();

    if defaults {
        run_headless(&mut yaml_val, overrides, &mut skipped);
        write_yaml(path, &yaml_val);
        write_env(path, &yaml_val, &skipped);
        write_metadata(path, &skipped);
        print_summary(path, &skipped);
        return;
    }

    print_banner("install");
    system_checks();

    let secs = sections();
    for section in &secs {
        run_section(section, &mut yaml_val, &mut skipped);
    }

    // Apply overrides
    apply_overrides(&mut yaml_val, overrides);

    // Offer systemd service
    offer_systemd(path, &yaml_val);

    write_yaml(path, &yaml_val);
    write_env(path, &yaml_val, &skipped);
    write_metadata(path, &skipped);
    print_summary(path, &skipped);
}

pub fn run_configure(config_path: &str, section_filter: Option<&str>) {
    let path = Path::new(config_path);
    if !path.exists() {
        print_error(&format!("No config found at {}", config_path));
        println!("  Run {} first.", style("nyquest install").magenta());
        return;
    }

    let mut yaml_val = load_yaml(path);
    let mut skipped: Vec<String> = Vec::new();

    print_banner("configure");
    println!("  Loaded existing config: {}", style(config_path).magenta());
    println!("  Current values shown as defaults. Press Enter to keep.");

    let secs = sections();
    for section in &secs {
        if let Some(filter) = section_filter {
            if section.key != filter {
                continue;
            }
        }
        run_section(section, &mut yaml_val, &mut skipped);
    }

    if section_filter.is_some() {
        // Check if the filter matched anything
        let secs = sections();
        let valid: Vec<&str> = secs.iter().map(|s| s.key).collect();
        if let Some(f) = section_filter {
            if !valid.contains(&f) {
                print_error(&format!("Unknown section: {}", f));
                println!("  Available: {}", valid.join(", "));
                return;
            }
        }
    }

    write_yaml(path, &yaml_val);
    write_env(path, &yaml_val, &skipped);
    print_summary(path, &skipped);
}

fn run_section(section: &Section, yaml_val: &mut serde_yaml::Value, skipped: &mut Vec<String>) {
    print_section(section.title, section.icon);

    if let Some(pre) = section.preamble {
        for line in pre.lines() {
            print_hint(line);
        }
        println!();
    }

    for p in &section.prompts {
        if let Err(_) = handle_prompt(p, yaml_val, skipped) {
            // Ctrl+C or error — bail gracefully
            println!();
            print_warn("Setup interrupted. Partial config NOT saved.");
            std::process::exit(1);
        }
    }
}

fn handle_prompt(
    p: &PromptDef,
    yaml_val: &mut serde_yaml::Value,
    skipped: &mut Vec<String>,
) -> Result<(), ()> {
    // Check condition
    if let Some(cond) = p.condition {
        if !cond(yaml_val) {
            return Ok(());
        }
    }

    let current = yaml_get(yaml_val, p.key);
    let default_display = current
        .as_ref()
        .map(|v| yaml_display(v))
        .unwrap_or_else(|| "not set".into());

    if let Some(help) = p.help {
        print_hint(help);
    }

    let result: Option<serde_yaml::Value> = match p.ptype {
        PType::Bool => {
            let default_bool = current.as_ref().and_then(|v| v.as_bool()).unwrap_or(true);
            let ans = Confirm::new()
                .with_prompt(format!("  {}", p.prompt))
                .default(default_bool)
                .interact_opt()
                .map_err(|_| ())?;
            ans.map(|b| serde_yaml::Value::Bool(b))
        }
        PType::Select => {
            let choices = p.choices.clone().unwrap_or_default();
            let default_idx = current
                .as_ref()
                .and_then(|v| v.as_str())
                .and_then(|s| choices.iter().position(|c| *c == s))
                .unwrap_or(0);
            let ans = Select::new()
                .with_prompt(format!("  {}", p.prompt))
                .items(&choices)
                .default(default_idx)
                .interact_opt()
                .map_err(|_| ())?;
            ans.map(|i: usize| serde_yaml::Value::String(choices[i].to_string()))
        }
        PType::Secret => {
            let ans = Password::new()
                .with_prompt(format!(
                    "  {} [{}]",
                    p.prompt,
                    style(&default_display).yellow()
                ))
                .allow_empty_password(true)
                .interact()
                .map_err(|_| ())?;
            if ans.trim().is_empty() {
                // Keep existing or skip
                if current.is_some() {
                    return Ok(()); // Keep existing value
                }
                None // Skip
            } else {
                Some(serde_yaml::Value::String(ans.trim().to_string()))
            }
        }
        PType::Text => {
            let default_str = current
                .as_ref()
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ans: String = Input::new()
                .with_prompt(format!("  {}", p.prompt))
                .default(default_str.clone())
                .allow_empty(true)
                .interact_text()
                .map_err(|_| ())?;
            if ans.trim().is_empty() && !p.required {
                None
            } else if ans.trim().is_empty() {
                Some(serde_yaml::Value::String(default_str))
            } else {
                Some(serde_yaml::Value::String(ans.trim().to_string()))
            }
        }
        PType::Int => {
            let default_int = current.as_ref().and_then(|v| v.as_u64()).unwrap_or(0);
            let ans: String = Input::new()
                .with_prompt(format!("  {}", p.prompt))
                .default(default_int.to_string())
                .interact_text()
                .map_err(|_| ())?;
            match ans.trim().parse::<u64>() {
                Ok(n) => Some(serde_yaml::Value::Number(serde_yaml::Number::from(n))),
                Err(_) => {
                    print_warn("Invalid number, keeping current value.");
                    return Ok(());
                }
            }
        }
        PType::Float => {
            let default_f = current.as_ref().and_then(|v| v.as_f64()).unwrap_or(0.0);
            let ans: String = Input::new()
                .with_prompt(format!("  {}", p.prompt))
                .default(format!("{}", default_f))
                .interact_text()
                .map_err(|_| ())?;
            match ans.trim().parse::<f64>() {
                Ok(f) => Some(serde_yaml::Value::Number(serde_yaml::Number::from(f))),
                Err(_) => {
                    print_warn("Invalid number, keeping current value.");
                    return Ok(());
                }
            }
        }
    };

    match result {
        Some(val) => {
            yaml_set(yaml_val, p.key, val);
        }
        None => {
            if !p.required {
                skipped.push(p.key.to_string());
                print_skip_hint(p.key, p.env_var);
            }
        }
    }

    Ok(())
}

// ── System Checks ──

fn system_checks() {
    print_success(&format!("Nyquest v{} (Rust engine)", crate::VERSION));

    let exe = std::env::current_exe().unwrap_or_default();
    print_success(&format!("Binary: {}", exe.display()));

    // Check port
    let port: u16 = 5400;
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_) => print_success(&format!("Port {} available", port)),
        Err(_) => print_warn(&format!(
            "Port {} in use (Nyquest may already be running)",
            port
        )),
    }

    // Check logs dir
    if Path::new("logs").exists() {
        print_success("Logs directory exists");
    } else {
        let _ = fs::create_dir_all("logs");
        print_success("Created logs directory");
    }

    println!();
}

// ── Headless Install ──

fn run_headless(yaml_val: &mut serde_yaml::Value, overrides: &[String], skipped: &mut Vec<String>) {
    println!("  Running headless install with defaults...");
    let secs = sections();
    for section in &secs {
        for p in &section.prompts {
            // Skip conditions
            if let Some(cond) = p.condition {
                if !cond(yaml_val) {
                    continue;
                }
            }
            // If no existing value, use default concept from struct defaults
            let existing = yaml_get(yaml_val, p.key);
            if existing.is_none() && !p.required {
                skipped.push(p.key.to_string());
            }
        }
    }
    apply_overrides(yaml_val, overrides);
    print_success("Headless install complete");
}

fn apply_overrides(yaml_val: &mut serde_yaml::Value, overrides: &[String]) {
    for ov in overrides {
        if let Some((k, v)) = ov.split_once('=') {
            let val = auto_type_yaml(v.trim());
            yaml_set(yaml_val, k.trim(), val);
            print_success(&format!("Override: {} = {}", k.trim(), v.trim()));
        }
    }
}

fn auto_type_yaml(s: &str) -> serde_yaml::Value {
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

// ── File I/O ──

fn load_yaml(path: &Path) -> serde_yaml::Value {
    if path.exists() {
        let content = fs::read_to_string(path).unwrap_or_default();
        serde_yaml::from_str(&content)
            .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    } else {
        // Load defaults from NyquestConfig
        let defaults = NyquestConfig::default();
        serde_yaml::to_value(&defaults)
            .unwrap_or(serde_yaml::Value::Mapping(serde_yaml::Mapping::new()))
    }
}

fn write_yaml(config_path: &Path, yaml_val: &serde_yaml::Value) {
    let header = format!(
        "# ──────────────────────────────────────────────\n\
         # Nyquest Configuration\n\
         # Semantic Compression Proxy for LLMs — v{}\n\
         # Generated by nyquest configure — {}\n\
         # ──────────────────────────────────────────────\n\n",
        crate::VERSION,
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    let yaml_str = serde_yaml::to_string(yaml_val).unwrap_or_default();
    let content = format!("{}{}", header, yaml_str);
    fs::write(config_path, &content).unwrap_or_else(|e| {
        print_error(&format!("Failed to write {}: {}", config_path.display(), e));
    });
    print_success(&format!("Config written: {}", config_path.display()));
}

fn write_env(config_path: &Path, yaml_val: &serde_yaml::Value, skipped: &[String]) {
    let env_path = config_path.with_file_name(".env");
    let mut lines = vec![
        "# Nyquest Environment Variables".to_string(),
        format!(
            "# Generated by nyquest configure — {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M")
        ),
        String::new(),
    ];

    // Map known env vars
    let env_mappings = vec![
        ("providers.anthropic.api_key", "ANTHROPIC_API_KEY"),
        ("providers.openai.api_key", "OPENAI_API_KEY"),
        ("providers.gemini.api_key", "GEMINI_API_KEY"),
        ("providers.xai.api_key", "XAI_API_KEY"),
        ("providers.openrouter.api_key", "OPENROUTER_API_KEY"),
    ];

    for (key, env_var) in &env_mappings {
        if skipped.contains(&key.to_string()) {
            lines.push(format!("# {}=  # TODO: add your key", env_var));
        } else if let Some(val) = yaml_get(yaml_val, key) {
            if let Some(s) = val.as_str() {
                if !s.is_empty() {
                    lines.push(format!("{}={}", env_var, s));
                }
            }
        }
    }

    lines.push(String::new());
    fs::write(&env_path, lines.join("\n")).unwrap_or_else(|e| {
        print_warn(&format!("Failed to write .env: {}", e));
    });
}

fn write_metadata(config_path: &Path, skipped: &[String]) {
    let home = dirs_fallback();
    let meta_dir = home.join(".nyquest");
    let _ = fs::create_dir_all(&meta_dir);

    let install_dir = config_path.parent().unwrap_or(Path::new("."));
    let meta = serde_json::json!({
        "version": crate::VERSION,
        "installed_at": chrono::Local::now().to_rfc3339(),
        "install_dir": install_dir.canonicalize().unwrap_or(install_dir.to_path_buf()).display().to_string(),
        "config_path": config_path.display().to_string(),
        "skipped_settings": skipped,
    });

    let meta_path = meta_dir.join("install.json");
    let _ = fs::write(
        &meta_path,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );
}

fn dirs_fallback() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/tmp"))
        })
}

// ── Systemd Service ──

fn offer_systemd(config_path: &Path, _yaml_val: &serde_yaml::Value) {
    println!();
    let install_svc = Confirm::new()
        .with_prompt("  Install/update systemd user service?")
        .default(true)
        .interact()
        .unwrap_or(false);

    if !install_svc {
        return;
    }

    // Resolve install_dir to an absolute path — config_path may be relative
    let install_dir = config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let install_dir = std::fs::canonicalize(install_dir).unwrap_or_else(|_| {
        std::env::current_dir().unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join("nyquest"))
                .unwrap_or_else(|_| PathBuf::from("nyquest"))
        })
    });
    let exe =
        std::env::current_exe().unwrap_or_else(|_| install_dir.join("target/release/nyquest"));
    let _user = std::env::var("USER").unwrap_or_else(|_| "mike".into());
    let home = dirs_fallback();

    // Ensure .env exists so EnvironmentFile doesn't fail
    let env_path = install_dir.join(".env");
    if !env_path.exists() {
        let _ = fs::write(&env_path, "# Nyquest environment variables\n");
    }

    // NOTE: User=/Group= are omitted — they cause 'Failed to determine
    // supplementary groups' in --user services. ProtectSystem=strict also
    // causes issues in user mode. WantedBy=default.target for user services.
    let service = format!(
        "[Unit]\n\
         Description=Nyquest Semantic Compression Proxy v{version} (Full Rust Stack)\n\
         After=network.target\n\n\
         [Service]\n\
         Type=simple\n\
         WorkingDirectory={workdir}\n\
         EnvironmentFile=-{workdir}/.env\n\
         ExecStart={exe}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         StandardOutput=journal\n\
         StandardError=journal\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        version = crate::VERSION,
        workdir = install_dir.display(),
        exe = exe.display(),
    );

    // Write to project dir
    let svc_path = install_dir.join("nyquest.service");
    fs::write(&svc_path, &service).unwrap_or_else(|e| {
        print_error(&format!("Failed to write service file: {}", e));
    });

    // Copy to systemd dir
    let systemd_dir = home.join(".config/systemd/user");
    let _ = fs::create_dir_all(&systemd_dir);
    let target = systemd_dir.join("nyquest.service");
    let _ = fs::copy(&svc_path, &target);
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "enable", "nyquest.service"])
        .status();

    print_success("Systemd service installed and enabled");
    print_hint("Start with: systemctl --user start nyquest");
}

// ── Summary ──

fn print_summary(config_path: &Path, skipped: &[String]) {
    let env_path = config_path.with_file_name(".env");
    println!();
    println!(
        "  {}",
        style("╔══════════════════════════════════════════════════╗").green()
    );
    println!(
        "  {}  {}  {}",
        style("║").green(),
        style("✓ Nyquest configured successfully!").green().bold(),
        style("            ║").green(),
    );
    println!(
        "  {}",
        style("╚══════════════════════════════════════════════════╝").green()
    );
    println!();
    println!("  Config: {}", style(config_path.display()).magenta());
    println!("  Env:    {}", style(env_path.display()).magenta());

    if !skipped.is_empty() {
        println!();
        println!(
            "  {}",
            style("⚠ Skipped settings (configure later):").yellow()
        );
        for key in skipped {
            let short = key.split('.').last().unwrap_or(key);
            println!(
                "    • {:30} → {}",
                short,
                style(format!("nyquest config set {} <value>", key)).green()
            );
        }
    }

    println!();
    println!("  {}", style("Quick commands:").bold());
    println!("    Start:       {}", style("nyquest serve").magenta());
    println!(
        "    Start (svc): {}",
        style("systemctl --user start nyquest").magenta()
    );
    println!("    Reconfigure: {}", style("nyquest configure").magenta());
    println!("    Health check: {}", style("nyquest doctor").magenta());
    println!(
        "    Show config: {}",
        style("nyquest config show").magenta()
    );
    println!();
}

/// List available section names
pub fn list_sections() -> Vec<&'static str> {
    sections().iter().map(|s| s.key).collect()
}
