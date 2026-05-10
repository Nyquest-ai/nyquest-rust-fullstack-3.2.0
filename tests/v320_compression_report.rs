//! v3.2.0 compression measurement & telemetry report.
//!
//! Run with:
//!     cargo test --test v320_compression_report -- --nocapture --test-threads=1
//!
//! Tests in this file share global mutable state (the LRU compression cache
//! and per-rule atomic hit counters). They are serialized internally by
//! `TEST_MUTEX` so they remain correct under bare `cargo test` (which would
//! otherwise parallelize them). The `--test-threads=1` flag is still useful
//! for clean ordered output but no longer required for correctness.

use nyquest::compression::compress_request;
use nyquest::compression::engine::CompressionEngine;
use nyquest::compression::rules;
use nyquest::compression::GLOBAL_CACHE;
use nyquest::profiles::detect_profile;
use nyquest::tokens::TokenCounter;
use serde_json::json;
use std::sync::Mutex;
use std::time::Instant;

/// Serializes the six tests in this file. Lock at the top of each #[test]
/// before touching global cache state or per-rule counters.
static TEST_MUTEX: Mutex<()> = Mutex::new(());

// ───────────────────────────────────────────────────────────────────────────
// Corpus — 10 scenarios chosen to exercise v3.2.0 features specifically.
// ───────────────────────────────────────────────────────────────────────────

struct Scenario {
    name: &'static str,
    #[allow(dead_code)] // surfaces in future per-category report rows
    category: &'static str,
    model: &'static str,
    system: &'static str,
    user: &'static str,
}

const SCENARIOS: &[Scenario] = &[
    // 1. Heavy filler/verbose system prompt — should compress hard.
    Scenario {
        name: "Verbose system prompt",
        category: "Verbosity",
        model: "claude-sonnet-4-5",
        system: "You are a customer support specialist. Please note that it is important to note that you should always be polite. \
                 In order to facilitate effective communication, please make sure to acknowledge their concerns and work diligently to \
                 resolve their issues. It is necessary to note that you should consider factors such as the customer's prior history. \
                 Always remember to thank the customer for their patience and for using our service. Please ensure that all responses \
                 are clear, accurate, and up-to-date.",
        user: "My order #4521 hasn't shipped yet. Can you check the status?",
    },
    // 2. Date-heavy prose — verify four-digit-year preservation.
    Scenario {
        name: "Date-heavy briefing",
        category: "Date safety",
        model: "claude-sonnet-4-5",
        system: "You are a historical briefing assistant.",
        user: "Compare the 2008 financial crisis (September 15, 2008) with the 1929 crash (October 29, 1929) \
               and the dot-com bust (March 10, 2000). Cross-reference January 14, 2099 hypotheticals.",
    },
    // 3. Markdown-heavy — verify SOURCE_CODE_COMPRESSION doesn't strip headings.
    Scenario {
        name: "Markdown-heavy doc",
        category: "Markdown safety",
        model: "claude-sonnet-4-5",
        system: "You are a technical writing assistant.",
        user: "# Project Overview\n\n## Goals\n\nDeliver a system that handles 1000 req/s with p99 < 50ms.\n\n## Architecture\n\n# Tier 1 components\n# Tier 2 components\n\nReview the above outline and suggest gaps.",
    },
    // 4. Code-block content — SOURCE_CODE rules SHOULD fire here.
    Scenario {
        name: "Code review request",
        category: "Code",
        model: "claude-sonnet-4-5",
        system: "You are a senior code reviewer. Always think step by step and carefully analyze the code.",
        user: "```python\n# This is a comment\ndef foo(  x  ,  y  ) :\n    # another comment\n    return x + y\n\n# end\n```\nReview the above for style and correctness.",
    },
    // 5. JSON tool-result heavy — exercises tool_result content compression + length contract.
    Scenario {
        name: "JSON tool result",
        category: "JSON",
        model: "claude-sonnet-4-5",
        system: "You are an API debugging assistant.",
        user: "Here's the response payload from /api/v2/users:\n{\"users\":[{\"id\":1,\"name\":\"alice\"},{\"id\":2,\"name\":\"bob\"},{\"id\":3,\"name\":\"carol\"},{\"id\":4,\"name\":\"dave\"},{\"id\":5,\"name\":\"erin\"}]}\nWhat looks off?",
    },
    // 6. Role-only short prompt — verify it's NEVER emptied.
    Scenario {
        name: "Role-only prompt",
        category: "Role safety",
        model: "claude-sonnet-4-5",
        system: "You are a senior staff network engineer specializing in BGP routing and high-availability systems.",
        user: "Walk me through diagnosing an asymmetric routing issue.",
    },
    // 7. Multi-turn-style repeated history — exercises the LRU cache.
    Scenario {
        name: "Repeated history (cache)",
        category: "Cache",
        model: "claude-sonnet-4-5",
        system: "You are a coding assistant. Please ensure that you always provide clear and accurate explanations. \
                 It is important to note that you should consider both performance and readability when reviewing code.",
        user: "Repeat the same context check three times — verify the cache is doing its job.",
    },
    // 8. Non-English text — should pass through largely intact.
    Scenario {
        name: "Non-English text",
        category: "i18n safety",
        model: "claude-sonnet-4-5",
        system: "你是一个翻译助手。Please always preserve technical terminology in the original language.",
        user: "Translate to English: 你好世界。Привет мир. مرحبا بالعالم. こんにちは世界。",
    },
    // 9. Conservative-profile model — should compress less aggressively.
    Scenario {
        name: "Conservative profile (haiku)",
        category: "Profile",
        model: "claude-haiku-4-5",
        system: "You are a customer support specialist. Please note that it is important to note that you should always be polite. \
                 In order to facilitate effective communication, please make sure to acknowledge their concerns and work diligently to \
                 resolve their issues.",
        user: "My order #4521 hasn't shipped yet. Can you check the status?",
    },
    // 10. Long technical doc — exercises many categories.
    Scenario {
        name: "Long technical doc",
        category: "Mixed",
        model: "claude-opus-4-7",
        system: "You are a senior infrastructure engineer. Always think step by step and carefully analyze. \
                 Please ensure that recommendations are clear, accurate, and up-to-date. Make sure to consider both \
                 performance and reliability. Please always include risk assessments where applicable. \
                 Due to the fact that downtime is expensive, prioritize stability. In order to facilitate \
                 cross-team alignment, recommend incremental rollouts. It is important to note that observability \
                 must be designed in from the start. With over 15 years of experience in the field, you should \
                 always provide concrete, actionable recommendations.",
        user: "Design a migration plan for moving 50 microservices from EC2 to EKS, including rollback steps, \
               observability requirements (metrics/logs/traces), and a 90-day timeline.",
    },
];

fn build_request(s: &Scenario) -> serde_json::Value {
    json!({
        "model": s.model,
        "max_tokens": 4096,
        "system": s.system,
        "messages": [{ "role": "user", "content": s.user }],
    })
}

// ───────────────────────────────────────────────────────────────────────────
// Test 1 — per-scenario compression at level 0.5, 0.7, 1.0
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t01_report_compression_by_level() {
    let _guard = TEST_MUTEX.lock().unwrap();
    let tc = TokenCounter::new();
    let levels = [0.5_f64, 0.7, 1.0];

    println!();
    println!("┌{:─<98}┐", "");
    println!(
        "│ {:^96} │",
        "v3.2.0 compression report — savings by scenario × level"
    );
    println!("├{:─<98}┤", "");
    println!(
        "│ {:<28} {:<14} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} │",
        "Scenario", "Profile", "orig", "L0.5", "save%", "L0.7", "save%", "L1.0", "save%"
    );
    println!("├{:─<98}┤", "");

    let mut total_orig = 0_usize;
    let mut total_l10 = 0_usize;

    for s in SCENARIOS {
        let req = build_request(s);
        let orig = tc.count_request_tokens(&req);
        total_orig += orig;

        let profile_name = detect_profile(s.model).name;
        let mut row = format!(
            "│ {:<28} {:<14} {:>6}",
            truncate(s.name, 28),
            profile_name,
            orig
        );
        for &lvl in &levels {
            let (compressed, _stats, _saved, _resp) =
                compress_request(&req, lvl, true, false, false, 4, s.model);
            let opt = tc.count_request_tokens(&compressed);
            let pct = if orig == 0 {
                0.0
            } else {
                (1.0 - opt as f64 / orig as f64) * 100.0
            };
            row.push_str(&format!(" {:>6} {:>5.1}%", opt, pct));
            if (lvl - 1.0).abs() < 1e-6 {
                total_l10 += opt;
            }
        }
        println!("{} │", row);
    }

    println!("├{:─<98}┤", "");
    let total_pct = (1.0 - total_l10 as f64 / total_orig.max(1) as f64) * 100.0;
    println!(
        "│ AGGREGATE @ L1.0   {:<24}{:>16} {:>6}  →  {:>6}  ({:>4.1}% saved){:>22} │",
        "",
        total_orig,
        total_l10,
        total_orig.saturating_sub(total_l10),
        total_pct,
        ""
    );
    println!("└{:─<98}┘", "");

    // Sanity floor — across the corpus we expect *some* savings at L1.0.
    assert!(
        total_orig > total_l10,
        "level=1.0 must reduce total tokens across the corpus"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Test 2 — cache hit-rate on replay
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t02_report_cache_hit_rate_on_replay() {
    let _guard = TEST_MUTEX.lock().unwrap();
    // The TEST_MUTEX guarantees no other test is flipping cache.set_enabled
    // while we're in the 200-replay loop. Make sure the cache is on regardless.
    GLOBAL_CACHE.set_enabled(true);
    // Use a unique-ish payload so we don't collide with prior test cache entries.
    let unique_marker = format!("[CACHE-TEST-{}]", std::process::id());
    let text = format!(
        "{} You are an assistant. Please always provide clear, accurate, and \
         up-to-date information. In order to facilitate good decision making, \
         consider both short-term and long-term implications.",
        unique_marker
    );

    let snap_before = GLOBAL_CACHE.snapshot();

    // First call MUST be a miss-then-store; subsequent identical calls MUST hit.
    let first_t = Instant::now();
    let mut engine = CompressionEngine::with_profile(1.0, detect_profile("claude-sonnet-4-5"));
    let _ = engine.compress_text(&text);
    let first_us = first_t.elapsed().as_micros();

    let mut hit_us_total = 0_u128;
    let replay_n = 200_u32;
    for _ in 0..replay_n {
        let mut e = CompressionEngine::with_profile(1.0, detect_profile("claude-sonnet-4-5"));
        let t = Instant::now();
        let _ = e.compress_text(&text);
        hit_us_total += t.elapsed().as_micros();
    }

    let snap_after = GLOBAL_CACHE.snapshot();
    let hits_delta = snap_after.hits.saturating_sub(snap_before.hits);
    let misses_delta = snap_after.misses.saturating_sub(snap_before.misses);
    let stores_delta = snap_after.stores.saturating_sub(snap_before.stores);
    let avg_hit_us = hit_us_total / replay_n as u128;

    println!();
    println!("┌{:─<70}┐", "");
    println!("│ {:^68} │", "v3.2.0 LRU cache effectiveness");
    println!("├{:─<70}┤", "");
    println!("│ first compress (cold)            {:>30} µs │", first_us);
    println!("│ avg replay (warm, n=200)         {:>30} µs │", avg_hit_us);
    println!(
        "│ cache delta — hits / misses / stores   {:>10} / {:>5} / {:>5}     │",
        hits_delta, misses_delta, stores_delta
    );
    println!(
        "│ cache size (entries / capacity)        {:>5} / {:>5}             │",
        snap_after.entries, snap_after.capacity
    );
    println!("└{:─<70}┘", "");

    assert!(
        hits_delta >= replay_n as usize,
        "expected at least {} cache hits, got {}",
        replay_n,
        hits_delta
    );
    assert!(
        avg_hit_us * 4 < first_us.max(10), // warm should be at least 4× faster
        "warm replay ({}µs) should be much faster than cold ({}µs)",
        avg_hit_us,
        first_us
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Test 3 — top-firing rules across the corpus
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t03_report_top_firing_rules() {
    let _guard = TEST_MUTEX.lock().unwrap();
    // Snapshot every category's per-rule hit counts BEFORE the corpus, then
    // run the corpus, then diff. This isolates THIS test's contribution from
    // any state left by earlier tests.
    //
    // Critical: temporarily disable the LRU cache. Earlier tests already
    // compressed every corpus payload, so the cache would short-circuit the
    // pipeline and no rules would actually fire on this run.
    GLOBAL_CACHE.set_enabled(false);

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

    // before snapshot
    let before: Vec<(String, Vec<usize>)> = categories
        .iter()
        .map(|c| {
            (
                c.name.to_string(),
                c.rules.iter().map(|r| r.hits()).collect(),
            )
        })
        .collect();

    // run corpus at L1.0 with normalize on
    for s in SCENARIOS {
        let req = build_request(s);
        let _ = compress_request(&req, 1.0, true, false, false, 4, s.model);
    }

    // Re-enable the cache for any tests that run after this one.
    GLOBAL_CACHE.set_enabled(true);

    // collect deltas
    let mut all_deltas: Vec<(String, String, usize)> = Vec::new();
    let mut category_totals: Vec<(String, usize)> = Vec::new();
    for (cat_idx, cat) in categories.iter().enumerate() {
        let mut cat_total = 0_usize;
        for (i, rule) in cat.rules.iter().enumerate() {
            let after = rule.hits();
            let delta = after.saturating_sub(before[cat_idx].1[i]);
            if delta > 0 {
                all_deltas.push((cat.name.to_string(), rule.id.clone(), delta));
            }
            cat_total += delta;
        }
        category_totals.push((cat.name.to_string(), cat_total));
    }

    // sort
    all_deltas.sort_by(|a, b| b.2.cmp(&a.2));
    category_totals.sort_by(|a, b| b.1.cmp(&a.1));

    println!();
    println!("┌{:─<70}┐", "");
    println!(
        "│ {:^68} │",
        "v3.2.0 per-category hit distribution (corpus run)"
    );
    println!("├{:─<70}┤", "");
    let mut shown_any = false;
    for (name, n) in &category_totals {
        if *n > 0 {
            println!("│ {:<40} {:>22} │", name, n);
            shown_any = true;
        }
    }
    if !shown_any {
        println!(
            "│ (no category fired — corpus may be too small)              {:>10} │",
            ""
        );
    }
    println!("└{:─<70}┘", "");

    println!();
    println!("┌{:─<70}┐", "");
    println!("│ {:^68} │", "v3.2.0 top 12 firing rules (corpus run)");
    println!("├{:─<70}┤", "");
    for (cat, id, n) in all_deltas.iter().take(12) {
        println!("│ {:<22} {:<32} {:>8} │", cat, id, n);
    }
    println!("└{:─<70}┘", "");

    assert!(
        category_totals.iter().any(|(_, n)| *n > 0),
        "at least one rule category should have fired against the corpus"
    );
}

// ───────────────────────────────────────────────────────────────────────────
// Test 4 — profile detection drives different compression depth
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t04_report_profile_aware_compression() {
    let _guard = TEST_MUTEX.lock().unwrap();
    let tc = TokenCounter::new();

    // Crafted to hit categories that are *disabled in conservative*
    // (adjective_collapse, clause_simplify, adverb_strip) so the per-profile
    // savings actually diverge:
    //   - intensifier adverbs:        very, extremely, highly, particularly, really
    //   - process adverbs:            carefully analyze, thoroughly review
    //   - triple-adjective patterns:  "X, Y, and Z [noun]"
    //   - clause_simplify hooks:      ", recognizing that ...", "while remaining X and Y"
    let system = "You are a very experienced customer support specialist. Please note that it is \
                  extremely important to carefully analyze each ticket and thoroughly review the customer's \
                  prior history. Provide responses that are clear, accurate, and timely. Address technical \
                  issues, billing questions, and product inquiries with care, recognizing that the customer's \
                  time is valuable for both retention and brand reputation. While remaining empathetic and \
                  professional, work diligently to resolve their issues and follow up within 24 hours.";
    let user = "My order #4521 hasn't shipped yet. Can you check the status?";

    let probe_models = [
        ("claude-opus-4-7", "aggressive"),
        ("claude-sonnet-4-5", "aggressive"),
        ("gpt-4o", "aggressive"),
        ("claude-haiku-4-5", "conservative"),
        ("gpt-4o-mini", "conservative"),
        ("gemini-2.5-flash", "conservative"),
        // Future-proof — sonnet mini should be conservative.
        ("claude-sonnet-4-6-mini", "conservative"),
        // Default → balanced
        ("unknown-model-2099", "balanced"),
    ];

    // Cache from earlier tests would mask profile differences (same content,
    // already cached for the first model). Disable around the loop.
    GLOBAL_CACHE.set_enabled(false);

    println!();
    println!("┌{:─<74}┐", "");
    println!("│ {:^72} │", "v3.2.0 profile detection × compression depth");
    println!("├{:─<74}┤", "");
    println!(
        "│ {:<28} {:<14} {:>6} {:>6} {:>8}    │",
        "Model", "Profile", "orig", "L1.0", "save%"
    );
    println!("├{:─<74}┤", "");

    let mut by_profile: std::collections::HashMap<&'static str, usize> =
        std::collections::HashMap::new();
    for (model, expected) in probe_models {
        let actual = detect_profile(model).name;
        assert_eq!(actual, expected, "profile mismatch for {}", model);

        let req = json!({
            "model": model,
            "max_tokens": 4096,
            "system": system,
            "messages": [{"role": "user", "content": user}],
        });
        let orig = tc.count_request_tokens(&req);
        let (compressed, _stats, _, _) = compress_request(&req, 1.0, true, false, false, 4, model);
        let opt = tc.count_request_tokens(&compressed);
        let pct = (1.0 - opt as f64 / orig.max(1) as f64) * 100.0;

        by_profile.entry(actual).or_insert(opt);

        println!(
            "│ {:<28} {:<14} {:>6} {:>6} {:>7.1}%    │",
            truncate(model, 28),
            actual,
            orig,
            opt,
            pct
        );
    }
    println!("└{:─<74}┘", "");

    GLOBAL_CACHE.set_enabled(true);

    // Sanity: aggressive must compress at least as hard as conservative.
    if let (Some(&agg), Some(&cons)) =
        (by_profile.get("aggressive"), by_profile.get("conservative"))
    {
        assert!(
            agg <= cons,
            "aggressive ({} tokens) should compress no worse than conservative ({} tokens)",
            agg,
            cons
        );
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Test 5 — fail-closed length contract holds across the corpus
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t05_length_contract_holds_across_corpus() {
    let _guard = TEST_MUTEX.lock().unwrap();
    // For every (scenario, level), compressed system prompt must never be longer
    // than the original. This is the v3.2.0 fail-closed length contract.
    let mut violations = 0_usize;
    for s in SCENARIOS {
        let mut e = CompressionEngine::with_profile(1.0, detect_profile(s.model));
        for level in [0.0_f64, 0.2, 0.5, 0.7, 1.0] {
            e.level = level;
            let out = e.compress_text(s.system);
            if out.len() > s.system.len() {
                eprintln!(
                    "VIOLATION: {} @ L{} grew {} → {}",
                    s.name,
                    level,
                    s.system.len(),
                    out.len()
                );
                violations += 1;
            }
        }
    }
    println!();
    println!(
        "v3.2.0 length contract: {}/50 scenario×level combos respected (no inflation)",
        50 - violations
    );
    assert_eq!(violations, 0, "compression must never inflate output");
}

// ───────────────────────────────────────────────────────────────────────────
// Test 6 — tool_use / image / numeric prose / four-digit dates survive
// ───────────────────────────────────────────────────────────────────────────

#[test]
fn t06_safety_invariants_on_realistic_payloads() {
    let _guard = TEST_MUTEX.lock().unwrap();
    // Build a request that contains: tool_use + image + date prose + Python tuple
    let req = json!({
        "model": "claude-sonnet-4-5",
        "max_tokens": 4096,
        "system": "You are a debugger. On January 14, 1999 the system shipped; on January 14, 2099 it might be retired.",
        "messages": [{
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_xyz",
                    "name": "search",
                    "input": {"query": "  please   note   the  whitespace  ", "limit": 42}
                },
                {
                    "type": "image",
                    "source": {"type": "base64", "media_type": "image/png", "data": "iVBORw0KGgoAAAANSUhEUg=="}
                },
                {
                    "type": "text",
                    "text": "Use ('foo',) for single-element tuples. Result: 42; this is prose, not code."
                }
            ]
        }]
    });

    let (out, _, _, _) = compress_request(&req, 1.0, true, false, false, 0, "claude-sonnet-4-5");

    // tool_use byte-identical
    assert_eq!(
        req["messages"][0]["content"][0], out["messages"][0]["content"][0],
        "tool_use mutated"
    );
    // image byte-identical
    assert_eq!(
        req["messages"][0]["content"][1], out["messages"][0]["content"][1],
        "image mutated"
    );
    // text content survives the safety invariants
    let text_after = out["messages"][0]["content"][2]["text"].as_str().unwrap();
    assert!(
        text_after.contains("('foo',)"),
        "Python tuple lost: {text_after}"
    );
    assert!(
        text_after.contains("Result: 42"),
        "numeric prose lost: {text_after}"
    );
    // System prompt: dates compressed but four-digit years preserved
    let sys_after = out["system"].as_str().unwrap();
    assert!(
        sys_after.contains("1999-01-14"),
        "four-digit year 1999 lost: {sys_after}"
    );
    assert!(
        sys_after.contains("2099-01-14"),
        "four-digit year 2099 lost: {sys_after}"
    );

    println!();
    println!("v3.2.0 safety invariants: tool_use ✓ image ✓ Python tuple ✓ numeric prose ✓ four-digit dates ✓");
}

// ───────────────────────────────────────────────────────────────────────────
// helpers
// ───────────────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}
