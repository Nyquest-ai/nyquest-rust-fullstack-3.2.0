//! Compression engine regression tests — safety invariants.
//!
//! Snapshot-style tests that lock in safety guarantees:
//!   - Python tuple syntax is preserved (`('foo',)` must survive)
//!   - Four-digit years are preserved (no century ambiguity)
//!   - Role declarations are never deleted
//!   - tool_use / image blocks pass through byte-identical
//!   - SOURCE_CODE_COMPRESSION rules do not mutate prose
//!   - Profile detection picks the right profile when both family-name and
//!     small-marker substrings are present (e.g. `claude-sonnet-4-6-mini`).
//!
//! Adding a guarantee → add a test in this file.

use nyquest::compression::compress_request;
use nyquest::compression::engine::CompressionEngine;
use nyquest::profiles::detect_profile;
use serde_json::json;

/// Test-only helper: compress a single text fragment using the same profile
/// resolution as the live request path.
fn compress_text_for_test(input: &str, level: f64, model: &str) -> String {
    let profile = detect_profile(model);
    let mut engine = CompressionEngine::with_profile(level, profile);
    engine.compress_text(input)
}

// ──────────────────────────────────────────────────────────
// SOURCE_CODE_COMPRESSION must not mutate prose
// ──────────────────────────────────────────────────────────

#[test]
fn preserves_python_single_element_tuple() {
    let input = "Use ('foo',) for single-element tuples in Python.";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert!(
        output.contains("('foo',)"),
        "Python tuple syntax must survive compression. Got: {output:?}"
    );
}

#[test]
fn source_code_rules_do_not_fire_on_prose() {
    let input = "Result: 42; this is prose, not code. Use ('foo',) in Python.";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert!(
        output.contains("Result: 42"),
        "Numeric prose `Result: 42` must not be collapsed. Got: {output:?}"
    );
    assert!(
        output.contains("; this") || !output.contains(";this"),
        "Semicolon prose spacing must not be collapsed. Got: {output:?}"
    );
    assert!(
        output.contains("('foo',)"),
        "Python tuple intact. Got: {output:?}"
    );
}

#[test]
fn markdown_heading_is_not_stripped_by_source_rules() {
    // SOURCE_CODE_COMPRESSION's `^\s*#\s+...$` rule would erase markdown
    // headings if it ever fired on prose. Confirm it does not.
    let input = "# Important Heading\n\nSome explanatory paragraph follows here.";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert!(
        output.contains("Important Heading"),
        "Markdown headings must survive in prose. Got: {output:?}"
    );
}

#[test]
fn fenced_json_can_still_be_minified() {
    // Confirmed JSON content may still be compressed (the gate above is for prose).
    let input = r#"{"a": 1, "b": 2, "c": 3, "d": 4, "e": "hello there"}"#;
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    // We don't assert exact form; just that the engine handles it safely.
    assert!(!output.is_empty(), "JSON output should not be empty");
}

// ──────────────────────────────────────────────────────────
// Date compression preserves four-digit years
// ──────────────────────────────────────────────────────────

#[test]
fn preserves_four_digit_years() {
    let input = "January 14, 1999 and January 14, 2099 are different dates.";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert!(
        output.contains("1999-01-14"),
        "1999 must compress with full year. Got: {output:?}"
    );
    assert!(
        output.contains("2099-01-14"),
        "2099 must compress with full year. Got: {output:?}"
    );
    assert!(
        !output.contains("99-01-14") || output.contains("1999-01-14"),
        "Must not produce ambiguous two-digit year. Got: {output:?}"
    );
}

#[test]
fn date_compression_handles_century_boundaries() {
    for (input_year, expected) in [
        ("1999", "1999-01-14"),
        ("2000", "2000-01-14"),
        ("2026", "2026-01-14"),
        ("2099", "2099-01-14"),
        ("2100", "2100-01-14"),
    ] {
        let input = format!("Meeting on January 14, {input_year} at 10am.");
        let output = compress_text_for_test(&input, 1.0, "claude-sonnet-4-5");
        assert!(
            output.contains(expected),
            "Year {input_year} must produce {expected}. Got: {output:?}"
        );
    }
}

// ──────────────────────────────────────────────────────────
// Role declarations preserved by telegraph
// ──────────────────────────────────────────────────────────

#[test]
fn preserves_role_declaration() {
    let input = "You are a senior network engineer.";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert!(
        output.contains("senior network engineer"),
        "Role information must not be deleted. Got: {output:?}"
    );
    assert!(
        !output.trim().is_empty(),
        "Role-only prompt must never be emptied. Got: {output:?}"
    );
}

#[test]
fn long_role_only_prompt_not_emptied() {
    // Telegraph compression at high level used to wipe single-sentence prompts.
    let input = "You are a highly experienced senior backend engineer with deep \
                 knowledge of distributed systems, databases, and Rust.";
    let output = compress_text_for_test(input, 1.0, "claude-opus-4-5");
    assert!(
        !output.trim().is_empty(),
        "Role-only prompt must never be emptied. Got: {output:?}"
    );
    assert!(
        output.to_lowercase().contains("backend") || output.to_lowercase().contains("engineer"),
        "Some role identifier must survive. Got: {output:?}"
    );
}

// ──────────────────────────────────────────────────────────
// Profile detection ordering
// ──────────────────────────────────────────────────────────

#[test]
fn profile_detection_specific_models_win_first() {
    assert_eq!(detect_profile("gpt-4o-mini").name, "conservative");
    assert_eq!(detect_profile("claude-haiku-4-5").name, "conservative");
    assert_eq!(detect_profile("claude-sonnet-4-5").name, "aggressive");
    assert_eq!(detect_profile("claude-opus-4-7").name, "aggressive");
}

#[test]
fn mini_suffix_overrides_family_name() {
    // Future-proof: a hypothetical sonnet mini variant must still be conservative.
    assert_eq!(
        detect_profile("claude-sonnet-4-6-mini").name,
        "conservative"
    );
    assert_eq!(detect_profile("grok-3-mini").name, "conservative");
    assert_eq!(detect_profile("gemini-2.5-flash").name, "conservative");
}

// ──────────────────────────────────────────────────────────
// Non-negotiable safety: tool_use & image pass-through
// ──────────────────────────────────────────────────────────

#[test]
fn tool_use_blocks_pass_through_unchanged() {
    let request = json!({
        "model": "claude-sonnet-4-5",
        "messages": [{
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_abc123",
                "name": "search_database",
                "input": {
                    "query": "  please   note   that  the  whitespace  must   survive  ",
                    "limit": 42,
                    "filters": {"date": "January 14, 2025"}
                }
            }]
        }]
    });

    let (out, _stats, _saved, _compressed) =
        compress_request(&request, 1.0, false, false, false, 0, "claude-sonnet-4-5");

    let original_tool_use = &request["messages"][0]["content"][0];
    let compressed_tool_use = &out["messages"][0]["content"][0];
    assert_eq!(
        original_tool_use, compressed_tool_use,
        "tool_use block must pass through unchanged. Diff:\n  before: {original_tool_use}\n  after: {compressed_tool_use}"
    );
}

#[test]
fn image_blocks_pass_through_unchanged() {
    let request = json!({
        "model": "claude-sonnet-4-5",
        "messages": [{
            "role": "user",
            "content": [{
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/png",
                    "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="
                }
            }, {
                "type": "text",
                "text": "Please note that this is a test image, please describe it."
            }]
        }]
    });

    let (out, _stats, _saved, _compressed) =
        compress_request(&request, 1.0, false, false, false, 0, "claude-sonnet-4-5");

    let original_image = &request["messages"][0]["content"][0];
    let compressed_image = &out["messages"][0]["content"][0];
    assert_eq!(
        original_image, compressed_image,
        "image block must pass through unchanged byte-for-byte"
    );
}

// ──────────────────────────────────────────────────────────
// Cache regression — same content compresses identically
// ──────────────────────────────────────────────────────────

#[test]
fn repeated_compression_is_deterministic() {
    // The compression cache must return the same string on a repeat.
    let input = "Please note that you should always carefully review the code \
                 for potential bugs and security vulnerabilities and other issues.";
    let first = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    let second = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    assert_eq!(
        first, second,
        "compress_text must be deterministic for identical inputs"
    );
}

// ──────────────────────────────────────────────────────────
// Don't make output longer than input (fail-closed contract)
// ──────────────────────────────────────────────────────────

#[test]
fn compression_never_inflates_output() {
    let inputs = [
        "Hello world",
        "January 14, 2025",
        "You are a helpful assistant.",
        "{\"a\": 1}",
        "Short.",
        "no rules match this nondescript sentence",
    ];
    for input in inputs {
        let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
        assert!(
            output.len() <= input.len(),
            "Compression inflated input ({} -> {}): {input:?} -> {output:?}",
            input.len(),
            output.len()
        );
    }
}

// ──────────────────────────────────────────────────────────
// Non-English text (should pass through largely intact)
// ──────────────────────────────────────────────────────────

#[test]
fn non_english_text_is_not_corrupted() {
    let input = "你好世界。Привет мир. مرحبا بالعالم. こんにちは世界。";
    let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
    // Each non-Latin sentence-fragment must survive (rules are word-boundary
    // anchored against ASCII so they should not match these scripts).
    for needle in ["你好", "Привет", "مرحبا", "こんにちは"] {
        assert!(
            output.contains(needle),
            "Non-English fragment {needle:?} must survive. Got: {output:?}"
        );
    }
}

// ──────────────────────────────────────────────────────────
// Short prompts: do not over-compress
// ──────────────────────────────────────────────────────────

#[test]
fn short_prompt_is_not_emptied() {
    for input in ["Hello.", "OK.", "Tell me about Rust."] {
        let output = compress_text_for_test(input, 1.0, "claude-sonnet-4-5");
        assert!(
            !output.trim().is_empty(),
            "Short prompt {input:?} must never become empty. Got: {output:?}"
        );
    }
}
