//! Audit: print every category's rule count and the grand total, and lock
//! the totals so they can't silently drift away from the numbers quoted in
//! `src/compression/rules.rs`, `docs/ARCHITECTURE.md`, `README.md`, and
//! `CHANGELOG.md`.
//!
//! Run with:
//!   cargo test --test rule_count_audit -- --nocapture
//!
//! When intentionally adding/removing rules, update both the per-category
//! `EXPECTED` table below AND every doc reference flagged by a search for
//! the old total.

use nyquest::compression::rules::*;

#[test]
fn print_rule_counts() {
    let cats: &[(&str, &RuleCategory)] = &[
        ("OPENCLAW_RULES", &OPENCLAW_RULES),
        ("FILLER_PHRASES", &FILLER_PHRASES),
        ("VERBOSE_PHRASES", &VERBOSE_PHRASES),
        ("IMPERATIVE_CONVERSIONS", &IMPERATIVE_CONVERSIONS),
        ("CLAUSE_COLLAPSE", &CLAUSE_COLLAPSE),
        ("DEVELOPER_BOILERPLATE", &DEVELOPER_BOILERPLATE),
        ("CONVERSATIONAL_STRIP", &CONVERSATIONAL_STRIP),
        ("AI_OUTPUT_NOISE", &AI_OUTPUT_NOISE),
        ("MARKDOWN_MINIFICATION", &MARKDOWN_MINIFICATION),
        ("SOURCE_CODE_COMPRESSION", &SOURCE_CODE_COMPRESSION),
        ("CONTEXT_DEDUPLICATION", &CONTEXT_DEDUPLICATION),
        ("SEMANTIC_FORMATTING", &SEMANTIC_FORMATTING),
        ("ANTI_NOISE", &ANTI_NOISE),
        ("CREDENTIAL_STRIP", &CREDENTIAL_STRIP),
        ("DISCLAIMER_COLLAPSE", &DISCLAIMER_COLLAPSE),
        ("ADJECTIVE_COLLAPSE", &ADJECTIVE_COLLAPSE),
        ("CLAUSE_SIMPLIFY", &CLAUSE_SIMPLIFY),
        ("ADVERB_STRIP", &ADVERB_STRIP),
        ("WHITESPACE_CLEANUP", &WHITESPACE_CLEANUP),
    ];

    // Per-category expected counts — update intentionally when rules change.
    let expected: &[(&str, usize)] = &[
        ("OPENCLAW_RULES", 16),
        ("FILLER_PHRASES", 61),
        ("VERBOSE_PHRASES", 131),
        ("IMPERATIVE_CONVERSIONS", 40),
        ("CLAUSE_COLLAPSE", 26),
        ("DEVELOPER_BOILERPLATE", 31),
        ("CONVERSATIONAL_STRIP", 46),
        ("AI_OUTPUT_NOISE", 24),
        ("MARKDOWN_MINIFICATION", 18),
        ("SOURCE_CODE_COMPRESSION", 11),
        ("CONTEXT_DEDUPLICATION", 5),  // was 7; removed 2 backref-based sentence-dedup rules that triggered fancy_regex BacktrackLimitExceeded
        ("SEMANTIC_FORMATTING", 33),
        ("ANTI_NOISE", 20),
        ("CREDENTIAL_STRIP", 8),
        ("DISCLAIMER_COLLAPSE", 6),
        ("ADJECTIVE_COLLAPSE", 17),
        ("CLAUSE_SIMPLIFY", 21),
        ("ADVERB_STRIP", 10),
        ("WHITESPACE_CLEANUP", 6),
    ];
    const EXPECTED_TOTAL: usize = 530;  // was 532; -2 from CONTEXT_DEDUPLICATION backref rule removal
    const EXPECTED_CATEGORIES: usize = 19;

    println!("\n=== Nyquest compression rule audit ===");
    let mut total = 0usize;
    for (i, (name, cat)) in cats.iter().enumerate() {
        let n = cat.rules.len();
        total += n;
        println!("{:<26} {:>4} rules", name, n);
        let (exp_name, exp_n) = expected[i];
        assert_eq!(*name, exp_name, "category order drifted at index {}", i);
        assert_eq!(
            n, exp_n,
            "{} count changed: docs say {} rules, code has {} — update docs and EXPECTED table together",
            name, exp_n, n,
        );
    }
    println!("---");
    println!("Categories: {}", cats.len());
    println!("Total rules: {}", total);
    println!("=======================================\n");

    assert_eq!(
        cats.len(),
        EXPECTED_CATEGORIES,
        "category count changed — update docs that quote \"19 categories\"",
    );
    assert_eq!(
        total, EXPECTED_TOTAL,
        "total rule count changed: docs say {}, code has {} — update README, CHANGELOG, ARCHITECTURE, and rules.rs docstring together",
        EXPECTED_TOTAL, total,
    );
}
