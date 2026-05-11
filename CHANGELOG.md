# Changelog

All notable changes to the Nyquest compression engine. The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.2.2] — 2026-05-11

### Added
- **Defensive panic containment around `fancy_regex` calls.** `Rule::replace_all_counted` now wraps the `fancy_regex::Regex::replace_all` call site in `std::panic::catch_unwind`. If a fancy_regex pattern triggers the internal `BacktrackLimitExceeded` panic (an upstream library bug at `fancy_regex-0.14.0/src/lib.rs:1073:45` where `unwrap()` is called on the documented `Err` return), the rule is now skipped for that input and a `warn!` log line is emitted with the rule id and pattern. The pipeline continues with remaining rules instead of unwinding into the tokio worker task. This is defensive insurance: 3.2.1 removed the specific patterns we knew would panic; 3.2.2 contains the *class* of bug.
- Two unit tests in `src/compression/rules.rs`:
  - `catch_unwind_suppresses_fancy_regex_panic_to_no_op`
  - `catch_unwind_path_returns_borrowed_on_panic`

### Changed
- `Cargo.toml` version bumped `3.2.0` → `3.2.2` (skipping `3.2.1` — see note under v3.2.1).
- `src/compression/rules.rs` module docstring rule count corrected `532` → `530` (catches up with the v3.2.1 removal).

### Constraint
- `catch_unwind` only catches unwinding panics. The crate currently uses the default `unwind` strategy in `[profile.release]` (verified: `opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`, no `panic = "abort"`). If `panic = "abort"` is ever added, this wrapper becomes a no-op. A code comment in `rules.rs` notes the dependency.

### Testing
- 50 tests pass (was 48 in v3.2.1; +2 from the new catch_unwind unit tests).
- Verified operationally: dispatcher path chat request served cleanly, container log shows 0 panics and 0 suppressed-panic warnings during the validation window.

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.2` (digest `sha256:12b931a4e104c800416982229b383f1bd5950c6c89084f1396e458169f24c9f7`).
- GHCR Actions run #10 built and published on tag push.

---

## [3.2.1] — 2026-05-11

### Fixed
- **`fancy_regex` `BacktrackLimitExceeded` panic in `CONTEXT_DEDUPLICATION`** (`src/compression/rules.rs`). Two backreference-based sentence-dedup rules with shape `(?s)(\b\w.{40,}?[.!?])\s*\1` and `(\b[A-Z][^.!?]{20,}[.!?])\s+\1` were panicking on long mixed-content prompts (web-grounded chats with URL citations and snippets, agent conversations with accumulated history — anything > ~2000 tokens with sentence-rich prose). Backreferences route to `fancy_regex` (the `regex` crate is non-backtracking and rejects `\1`), and the lazy `.{40,}?` quantifier combined with capture groups on similar-sentence text triggered exponential backtracking. Sentence-level deduplication is correctly the job of the `telegraph` stage, which operates on a sentence-segmented `Vec<String>` rather than via backref regex on raw text, so the two rules were removed entirely. As a bonus, the long-base64 lookaround detector `(?<![a-zA-Z])[A-Za-z0-9+/]{500,}={0,2}(?![a-zA-Z])` was switched from `fancy_regex` lookarounds to a `std::Regex` `\b` variant (same semantic match in practice, no `fancy_regex` code path). Total rule count `532` → `530`.
- **Semantic stage HTTP 401 silent fallback** (`src/semantic.rs`, `src/config.rs`). `SemanticConfig` had no `api_key` field at all — `call_model()` was issuing requests with no `Authorization` header, which worked for the original local-Ollama design but failed against hosted endpoints like OpenRouter. The yaml `semantic_api_key: "${OPENROUTER_KEY}"` was being silently dropped by `serde_yaml` because the field didn't exist on the data model. Added `pub api_key: String` to `SemanticConfig` (default empty — preserves Ollama-no-auth behavior), added `pub semantic_api_key: String` to `NyquestConfig`, wired through `semantic_config()`. `call_model()` now uses `.bearer_auth(...)` when `api_key` is non-empty. Also added env-var override `NYQUEST_SEMANTIC_API_KEY` for production secret management. Verified end-to-end: 6721-token system prompt compresses `7603 → 184` tokens (97.4% savings) via deepseek-v3.2 on OpenRouter.

### Known Issue
- **Cargo.toml version was not bumped for this tag.** The two fix commits were tagged `v3.2.1` but the `[package] version` field in `Cargo.toml` was left at `3.2.0`. Binaries built from the `v3.2.1` tag (and the `ghcr.io/nyquest-ai/nyquest-engine:3.2.1` image) self-report as `3.2.0`. The code at this tag *contains* the v3.2.1 fixes — only the self-reported version is wrong. **Resolved in v3.2.2**, which correctly reports `3.2.2`. The tag has not been retagged because force-pushing tags is messier than the issue it would fix.

### Testing
- 48 tests pass.

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.1` (digest `sha256:0a9d16115a6d95112f95f3f2860fa717f8686532cc0527f1be1ede10cd1a855c`).

---

## [3.2.0] — initial 3.2 line release

Highlights from the source history (see `git log v3.2.0`):

- Five-stage compression pipeline: openclaw rules → category rules (19 categories, 532 rules at release; reduced to 530 in v3.2.1) → telegraph (sentence-level) → semantic (LLM-assisted condensation of long system prompts / conversation histories) → end-of-pipeline whitespace cleanup.
- `Cow<'a, str>` zero-allocation passes — non-matching rules and categories return the original `&str` without allocation.
- Per-category `RegexSet` prefilter — a category is only scanned rule-by-rule if its prefilter signals at least one match.
- Bounded LRU cache (2048 entries / 64KB / sha256-keyed) with ~3,730× warm-replay speedup.
- Auto-detected per-model profile (e.g. `claude-haiku-4.5`, `mini`, `flash`, `nano`, `light` → `conservative`; otherwise `aggressive`). The conservative profile caps level at 0.8, disables adjective/clause/adverb categories, and raises the telegraph threshold.
- Fail-closed length contract — if compression makes output longer than input, return input unchanged.
- Docker container support and GHCR publishing workflow (`.github/workflows/docker-publish.yml`).
- Anthropic Messages API shim (`POST /v1/messages`).

## Versioning policy going forward

- **Every tag bumps `Cargo.toml` version first.** The `v3.2.1` Cargo.toml hygiene issue described above is a one-off and not repeated.
- **Tags are immutable** once pushed. If a tagged release ships with an issue, the fix is shipped under a new tag rather than by retagging.
- **GHCR images are tagged by version** and never overwritten. Each `ghcr.io/nyquest-ai/nyquest-engine:X.Y.Z` is built from exactly the `vX.Y.Z` git tag.