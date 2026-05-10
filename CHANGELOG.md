# Changelog

All notable changes to this project are documented in this file.

This project follows [Semantic Versioning](https://semver.org/).

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
