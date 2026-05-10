//! Nyquest Rule Analytics
//! Lock-free atomic counters for cumulative rule hit tracking.
//! Thread-safe: every tokio worker can update without locking.

use crate::compression::CompressionStats;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};

/// Global cumulative rule analytics — one instance in AppState, shared across all requests.
/// Uses relaxed atomics: exact precision isn't critical, throughput is.
#[derive(Debug)]
pub struct RuleAnalytics {
    pub total_requests: AtomicU64,
    pub total_rule_hits: AtomicU64,
    pub total_tokens_before: AtomicU64,
    pub total_tokens_after: AtomicU64,
    // Per-category cumulative hit counts
    pub openclaw_rules: AtomicU64,
    pub filler_phrases: AtomicU64,
    pub verbose_phrases: AtomicU64,
    pub imperative_conversions: AtomicU64,
    pub clause_collapse: AtomicU64,
    pub developer_boilerplate: AtomicU64,
    pub semantic_formatting: AtomicU64,
    pub credential_strip: AtomicU64,
    pub whitespace_cleanup: AtomicU64,
    pub conversational_strip: AtomicU64,
    pub ai_output_noise: AtomicU64,
    pub markdown_minification: AtomicU64,
    pub source_code_compression: AtomicU64,
    pub context_deduplication: AtomicU64,
    pub anti_noise: AtomicU64,
    pub disclaimer_collapse: AtomicU64,
    pub adjective_collapse: AtomicU64,
    pub clause_simplify: AtomicU64,
    pub adverb_strip: AtomicU64,
    // Response-specific counters
    pub response_compressions: AtomicU64,
    pub response_tokens_saved: AtomicU64,
}

impl RuleAnalytics {
    pub fn new() -> Self {
        Self {
            total_requests: AtomicU64::new(0),
            total_rule_hits: AtomicU64::new(0),
            total_tokens_before: AtomicU64::new(0),
            total_tokens_after: AtomicU64::new(0),
            openclaw_rules: AtomicU64::new(0),
            filler_phrases: AtomicU64::new(0),
            verbose_phrases: AtomicU64::new(0),
            imperative_conversions: AtomicU64::new(0),
            clause_collapse: AtomicU64::new(0),
            developer_boilerplate: AtomicU64::new(0),
            semantic_formatting: AtomicU64::new(0),
            credential_strip: AtomicU64::new(0),
            whitespace_cleanup: AtomicU64::new(0),
            conversational_strip: AtomicU64::new(0),
            ai_output_noise: AtomicU64::new(0),
            markdown_minification: AtomicU64::new(0),
            source_code_compression: AtomicU64::new(0),
            context_deduplication: AtomicU64::new(0),
            anti_noise: AtomicU64::new(0),
            disclaimer_collapse: AtomicU64::new(0),
            adjective_collapse: AtomicU64::new(0),
            clause_simplify: AtomicU64::new(0),
            adverb_strip: AtomicU64::new(0),
            response_compressions: AtomicU64::new(0),
            response_tokens_saved: AtomicU64::new(0),
        }
    }

    /// Merge per-request CompressionStats into global counters
    pub fn record_request(&self, stats: &CompressionStats, original: usize, optimized: usize) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.total_rule_hits
            .fetch_add(stats.total_rule_hits as u64, Ordering::Relaxed);
        self.total_tokens_before
            .fetch_add(original as u64, Ordering::Relaxed);
        self.total_tokens_after
            .fetch_add(optimized as u64, Ordering::Relaxed);

        self.openclaw_rules
            .fetch_add(stats.openclaw_rules as u64, Ordering::Relaxed);
        self.filler_phrases
            .fetch_add(stats.filler_phrases as u64, Ordering::Relaxed);
        self.verbose_phrases
            .fetch_add(stats.verbose_phrases as u64, Ordering::Relaxed);
        self.imperative_conversions
            .fetch_add(stats.imperative_conversions as u64, Ordering::Relaxed);
        self.clause_collapse
            .fetch_add(stats.clause_collapse as u64, Ordering::Relaxed);
        self.developer_boilerplate
            .fetch_add(stats.developer_boilerplate as u64, Ordering::Relaxed);
        self.semantic_formatting
            .fetch_add(stats.semantic_formatting as u64, Ordering::Relaxed);
        self.credential_strip
            .fetch_add(stats.credential_strip as u64, Ordering::Relaxed);
        self.whitespace_cleanup
            .fetch_add(stats.whitespace_cleanup as u64, Ordering::Relaxed);
        self.conversational_strip
            .fetch_add(stats.conversational_strip as u64, Ordering::Relaxed);
        self.ai_output_noise
            .fetch_add(stats.ai_output_noise as u64, Ordering::Relaxed);
        self.markdown_minification
            .fetch_add(stats.markdown_minification as u64, Ordering::Relaxed);
        self.source_code_compression
            .fetch_add(stats.source_code_compression as u64, Ordering::Relaxed);
        self.context_deduplication
            .fetch_add(stats.context_deduplication as u64, Ordering::Relaxed);
        self.anti_noise
            .fetch_add(stats.anti_noise as u64, Ordering::Relaxed);
        self.disclaimer_collapse
            .fetch_add(stats.disclaimer_collapse as u64, Ordering::Relaxed);
        self.adjective_collapse
            .fetch_add(stats.adjective_collapse as u64, Ordering::Relaxed);
        self.clause_simplify
            .fetch_add(stats.clause_simplify as u64, Ordering::Relaxed);
        self.adverb_strip
            .fetch_add(stats.adverb_strip as u64, Ordering::Relaxed);
    }

    /// Record response compression savings
    pub fn record_response_compression(&self, tokens_saved: usize) {
        self.response_compressions.fetch_add(1, Ordering::Relaxed);
        self.response_tokens_saved
            .fetch_add(tokens_saved as u64, Ordering::Relaxed);
    }

    /// Export as serializable snapshot for /analytics and dashboard
    pub fn snapshot(&self) -> AnalyticsSnapshot {
        let total_before = self.total_tokens_before.load(Ordering::Relaxed);
        let total_after = self.total_tokens_after.load(Ordering::Relaxed);
        let total_saved = total_before.saturating_sub(total_after);
        let avg_savings = if total_before > 0 {
            total_saved as f64 / total_before as f64 * 100.0
        } else {
            0.0
        };

        let categories = vec![
            CategoryHits {
                name: "filler_phrases".into(),
                hits: self.filler_phrases.load(Ordering::Relaxed),
                tier: "0.2+",
            },
            CategoryHits {
                name: "verbose_phrases".into(),
                hits: self.verbose_phrases.load(Ordering::Relaxed),
                tier: "0.2+",
            },
            CategoryHits {
                name: "imperative_conversions".into(),
                hits: self.imperative_conversions.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "clause_collapse".into(),
                hits: self.clause_collapse.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "developer_boilerplate".into(),
                hits: self.developer_boilerplate.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "semantic_formatting".into(),
                hits: self.semantic_formatting.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "credential_strip".into(),
                hits: self.credential_strip.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "whitespace_cleanup".into(),
                hits: self.whitespace_cleanup.load(Ordering::Relaxed),
                tier: "0.5+",
            },
            CategoryHits {
                name: "conversational_strip".into(),
                hits: self.conversational_strip.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "ai_output_noise".into(),
                hits: self.ai_output_noise.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "markdown_minification".into(),
                hits: self.markdown_minification.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "source_code_compression".into(),
                hits: self.source_code_compression.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "context_deduplication".into(),
                hits: self.context_deduplication.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "anti_noise".into(),
                hits: self.anti_noise.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "disclaimer_collapse".into(),
                hits: self.disclaimer_collapse.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "adjective_collapse".into(),
                hits: self.adjective_collapse.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "clause_simplify".into(),
                hits: self.clause_simplify.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "adverb_strip".into(),
                hits: self.adverb_strip.load(Ordering::Relaxed),
                tier: "0.8+",
            },
            CategoryHits {
                name: "openclaw_rules".into(),
                hits: self.openclaw_rules.load(Ordering::Relaxed),
                tier: "all",
            },
        ];

        AnalyticsSnapshot {
            total_requests: self.total_requests.load(Ordering::Relaxed),
            total_rule_hits: self.total_rule_hits.load(Ordering::Relaxed),
            total_tokens_before: total_before,
            total_tokens_after: total_after,
            total_tokens_saved: total_saved,
            avg_savings_percent: avg_savings,
            response_compressions: self.response_compressions.load(Ordering::Relaxed),
            response_tokens_saved: self.response_tokens_saved.load(Ordering::Relaxed),
            categories,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryHits {
    pub name: String,
    pub hits: u64,
    #[serde(skip)]
    pub tier: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalyticsSnapshot {
    pub total_requests: u64,
    pub total_rule_hits: u64,
    pub total_tokens_before: u64,
    pub total_tokens_after: u64,
    pub total_tokens_saved: u64,
    pub avg_savings_percent: f64,
    pub response_compressions: u64,
    pub response_tokens_saved: u64,
    pub categories: Vec<CategoryHits>,
}

impl AnalyticsSnapshot {
    /// Top N categories sorted by hit count descending
    pub fn top_categories(&self, n: usize) -> Vec<&CategoryHits> {
        let mut sorted: Vec<&CategoryHits> =
            self.categories.iter().filter(|c| c.hits > 0).collect();
        sorted.sort_by(|a, b| b.hits.cmp(&a.hits));
        sorted.into_iter().take(n).collect()
    }
}
