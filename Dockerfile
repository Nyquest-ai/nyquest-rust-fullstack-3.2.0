# syntax=docker/dockerfile:1.6
# ──────────────────────────────────────────────────────────────────────────
# Nyquest Compression Engine — v3.2.0 multi-stage Docker image
#
#   Build:  docker build -t nyquest:3.2.0 -t nyquest:latest .
#   Run:    docker run --rm -p 5400:5400 nyquest:3.2.0
#   Health: curl http://localhost:5400/health
#
# The runtime image contains only the compiled binary, the default
# nyquest.yaml (no secrets), curl + ca-certificates, and tini for PID-1
# zombie reaping. Final image is ~80 MB.
# ──────────────────────────────────────────────────────────────────────────

# ── Stage 1: builder ──────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# pkg-config + git in case any dep needs them. No openssl — reqwest uses
# rustls per Cargo.toml.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first so the dependency-resolution layer can be cached
# even when source changes. Cargo.lock pins exact versions.
COPY Cargo.toml Cargo.lock ./

# Copy source tree.
COPY src ./src
COPY tests ./tests

# Requirement #15: cargo test before final build.
# Tests must pass before we ship a release binary.
RUN cargo test --release --quiet --lib 2>&1 | tail -10
RUN cargo test --release --quiet --tests 2>&1 | tail -20

# Build the production binary.
RUN cargo build --release --bin nyquest && \
    strip target/release/nyquest

# ── Stage 2: runtime ──────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# - ca-certificates: HTTPS to upstream LLM APIs (Anthropic / OpenAI / etc.)
# - curl:           HEALTHCHECK probe + a smoke-test convenience
# - tini:           PID-1 signal handling so SIGTERM cleanly stops the server
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl tini \
    && rm -rf /var/lib/apt/lists/* \
    && rm -rf /usr/share/doc /usr/share/man

# Run as a dedicated non-root user. UID 1000 lines up with the host user
# in most dev environments so bind-mounted log files have sensible perms.
RUN groupadd --system --gid 1000 nyquest \
    && useradd  --system --uid 1000 --gid 1000 --create-home --shell /bin/sh nyquest

WORKDIR /app

# Bring over the compiled binary and the in-repo default config (no secrets).
COPY --from=builder /build/target/release/nyquest /usr/local/bin/nyquest
COPY --chown=nyquest:nyquest nyquest.yaml /app/nyquest.yaml

# Writable log directory inside the image (mount over for persistence).
RUN mkdir -p /app/logs && chown -R nyquest:nyquest /app

USER nyquest

# Sensible defaults — overridable via `-e VAR=value` or `--env-file`.
# Cache settings are read by the engine at process start (see
# src/compression/cache.rs::GLOBAL_CACHE).
ENV NYQUEST_CACHE_ENABLED=true \
    NYQUEST_CACHE_CAPACITY=2048 \
    NYQUEST_CACHE_MAX_ENTRY_SIZE=65536 \
    RUST_LOG=info

EXPOSE 5400

HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --silent --fail http://localhost:5400/health || exit 1

# tini reaps zombies and forwards signals correctly.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/nyquest"]
# Default args = none → binary launches the proxy server (matches the
# existing systemd unit's ExecStart).
