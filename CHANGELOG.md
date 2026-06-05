# Changelog

All notable changes to the Nyquest compression engine. The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [3.2.8] — 2026-06-04

### Added
- **`POST /v2/compress` — compress-only endpoint.** Runs the full compression pipeline (`run_pipeline`: auto-scale → context optimizer → OpenClaw → semantic) and returns `{messages, model, original_tokens, optimized_tokens}` **without forwarding upstream to any model**. Powers the Nyquest Splicer cost-moat: compress a prompt once, then fan the optimized prompt out to N models, so input cost drops from `N × original` to `(one compression) + N × optimized`. Honors the same `x-nyquest-*` tuning headers as the proxy endpoints. `src/server.rs::compress_only` + route registration.

### Unchanged
- Pure addition — `/v1/chat/completions` and `/v1/messages` proxy behavior, the compression pipeline, and the 532-rule set are byte-identical to 3.2.7.

### Testing
- In-build cargo test suite green (lib + integration). Container boots; `/health` reports `3.2.8`; `/v2/compress` verified end-to-end (e.g. 1108 → 988 tokens on a filler-heavy prompt).

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.8` (+ `3.2`, `3`, `latest`), auto-published by the `docker-publish.yml` workflow once the GHCR package was granted Write access to the repo.
- `engine/start.sh` default IMAGE bumped 3.2.7 → 3.2.8.

---

## [3.2.7] — 2026-05-29

### Fixed
- **fancy-regex `BacktrackLimitExceeded` panic eliminated at the source.** `src/compression/rules.rs` now calls `re.try_replacen(text, 0, replacement)` directly; on `Err` it logs and returns `(Cow::Borrowed(text), false)` so the rule no-ops for that input and the pipeline continues with the remaining rules. Replaces the prior `catch_unwind` containment, which couldn't work under `panic = "abort"`. Same user-visible behavior on pathological input — the panic path simply no longer exists. Closes the 2026-05-20 panic-risk item with no wait on an upstream fancy-regex fix.

### Testing
- Full in-build cargo test suite green; `/health` reports `3.2.7` on engine `:5400` and the backend's forwarded probe.

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.7` + `:latest` (digest `sha256:f9e27f532c02f535f66a46a5ace26909edd63ed453ee4ab1d686ab5963268328`).
- `engine/start.sh` default IMAGE bumped 3.2.6 → 3.2.7.

---

## [3.2.6] — 2026-05-20

### Added
- **Workspace and user header threading from proxy into semantic cache.** `src/server.rs::proxy_messages` and `proxy_chat_completions` now extract `X-Nyquest-Workspace-Id` and `X-Nyquest-User-Id` from incoming request headers and thread them into `run_pipeline` (two new params), which forwards them into both semantic-stage condense paths. Closes the cache-isolation gap that v3.2.5 set up but couldn't exercise (callers were still passing `(None, None)`).
- `src/semantic.rs::condense_history` gains `(workspace_id, user_id) -> cache_key` parameters.
- `src/semantic.rs::condense_system` folds `(workspace_id, user_id)` into its `sys:` cache key. The key is now `sha256(normalized_system_text + workspace + user)` — matches the history-cache discipline introduced in v3.2.5.

### Effect
- Two users (or two workspaces) can no longer share each other's semantic-stage condensations.
- Managed and anonymous traffic without the headers falls back to the shared global cache, same as before (empty values = no isolation, same behavior as v3.2.5).

### Testing
- 30 / 30 pass (post-rename of older test counters).

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.6` (digest `sha256:3e66b6cb89d24264df83dcb49b3179f8febbda0fb8de9c324846e0f14df30941`).
- `:latest` re-pointed.

---

## [3.2.5] — 2026-05-19

### Changed — semantic-stage cache improvements (Sprint 2 Task 8)

Three changes shipped together:

1. **Cache key: `md5(raw json)` → `sha256(normalized + workspace + user)`.** The old cache key was an `md5` over `messages.to_string().join("|")` — case-sensitive *and* whitespace-sensitive. So `"What's my name?"`, `"what is my name"`, and `"  whats my name  "` all produced different `md5` keys and missed the cache. New normalization: lowercase every char, collapse runs of whitespace to a single space, append unit-separator (`\x1F`)-delimited `workspace_id + user_id` at the tail, then `sha256` over the result. New signature: `cache_key(messages, workspace_id: Option<&str>, user_id: Option<&str>) -> String`. Current callers pass `(None, None)` — backwards-compatible. Full header plumbing is the v3.2.6 follow-up.

2. **`CacheEntry` gains `original_latency_ms`.** Every cache *miss* records the latency it paid. Every subsequent *hit* adds that to a new running counter — answers "how much semantic-stage time did we save in the last hour?"

3. **`SemanticStats` gains `cache_savings_ms: f64`.**

### Dependency
- `+ sha2 = "0.10"` added to `Cargo.toml`.
- `md5` retained for the legacy `cache_key` signature (unused now, kept to avoid a cascading dep cleanup inside the same commit).

### Verified post-deploy
- Container reports `version: "3.2.5"` via `/health`.
- BYOK smoke: 3 / 3 PASS (mock, openrouter, generic; 4 SKIP without keys).

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.5` (digest `sha256:1c1228342926738f8e3ced8ca8928d2a28491eae512835328db5d6008b1e817a`).
- `:latest` re-pointed.

---

## [3.2.4] — 2026-05-19

### Added — strict config loader (Sprint 1 Task 6)

Engine YAML previously silently dropped unknown keys, so a typo like `semantic_history_threshhold` (extra `h`) loaded the default value and the engine ran without anyone knowing. Two changes, one outcome — startup now LOUDLY fails on bad YAML instead of silently defaulting.

- **`src/config.rs::NyquestConfig` now derives `#[serde(default, deny_unknown_fields)]`.** `default` still provides per-field defaults for *missing* keys (minimal YAML still loads). *Extra* keys now trigger:
  ```
  unknown field `semantic_history_threshhold`, expected one of
    `compression_level`, `adaptive_mode`, `semantic_validation`, ... <full field list>
  ```

- **`load_config` no longer silently falls back to `default()`.** The old `serde_yaml::from_str(&content).unwrap_or_default()` swallowed every parse failure — typos in numeric fields, wrong types, malformed structures, AND the new `deny_unknown_fields` rejections. Replaced with an explicit match that prints `FATAL: failed to parse config <path>: <serde error>`, prints a help line pointing to `NyquestConfig` in `src/config.rs`, then `std::process::exit(2)`. Same treatment for file-read errors.

### Verified
- Container run with typo'd YAML exits with the descriptive FATAL line above.
- Canonical good YAML boots normally — container reports `version: "3.2.4"`, `/health` returns ok, BYOK smoke 3 / 3 PASS.

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.4` (digest `sha256:05f0f9aaa78568beedb6135fc134baa46e4ad8f5e0e24cf194b4cf463574e87c`).
- `:latest` re-pointed.

---

## [3.2.3] — 2026-05-18

### Added — compression telemetry on `/v1/chat/completions`

The `/v1/messages` handler has emitted `x-nyquest-original-tokens`, `x-nyquest-optimized-tokens`, and `x-nyquest-savings-percent` on every response since v3.2.0. The `/v1/chat/completions` equivalent only emitted `x-nyquest-request-id`. That meant any consumer using the OpenAI-format endpoint — which is the bulk of upstream traffic, and all of the BYOK-through-engine traffic from `app-nyquest` — had no way to read the compression result off the response.

- Adds the same four telemetry headers to all six `Response::builder()` sites in `src/server.rs::proxy_chat_completions`: success body, streaming start, two retry paths on conversion failures, and the two upstream-error pass-throughs.
- `original_tokens` / `optimized_tokens` were already computed earlier in the function. This is a pure header-emission change.

### Verified
- Deployed as `nyquest:3.2.3-local` on prod; `curl` confirms all four telemetry headers now appear on chat-completions responses.

### Deployment
- Image `ghcr.io/nyquest-ai/nyquest-engine:3.2.3` (digest `sha256:10b6e44b54ba71321d82c706239680a9b48021bc4e6ab1339cfead44f160a5e9`).

---
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

## [3.2.0] — 2026-05-10

### Compression engine

- 532 regex rules across 19 categories applied in tiered passes (filler removal, structural compression, aggressive canonicalization).
- `RuleCategory` wrapper around each rule set with a `regex::RegexSet` prefilter — only candidate rules run on a given text fragment.
- `Rule::replace_all_counted` returns `Cow<'a, str>`; rules that don't match return `Cow::Borrowed(text)` and allocate nothing.
- Whitespace normalization runs once at the end of `compress_text` instead of per category.
- `compress_dates` preserves four-digit years (`1999-01-14`, `2099-01-14` — no century ambiguity).
- `SOURCE_CODE_COMPRESSION` rules gated to fenced code blocks, parseable JSON, or content where `minify::detect_language` returns a hit. Markdown headings, numeric prose, Python tuples, and semicolon-separated clauses are not modified.
- Telegraph compression rewrites role declarations as `Role: X.` instead of stripping them; single-sentence prompts go through a dedicated path that cannot empty them.
- Fail-closed length contract: `compress_text` returns the original input if compression produced a longer string.

### Profile system

- `detect_profile(model)` uses specific-marker matching first: `mini` / `haiku` / `flash` / `nano` / `light` / `-lite` markers map to the conservative profile regardless of family-name substrings.
- Three hard-coded profiles (`aggressive` / `balanced` / `conservative`) define the per-category compression-level threshold table.

### Bounded LRU compression cache

- New `compression::cache` module backed by `lru::LruCache` behind a `Mutex`.
- Key: `sha256(content) + level + profile + mode` (request vs response).
- Default capacity 2048 entries, 64KB max entry size — both tunable via env vars (`NYQUEST_CACHE_CAPACITY`, `NYQUEST_CACHE_MAX_ENTRY_SIZE`, `NYQUEST_CACHE_ENABLED`).
- Atomic counters expose hits / misses / evictions / stores.
- Skips `tool_use` blocks, `image` blocks, and oversized content.
- ~3,730× speedup on warm replay measured against a ~500-char prompt.

### Per-rule telemetry

- Every `Rule` carries an `AtomicUsize` hit counter and a stable `category.NNN` id.
- Each `RuleCategory` carries an `AtomicUsize` aggregate hit counter.

### HTTP endpoints

- `GET /admin/v2/compression/rules` — per-category and per-rule hit counts plus the rule's regex source (no prompt content exposed).
- `GET /admin/v2/compression/cache` — cache snapshot (entries, capacity, hits, misses, evictions, stores, enabled).

### Tests

- `tests/compression_engine_regression.rs` — 16 snapshot tests for safety invariants: Python tuple syntax, four-digit dates, role declarations, profile-detection ordering, `tool_use` / `image` byte-identical pass-through, non-English text preservation, deterministic re-compression, fail-closed length contract.
- `tests/v320_compression_report.rs` — 6 measurement tests that double as a runtime report (compression by level, cache hit rate, top-firing rules, profile-aware compression, length contract verification, safety invariants on realistic payloads).
- `tests/role_based_test.rs` — 7 aggregate-savings tests across 25 personas in 7 categories.
- 47 / 47 tests passing.

### Tooling

- `scripts/install-hooks.sh` installs a tracked pre-commit hook that auto-runs `cargo fmt` + `cargo check` on commits that touch `.rs` files.
- `scripts/build.sh` / `scripts/run-local.sh` / `scripts/smoke-test.sh` for Docker workflows.
- `scripts/benchmark.sh` / `scripts/benchmark-semantic.sh` / `scripts/integration-test.sh` for measurement and integration testing.

### Distribution

- Multi-stage `Dockerfile` (Rust builder + Debian-slim runtime, non-root user, tini PID-1, ~146 MB image).
- `docker-compose.yml` with `network_mode: host` for full throughput; `--profile prod` for read-only root FS, restart=always, and resource caps.
- `.github/workflows/docker-publish.yml` auto-publishes to GHCR (`ghcr.io/nyquest-ai/nyquest-engine`) on every `v*` tag with semver tag expansion (`3.2.0`, `3.2`, `3`, `latest`).

---

## Versioning policy going forward

- **Every tag bumps `Cargo.toml` version first.** The `v3.2.1` Cargo.toml hygiene issue described above is a one-off and not repeated.
- **Tags are immutable** once pushed. If a tagged release ships with an issue, the fix is shipped under a new tag rather than by retagging.
- **GHCR images are tagged by version** and never overwritten. Each `ghcr.io/nyquest-ai/nyquest-engine:X.Y.Z` is built from exactly the `vX.Y.Z` git tag.
