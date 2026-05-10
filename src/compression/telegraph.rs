//! Nyquest Sentence-Level Telegraph Compressor
//!
//! Operates AFTER regex rules. Splits text into sentences and applies
//! structural transforms:
//!   1. Strip sentence-initial hedge/preamble patterns
//!   2. Merge consecutive short imperative sentences into lists
//!   3. Deduplicate semantically redundant sentences
//!   4. Convert verbose instructional prose to telegraph style

use fancy_regex::Regex as FancyRegex;
use once_cell::sync::Lazy;
use regex::Regex;

// ── Sentence splitter ──

static SENTENCE_SPLIT: Lazy<FancyRegex> = Lazy::new(|| {
    // Split on sentence-ending punctuation followed by whitespace + capital
    // But don't split on abbreviations (e.g., Mr., Dr., i.e., e.g., U.S.)
    FancyRegex::new(r"(?<=[.!?])\s+(?=[A-Z])").unwrap()
});

// ── Sentence-initial preamble patterns (ordered by specificity) ──

static PREAMBLE_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    let patterns: Vec<(&str, &str)> = vec![
        // Role/identity declarations → compact "Role: X." form (never empty).
        // Capture the role description so we don't lose persona/identity information.
        (r"^(?i)you\s+are\s+(?:a|an)\s+(.+?)\.\s*$", "Role: $1."),
        (r"^(?i)you\s+are\s+(?:a|an)\s+(.+)$", "Role: $1"),
        // Responsibility declarations → strip entirely
        (
            r"^(?i)your\s+(?:primary\s+)?(?:role|responsibility|goal|job|task|objective)\s+is\s+to\s+",
            "",
        ),
        // "When X-ing, [you should]" → strip preamble
        (
            r"^(?i)when\s+(?:you\s+are\s+)?(?:\w+ing)\s+\w+(?:\s+\w+)?,?\s*(?:you\s+should\s+)?",
            "",
        ),
        // "Please [make sure to / ensure that / note that / remember to]"
        (
            r"^(?i)please\s+(?:make\s+sure\s+(?:to\s+)?|ensure\s+(?:that\s+)?|note\s+that\s+|remember\s+(?:to\s+)?|always\s+)?",
            "",
        ),
        // "It is important/essential/critical to/that"
        (
            r"^(?i)it\s+is\s+(?:important|essential|critical|crucial|vital|necessary)\s+(?:to\s+|that\s+(?:you\s+)?)",
            "",
        ),
        // "You should [always/never]"
        (r"^(?i)you\s+should\s+(?:also\s+)?", ""),
        (r"^(?i)you\s+must\s+(?:also\s+)?", ""),
        (r"^(?i)you\s+need\s+to\s+", ""),
        (r"^(?i)you\s+will\s+(?:need\s+to\s+|want\s+to\s+)?", ""),
        // "Make sure to / Be sure to / Remember to"
        (
            r"^(?i)(?:make\s+sure|be\s+sure|ensure)\s+(?:to\s+|that\s+(?:you\s+)?)",
            "",
        ),
        (r"^(?i)remember\s+(?:to\s+)?(?:always\s+)?", ""),
        (r"^(?i)don'?t\s+forget\s+to\s+", ""),
        // "Always [make sure to / remember to / ensure]"
        (
            r"^(?i)always\s+(?:make\s+sure\s+(?:to\s+)?|ensure\s+(?:that\s+)?|remember\s+(?:to\s+)?)",
            "Always ",
        ),
        // "Keep in mind that / Be aware that / Note that"
        (
            r"^(?i)(?:keep\s+in\s+mind|be\s+(?:aware|mindful)|note|bear\s+in\s+mind)\s+that\s+",
            "",
        ),
        // "Consider the fact that / Given that"
        (
            r"^(?i)(?:consider|given|recognizing|acknowledging)\s+(?:the\s+fact\s+)?that\s+",
            "",
        ),
        // "Additionally / Furthermore / Moreover"
        (
            r"^(?i)(?:additionally|furthermore|moreover|also|in\s+addition)\s*,?\s*",
            "",
        ),
    ];

    patterns
        .iter()
        .map(|(p, r)| (Regex::new(p).unwrap(), *r))
        .collect()
});

// ── Sentence-final trim patterns ──

static TAIL_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    let patterns: Vec<(&str, &str)> = vec![
        // "... in your responses/analysis/reviews/recommendations."
        (
            r"(?i)\s+in\s+(?:your|the)\s+(?:responses?|analysis|reviews?|recommendations?|explanations?|feedback|suggestions?|assessments?)\s*[.]$",
            ".",
        ),
        // "... at all times."
        (r"(?i)\s+at\s+all\s+times\s*[.]$", "."),
        // "... when possible/appropriate/applicable."
        (
            r"(?i)\s+(?:when|where|if|whenever)\s+(?:possible|appropriate|applicable|relevant|necessary|needed)\s*[.]$",
            ".",
        ),
        // "... for the user/customer/client."
        (
            r"(?i)\s+for\s+the\s+(?:user|customer|client|reader|student|person)\s*[.]$",
            ".",
        ),
        // "... to the user/customer."
        (r"(?i)\s+to\s+the\s+(?:user|customer|client)\s*[.]$", "."),
    ];
    patterns
        .iter()
        .map(|(p, r)| (Regex::new(p).unwrap(), *r))
        .collect()
});

// ── Imperative sentence consolidation ──

static IMPERATIVE_START: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?i)(?:Always|Never|Do(?:n'?t)?|Avoid|Include|Ensure|Provide|Consider|Use|Follow|Check|Verify|Keep|Maintain|Apply|Review) ").unwrap()
});

// ── Dedup: near-identical sentence detector ──

fn sentence_fingerprint(s: &str) -> String {
    // Lowercase, strip punctuation, collapse whitespace → fingerprint
    let lower = s.to_lowercase();
    let stripped: String = lower
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect();
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn similarity(a: &str, b: &str) -> f64 {
    let fa = sentence_fingerprint(a);
    let fb = sentence_fingerprint(b);

    let wa: std::collections::HashSet<&str> = fa.split_whitespace().collect();
    let wb: std::collections::HashSet<&str> = fb.split_whitespace().collect();

    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }

    let intersection = wa.intersection(&wb).count() as f64;
    let union = wa.union(&wb).count() as f64;
    intersection / union // Jaccard similarity
}

/// Main entry point: telegraph-compress a block of text.
/// Call AFTER regex rules have already been applied.
pub fn telegraph_compress(text: &str, level: f64) -> String {
    if level < 0.5 || text.len() < 100 {
        return text.to_string();
    }

    // Check if text has sentence structure (prose). Skip structured content like
    // XML, code, JSON, markdown with headers, etc.
    if text.starts_with('<')
        || text.starts_with('{')
        || text.starts_with('[')
        || text.contains("```")
        || text.contains("def ")
        || text.contains("fn ")
        || text.contains("class ")
        || text.contains("import ")
    {
        return text.to_string();
    }

    // Short role-only prompts (e.g. "You are a senior network engineer.") are
    // single-sentence; rewrite to compact role form rather than running the
    // multi-sentence pipeline on them — stripping their only sentence would
    // erase the role declaration entirely.
    let sentences = split_sentences(text);
    if sentences.len() < 3 {
        let single = strip_preamble(text.trim());
        if single.trim().is_empty() {
            return text.to_string();
        }
        return single;
    }

    let mut result: Vec<String> = Vec::new();

    // Phase 1: Strip preambles from each sentence
    for sentence in &sentences {
        let trimmed = strip_preamble(sentence);
        result.push(trimmed);
    }

    // Phase 2 (level 0.8+): Trim sentence tails
    if level >= 0.8 {
        result = result.iter().map(|s| strip_tail(s)).collect();
    }

    // Phase 3 (level 0.8+): Deduplicate near-identical sentences
    if level >= 0.8 {
        result = dedup_sentences(result);
    }

    // Phase 4 (level 0.8+): Merge consecutive short imperatives into semicolon lists
    if level >= 0.8 {
        result = merge_imperatives(result);
    }

    // Reassemble
    let mut output = result.join(" ");

    // Clean up artifacts
    output = output.replace("  ", " ");
    output = output.replace(" .", ".");
    output = output.replace(" ,", ",");
    output = output.replace("..", ".");

    // Capitalize first character
    if let Some(first) = output.chars().next() {
        if first.is_lowercase() {
            output = first.to_uppercase().to_string() + &output[first.len_utf8()..];
        }
    }

    // Fail-closed guard: if the transform stripped everything, return original.
    // This protects role-only or otherwise terse prompts from being emptied.
    if output.trim().is_empty() {
        return text.to_string();
    }

    output
}

fn split_sentences(text: &str) -> Vec<String> {
    // fancy_regex doesn't have split(), so use find_iter to get split points
    let mut sentences = Vec::new();
    let mut last_end = 0;

    for m in SENTENCE_SPLIT.find_iter(text) {
        if let Ok(mat) = m {
            let before = text[last_end..mat.start()].trim();
            if !before.is_empty() {
                sentences.push(before.to_string());
            }
            last_end = mat.end();
        }
    }
    // Don't forget the last segment
    let remainder = text[last_end..].trim();
    if !remainder.is_empty() {
        sentences.push(remainder.to_string());
    }
    sentences
}

fn strip_preamble(sentence: &str) -> String {
    let mut s = sentence.to_string();

    // Try each pattern — apply only the first match
    for (re, replacement) in PREAMBLE_PATTERNS.iter() {
        if re.is_match(&s) {
            s = re.replace(&s, *replacement).to_string();
            // Capitalize first letter after stripping
            s = s.trim_start().to_string();
            if let Some(first) = s.chars().next() {
                if first.is_lowercase() {
                    s = first.to_uppercase().to_string() + &s[first.len_utf8()..];
                }
            }
            break; // Only strip one preamble per sentence
        }
    }

    s
}

fn strip_tail(sentence: &str) -> String {
    let mut s = sentence.to_string();
    for (re, replacement) in TAIL_PATTERNS.iter() {
        s = re.replace(&s, *replacement).to_string();
    }
    s
}

fn dedup_sentences(sentences: Vec<String>) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();

    for s in &sentences {
        let dominated = result.iter().any(|existing| similarity(existing, s) > 0.70);
        if !dominated {
            result.push(s.clone());
        }
    }

    result
}

fn merge_imperatives(sentences: Vec<String>) -> Vec<String> {
    if sentences.len() < 3 {
        return sentences;
    }

    let mut result: Vec<String> = Vec::new();
    let mut i = 0;

    while i < sentences.len() {
        // Look for runs of short imperative sentences
        if IMPERATIVE_START.is_match(&sentences[i]) && sentences[i].len() < 80 {
            let mut run = vec![sentences[i].clone()];
            let mut j = i + 1;
            while j < sentences.len()
                && IMPERATIVE_START.is_match(&sentences[j])
                && sentences[j].len() < 80
            {
                // Strip period from previous, add semicolon
                run.push(sentences[j].clone());
                j += 1;
            }

            if run.len() >= 3 {
                // Merge: strip periods from all but last, join with "; "
                let merged: Vec<String> = run
                    .iter()
                    .enumerate()
                    .map(|(k, s)| {
                        if k < run.len() - 1 {
                            s.trim_end_matches('.').to_string()
                        } else {
                            s.clone()
                        }
                    })
                    .collect();
                result.push(merged.join("; "));
                i = j;
                continue;
            }
        }

        result.push(sentences[i].clone());
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_telegraph() {
        let input = "You are a helpful customer support agent. Your primary responsibility is to assist customers with their issues. Please make sure to always be polite. You should provide step-by-step instructions.";
        let output = telegraph_compress(input, 0.7);
        assert!(
            output.len() < input.len(),
            "Should compress: {} vs {}",
            output.len(),
            input.len()
        );
        // Should not start with "You are"
        assert!(
            !output.starts_with("You are"),
            "Should strip role declaration"
        );
    }

    #[test]
    fn test_skips_code() {
        let input = "def hello():\n    print('world')\n\nclass Foo:\n    pass";
        let output = telegraph_compress(input, 1.0);
        assert_eq!(input, output, "Should not modify code");
    }

    #[test]
    fn test_skips_short_text() {
        let input = "Hello world.";
        let output = telegraph_compress(input, 1.0);
        assert_eq!(input, output, "Should not modify short text");
    }

    #[test]
    fn test_merge_imperatives() {
        let sentences = vec![
            "Always verify credentials.".to_string(),
            "Never share passwords.".to_string(),
            "Follow security protocols.".to_string(),
            "Check access logs daily.".to_string(),
        ];
        let merged = merge_imperatives(sentences);
        assert!(merged.len() < 4, "Should merge consecutive imperatives");
    }
}
