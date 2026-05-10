# Nyquest Compression Engine — Architecture

> **Audience:** engineers reviewing the engine for correctness, performance, or extension.
> **Scope:** `src/compression/*` plus the tightly-coupled `src/profiles.rs` and `src/normalizer.rs`. Provider routing, dashboard, and the semantic LLM stage are out of scope.

---

## 1. Executive Summary

The compression engine is a **pure-function pipeline** that takes a JSON request body (Anthropic Messages API shape, normalized from OpenAI etc. upstream), runs it through a tiered set of transformations, and returns a smaller JSON request body and a `CompressionStats` struct. Every transformation is **opt-in via a level threshold** declared on the active `ModelProfile`, so the same engine produces different output for `claude-opus` vs `claude-haiku`.

The hot path is **regex-driven** with three optimizations layered on top:

1. **Cow-based zero-allocation rule misses** — most rule applications don't match; misses allocate nothing.
2. **`RegexSet` per-category prefilter** — ask the set "which rules can possibly match?" before iterating the rules.
3. **Bounded LRU cache** — agent histories repeat verbatim across turns; the second compression of identical content returns instantly.

Behavior is **fail-closed**: every transformation that could produce a worse-than-input result has an explicit guard that returns the original. Each guard is locked in by a regression test.

```
┌────────────────┐
│ request body   │   Anthropic Messages JSON shape
└────┬───────────┘
     │
     ▼
┌────────────────────────┐
│ normalizer.rs          │   dedup repeated instructions, resolve conflicts,
│                        │   inject speculation boundaries (optional)
└────┬───────────────────┘
     │
     ▼
┌────────────────────────┐
│ engine::compress_request│  iterate messages + system, route by role/type
└────┬───────────────────┘
     │
     ▼
┌────────────────────────┐
│ engine::compress_text   │  per-text-fragment pipeline (the hot path)
│                         │  ──────────────────────────────────────────
│                         │  cache.get() — early return on hit
│                         │  apply_counted(category) × 19 categories
│                         │  rules::compress_dates
│                         │  rules::minify_json_payload   (level ≥ code_minify)
│                         │  minify::minify_code_block    (level ≥ code_minify)
│                         │  format::compact_json_blocks  (level ≥ format_optimize)
│                         │  format::flatten_markdown_tables
│                         │  telegraph::telegraph_compress (level ≥ telegraph)
│                         │  apply_whitespace_normalization (single final pass)
│                         │  fail-closed length contract → original on inflation
│                         │  cache.put()
└────┬───────────────────┘
     │
     ▼
┌────────────────┐
│ smaller body   │  + CompressionStats { total_rule_hits, per-category counters,
│                │                       response_tokens_saved, ... }
└────────────────┘
```

---

## 2. Module Map

| File | Lines | Responsibility |
|---|---|---|
| `compression/mod.rs` | 9 | Re-exports + module declarations |
| `compression/engine.rs` | ~410 | Orchestrator. Pipeline ordering, profile dispatch, JSON walking, cache wiring, fail-closed guards |
| `compression/rules.rs` | ~1170 | 532 regex rules across 19 `RuleCategory` statics + `Rule` / `RuleCategory` types + `apply_category_counted` |
| `compression/cache.rs` | ~210 | Bounded LRU cache (sha256-keyed) for compressed text fragments |
| `compression/telegraph.rs` | ~350 | Sentence-level transforms: preamble strip, tail trim, dedup, imperative merging |
| `compression/minify.rs` | ~270 | Language-aware code-block minification (Python, JS, shell) |
| `compression/format.rs` | ~280 | JSON→YAML/CSV, markdown table flattening, JSON-Schema→TS-interface |
| `profiles.rs` | ~210 | `ModelProfile` struct + `detect_profile(model)` + 3 hardcoded profiles |
| `normalizer.rs` | ~250 | Optional pre-stage: deduplication, conflict resolution, speculation boundaries |

Total: ~3,200 LOC engine + ~210 LOC profiles + ~250 LOC normalizer = **~3,660 LOC**.

---

## 3. Data Model

### 3.1 `Rule`

```rust
pub struct Rule {
    pub id: String,                          // "category.NNN" — stable id for telemetry
    pub pattern_str: String,                 // regex source (for /admin endpoint)
    pub std_pattern: Option<Regex>,          // Some when the rule is std::regex
    pub fancy_pattern: Option<fancy_regex::Regex>,  // Some when lookaround/backref needed
    pub replacement: String,
    pub hits: AtomicUsize,                   // per-process cumulative hit count
}
```

A rule carries **either** a `std_pattern` **or** a `fancy_pattern`, never both — the constructor falls back to `fancy_regex` only when `regex` rejects the pattern (lookaround / backreferences).

The `hits` counter is incremented in `replace_all_counted` whenever the rule matches. It is process-cumulative; reset on restart.

### 3.2 `RuleCategory`

```rust
pub struct RuleCategory {
    pub name: &'static str,                  // "filler_phrases", "verbose_phrases", ...
    pub rules: Vec<Rule>,
    pub set: RegexSet,                       // prefilter over std_pattern rules only
    pub set_to_rule: Vec<usize>,             // map: set match index → rules[] index
    pub category_hits: AtomicUsize,          // sum of hits across this category, this process
}
```

The `set` is the **performance lever**. Before iterating `rules`, `apply_category_counted` calls `set.matches(text)` and skips the std-regex rules whose index isn't in the result. Fancy-regex rules can't go in a `RegexSet` (regex crate limitation) and always run.

`RuleCategory::new(name, rules)` assigns each rule's `id` to `"{name}.{i:03}"`. Rule IDs are therefore **stable across builds** for the same source ordering, which the per-rule telemetry endpoint relies on.

### 3.3 `ModelProfile`

```rust
pub struct ModelProfile {
    pub name: &'static str,                  // "aggressive" | "balanced" | "conservative"
    pub filler_phrases:        f64,          // minimum compression level for this category to fire
    pub verbose_phrases:       f64,
    pub imperative_conversions: f64,
    pub clause_collapse:       f64,
    // ... one f64 per category, plus telegraph_intensity multiplier
}
```

A profile is a **threshold table**, one f64 per rule category. The engine fires a category iff `compression_level >= profile.<category>`. Setting a threshold to `1.1` effectively disables the category (max level is 1.0).

The conservative profile disables `adjective_collapse`, `clause_simplify`, `adverb_strip` (set to 1.1) — these are the rule families most likely to drift meaning on smaller models.

### 3.4 `CompressionEngine`

```rust
pub struct CompressionEngine {
    pub level: f64,                          // 0.0 = pass-through, 1.0 = max
    pub stats: CompressionStats,             // mutable per-request counters
    pub profile: &'static ModelProfile,      // pinned at construction time
}
```

The engine is **per-request and short-lived** (constructed at the top of `compress_request`, dropped after). The profile is a static reference; profiles are interned at process start.

---

## 4. The Hot Path — `compress_text`

```rust
pub fn compress_text(&mut self, text: &str) -> String {
    if self.level == 0.0 { return text.to_string(); }

    // Cache probe — early return on hit
    if let Some((cached, hits)) = GLOBAL_CACHE.get(
        text, self.level, self.profile.name, Mode::Request,
    ) {
        self.stats.total_rule_hits += hits;
        return cached;
    }

    let mut result = text.to_string();
    let p = self.profile;

    // Tier 0: always-on OpenClaw metadata strip
    result = self.apply_counted(&result, &OPENCLAW_RULES, "openclaw_rules");

    // Tier 1: filler removal & normalization (level ≥ 0.2)
    if self.level >= p.filler_phrases {
        result = rules::apply_whitespace_normalization(&result);
        result = self.apply_counted(&result, &FILLER_PHRASES, "filler_phrases");
    }
    if self.level >= p.verbose_phrases {
        result = self.apply_counted(&result, &VERBOSE_PHRASES, "verbose_phrases");
    }

    // Tier 2: structural compression (level ≥ 0.5)
    //  imperative_conversions, clause_collapse, developer_boilerplate,
    //  semantic_formatting (incl. compress_dates), credential_strip,
    //  whitespace_cleanup
    // ...

    // Tier 3: aggressive canonical compression (level ≥ 0.8)
    //  conversational_strip, ai_output_noise, markdown_minification,
    //  source_code_compression (gated by is_code_or_json_content!),
    //  context_deduplication, anti_noise, disclaimer_collapse,
    //  adjective_collapse, clause_simplify, adverb_strip
    // ...

    // code_minify + format_optimize (level ≥ 0.8 default)
    if self.level >= p.code_minify {
        result = rules::minify_json_payload(&result);
        result = super::minify::minify_code_block(&result);
    }
    if self.level >= p.format_optimize {
        result = format::compact_json_blocks(&result);
        result = format::flatten_markdown_tables(&result);
    }

    // telegraph (sentence-level rewrites)
    if self.level >= p.telegraph {
        let effective = p.effective_telegraph_level(self.level);
        result = telegraph::telegraph_compress(&result, effective);
    }

    // Single end-of-pipeline cleanup
    result = rules::apply_whitespace_normalization(&result);

    // Fail-closed length contract
    if result.len() > text.len() { return text.to_string(); }

    // Cache store
    GLOBAL_CACHE.put(
        text, self.level, self.profile.name, Mode::Request,
        result.clone(), self.stats.total_rule_hits,
    );

    result
}
```

Three load-bearing details:

1. **Cache probe at the top + cache store at the bottom** — opt-in via `GLOBAL_CACHE.set_enabled(false)` if you need to bypass.
2. **`is_code_or_json_content` gate** on `SOURCE_CODE_COMPRESSION` — the rules in that category mutate JSON-style whitespace and strip `#`/`//` comment lines. On prose they would corrupt markdown headings, numeric prose, and Python tuples. The gate fires only when the content is fenced code, parseable JSON, or `minify::detect_language` returns `Some`.
3. **`apply_whitespace_normalization` as a single final pass** — runs once at the end of the pipeline, not after every category.

`compress_response_text` follows the same shape but with a lighter rule mix (assistant responses).

---

## 5. Per-Category Rule Application

```rust
pub fn apply_category_counted<'a>(
    text: &'a str,
    category: &RuleCategory,
) -> (Cow<'a, str>, usize) {
    // 1. Ask the RegexSet which std-regex rules COULD match.
    let matched: HashSet<usize> = category.set.matches(text).into_iter().collect();

    // 2. Build a should_run mask: std rules only run if their set index hit;
    //    fancy rules always run.
    let mut should_run = vec![true; category.rules.len()];
    for (set_idx, rule_idx) in category.set_to_rule.iter().enumerate() {
        should_run[*rule_idx] = matched.contains(&set_idx);
    }

    // 3. Iterate. After the first owned mutation, prefilter-skipping is
    //    disabled (the text changed; rules that didn't match the *initial*
    //    text might now match). Conservative: re-run remaining candidates.
    let mut current: Cow<'a, str> = Cow::Borrowed(text);
    let mut hits = 0;
    let mut mutated = false;
    for (i, rule) in category.rules.iter().enumerate() {
        if !mutated && !should_run[i] { continue; }
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
```

Two things to note:

- **The `Cow<'a, str>` thread**: misses cost zero allocations. Hits cost one `String` per rule that matched (unavoidable — `regex::replace_all` returns owned).
- **Why `mutated` toggles off the prefilter**: once any rule has rewritten the text, the original `set.matches(text)` result no longer reflects what's actually in the buffer. Rather than re-running the prefilter on every rewrite (expensive), we conservatively run all *candidate* rules from there on. In practice the candidate set is small.

---

## 6. Per-Rule Telemetry

Every `Rule` carries an `AtomicUsize hits` field. Every call to `replace_all_counted` increments it on a match (relaxed ordering — we don't need any synchronization beyond atomicity).

The data is exposed via two HTTP endpoints (`src/server.rs`):

### `GET /admin/v2/compression/rules`

```json
{
  "status": "ok",
  "version": "3.2.0",
  "categories": [
    {
      "name": "filler_phrases",
      "category_hits": 1234,
      "rules": [
        { "id": "filler_phrases.000", "pattern": "\\bplease\\s+note\\s+that\\b", "hits": 412 }
      ]
    }
  ]
}
```

### `GET /admin/v2/compression/cache`

```json
{
  "status": "ok",
  "cache": {
    "entries": 61,
    "capacity": 2048,
    "hits": 200,
    "misses": 1,
    "evictions": 0,
    "stores": 1,
    "enabled": true
  }
}
```

**No prompt content is exposed by either endpoint.** The pattern source and the rule ID are static, hit counts are aggregates.

The intended uses:

- **Identify dead rules** (zero hits across millions of requests → candidate for removal).
- **Identify over-firing rules** (disproportionate hits for low expected coverage → suggests an unintended match pattern).
- **Tune profile thresholds** by category.
- **Verify the cache is doing its job** (hits ≫ misses on agent traffic).

---

## 7. Bounded LRU Cache

Source: `compression/cache.rs`.

### Key

```text
sha256(content_bytes)  ‖  mode_byte  ‖  profile_name  ‖  level_le_bytes
                                                  ↓
                                          [u8; 32]
```

`mode_byte` is `b'q'` for request prompts and `b's'` for response (assistant) prompts. This prevents collision when the same text appears in both contexts under different rule pipelines.

### Storage

`lru::LruCache<CacheKey, Entry>` behind `std::sync::Mutex`. Default capacity 2048 entries, 64KB max entry size. Both tunable via `set_max_entry_size` / `CompressionCache::new(capacity)` or via env vars (`NYQUEST_CACHE_CAPACITY`, `NYQUEST_CACHE_MAX_ENTRY_SIZE`, `NYQUEST_CACHE_ENABLED`).

### Eviction detection

`LruCache::push` returns the displaced (key, value) on eviction OR the prior value if the same key is overwritten. We distinguish by comparing the returned key against the inserted key — only a different key indicates a true LRU eviction. The eviction counter only increments on the former.

### What's never cached

- Empty content
- Content larger than `max_entry_size` (default 64KB)
- When `enabled() == false` (set via `set_enabled(false)`)

The engine's `compress_content_block` already routes `tool_use` and `image` blocks around `compress_text` entirely — they never reach the cache because they never reach the rule pipeline.

### Performance

Measured: cold = ~71 ms, warm = ~19 µs → **3,730× speedup** on cache hit (synthetic 200-replay test). Real-world agent traffic where ≥30% of message bodies repeat verbatim sees most of this benefit.

---

## 8. Safety Guarantees (and where each lives)

| Guarantee | Enforced in | Test |
|---|---|---|
| `tool_use` blocks pass through byte-identical | `engine::compress_content_block` early return | `tool_use_blocks_pass_through_unchanged` |
| `image` blocks pass through byte-identical | same | `image_blocks_pass_through_unchanged` |
| Output is never longer than input | `compress_text` & `compress_response_text` final guard | `compression_never_inflates_output` |
| Four-digit years preserved in dates | `rules::compress_dates` returns full year | `preserves_four_digit_years`, `date_compression_handles_century_boundaries` |
| Role declarations never deleted | `telegraph::strip_preamble` rewrites to `"Role: X."` + empty-output guard | `preserves_role_declaration`, `long_role_only_prompt_not_emptied` |
| Source-code rules don't mutate prose | `engine::is_code_or_json_content` gate | `source_code_rules_do_not_fire_on_prose`, `markdown_heading_is_not_stripped_by_source_rules` |
| Profile detection is specific-first | `profiles::detect_profile` checks small markers before family names | `profile_detection_specific_models_win_first`, `mini_suffix_overrides_family_name` |
| Non-English text unaffected | rules are word-boundary anchored to ASCII | `non_english_text_is_not_corrupted` |
| Same input → same output | engine is pure modulo cache state | `repeated_compression_is_deterministic` |

All 9 are locked in by `tests/compression_engine_regression.rs`. Add a guarantee → add a test in the same file.

---

## 9. Profile System

Three hard-coded profiles (`profiles.rs`):

| Category threshold | aggressive | balanced | conservative |
|---|---|---|---|
| filler_phrases | 0.2 | 0.2 | 0.2 |
| verbose_phrases | 0.2 | 0.2 | 0.3 |
| imperative_conversions | 0.5 | 0.5 | 0.6 |
| clause_collapse | 0.5 | 0.5 | 0.7 |
| developer_boilerplate | 0.5 | 0.5 | 0.5 |
| semantic_formatting | 0.5 | 0.5 | 0.6 |
| credential_strip | 0.5 | 0.5 | 0.5 |
| whitespace_cleanup | 0.5 | 0.5 | 0.5 |
| conversational_strip | 0.8 | 0.8 | 0.9 |
| ai_output_noise | 0.8 | 0.8 | 0.8 |
| markdown_minification | 0.8 | 0.8 | 0.9 |
| source_code_compression | 0.8 | 0.8 | 0.9 |
| context_deduplication | 0.8 | 0.8 | 0.8 |
| anti_noise | 0.8 | 0.8 | 0.9 |
| disclaimer_collapse | 0.8 | 0.8 | 0.9 |
| **adjective_collapse** | **0.8** | **0.9** | **1.1** ← disabled |
| **clause_simplify** | **0.8** | **0.9** | **1.1** ← disabled |
| **adverb_strip** | **0.8** | **0.9** | **1.1** ← disabled |
| telegraph | 0.5 | 0.6 | 0.8 |
| code_minify | 0.8 | 0.8 | 0.9 |
| format_optimize | 0.8 | 0.8 | 0.9 |
| telegraph_intensity (multiplier) | 1.0 | 0.85 | 0.6 |

Detection order in `detect_profile(model)`:

1. **Tier 1 — small markers always win**: `mini`, `haiku`, `flash`, `nano`, `light`, `-lite` → conservative.
2. **Tier 2 — explicit conservative IDs**: `gpt-3.5`, `command-r-light`, `llama-3.1-8b`, `llama-3.2`, `mistral-7b`, `mixtral-8x7b`, `phi-`, `qwen-2.5-{7b,14b}`.
3. **Tier 3 — aggressive premium IDs**: `opus`, `sonnet`, `gpt-4o`, `gpt-4-turbo`, `grok-3`, `gemini-{2.5,1.5}-pro`, `command-r-plus`, `llama-3.{1-405b,3-70b}`, `deepseek-v3`, `qwen-2.5-72b`.
4. **Default**: balanced.

The Tier 1 check is critical: a hypothetical model name like `claude-sonnet-4-6-mini` would otherwise match aggressive on the `sonnet` substring before the `mini` check ever ran.

---

## 10. Performance Characteristics

Measured on the test corpus:

| Metric | Value |
|---|---|
| Cold compression (~500-char prompt) | ~70 ms |
| Warm compression (cache hit) | ~19 µs |
| Cache speedup factor | ~3,730× |
| Aggregate token reduction (10-scenario corpus, L1.0) | 24% |
| Per-rule allocation cost on miss | **0** (Cow::Borrowed) |
| Rules fired per text fragment, average | ~6 of 532 |
| Categories with std-regex prefilter coverage | 19 / 19 |

Memory overhead on top of the v3.0-era engine baseline:

- `Rule`: +56 bytes (id String header, pattern_str String header, AtomicUsize)
- `RuleCategory`: ~10KB per category for the RegexSet (compiled DFA)
- 19 categories × ~10KB = ~190KB compiled prefilter DFA, one-time at process start
- Cache: 2048 × (~32 bytes key + entry size) ≈ ~150KB at peak

Total: under 400KB additional process memory.

---

## 11. Extension Points

| Want to | Touch | Notes |
|---|---|---|
| Add a new compression rule to an existing category | `rules.rs` static block for that category | Stable IDs come from list ordering — appending preserves earlier IDs |
| Add a new rule category | `rules.rs` (new `Lazy<RuleCategory>` static) + `engine.rs` (new threshold field on profile + new `apply_counted` call site) + `profiles.rs` (threshold defaults) + `server.rs` (add to telemetry endpoint list) |
| Add a new model profile | `profiles.rs` (new `const PROFILE_X` + match in `get_profile` + match in `detect_profile`) |
| Tune existing profile thresholds | `profiles.rs` only |
| Cache-bypass a specific code path | call `GLOBAL_CACHE.set_enabled(false)` around the section, restore after |
| Add a new safety invariant | `tests/compression_engine_regression.rs` (failing test first), then enforce |

---

## Appendix: Full file walk

```
src/
├── compression/
│   ├── mod.rs               re-exports
│   ├── engine.rs            orchestrator + JSON walking + cache wiring + length contract
│   ├── rules.rs             Rule, RuleCategory, 19 categories (532 rules), apply_category_counted,
│   │                        apply_whitespace_normalization, compress_dates, minify_json_payload
│   ├── cache.rs             bounded LRU compression cache (sha256-keyed, mode-aware)
│   ├── telegraph.rs         sentence-level transforms (preamble, tail, dedup, imperative merge)
│   ├── minify.rs            Python/JS/shell code-block minifier (no AST, state-machine)
│   └── format.rs            JSON→YAML/CSV, markdown table flatten, JSON-Schema→TS interface
├── profiles.rs              ModelProfile struct + 3 profiles + detect_profile
├── normalizer.rs            optional pre-stage: dedup + conflict resolution + speculation boundaries
└── server.rs                axum router; /admin/v2/compression/{rules,cache} endpoints live here
```

End of architecture.
