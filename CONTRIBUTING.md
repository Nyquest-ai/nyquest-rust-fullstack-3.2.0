# Contributing to Nyquest

Thanks for your interest in contributing. This document covers local setup, project structure, and how to make changes safely.

## Local Setup

### Prerequisites

- Rust 1.75+ (install via [rustup](https://rustup.rs/))
- Linux or macOS (Windows via WSL2)
- Optional: Ollama + `qwen2.5:1.5b-instruct` for semantic compression testing

### Build

```bash
git clone https://github.com/Nyquest-ai/nyquest-rust-fullstack-3.2.0.git
cd nyquest-rust-fullstack-3.2.0
cp nyquest.example.yaml nyquest.yaml
cargo build --release
```

### Run

```bash
cargo run -- serve           # Start proxy on localhost:5400
cargo run -- preflight -v    # Validate system requirements
cargo run -- doctor          # Quick health check
```

### Test

```bash
cargo test                   # All tests
cargo test -- --nocapture    # With output
```

### Lint

```bash
cargo fmt --check            # Format check
```

### Git hooks

Install once per fresh clone so commits auto-format and run `cargo check`
before landing — this prevents the formatting drift CI rejects.

```bash
bash scripts/install-hooks.sh
```

The hook only fires when staged changes include `.rs` files. Skip with
`git commit --no-verify` if you really need to (CI will still gate on fmt+clippy).

```bash
cargo clippy                 # Lint
```

## Project Structure

```
src/
├── main.rs              # CLI entry point (clap)
├── lib.rs               # Crate root, version constant
├── server.rs            # Axum HTTP server, routes, SSE streaming
├── config.rs            # YAML config loader, env overrides
├── compression/
│   ├── mod.rs           # Compression pipeline entry point
│   ├── engine.rs        # Core compression engine (applies rules to messages)
│   ├── rules.rs         # 350+ compiled regex rules, 18 categories
│   ├── telegraph.rs     # Telegraphic compression mode
│   ├── format.rs        # JSON→CSV/YAML format optimization
│   └── minify.rs        # Code minification (Python, JS, Bash)
├── normalizer.rs        # Prompt normalization (dedup, conflict resolution)
├── context.rs           # Context window optimization
├── semantic.rs          # Semantic LLM compression stage (Ollama)
├── openclaw.rs          # Agent-mode optimizations (tool pruning, sliding window)
├── providers/mod.rs     # Provider detection, request translation
├── security.rs          # Encrypted key vault (AES-256-GCM)
├── analytics.rs         # Per-rule hit tracking
├── profiles.rs          # Model-specific compression profiles
├── cache_reorder.rs     # Message reordering for provider prefix caching
├── tokens.rs            # Token counting and metrics logging
├── stability.rs         # Stability validation mode
├── dashboard.rs         # HTML metrics dashboard
└── cli/                 # CLI subcommands (install, preflight, doctor, config)

extension/               # Chrome extension (browser-side compression, 281 rules)
docs/                    # Architecture docs, benchmarks
tests/                   # Integration tests
```

### What runs in production

Users run the **single `nyquest` binary** via `cargo build --release`. It acts as a transparent compression proxy between any LLM client and upstream providers.

### What is experimental

- `extension/` — Chrome extension (published on Web Store, but separate packaging)
- `site/` — Marketing site assets (not part of engine)
- `src/stability.rs` — Stability validation mode (dev/testing only)

## How to Add Compression Rules

Rules live in `src/compression/rules.rs`. Each rule is a compiled regex with a category tag.

1. Add your regex pattern and replacement to the appropriate category
2. Run `cargo test` to verify no regressions
3. Run the benchmark to check impact: `cargo run --bin benchmark`
4. Include before/after examples in your PR description

### Rules must NEVER modify

- Tool definitions / JSON schemas
- `tool_use` / `tool_result` blocks
- Image content blocks
- Assistant messages (in the compression stage)
- Code blocks within backtick fences
- Model parameters (temperature, max_tokens, etc.)

## How to Add Tests

- Unit tests go in the source file they test (`#[cfg(test)] mod tests {}`)
- Integration tests go in `tests/`
- For compression rule changes, include sample input/output in the test

## Pull Request Process

1. Fork the repository
2. Create a feature branch from `main`
3. Run `cargo fmt`, `cargo clippy`, and `cargo test` before submitting
4. Include benchmark results if changing compression behavior
5. Describe what changed and why in the PR body
6. One approval required for merge

## Code Style

- Follow standard Rust conventions (`cargo fmt`)
- Use `tracing` for logging (not `println!`)
- Keep functions focused — prefer small, testable units
- Document public APIs with doc comments
- No `unwrap()` in request handling paths — use proper error handling

## License

By contributing, you agree that your contributions will be licensed under the MIT OR Apache-2.0 license (your choice).
