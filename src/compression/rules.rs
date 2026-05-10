//! Nyquest Compression Rules
//! Pattern-based text compression at various levels.
//!
//! 532 compression rules across 19 categories. Each rule carries an atomic
//! hit counter and a stable `category.NNN` id for telemetry. Each category
//! is wrapped in a `RuleCategory` with a `regex::RegexSet` prefilter so
//! non-matching rules are skipped without a full text scan. Rule
//! application is `Cow`-based: misses are zero-allocation.
//!
//! Rule count is locked by `tests/rule_count_audit.rs` — run
//! `cargo test --test rule_count_audit -- --nocapture` to print the
//! per-category breakdown.

use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};
use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A single replacement rule: compiled regex + replacement string.
/// Uses fancy_regex for patterns with lookaround/backreferences,
/// standard regex crate for everything else (faster).
///
/// Carries an atomic per-rule hit counter for telemetry: identifies dead
/// rules (zero hits) and over-firing rules (disproportionate hits).
pub struct Rule {
    pub id: String,
    pub pattern_str: String,
    pub std_pattern: Option<Regex>,
    pub fancy_pattern: Option<fancy_regex::Regex>,
    pub replacement: String,
    pub hits: AtomicUsize,
}

impl Rule {
    fn new(pattern: &str, replacement: &str) -> Self {
        match Regex::new(pattern) {
            Ok(re) => Self {
                id: String::new(),
                pattern_str: pattern.to_string(),
                std_pattern: Some(re),
                fancy_pattern: None,
                replacement: replacement.to_string(),
                hits: AtomicUsize::new(0),
            },
            Err(_) => Self {
                id: String::new(),
                pattern_str: pattern.to_string(),
                std_pattern: None,
                fancy_pattern: Some(fancy_regex::Regex::new(pattern).unwrap_or_else(|e| {
                    panic!("Invalid rule regex '{}': {}", pattern, e);
                })),
                replacement: replacement.to_string(),
                hits: AtomicUsize::new(0),
            },
        }
    }

    fn new_case_insensitive(pattern: &str, replacement: &str) -> Self {
        let ci_pattern = format!("(?i){}", pattern);
        match Regex::new(&ci_pattern) {
            Ok(re) => Self {
                id: String::new(),
                pattern_str: ci_pattern,
                std_pattern: Some(re),
                fancy_pattern: None,
                replacement: replacement.to_string(),
                hits: AtomicUsize::new(0),
            },
            Err(_) => Self {
                id: String::new(),
                pattern_str: ci_pattern.clone(),
                std_pattern: None,
                fancy_pattern: Some(fancy_regex::Regex::new(&ci_pattern).unwrap_or_else(|e| {
                    panic!("Invalid rule regex '{}': {}", ci_pattern, e);
                })),
                replacement: replacement.to_string(),
                hits: AtomicUsize::new(0),
            },
        }
    }

    /// Apply this rule to text (eager, returns owned String).
    /// Prefer `replace_all_counted` on the hot path.
    pub fn replace_all(&self, text: &str) -> String {
        let (cow, _) = self.replace_all_counted(text);
        cow.into_owned()
    }

    /// Apply this rule and return (Cow result, did_match).
    ///
    /// On miss returns `Cow::Borrowed(text)` — zero allocation. On match
    /// returns `Cow::Owned` and increments the per-rule hit counter.
    pub fn replace_all_counted<'a>(&self, text: &'a str) -> (Cow<'a, str>, bool) {
        if let Some(re) = &self.std_pattern {
            let result = re.replace_all(text, self.replacement.as_str());
            let did = matches!(result, Cow::Owned(_));
            if did {
                self.hits.fetch_add(1, Ordering::Relaxed);
            }
            (result, did)
        } else if let Some(re) = &self.fancy_pattern {
            let result = re.replace_all(text, self.replacement.as_str());
            let did = matches!(result, Cow::Owned(_));
            if did {
                self.hits.fetch_add(1, Ordering::Relaxed);
            }
            (result, did)
        } else {
            (Cow::Borrowed(text), false)
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }
}

/// A rule category: name + ordered rules + RegexSet prefilter.
///
/// The RegexSet covers all standard-regex rules in the category. Before
/// running the rules, we ask the set "which rules can possibly match this
/// text?" and skip the rest. Fancy-regex rules cannot live in a RegexSet,
/// so they always run.
pub struct RuleCategory {
    pub name: &'static str,
    pub rules: Vec<Rule>,
    /// RegexSet over the std-pattern rules only.
    pub set: RegexSet,
    /// For each entry in `set`, the index into `rules` it corresponds to.
    pub set_to_rule: Vec<usize>,
    pub category_hits: AtomicUsize,
}

impl RuleCategory {
    pub fn new(name: &'static str, rules: Vec<Rule>) -> Self {
        let mut patterns: Vec<String> = Vec::new();
        let mut set_to_rule: Vec<usize> = Vec::new();
        for (i, r) in rules.iter().enumerate() {
            if let Some(re) = &r.std_pattern {
                patterns.push(re.as_str().to_string());
                set_to_rule.push(i);
            }
        }
        let set = RegexSet::new(&patterns)
            .unwrap_or_else(|e| panic!("RegexSet build failed for {}: {}", name, e));

        // Assign stable per-rule IDs (`category.NNN`) for telemetry.
        let mut rules = rules;
        for (i, r) in rules.iter_mut().enumerate() {
            r.id = format!("{}.{:03}", name, i);
        }

        Self {
            name,
            rules,
            set,
            set_to_rule,
            category_hits: AtomicUsize::new(0),
        }
    }

    pub fn category_hits(&self) -> usize {
        self.category_hits.load(Ordering::Relaxed)
    }
}

/// Build rules with case-insensitive flag
fn ci_rules(pairs: &[(&str, &str)]) -> Vec<Rule> {
    pairs
        .iter()
        .map(|(p, r)| Rule::new_case_insensitive(p, r))
        .collect()
}

/// Build rules with exact case
fn exact_rules(pairs: &[(&str, &str)]) -> Vec<Rule> {
    pairs.iter().map(|(p, r)| Rule::new(p, r)).collect()
}

/// Build a category from case-insensitive rule pairs (most common case).
/// Lets each `Lazy<RuleCategory>` static stay a near-one-line wrapper.
fn category_from_ci(name: &'static str, pairs: &[(&str, &str)]) -> RuleCategory {
    RuleCategory::new(name, ci_rules(pairs))
}

/// Build a category from exact-case rule pairs.
fn category_from_exact(name: &'static str, pairs: &[(&str, &str)]) -> RuleCategory {
    RuleCategory::new(name, exact_rules(pairs))
}

// ══════════════════════════════════════════════
// OpenClaw-specific rules
// ══════════════════════════════════════════════

#[allow(clippy::vec_init_then_push)]
pub static OPENCLAW_RULES: Lazy<RuleCategory> = Lazy::new(|| {
    RuleCategory::new("openclaw_rules", {
        let mut rules = Vec::new();

        // Untrusted metadata blocks (multiline)
        rules.push(Rule::new(r"(?s)(?:user:\s*)?Conversation info \(untrusted metadata\):\s*```json\s*\{.*?\}\s*```\s*\n?", ""));
        rules.push(Rule::new(
            r#"(?s)```json\s*\{\s*"message_id".*?\}\s*```\s*\n?"#,
            "",
        ));

        // Reply directive tags
        rules.push(Rule::new(r"\[\[reply_to_current\]\]\s*", ""));
        rules.push(Rule::new(r"\[\[reply_to_[^\]]+\]\]\s*", ""));
        rules.push(Rule::new(r"\[\[[^\]]+\]\]\s*", ""));

        // Session header block
        rules.push(Rule::new(r"(?s)# Session: \d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2} UTC\n.*?## Conversation Summary\n+", ""));
        rules.push(Rule::new(r"(?m)^- \*\*Session Key\*\*:.*\n", ""));
        rules.push(Rule::new(r"(?m)^- \*\*Session ID\*\*:.*\n", ""));
        rules.push(Rule::new(r"(?m)^- \*\*Source\*\*:.*\n", ""));
        rules.push(Rule::new(r"(?m)^## Conversation Summary\s*\n", ""));

        // Timestamp prefix
        rules.push(Rule::new(
            r"\[\w{2,4} \d{4}-\d{2}-\d{2} \d{2}:\d{2} UTC\]\s*",
            "",
        ));

        // New session boilerplate
        rules.push(Rule::new(r"(?s)(?:user: )?A new session was started via /new or /reset\..*?what they want to do\.\s*\n?", ""));

        // Deduplicate large blocks
        rules.push(Rule::new(r"(?s)(U: .{200,}?)\n+\1", "$1"));

        // Collapse role prefixes
        rules.push(Rule::new(r"(?m)^assistant:\s+", "A: "));
        rules.push(Rule::new(r"(?m)^user:\s+", "U: "));

        // Orphaned empty lines
        rules.push(Rule::new(r"\n{3,}", "\n\n"));

        rules
    })
});

// ══════════════════════════════════════════════
// Level 0.2-0.4: Filler removal & normalization
// ══════════════════════════════════════════════

pub static FILLER_PHRASES: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "filler_phrases",
        &[
            // Politeness fillers
            (r"\bplease\s+note\s+that\b", ""),
            (r"\bit\s+is\s+important\s+to\s+note\s+that\b", ""),
            (r"\bit\s+should\s+be\s+noted\s+that\b", ""),
            (r"\bplease\s+make\s+sure\s+to\b", ""),
            (r"\bplease\s+ensure\s+that\b", "ensure"),
            (r"\bplease\s+remember\s+to\b", ""),
            (r"\bkindly\b", ""),
            (r"\bplease\b(?!\s+\w+\s+the)", ""),
            // Verbose connectors
            (r"\bin\s+order\s+to\b", "to"),
            (r"\bfor\s+the\s+purpose\s+of\b", "for"),
            (r"\bwith\s+the\s+goal\s+of\b", "to"),
            (r"\bdue\s+to\s+the\s+fact\s+that\b", "because"),
            (r"\bin\s+the\s+event\s+that\b", "if"),
            (r"\bat\s+this\s+point\s+in\s+time\b", "now"),
            (r"\bat\s+the\s+present\s+time\b", "now"),
            (r"\bin\s+a\s+manner\s+that\b", "so that"),
            (r"\bas\s+a\s+result\s+of\b", "from"),
            (r"\bprior\s+to\b", "before"),
            (r"\bsubsequent\s+to\b", "after"),
            (r"\bin\s+the\s+case\s+that\b", "if"),
            (r"\bfor\s+the\s+reason\s+that\b", "because"),
            (r"\bwith\s+regard\s+to\b", "re:"),
            (r"\bwith\s+respect\s+to\b", "re:"),
            (r"\bin\s+regard\s+to\b", "re:"),
            (r"\bin\s+terms\s+of\b", "for"),
            // Redundant qualifiers
            (r"\bactually\b", ""),
            (r"\bbasically\b", ""),
            (r"\bessentially\b", ""),
            (r"\bfundamentally\b", ""),
            (r"\bliterally\b", ""),
            (r"\bobviously\b", ""),
            (r"\bclearly\b", ""),
            (r"\bneedless\s+to\s+say\b", ""),
            (r"\bit\s+goes\s+without\s+saying\b", ""),
            (r"\bas\s+you\s+know\b", ""),
            (r"\bas\s+we\s+all\s+know\b", ""),
            // Synonymous substitution (light)
            (r"\butilize\b", "use"),
            (r"\butilization\b", "use"),
            (r"\bimplement\b", "add"),
            (r"\bimplementation\b", "adding"),
            (r"\bfacilitate\b", "help"),
            (r"\bfacilitation\b", "help"),
            (r"\bdemonstrate\b", "show"),
            (r"\bdemonstration\b", "demo"),
            (r"\bsubsequently\b", "then"),
            (r"\bnevertheless\b", "but"),
            (r"\bnonetheless\b", "but"),
            (r"\bwith\s+the\s+exception\s+of\b", "except"),
            (r"\bof\s+the\s+opinion\s+that\b", "think"),
            (r"\ba\s+sufficient\s+amount\s+of\b", "enough"),
            (r"\ban\s+insufficient\s+amount\s+of\b", "not enough"),
            // Semantic formatting (light)
            (r"\bfor\s+example\b", "e.g."),
            (r"\bthat\s+is\s+to\s+say\b", "i.e."),
            (r"\bthe\s+code\s+provided\s+below\s*:\s*", "Code: "),
            (r"\bthe\s+following\s+code\s*:\s*", "Code: "),
            (r"\bthe\s+example\s+below\s*:\s*", "Example: "),
            (r"\bas\s+shown\s+below\s*:\s*", ""),
            (r"\bas\s+described\s+below\s*:\s*", ""),
            (r"\bthis\s+means\s+that\b", "meaning"),
            (r"\bwhat\s+this\s+means\s+is\b", "meaning"),
            (r"\bindicate\b", "show"),
        ],
    )
});

pub static VERBOSE_PHRASES: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "verbose_phrases",
        &[
            (r"\bmake\s+sure\s+to\b", "ensure"),
            (r"\btake\s+into\s+account\b", "consider"),
            (r"\btake\s+into\s+consideration\b", "consider"),
            (r"\bis\s+able\s+to\b", "can"),
            (r"\bare\s+able\s+to\b", "can"),
            (r"\bhas\s+the\s+ability\s+to\b", "can"),
            (r"\bhave\s+the\s+ability\s+to\b", "can"),
            (r"\bin\s+addition\s+to\b", "also"),
            (r"\ba\s+large\s+number\s+of\b", "many"),
            (r"\ba\s+significant\s+number\s+of\b", "many"),
            (r"\bthe\s+vast\s+majority\s+of\b", "most"),
            (r"\bthe\s+majority\s+of\b", "most"),
            (r"\bin\s+close\s+proximity\s+to\b", "near"),
            (r"\bin\s+the\s+vicinity\s+of\b", "near"),
            (r"\buntil\s+such\s+time\s+as\b", "until"),
            (r"\bduring\s+the\s+course\s+of\b", "during"),
            (r"\bon\s+a\s+daily\s+basis\b", "daily"),
            (r"\bon\s+a\s+regular\s+basis\b", "regularly"),
            (r"\bin\s+the\s+near\s+future\b", "soon"),
            (r"\bat\s+the\s+end\s+of\s+the\s+day\b", "ultimately"),
            (r"\bthe\s+fact\s+that\b", "that"),
            (r"\bin\s+light\s+of\b", "given"),
            (r"\bthere\s+is\s+a\s+need\s+to\b", "must"),
            (r"\bit\s+is\s+necessary\s+to\b", "must"),
            (r"\bit\s+is\s+recommended\s+that\b", "should"),
            (r"\bit\s+is\s+suggested\s+that\b", "should"),
            (r"\bit\s+is\s+advisable\s+to\b", "should"),
            // Natural system prompt verbosity
            (
                r"\byour\s+(?:primary\s+)?(?:role|responsibility|job|task)\s+is\s+to\b",
                "",
            ),
            (r"\byour\s+goal\s+is\s+to\b", ""),
            (
                r"\byou\s+are\s+(?:designed|tasked|meant|intended)\s+to\b",
                "",
            ),
            (r"\byou\s+are\s+responsible\s+for\b", "handle"),
            (
                r"\bin\s+your\s+(?:responses?|analysis|review|recommendations?)\b",
                "",
            ),
            (
                r"\bin\s+your\s+(?:explanations?|feedback|suggestions?)\b",
                "",
            ),
            (
                r"\bwhen\s+(?:possible|appropriate|applicable|relevant|necessary)\s*,?\b",
                "",
            ),
            (
                r"\bwhere\s+(?:possible|appropriate|applicable|relevant|necessary)\s*,?\b",
                "",
            ),
            (r"\bat\s+all\s+times\b", "always"),
            (
                r"\bif\s+you\s+are\s+(?:unable|not\s+able)\s+to\b",
                "if unable to",
            ),
            (r"\bwhenever\s+possible\b", ""),
            (r"\bas\s+(?:needed|required|appropriate|necessary)\b", ""),
            // Wordy clause openers
            (r"\bwhen\s+dealing\s+with\b", "for"),
            (r"\bwhen\s+working\s+with\b", "for"),
            (r"\bwhen\s+responding\s+to\b", "for"),
            (r"\bwhen\s+providing\b", "for"),
            (r"\bwhen\s+discussing\b", "for"),
            (r"\bwhen\s+reviewing\b", "for"),
            (r"\bwhen\s+analyzing\b", "for"),
            (r"\bwhen\s+explaining\b", "for"),
            (r"\bwhen\s+helping\b", "for"),
            (r"\bwhen\s+making\b", "for"),
            // Redundant hedge phrases
            (r"\bthorough\s+and\s+comprehensive\b", "thorough"),
            (r"\bclear\s+and\s+concise\b", "concise"),
            (r"\baccurate\s+and\s+up[- ]to[- ]date\b", "current"),
            (
                r"\bhelpful\s+and\s+(?:friendly|supportive|informative)\b",
                "helpful",
            ),
            (
                r"\bpolite\s*,?\s+professional\s*,?\s+and\s+(?:empathetic|courteous|respectful)\b",
                "professional",
            ),
            (
                r"\brespectful\s+and\s+(?:educational|constructive|professional)\b",
                "constructive",
            ),
            (r"\bclear\s*,?\s+(?:accessible|understandable)\b", "clear"),
            (
                r"\bpractical\s+and\s+(?:realistic|actionable|immediately\s+actionable)\b",
                "practical",
            ),
            (
                r"\benthusiastic\s+and\s+(?:inspiring|supportive)\b",
                "enthusiastic",
            ),
            (
                r"\bbalanced\s+and\s+(?:nuanced|objective|fair)\b",
                "balanced",
            ),
            (
                r"\bpatient\s+and\s+(?:supportive|understanding|encouraging)\b",
                "patient",
            ),
            // Redundant sentence starters in instructions
            (r"\balways\s+consider\s+the\b", "consider"),
            (r"\balways\s+remember\s+(?:that|to)\b", ""),
            (r"\balways\s+make\s+sure\b", "ensure"),
            (r"\balways\s+keep\s+in\s+mind\b", "note:"),
            (r"\bplease\s+be\s+(?:sure|careful|mindful)\s+to\b", ""),
            (r"\bplease\s+consider\b", "consider"),
            (r"\bplease\s+provide\b", "provide"),
            (r"\bplease\s+include\b", "include"),
            (r"\bplease\s+recommend\b", "recommend"),
            (r"\bplease\s+also\b", "also"),
            // Filler meta-instructions that LLMs ignore anyway
            (r"\byou\s+should\s+also\b", "also"),
            (r"\byou\s+should\s+always\b", "always"),
            (r"\byou\s+should\s+never\b", "never"),
            (r"\bplease\s+always\b", "always"),
            (r"\bplease\s+never\b", "never"),
            // Wordy verb phrases
            (r"\bassist\s+(?:\w+\s+)?with\s+their\b", "help with"),
            (r"\bwork\s+diligently\s+to\b", ""),
            (
                r"\bwork\s+(?:closely|collaboratively)\s+with\b",
                "work with",
            ),
            (
                r"\backnowledge\s+their\s+(?:frustration|concerns?)\s+and\b",
                "address concerns and",
            ),
            (
                r"\blet\s+(?:the\s+)?(?:customer|user|client)\s+know\s+that\b",
                "inform them",
            ),
            (r"\bstep-by-step\s+instructions\b", "steps"),
            (
                r"\bstep-by-step\s+(?:guide|process|procedure|explanation)\b",
                "steps",
            ),
            (r"\bdocument\s+all\s+interactions\s+in\b", "log in"),
            (
                r"\bfor\s+their\s+patience\s+and\s+for\s+(?:choosing|using)\b",
                "for using",
            ),
            (r"\bfor\s+their\s+patience\b", ""),
            (
                r"\bconsult\s+with\s+(?:a\s+)?(?:qualified|licensed)?\s*(?:their\s+)?\b",
                "consult ",
            ),
            (r"\bconsider\s+factors?\s+such\s+as\b", "consider"),
            (
                r"\bthis\s+includes?\s+(?:but\s+is\s+)?not\s+limited\s+to\b",
                "including",
            ),
            (r"\bincluding\s+but\s+not\s+limited\s+to\b", "including"),
            // Redundant prepositional phrases
            (
                r"\bin\s+(?:a|the)\s+(?:clear|concise|timely|professional|detailed)\s+(?:manner|way|fashion)\b",
                "clearly",
            ),
            (
                r"\bin\s+(?:a|the)\s+(?:friendly|helpful|supportive)\s+(?:manner|way|fashion)\b",
                "helpfully",
            ),
            (r"\bon\s+behalf\s+of\s+the\b", "for the"),
            (r"\bwith\s+(?:a\s+)?focus\s+on\b", "focusing on"),
            (r"\bwith\s+(?:an\s+)?emphasis\s+on\b", "emphasizing"),
            (
                r"\bbased\s+on\s+(?:their|the\s+user'?s?)\s+(?:preferences?|needs?|requirements?)\b",
                "per their needs",
            ),
            (
                r"\bbased\s+on\s+(?:your|the)\s+(?:analysis|assessment|evaluation|review)\b",
                "from your review",
            ),
            // Wordy conditionals & qualifiers
            (r"\bregardless\s+of\s+(?:whether|how|what)\b", "regardless"),
            (r"\birrespective\s+of\b", "regardless of"),
            (
                r"\beven\s+in\s+(?:the\s+)?(?:case|event|situation)\s+(?:of|where|that)\b",
                "even if",
            ),
            (
                r"\bspecific(?:ally)?\s+(?:designed|intended|meant|tailored)\s+(?:for|to)\b",
                "for",
            ),
            (r"\bspecifically\b", ""),
            (r"\bparticularly\b", ""),
            (r"\bespecially\b", ""),
            // Common list verbosity
            (
                r"\b(?:various|a\s+variety\s+of|different\s+types?\s+of|a\s+range\s+of|a\s+number\s+of)\b",
                "various",
            ),
            (
                r"\b(?:all|any)\s+(?:relevant|applicable|appropriate|necessary|required)\b",
                "relevant",
            ),
            (
                r"\b(?:relevant|applicable)\s+(?:laws?|regulations?|requirements?|standards?|guidelines?)\b",
                "regulations",
            ),
            (
                r"\b(?:current|latest|up-to-date|most\s+recent)\s+(?:information|data|research|evidence|findings?)\b",
                "current data",
            ),
            (
                r"\b(?:clear|detailed|thorough|comprehensive|in-depth)\s+explanation\b",
                "explanation",
            ),
            (
                r"\b(?:clear|detailed|thorough|comprehensive|in-depth)\s+analysis\b",
                "analysis",
            ),
            (
                r"\b(?:clear|detailed|thorough|comprehensive|in-depth)\s+(?:overview|summary)\b",
                "summary",
            ),
            // Wordy adjective chains
            (
                r"\bbetter,\s+more\s+\w+,\s+and\s+more\s+(\w+)\b",
                "better, $1",
            ),
            (r"\bmore\s+(\w+)\s+and\s+more\s+(\w+)\b", "$1 and $2"),
            // Wordy constructions
            (
                r"\bnot\s+just\s+(\w+)\s+but\s+(?:also\s+)?(\w+)\b",
                "$1 and $2",
            ),
            (
                r"\bnot\s+only\s+(\w+)\s+but\s+(?:also\s+)?(\w+)\b",
                "$1 and $2",
            ),
            (
                r"\bfor\s+both\s+the\s+(\w+)\s+and\s+the\s+(\w+)\b",
                "for $1 and $2",
            ),
            (r"\bboth\s+(\w+)\s+and\s+(\w+)\b", "$1/$2"),
            // Remove "that" after common verbs
            (r"\b(ensure|verify|confirm|check|note)\s+that\b", "$1"),
            (r"\b(explain|mention|indicate|state|report)\s+that\b", "$1"),
            (r"\b(acknowledge|recognize|understand)\s+that\b", "$1"),
            (r"\b(know|remember|realize)\s+that\b", "$1"),
            // Wordy prepositional phrases
            (r"\bin\s+the\s+process\s+of\b", "while"),
            (r"\bwith\s+a\s+focus\s+on\b", "focusing on"),
            (r"\bon\s+the\s+basis\s+of\b", "based on"),
            (r"\bby\s+means\s+of\b", "via"),
            (r"\bin\s+the\s+absence\s+of\b", "without"),
            (r"\bfor\s+the\s+benefit\s+of\b", "for"),
            (r"\bwith\s+the\s+intention\s+of\b", "to"),
            // Common no-info phrases
            (r"\bwork\s+diligently\s+to\b", ""),
            (r"\bwork\s+hard\s+to\b", ""),
            (r"\bdo\s+your\s+best\s+to\b", ""),
            (
                r"\bto\s+the\s+best\s+of\s+your\s+(?:ability|abilities|knowledge)\b",
                "",
            ),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.5-0.7: Structural compression
// ══════════════════════════════════════════════

pub static IMPERATIVE_CONVERSIONS: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "imperative_conversions",
        &[
            (r"\byou\s+should\b", ""),
            (r"\byou\s+must\b", ""),
            (r"\byou\s+need\s+to\b", ""),
            (r"\byou\s+are\s+required\s+to\b", ""),
            (r"\byou\s+are\s+expected\s+to\b", ""),
            (r"\bmake\s+sure\s+you\b", ""),
            (r"\bensure\s+that\s+you\b", ""),
            (r"\bremember\s+to\b", ""),
            (r"\bdon'?t\s+forget\s+to\b", ""),
            (r"\balways\s+make\s+sure\s+to\b", "always"),
            (r"\bbe\s+sure\s+to\b", ""),
            // System prompt instruction patterns
            (r"\byou\s+will\s+(?:need\s+to|want\s+to|have\s+to)\b", ""),
            (
                r"\bit\s+is\s+(?:important|essential|critical|crucial|vital)\s+(?:that\s+you|to)\b",
                "",
            ),
            (r"\bkeep\s+in\s+mind\s+that\b", ""),
            (r"\bbe\s+(?:mindful|aware|careful)\s+(?:of|that|to)\b", ""),
            (r"\bbe\s+(?:thorough|comprehensive|detailed)\b", ""),
            (r"\bpay\s+(?:attention|close\s+attention)\s+to\b", "note"),
            (r"\btake\s+care\s+to\b", ""),
            (r"\bstrive\s+to\b", ""),
            // "Always X" → "X" (imperative is implied in system prompts)
            (r"\balways\s+follow\b", "follow"),
            (r"\balways\s+prioritize\b", "prioritize"),
            (r"\balways\s+verify\b", "verify"),
            (r"\balways\s+check\b", "check"),
            (r"\balways\s+include\b", "include"),
            (r"\balways\s+ensure\b", "ensure"),
            (r"\balways\s+provide\b", "provide"),
            (r"\balways\s+thank\b", "thank"),
            (r"\balways\s+consider\b", "consider"),
            (r"\balways\s+use\b", "use"),
            (r"\balways\s+maintain\b", "maintain"),
            (r"\balways\s+recommend\b", "recommend"),
            // "Never X" → "Don't X" (shorter)
            (r"\bnever\s+share\b", "don't share"),
            (r"\bnever\s+provide\b", "don't provide"),
            (r"\bnever\s+reveal\b", "don't reveal"),
            // "You are a" → "Act as" (shorter role declaration)
            (r"\byou\s+are\s+a\b", "Act as"),
            (r"\byou\s+are\s+an\b", "Act as"),
            // Sentence connectors
            (r"\bin\s+addition\s*,?\s*\b", "Also "),
            (r"\bfurthermore\s*,?\s*\b", "Also "),
            (r"\bmoreover\s*,?\s*\b", "Also "),
            (r"\badditionally\s*,?\s*\b", "Also "),
        ],
    )
});

pub static CLAUSE_COLLAPSE: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "clause_collapse",
        &[
            (
                r"\bwhen\s+you\s+are\s+(?:doing|performing|completing|executing)\b",
                "when",
            ),
            (
                r"\bwhile\s+you\s+are\s+(?:doing|performing|completing|executing)\b",
                "while",
            ),
            (r"\bif\s+this\s+is\s+the\s+case\s*,?\s*then\b", "if so,"),
            (r"\bif\s+that\s+is\s+the\s+case\s*,?\s*then\b", "if so,"),
            (r"\bin\s+such\s+a\s+case\b", "then"),
            (r"\bin\s+this\s+case\b", "then"),
            // Conditional wordy patterns
            (r"\bin\s+(?:the\s+)?case\s+(?:of|where|that)\b", "if"),
            (r"\bin\s+situations?\s+where\b", "when"),
            (r"\bin\s+(?:a|the)\s+scenario\s+where\b", "when"),
            (r"\bin\s+the\s+context\s+of\b", "for"),
            (r"\bwith\s+the\s+aim\s+of\b", "to"),
            (r"\bso\s+as\s+to\b", "to"),
            (r"\bfor\s+the\s+sake\s+of\b", "for"),
            // Wordy "should" patterns
            (
                r"\bshould\s+be\s+(?:written|presented|structured)\s+in\s+(?:a\s+)?(?:clear|concise|simple)\b",
                "should be clear",
            ),
            (
                r"\bshould\s+be\s+(?:easily|readily)\s+(?:understood|understandable|accessible)\b",
                "should be clear",
            ),
            (
                r"\bcan\s+be\s+(?:easily|readily)\s+(?:understood|understandable|accessible)\b",
                "is clear",
            ),
            // Relative clause compression
            (r"\bthat\s+(?:can|could|may|might)\s+be\s+used\s+to\b", "to"),
            (
                r"\bthat\s+are\s+(?:relevant|applicable|appropriate)\s+to\b",
                "for",
            ),
            (r"\bthat\s+are\s+(?:related|pertaining)\s+to\b", "about"),
            // Context/scope clauses
            (r"\bin\s+(?:a|the)\s+scenario\s+where\b", "when"),
            (r"\bin\s+situations?\s+where\b", "when"),
            (r"\bin\s+the\s+context\s+of\b", "for"),
            (r"\bwith\s+the\s+aim\s+of\b", "to"),
            (r"\bso\s+as\s+to\b", "to"),
            (r"\bfor\s+the\s+sake\s+of\b", "for"),
            // List compression (common triple patterns)
            (
                r"\b(\w+)\s+issues?,\s+(\w+)\s+(?:issues?|questions?),\s+and\s+(\w+)\s+(?:issues?|questions?|inquiries?)\b",
                "$1/$2/$3 issues",
            ),
        ],
    )
});

pub static DEVELOPER_BOILERPLATE: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "developer_boilerplate",
        &[
            (
                r"\bact\s+as\s+a\s+senior\s+(?:developer|engineer|programmer)\s+who\s+is\s+an?\s+expert\s+in\b",
                "Expert:",
            ),
            (r"\bact\s+as\s+(?:a|an)\s+experienced\b", "Expert"),
            (
                r"\bact\s+as\s+(?:a|an)\s+(?:senior|expert|professional)\b",
                "Expert",
            ),
            (
                r"\bthink\s+step\s+by\s+step\s+and\s+carefully\s+analyze\b",
                "Step by step:",
            ),
            (
                r"\blet'?s?\s+think\s+(?:about\s+this\s+)?step\s+by\s+step\b",
                "Step by step:",
            ),
            (r"\bthink\s+through\s+this\s+carefully\b", "Step by step:"),
            (
                r"\btake\s+a\s+deep\s+breath\s+and\s+(?:give\s+me\s+your\s+best|work\s+through\s+this)\b",
                "",
            ),
            (r"\btake\s+a\s+deep\s+breath\b", ""),
            (r"\byou\s+are\s+the\s+best\s+at\s+this\b", ""),
            (r"\byou\s+are\s+an\s+expert\s+at\s+this\b", ""),
            (r"\byour\s+task\s+is\s+to\b", "Task:"),
            (r"\byour\s+job\s+is\s+to\b", "Task:"),
            (r"\byour\s+goal\s+is\s+to\b", "Goal:"),
            (r"\byour\s+objective\s+is\s+to\b", "Goal:"),
            (
                r"\byou\s+are\s+an?\s+AI\s+assistant\s+designed\s+to\b",
                "Assistant:",
            ),
            (
                r"\byou\s+are\s+an?\s+AI\s+(?:language\s+)?model\s+(?:designed|trained|built)\s+to\b",
                "Assistant:",
            ),
            (
                r"\byou\s+are\s+a\s+helpful\s+(?:AI\s+)?assistant\s+(?:that|who|designed\s+to)\b",
                "Assistant:",
            ),
            (
                r"\bfollow\s+these\s+instructions\s+carefully\s*:\s*",
                "Instructions: ",
            ),
            (
                r"\bhere\s+are\s+(?:the|your|my)\s+instructions\s*:\s*",
                "Instructions: ",
            ),
            (
                r"\bbelow\s+are\s+(?:the|your)\s+instructions\s*:\s*",
                "Instructions: ",
            ),
            (
                r"\bI\s+want\s+you\s+to\s+write\s+(?:a|an)\s+(?:program|script|function)\s+that\b",
                "Program:",
            ),
            (
                r"\bwrite\s+(?:a|an)\s+(?:program|script|function)\s+that\b",
                "Program:",
            ),
            (
                r"\bmake\s+sure\s+the\s+code\s+is\s+production[\s-]ready\s+and\s+optimized\b",
                "Production ready.",
            ),
            (
                r"\bensure\s+the\s+code\s+is\s+(?:clean|efficient|optimized|well[\s-]structured)\s+and\b",
                "Clean code.",
            ),
            (
                r"\bdo\s+not\s+include\s+any\s+explanations?\s*,?\s*just\s+(?:the\s+)?code\b",
                "Only code.",
            ),
            (
                r"\bonly\s+(?:respond|reply|output)\s+with\s+(?:the\s+)?code\b",
                "Only code.",
            ),
            (r"\bno\s+explanations?\s+needed\b", "Only code."),
            (r"\bCRITICAL\s+INSTRUCTION\s*:", "CRITICAL:"),
            (r"\bIMPORTANT\s+NOTE\s*:", "NOTE:"),
            (r"\bPLEASE\s+NOTE\s+THE\s+FOLLOWING\s*:", "NOTE:"),
            (r"\bIMPORTANT\s+INSTRUCTION\s*:", "IMPORTANT:"),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.8-1.0: Aggressive canonical compression
// ══════════════════════════════════════════════

pub static CONVERSATIONAL_STRIP: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "conversational_strip",
        &[
            (r"\bI'?d\s+like\s+you\s+to\b", ""),
            (r"\bcould\s+you\s+please\b", ""),
            (r"\bwould\s+you\s+please\b", ""),
            (r"\bcan\s+you\s+please\b", ""),
            (r"\bI\s+want\s+you\s+to\b", ""),
            (r"\bI\s+need\s+you\s+to\b", ""),
            (r"\bI'?m\s+asking\s+you\s+to\b", ""),
            (r"\bwhat\s+I\s+want\s+is\s+for\s+you\s+to\b", ""),
            (r"\bif\s+possible\s*,?\b", ""),
            (r"\bif\s+you\s+can\s*,?\b", ""),
            (r"\bwhen\s+possible\s*,?\b", ""),
            (r"\bwhere\s+possible\s*,?\b", ""),
            (r"\bto\s+the\s+extent\s+possible\s*,?\b", ""),
            (r"\bthis\s+is\s+very\s+important\s*[.!]?\b", ""),
            (r"\bthis\s+is\s+critical\s*[.!]?\b", ""),
            (r"\bpay\s+close\s+attention\s+to\s+this\b", ""),
            (r"\bthis\s+is\s+a\s+key\s+point\b", ""),
            // Meta-instructions that don't add information
            (
                r"\bprovide\s+(?:detailed|comprehensive|thorough)\s+(?:explanations?|responses?|analysis)\b",
                "explain thoroughly",
            ),
            (
                r"\bprovide\s+(?:clear|concise|helpful)\s+(?:explanations?|responses?|information)\b",
                "explain clearly",
            ),
            (
                r"\bconsider\s+the\s+(?:broader|wider|overall|general)\s+(?:context|picture|implications?)\b",
                "consider context",
            ),
            (
                r"\bthat\s+(?:can|could|may|might|would)\s+be\s+(?:easily\s+)?understood\s+by\b",
                "for",
            ),
            (r"\brecognizing\s+that\b", "since"),
            (r"\bgiven\s+the\s+fact\s+that\b", "since"),
            (r"\bconsidering\s+the\s+fact\s+that\b", "since"),
            (r"\bdistinguish\s+between\b", "separate"),
            (
                r"\bprioritize\s+the\s+most\s+(?:critical|important)\b",
                "prioritize",
            ),
            // Sentence-level meta-compression
            (
                r"\byou\s+are\s+(?:a|an)\s+(?:experienced|expert|knowledgeable|skilled|seasoned|senior)\b",
                "you are a",
            ),
            (
                r"\bwith\s+(?:over\s+)?\d+\s+years?\s+of\s+(?:experience|expertise)(?:\s+in\s+(?:the\s+)?field)?\b",
                "",
            ),
            (
                r"\byour\s+(?:role|job|task)\s+is\s+to\s+(?:help|assist|aid|support)\b",
                "help",
            ),
            (
                r"\b(?:individuals|people|users|clients|customers)\s+(?:understand|learn|comprehend)\b",
                "users understand",
            ),
            (
                r"\bprovide\s+(?:personalized|tailored|custom)\s+(?:recommendations?|suggestions?|guidance|advice)\b",
                "recommend",
            ),
            (
                r"\bprovide\s+(?:practical|useful|helpful|actionable)\s+(?:tips?|advice|guidance|suggestions?|information)\b",
                "advise",
            ),
            (
                r"\bmake\s+(?:informed|better|good|sound)\s+decisions?\b",
                "decide well",
            ),
            (
                r"\bhelp\s+(?:them|users?|clients?|customers?)\s+(?:understand|learn|make\s+sense\s+of)\b",
                "explain",
            ),
            // Footer/disclaimer compression
            (
                r"\bshould\s+not\s+be\s+(?:used\s+as|considered)\s+(?:a\s+)?(?:substitute|replacement)\s+for\s+(?:professional\s+)?\b",
                "is not a substitute for ",
            ),
            (
                r"\bfor\s+(?:educational|informational)\s+purposes\s+only\b",
                "educational only",
            ),
            (
                r"\bnot\s+(?:be\s+)?considered\s+(?:as\s+)?(?:personalized\s+)?(?:professional\s+)?(?:financial|legal|medical|tax)\s+advice\b",
                "not professional advice",
            ),
            (
                r"\b(?:consult|speak)\s+with\s+(?:a\s+)?(?:qualified|licensed|certified)?\s*(?:professional|advisor|attorney|doctor|specialist|expert)\s+before\b",
                "consult a professional before",
            ),
            // Strip obvious/tautological instructions
            (
                r"\bensure\s+(?:that\s+)?all\s+(?:examples|information|data|responses?)\s+(?:are|is)\s+(?:accurate|correct|up-to-date)\b",
                "be accurate",
            ),
            (
                r"\b(?:maintain|keep)\s+(?:a\s+)?(?:professional|respectful|friendly|positive)\s+(?:tone|demeanor|attitude|approach)\b",
                "be professional",
            ),
            // Redundant instruction closers
            (
                r"\bshould\s+not\s+be\s+(?:considered|used)\s+as\s+(?:a\s+substitute\s+for|replacement\s+for|personalized)\b",
                "is not",
            ),
            (
                r"\bshould\s+consult\s+with\s+(?:a|their|an?)\s+(?:qualified|licensed|professional)\b",
                "should consult",
            ),
            // Strip known tautological instructions
            (r"\.\s*(?:Always|Ensure|Make sure)\s+", ". "),
            // Subordinate clauses restating the obvious
            (r",\s*recognizing\s+that\s+[^.]{10,60}\.\s*", ". "),
            (r",\s*understanding\s+that\s+[^.]{10,60}\.\s*", ". "),
            (r",\s*keeping\s+in\s+mind\s+that\s+[^.]{10,60}\.\s*", ". "),
        ],
    )
});

pub static AI_OUTPUT_NOISE: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "ai_output_noise",
        &[
            (
                r"\bas\s+an?\s+AI\s+(?:language\s+)?model\s*,?\s*(?:I\s+)?",
                "",
            ),
            (r"\bas\s+an?\s+(?:artificial\s+intelligence|AI)\s*,?\s*", ""),
            (
                r"\bas\s+a\s+(?:large\s+)?language\s+model\s*,?\s*(?:I\s+)?",
                "",
            ),
            (
                r"\bI\s+(?:am|'m)\s+sorry\s+for\s+(?:the|any)\s+confusion\s*[.,]?\s*",
                "",
            ),
            (
                r"\bI\s+apologize\s+for\s+(?:the|any)\s+confusion\s*[.,]?\s*",
                "",
            ),
            (
                r"\bsorry\s+(?:about|for)\s+(?:the|that|any)\s+(?:confusion|misunderstanding)\s*[.,]?\s*",
                "",
            ),
            (
                r"\bI\s+apologize\s+for\s+(?:the|my)\s+oversight\s*[.,]?\s*",
                "",
            ),
            (
                r"\bI\s+(?:am|'m)\s+sorry\s+for\s+(?:the|my)\s+oversight\s*[.,]?\s*",
                "",
            ),
            (
                r"\bmy\s+apologies\s+for\s+(?:the|that|any)\s+(?:oversight|mistake|error)\s*[.,]?\s*",
                "",
            ),
            (
                r"\bhowever\s*,?\s*it'?s?\s+important\s+to\s+(?:consider|note|remember|keep\s+in\s+mind)\b",
                "Note:",
            ),
            (
                r"\bit'?s?\s+worth\s+(?:noting|mentioning|considering)\s+that\b",
                "Note:",
            ),
            (
                r"\bit'?s?\s+important\s+to\s+(?:keep\s+in\s+mind|understand|recognize)\s+that\b",
                "Note:",
            ),
            (
                r"\bI\s+cannot\s+provide\s+.*?,\s*but\s+I\s+can\b",
                "Instead, I can",
            ),
            (
                r"\bI'?m\s+(?:not\s+able|unable)\s+to\s+.*?,\s*(?:but|however)\s*,?\s*",
                "",
            ),
            (
                r"\blet\s+me\s+know\s+if\s+you\s+need\s+any\s+(?:further|more|additional)\s+(?:help|assistance|information|clarification)\s*[.!]?\s*",
                "",
            ),
            (
                r"\bdon'?t\s+hesitate\s+to\s+(?:ask|reach\s+out)\s*[.!]?\s*",
                "",
            ),
            (r"\bI\s+hope\s+this\s+helps\s*[.!]?\s*", ""),
            (
                r"\bhope\s+(?:this|that)\s+(?:helps|is\s+helpful)\s*[.!]?\s*",
                "",
            ),
            (
                r"\bfeel\s+free\s+to\s+ask\s+if\s+you\s+have\s+(?:any\s+)?(?:more|other|further|additional)\s+questions?\s*[.!]?\s*",
                "",
            ),
            (
                r"\bif\s+you\s+have\s+any\s+(?:other|more|further)\s+questions?\s*,?\s*(?:feel\s+free\s+to\s+ask|let\s+me\s+know)\s*[.!]?\s*",
                "",
            ),
            (r"\bplease\s+keep\s+in\s+mind\s+that\b", "note:"),
            (r"\bkeep\s+in\s+mind\s+that\b", "note:"),
            (
                r"\bit\s+depends\s+on\s+(?:various|many|several|a\s+number\s+of)\s+factors\b",
                "it varies",
            ),
            (
                r"\bthere\s+are\s+(?:many|several|various)\s+(?:factors|considerations)\s+(?:to\s+)?(?:consider|keep\s+in\s+mind)\b",
                "it varies",
            ),
        ],
    )
});

pub static MARKDOWN_MINIFICATION: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_exact(
        "markdown_minification",
        &[
            (r"(?m)^#{4,6}\s+", "# "),
            (r"(?m)^###\s+", "## "),
            (r"(?m)^(?:>\s*){2,}", "> "),
            (r"!\[\s*\]\(([^)]+)\)", "$1"),
            (r"!\[image\]\(([^)]+)\)", "$1"),
            (r"(?m)^(\s*)[*+]\s+", "$1- "),
            (r"\*{2}([^*]+)\*{2}", "$1"),
            (r"(?<!\*)\*(?!\*)([^*]+)(?<!\*)\*(?!\*)", "$1"),
            (r"\|\s{2,}", "| "),
            (r"\s{2,}\|", " |"),
            (r"<br\s*/?\s*>", "\n"),
            (r"</?(?:b|strong)>", ""),
            (r"</?(?:i|em)>", ""),
            (r"</?(?:p|div|span)>", ""),
            (r"(?m)^\s*(?:---+|___+|\*\*\*+)\s*$", ""),
            (
                r"\[(?:Read\s+(?:this|more|the)\s+(?:documentation|docs|here|article))\]\(([^)]+)\)",
                "$1",
            ),
            (r"(?i)\[(?:click\s+here|here|link)\]\(([^)]+)\)", "$1"),
            (
                r"```(?:python|javascript|typescript|bash|shell|sh|json|yaml|yml|xml|html|css|sql|java|go|rust|ruby|php|c|cpp|csharp|swift|kotlin)\s*\n",
                "```\n",
            ),
        ],
    )
});

pub static SOURCE_CODE_COMPRESSION: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_exact(
        "source_code_compression",
        &[
            (r"(?m)^\s*//(?!\s*TODO)[^\n]{2,}$", ""),
            (r"(?m)^\s*#\s+(?!TODO|!|\s*$)[^\n]{3,}$", ""),
            (r"(?s)/\*(?!\*/)(?:(?!\*/).)*?\*/", ""),
            (r#"(?s)"""(?!.*(?:TODO|FIXME|NOTE)).*?""""#, ""),
            (r"(?s)'''(?!.*(?:TODO|FIXME|NOTE)).*?'''", ""),
            (r"(?m)^\s*\n(?=\s*\n)", ""),
            (r",\s*([}\]])", "$1"),
            (r":\s+(\d)", ":$1"),
            (r";\s+", ";"),
            (r"\{\s+", "{"),
            (r"\s+\}", "}"),
        ],
    )
});

pub static CONTEXT_DEDUPLICATION: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_exact(
        "context_deduplication",
        &[
            (
                r"(?m)^(?:Thanks!?|Thank you!?|Got it!?|That worked!?|OK!?|Okay!?|Great!?|Perfect!?|Understood!?)\s*$",
                "",
            ),
            (
                r"data:[a-zA-Z/]+;base64,[A-Za-z0-9+/=]{100,}",
                "[BASE64_TRUNCATED]",
            ),
            (
                r"(?<![a-zA-Z])[A-Za-z0-9+/]{500,}={0,2}(?![a-zA-Z])",
                "[BASE64_TRUNCATED]",
            ),
            (
                r"(?i)\bI\s+will\s+(?:look\s+into|work\s+on|get\s+(?:back\s+to|on))\s+(?:this|that)\s+(?:right\s+now|immediately|shortly)\s*[.!]?\s*",
                "",
            ),
            (
                r"(?i)\bLet\s+me\s+(?:check|look\s+into|investigate)\s+(?:this|that)\s+for\s+you\s*[.!]?\s*",
                "",
            ),
            (r"(?s)(\b\w.{40,}?[.!?])\s*\1", "$1"),
            (r"(\b[A-Z][^.!?]{20,}[.!?])\s+\1", "$1"),
        ],
    )
});

pub static SEMANTIC_FORMATTING: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "semantic_formatting",
        &[
            (r"\bforty[\s-]two\s+thousand\b", "42k"),
            (r"\bten\s+thousand\b", "10k"),
            (r"\bone\s+hundred\s+thousand\b", "100k"),
            (r"\bone\s+million\b", "1M"),
            (r"\btwo\s+million\b", "2M"),
            (r"\bone\s+billion\b", "1B"),
            (r"\bone\s+hundred\b", "100"),
            (r"\btwo\s+hundred\b", "200"),
            (r"\bfive\s+hundred\b", "500"),
            (r"\bone\s+thousand\b", "1k"),
            (r"(?m)^First\s*,\s*", "1. "),
            (r"(?m)^Second\s*,\s*", "2. "),
            (r"(?m)^Third\s*,\s*", "3. "),
            (r"(?m)^Fourth\s*,\s*", "4. "),
            (r"(?m)^Fifth\s*,\s*", "5. "),
            (r"[?&]utm_\w+=[^&\s)]*", ""),
            (
                r"[?&](?:ref|source|campaign|medium|fbclid|gclid|mc_[a-z]+)=[^&\s)]*",
                "",
            ),
            (r"\bdo\s+not\b", "don't"),
            (r"\bcannot\b", "can't"),
            (r"\bwill\s+not\b", "won't"),
            (r"\bshould\s+not\b", "shouldn't"),
            (r"\bwould\s+not\b", "wouldn't"),
            (r"\bcould\s+not\b", "couldn't"),
            (r"\bdoes\s+not\b", "doesn't"),
            (r"\bdid\s+not\b", "didn't"),
            (r"\bis\s+not\b", "isn't"),
            (r"\bare\s+not\b", "aren't"),
            (r"\bwas\s+not\b", "wasn't"),
            (r"\bwere\s+not\b", "weren't"),
            (r"\bhas\s+not\b", "hasn't"),
            (r"\bhave\s+not\b", "haven't"),
            (r"\bhad\s+not\b", "hadn't"),
            (r"\b([A-Z]\w+)\s+and\s+([A-Z]\w+)\b", "$1 & $2"),
        ],
    )
});

#[allow(clippy::vec_init_then_push)]
pub static ANTI_NOISE: Lazy<RuleCategory> = Lazy::new(|| {
    RuleCategory::new("anti_noise", {
        let mut rules = Vec::new();

        // Log path truncation
        rules.push(Rule::new(
            r"(?:[A-Z]:\\(?:Users|Program Files)\\[^\s]*\\)",
            ".../",
        ));
        rules.push(Rule::new(
            r"/(?:home|usr|var|opt)/[^\s/]+(?:/[^\s/]+){2,}/",
            ".../",
        ));
        rules.push(Rule::new(r"/node_modules/[^\s]+/", "[node_modules]/ "));

        // UUID truncation
        rules.push(Rule::new(
            r"\b([0-9a-f]{8})-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b",
            "$1-...",
        ));

        // ANSI codes
        rules.push(Rule::new(r"\x1b\[[0-9;]*m", ""));
        rules.push(Rule::new(r"\\x1b\[[0-9;]*m", ""));

        // Repeating error lines
        rules.push(Rule::new(
            r"(?m)(^.{30,}$)\n(?:\1\n){2,}",
            "$1\n[repeated]\n",
        ));

        // Stack trace condensation
        rules.push(Rule::new(r"(?m)^\s+(?:at\s+)?(?:/node_modules/|/usr/lib/python|/usr/local/lib/|site-packages/).*$\n?", ""));
        rules.push(Rule::new(r"(?m)^\s+at\s+(?:internal/|node:).*$\n?", ""));

        // Long hash truncation
        rules.push(Rule::new(r"\b([0-9a-f]{7})[0-9a-f]{33}\b", "$1"));
        rules.push(Rule::new(r"\b([0-9a-f]{8})[0-9a-f]{24,}\b", "$1"));

        // System meta-prompt muting
        rules.push(Rule::new_case_insensitive(
            r"\bdo\s+not\s+share\s+this\s+system\s+prompt\s+with\s+the\s+user\s*[.!]?\s*",
            "",
        ));
        rules.push(Rule::new_case_insensitive(r"\bnever\s+reveal\s+(?:your|these?|this)\s+(?:system\s+)?(?:prompt|instructions?)\s*[.!]?\s*", ""));
        rules.push(Rule::new_case_insensitive(
            r"\bkeep\s+(?:this|these)\s+instructions?\s+(?:confidential|private|secret)\s*[.!]?\s*",
            "",
        ));

        // Empty objects
        rules.push(Rule::new(r":\s*\{\s*\}", ": {}"));
        rules.push(Rule::new(r":\s*\[\s*\]", ": []"));

        // Empty XML tags
        rules.push(Rule::new(r"<thought>\s*</thought>", ""));
        rules.push(Rule::new(r"<thinking>\s*</thinking>", ""));
        rules.push(Rule::new(r"<scratchpad>\s*</scratchpad>", ""));
        rules.push(Rule::new(r"<inner_monologue>\s*</inner_monologue>", ""));

        rules
    })
});

// ══════════════════════════════════════════════
// Rule application functions
// ══════════════════════════════════════════════

/// Apply whitespace normalization — the single end-of-pipeline cleanup pass.
///
/// Runs ONCE at the end of `compress_text` / `compress_response_text` rather
/// than after every category, so per-category passes don't re-do the same
/// double-space, space-before-punctuation, and empty-line collapsing N times.
pub fn apply_whitespace_normalization(text: &str) -> String {
    static RE_MULTI_NEWLINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());
    static RE_MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r" {2,}").unwrap());
    static RE_TRAILING_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r" +\n").unwrap());
    static RE_SPACE_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r" +([.,;:!?])").unwrap());
    static RE_EMPTY_LINES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n +\n").unwrap());

    let text = RE_MULTI_NEWLINE.replace_all(text, "\n\n");
    let text = RE_MULTI_SPACE.replace_all(&text, " ");
    let text = RE_TRAILING_SPACE.replace_all(&text, "\n");
    let text = RE_SPACE_PUNCT.replace_all(&text, "$1");
    let text = RE_EMPTY_LINES.replace_all(&text, "\n\n");
    text.trim().to_string()
}

/// Apply a category of rules with RegexSet prefilter and count hits.
///
/// The RegexSet decides which std-regex rules can possibly match the
/// input `text`; the rest are skipped without a full scan. Fancy-regex
/// rules always run (they cannot live in a RegexSet).
///
/// Cleanup whitespace normalization is intentionally NOT applied here —
/// it runs once at the end of `compress_text` / `compress_response_text`
/// instead of after every category.
pub fn apply_category_counted<'a>(text: &'a str, category: &RuleCategory) -> (Cow<'a, str>, usize) {
    let matched: std::collections::HashSet<usize> =
        category.set.matches(text).into_iter().collect();

    // Map each rule index → whether it should run on the *initial* text.
    // After the first owned mutation, we conservatively run remaining
    // candidate rules (they may or may not hit on the rewritten text).
    let mut should_run = vec![true; category.rules.len()];
    for (set_idx, rule_idx) in category.set_to_rule.iter().enumerate() {
        should_run[*rule_idx] = matched.contains(&set_idx);
    }

    let mut current: Cow<'a, str> = Cow::Borrowed(text);
    let mut hits = 0usize;
    let mut mutated = false;
    for (i, rule) in category.rules.iter().enumerate() {
        if !mutated && !should_run[i] {
            continue;
        }
        let (next, did_match) = rule.replace_all_counted(current.as_ref());
        if did_match {
            hits += 1;
            current = Cow::Owned(next.into_owned());
            mutated = true;
        }
    }

    if hits > 0 {
        category.category_hits.fetch_add(hits, Ordering::Relaxed);
    }
    (current, hits)
}

/// Backward-compat wrapper used by callers that still hold a `&[Rule]`.
/// Skips the prefilter (no category context) but keeps Cow-based misses.
pub fn apply_rules_counted(text: &str, rules: &[Rule]) -> (String, usize) {
    let mut current: Cow<str> = Cow::Borrowed(text);
    let mut hits = 0usize;
    for rule in rules {
        let (next, did_match) = rule.replace_all_counted(current.as_ref());
        if did_match {
            hits += 1;
            current = Cow::Owned(next.into_owned());
        }
    }
    (current.into_owned(), hits)
}

/// Backward-compat: apply rules without counting (legacy callers).
pub fn apply_rules(text: &str, rules: &[Rule]) -> String {
    let (out, _) = apply_rules_counted(text, rules);
    out
}

/// Minify JSON payloads (standalone blocks)
pub fn minify_json_payload(text: &str) -> String {
    static RE_JSON_OBJ: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?ms)^(\{[^}]{20,}\})\s*$").unwrap());
    static RE_JSON_ARR: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?ms)^(\[[^\]]{20,}\])\s*$").unwrap());

    let try_minify = |text: &str| -> Option<String> {
        let val: serde_json::Value = serde_json::from_str(text).ok()?;
        serde_json::to_string(&val).ok()
    };

    let result = RE_JSON_OBJ.replace_all(text, |caps: &regex::Captures| {
        try_minify(&caps[1]).unwrap_or_else(|| caps[0].to_string())
    });
    let result = RE_JSON_ARR.replace_all(&result, |caps: &regex::Captures| {
        try_minify(&caps[1]).unwrap_or_else(|| caps[0].to_string())
    });

    result.to_string()
}

/// Pre-compiled date patterns: (month_name_re, month_num, format1_re, format2_re)
static DATE_PATTERNS: Lazy<Vec<(&str, Regex, Regex)>> = Lazy::new(|| {
    let months = [
        ("january", "01"),
        ("february", "02"),
        ("march", "03"),
        ("april", "04"),
        ("may", "05"),
        ("june", "06"),
        ("july", "07"),
        ("august", "08"),
        ("september", "09"),
        ("october", "10"),
        ("november", "11"),
        ("december", "12"),
    ];
    months
        .iter()
        .map(|(name, num)| {
            let re1 = Regex::new(&format!(
                r"(?i)\b{}\s+(\d{{1,2}})(?:st|nd|rd|th)?\s*,?\s*(\d{{4}})\b",
                name
            ))
            .unwrap();
            let re2 = Regex::new(&format!(
                r"(?i)\b(\d{{1,2}})(?:st|nd|rd|th)?\s+{}\s*,?\s*(\d{{4}})\b",
                name
            ))
            .unwrap();
            (*num, re1, re2)
        })
        .collect()
});

// ══════════════════════════════════════════════
// Level 0.5+: Credential & role declaration stripping
// ══════════════════════════════════════════════

pub static CREDENTIAL_STRIP: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "credential_strip",
        &[
            // "with over N years of experience in the field"
            (
                r"\bwith\s+(?:over\s+|more\s+than\s+)?\d+\+?\s+years?\s+(?:of\s+)?experience(?:\s+in\s+[\w\s]+?)?(?=\.)",
                "",
            ),
            // "with extensive knowledge of X worldwide"
            (
                r"\bwith\s+extensive\s+(?:knowledge|experience)\s+(?:of|in)\s+[\w\s]+(?=\.)",
                "",
            ),
            // "You are a [adj] [adj] X" → strip "You are a/an" (role kept)
            (
                r"\bYou\s+are\s+(?:a|an)\s+(?:very\s+)?(?:helpful|experienced|knowledgeable|senior|skilled|expert)\s+(?:and\s+(?:friendly|supportive|experienced|knowledgeable)\s+)?",
                "",
            ),
            // "designed to help X understand Y"
            (r"\bdesigned\s+to\s+(?:help|assist|support)\b", "helps"),
            // "specializing in X" after role declaration → strip
            (
                r"\bspecializing\s+in\s+([\w\s,]+?)(?=\.)",
                "($1 specialist)",
            ),
            // "with expertise in X" → strip
            (r"\bwith\s+expertise\s+in\s+([\w\s]+?)(?=\.)", "($1 expert)"),
            // "in the field" after experience statement → remove
            (r"\bin\s+the\s+field\b", ""),
            // "for [Company Name]" when company is generic → keep but compact
            // "for a [5000-employee|Fortune 500] organization" → strip numeric
            (
                r"\bfor\s+a\s+\d[\d,]*-?\s*employee\s+(?:organization|company)\b",
                "",
            ),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.8+: Disclaimer & boilerplate collapse
// ══════════════════════════════════════════════

pub static DISCLAIMER_COLLAPSE: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "disclaimer_collapse",
        &[
            // "should not be considered legal/financial/medical advice"
            (
                r"(?i)(?:Please\s+note\s+that\s+)?(?:your|this)\s+(?:analysis|information|response)\s+should\s+not\s+be\s+considered\s+(?:as\s+)?(?:personalized\s+)?(?:legal|financial|medical|professional)\s+advice[^.]*\.",
                "[Not professional advice.]",
            ),
            // "consult with a licensed X"
            (
                r"(?i)(?:They|Users?|You)\s+should\s+(?:always\s+)?consult\s+with\s+(?:a\s+|their\s+)?(?:licensed\s+|qualified\s+)?(?:financial\s+advisor|legal\s+counsel|healthcare\s+professional|doctor|attorney)[^.]*\.",
                "[Consult a professional.]",
            ),
            // "for educational purposes only, not a substitute for..."
            (
                r"(?i)(?:your\s+)?information\s+is\s+for\s+educational\s+purposes\s+only\s+and\s+should\s+not\s+be\s+used\s+as\s+a\s+substitute\s+for\s+professional[^.]*\.",
                "[Educational only.]",
            ),
            // "Always encourage/remind users to consult..."
            (
                r"(?i)Always\s+(?:encourage|remind)\s+users?\s+to\s+consult\s+with\s+(?:qualified\s+)?(?:healthcare\s+)?professionals?[^.]*\.",
                "",
            ),
            // "recommend that X be reviewed by legal counsel"
            (
                r"(?i)(?:Always\s+)?(?:recommend|suggest)\s+that\s+[^.]*(?:be\s+reviewed|consult|seek\s+(?:legal|professional))[^.]*\.",
                "",
            ),
            // "Please remind users about the importance of..."
            (
                r"(?i)(?:Please\s+)?remind\s+users?\s+(?:about\s+)?(?:the\s+importance\s+of|that\s+they\s+should)[^.]*\.",
                "",
            ),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.8+: Adjective chain & clause collapse
// ══════════════════════════════════════════════

pub static ADJECTIVE_COLLAPSE: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "adjective_collapse",
        &[
            // "better, more X, and more Y Z" → "better Z"
            (
                r"\b(better|improved|higher-quality),?\s+more\s+\w+,?\s+and\s+more\s+\w+\s+(\w+)\b",
                "$1 $2",
            ),
            // "clear, comprehensive, and legally compliant" → "clear, compliant"
            (
                r"\bclear,?\s+comprehensive,?\s+and\s+legally\s+compliant\b",
                "clear, compliant",
            ),
            // "clear, accessible language that can be easily understood by X"
            (
                r"\bclear,?\s+accessible\s+language\s+that\s+can\s+be\s+(?:easily\s+)?understood\s+by\s+[^.]+(?=\.)",
                "clear language",
            ),
            // "accurate, evidence-based X info in a clear and educational format"
            (
                r"\baccurate,?\s+evidence-based\s+(\w+)\s+information\s+in\s+a\s+clear\s+and\s+educational\s+format\b",
                "evidence-based $1 info",
            ),
            // "simple, easy-to-understand terms while maintaining technical accuracy"
            (
                r"\bsimple,?\s+easy-to-understand\s+terms?\s+while\s+(?:still\s+)?maintaining\s+technical\s+accuracy\b",
                "simple but accurate terms",
            ),
            // "polite, professional, and empathetic" (remaining after pair rules)
            (
                r"\brespectful\s+and\s+educational\s+in\s+(?:your|the)\s+\w+,?\s*",
                "",
            ),
            // Long enumeration collapse: "X, Y, Z, W, and V" with 4+ items → "X/Y/Z/W/V"
            // "technical issues, billing questions, and product inquiries" → "technical/billing/product issues"
            (
                r"\btechnical\s+issues,?\s+billing\s+questions,?\s+and\s+product\s+inquiries\b",
                "technical/billing/product issues",
            ),
            // "potential bugs, security vulnerabilities, performance bottlenecks, code style issues, and opportunities for refactoring"
            (
                r"\bpotential\s+bugs,?\s+security\s+vulnerabilities,?\s+performance\s+bottlenecks,?\s+code\s+style\s+issues,?\s+and\s+opportunities\s+for\s+refactoring\b",
                "bugs/security/perf/style/refactoring",
            ),
            // "error handling, edge cases, input validation, and resource management"
            (
                r"\berror\s+handling,?\s+edge\s+cases,?\s+input\s+validation,?\s+and\s+resource\s+management\b",
                "errors/edge-cases/validation/resources",
            ),
            // "computational costs, latency requirements, and model interpretability needs"
            (
                r"\bcomputational\s+costs,?\s+latency\s+requirements,?\s+and\s+(?:model\s+)?interpretability\s+needs\b",
                "cost/latency/interpretability",
            ),
            // "experiment design, cross-validation techniques, feature engineering approaches, and model evaluation metrics"
            (
                r"\b(?:proper\s+)?experiment\s+design,?\s+cross-validation\s+techniques,?\s+feature\s+engineering\s+approaches,?\s+and\s+model\s+evaluation\s+metrics\b",
                "experiment design/CV/features/evaluation",
            ),
            // Generic triple-adjective patterns: "X, Y, and Z [noun]" → "X/Y/Z [noun]"
            (
                r"\b(\w+),?\s+(\w+),?\s+and\s+(\w+)\s+(issues?|concerns?|requirements?|considerations?|problems?)\b",
                "$1/$2/$3 $4",
            ),
            // Verbose → compact phrase pairs
            (r"\bstep-by-step\s+instructions\b", "steps"),
            (
                r"\bpractical\s+examples\s+and\s+real-world\s+use\s+cases\b",
                "examples",
            ),
            (
                r"\bconcrete\s+code\s+examples\s+showing\s+the\s+recommended\s+changes\b",
                "code examples",
            ),
            (
                r"\b(?:clear|detailed)\s+(?:explanations?|commentary)\s+(?:on|that|for)\b",
                "explaining",
            ),
            (
                r"\b(?:the\s+)?mathematical\s+intuition\s+behind\s+them\b",
                "the intuition",
            ),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.8+: Verbose clause simplification
// ══════════════════════════════════════════════

pub static CLAUSE_SIMPLIFY: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "clause_simplify",
        &[
            // "that explains not just X but also Y"
            (
                r"\bthat\s+explains\s+not\s+(?:just|only)\s+(\w+)(?:\s+should\s+be\s+\w+)?\s+but\s+(?:also\s+)?(\w+)\b",
                "explaining $1 and $2",
            ),
            // ", recognizing that X is a Y for both Z and W"
            (r",?\s*recognizing\s+that\s+[^.]{10,80}(?=\.)", ""),
            // "and work diligently/carefully to resolve their issues"
            (
                r"\band\s+work\s+(?:diligently|hard|carefully|tirelessly)\s+to\s+resolve\s+(?:their|the|these)\s+issues?\b",
                "and resolve them",
            ),
            // "without getting overly theoretical/complex"
            (r"\bwithout\s+getting\s+overly\s+\w+\b", "concisely"),
            // "help X make the most of their Y"
            (
                r"\bhelp\s+\w+\s+make\s+the\s+most\s+of\s+their\s+(\w+)\b",
                "maximize their $1",
            ),
            // "while remaining X and Y"
            (
                r"\bwhile\s+(?:remaining|staying|being)\s+\w+\s+and\s+\w+\b",
                "",
            ),
            // "that will help X do Y" → "to Y"
            (r"\bthat\s+will\s+help\s+\w+\s+(\w+)\b", "to $1"),
            // "before making any decisions/changes"
            (
                r"\bbefore\s+making\s+any\s+(?:decisions?|changes?|modifications?)\b",
                "first",
            ),
            // "based on their/the X, Y, and Z"
            (
                r"\bbased\s+on\s+(?:their|the)\s+(?:preferences,?\s+)?(?:budget,?\s+)?(?:and\s+)?(?:travel\s+style|needs|requirements|goals)\b",
                "per preferences",
            ),
            // "saving, investing, budgeting, and planning for retirement"
            (
                r"\bsaving,?\s+investing,?\s+budgeting,?\s+and\s+planning\s+for\s+retirement\b",
                "personal finance",
            ),
            // "diversity, equity, and inclusion principles"
            (
                r"\bdiversity,?\s+equity,?\s+and\s+inclusion\s*(?:principles)?\b",
                "DEI",
            ),
            // "benefits, risks, and alternatives/side effects"
            (
                r"\bbenefits,?\s+risks,?\s+and\s+(?:alternatives|side\s+effects)\b",
                "tradeoffs",
            ),
            // "while still maintaining/ensuring/preserving X"
            (
                r"\bwhile\s+(?:still\s+)?(?:maintaining|ensuring|preserving)\s+(\w+(?:\s+\w+)?)\b",
                "(keep $1)",
            ),
            // "without getting overly X" → concisely (already exists, add more)
            (
                r"\bwithout\s+(?:being|becoming)\s+(?:too|overly)\s+\w+\b",
                "",
            ),
            // "for both X and Y" when context is clear → remove
            (r"\bfor\s+both\s+the\s+\w+\s+and\s+the\s+\w+\b", ""),
            // "not just X but [also] Y" → "X and Y"
            (
                r"\bnot\s+(?:just|only)\s+(\w+(?:\s+\w+)?)\s+but\s+(?:also\s+)?(\w+)\b",
                "$1 and $2",
            ),
            // "a learning opportunity for X" → learning moment
            (
                r"\ba\s+learning\s+(?:opportunity|experience)\s+for\s+[^.]+?(?=\.)",
                "a learning opportunity",
            ),
            // "who will follow up within X" → "(follow-up within X)"
            (
                r"\bwho\s+will\s+follow\s+up\s+within\s+(\w+(?:\s+\w+)?)\b",
                "(follow-up $1)",
            ),
            // "let the X know that Y" → "tell X Y"
            (r"\blet\s+the\s+(\w+)\s+know\s+that\b", "tell $1"),
            // "to illustrate your points" → drop
            (r"\bto\s+illustrate\s+(?:your|the)\s+points?\b", ""),
            // "in your teaching approach" → drop
            (
                r"\bin\s+(?:your|the)\s+(?:teaching|mentoring|coaching|advising)\s+approach\b",
                "",
            ),
        ],
    )
});

// ══════════════════════════════════════════════
// Level 0.5+: Multi-whitespace and article cleanup
// ══════════════════════════════════════════════

// ══════════════════════════════════════════════
// Level 0.8+: Adverb stripping for maximum compression
// ══════════════════════════════════════════════

pub static ADVERB_STRIP: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_ci(
        "adverb_strip",
        &[
            // Intensifier adverbs that don't change meaning
            (
                r"\b(?:very|extremely|highly|incredibly|particularly|especially|really|truly)\s+",
                "",
            ),
            // Process adverbs that LLMs don't need
            (
                r"\bwork\s+(?:diligently|carefully|hard|tirelessly)\b",
                "work",
            ),
            (
                r"\bcarefully\s+(?:analyze|consider|review|evaluate|examine)\b",
                "$1",
            ),
            (
                r"\bthoroughly\s+(?:review|analyze|check|test|examine)\b",
                "$1",
            ),
            (r"\brigorously\s+(?:evaluate|test|analyze|check)\b", "$1"),
            (
                r"\bconsistently\s+(?:applied|used|followed|enforced)\b",
                "$1",
            ),
            (r"\bexplicitly\s+(?:stated|mentioned|noted|listed)\b", "$1"),
            (r"\bproactively\s+(?:alert|warn|notify|inform)\b", "$1"),
            // "immediately actionable" → "actionable"
            (r"\bimmediately\s+actionable\b", "actionable"),
            // Remove trailing "on this/on this topic" etc
            (r"\bon\s+this\s+(?:topic|matter|issue|subject)\b", ""),
        ],
    )
});

pub static WHITESPACE_CLEANUP: Lazy<RuleCategory> = Lazy::new(|| {
    category_from_exact(
        "whitespace_cleanup",
        &[
            // Double spaces (from rule removals)
            (r"  +", " "),
            // Space before punctuation
            (r" ,", ","),
            (r" \.", "."),
            // Double periods
            (r"\.\.", "."),
            // Leading space in sentence
            (r"(?m)^\s+", ""),
            // Orphan periods from deleted phrases
            (r"\. \.", "."),
        ],
    )
});

/// Compress dates: "January 14th, 2025" → "2025-01-14".
/// Always preserves four-digit years to avoid century ambiguity
/// (1999 vs 2099 both compress to distinct outputs).
pub fn compress_dates(text: &str) -> String {
    let mut result = text.to_string();
    for (num, re1, re2) in DATE_PATTERNS.iter() {
        let num = *num;
        result = re1
            .replace_all(&result, |caps: &regex::Captures| {
                let year = &caps[2];
                let day = format!("{:0>2}", &caps[1]);
                format!("{}-{}-{}", year, num, day)
            })
            .to_string();
        result = re2
            .replace_all(&result, |caps: &regex::Captures| {
                let year = &caps[2];
                let day = format!("{:0>2}", &caps[1]);
                format!("{}-{}-{}", year, num, day)
            })
            .to_string();
    }
    result
}
