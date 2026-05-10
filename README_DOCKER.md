# Nyquest Engine — Docker Deployment

Container support for **Nyquest v3.2.0**. Multi-stage build, Debian-slim
runtime, non-root user, `/health` healthcheck baked in.

---

## Quick start

### Option A — pull the prebuilt image (recommended)

```bash
docker pull ghcr.io/nyquest-ai/nyquest-engine:3.2.0

docker run -d --name nyquest \
    --network host \
    -v "$(pwd)/logs:/app/logs" \
    ghcr.io/nyquest-ai/nyquest-engine:3.2.0

bash scripts/smoke-test.sh    # verify
```

Available tags (auto-published from git tags by
[`.github/workflows/docker-publish.yml`](.github/workflows/docker-publish.yml)):

* `3.2.0` — pinned exact version
* `3.2`   — latest 3.2.x
* `3`     — latest 3.x.x
* `latest` — latest stable release

### Option B — build locally from source

```bash
# 1. Build the image (runs cargo test inside the builder, then compiles)
bash scripts/build.sh

# 2. Bring it up locally (foreground, Ctrl-C to stop)
bash scripts/run-local.sh

# 3. In another shell — verify everything is responding
bash scripts/smoke-test.sh
```

You should see:

```
Nyquest smoke test against http://127.0.0.1:5400
──────────────────────────────────────────────────────────────
  ✓  /health reports v3.2.0
  ✓  /metrics returns 200
  ✓  /admin/v2/compression/cache             capacity=2048
  ✓  /admin/v2/compression/rules             19 categories
  ✓  POST /v1/messages compresses            saved 22.6% on this prompt
──────────────────────────────────────────────────────────────
5 passed, 0 failed
```

---

## What's in the image

| Layer | Content |
|---|---|
| `rust:1-slim-bookworm` (builder) | Compiles the binary, runs `cargo test` first (req #15) |
| `debian:bookworm-slim` (runtime) | `/usr/local/bin/nyquest` (stripped), `/app/nyquest.yaml` default config, `curl` for healthcheck, `tini` for PID-1 zombie reaping |
| User | non-root `nyquest:nyquest` (UID 1000) |
| Workdir | `/app` |
| Exposed port | `5400` |
| Logs | `/app/logs/` (mount over for persistence) |
| Healthcheck | `curl --fail http://localhost:5400/health` every 30s |

Final image size: ~146 MB (multi-arch manifest; ~90 MB per arch).

**No secrets in the image.** The shipped `nyquest.yaml` has no API keys.
Provide them via `--env-file` / `-e VAR=value` at runtime, never via build
args.

---

## Build

```bash
# Standard build — tags both nyquest:3.2.0 and nyquest:latest
bash scripts/build.sh

# Force a clean rebuild (ignore Docker cache)
bash scripts/build.sh --no-cache

# Custom tag
IMAGE_TAG=ghcr.io/nyquest-ai/nyquest bash scripts/build.sh
```

`scripts/build.sh` reads the `version` field from `Cargo.toml` so the tag
always matches the source.

The build runs `cargo test --release` *inside the container* before
producing the binary. A failing test fails `docker build` — by design.

---

## Run

### Foreground (development)

```bash
bash scripts/run-local.sh
# server is on http://localhost:5400
# Ctrl-C cleanly shuts down (tini forwards SIGTERM)
```

### Detached

```bash
bash scripts/run-local.sh -d
docker compose logs -f nyquest
```

### Production-style

```bash
docker compose --profile prod up -d
# Pinned image tag, restart=always, read-only root FS, 256 MB memory cap,
# 1 CPU limit. /app/logs is the only writable mount.
```

### Plain `docker run` (no compose)

```bash
docker run --rm -d \
    --name nyquest \
    --network host \
    -e NYQUEST_CACHE_CAPACITY=4096 \
    -e RUST_LOG=info \
    -v "$(pwd)/logs:/app/logs" \
    nyquest:3.2.0
```

With `--network host`, the container shares the host's network namespace
— no port mapping needed and no `docker-proxy` overhead. Throughput
recovers to systemd parity (see the next section).

If you specifically need the bridge network (e.g., to put nyquest on a
docker-internal network with other services), drop `--network host` and
add `-p 5400:5400`. Compression results are identical; only request-path
latency / throughput on tiny endpoints (like `/health`) changes.

---

## Configuration

### Environment variables (cache + logging)

Read at process start. Copy `.env.example` to `.env` and edit, or pass with
`-e`:

| Var | Default | Purpose |
|---|---|---|
| `NYQUEST_CACHE_ENABLED` | `true` | Master switch for the bounded LRU compression cache. Accepts `1`/`true`/`yes`/`on` (case-insensitive). |
| `NYQUEST_CACHE_CAPACITY` | `2048` | Max cached entries. Must be > 0; falls back to default otherwise. |
| `NYQUEST_CACHE_MAX_ENTRY_SIZE` | `65536` | Refuse to cache content larger than this many bytes. |
| `RUST_LOG` | `info` | `tracing-subscriber` directive. e.g. `nyquest=trace,info` |

These are read by `src/compression/cache.rs::GLOBAL_CACHE` the first time
the cache is touched. Unset → current behavior (matches the systemd
deployment exactly).

### YAML (everything else)

The image ships `/app/nyquest.yaml` with the in-repo defaults
(compression level, normalize, openclaw, response compression, semantic
stage settings, etc.). Override by mounting your own:

```yaml
# docker-compose.yml fragment
volumes:
  - ./my-nyquest.yaml:/app/nyquest.yaml:ro
```

Or `docker run -v $(pwd)/my-nyquest.yaml:/app/nyquest.yaml:ro …`.

The default config binds to `0.0.0.0:5400` so the port mapping just works.

---

## Logs

```bash
# Stream container logs (compose)
docker compose logs -f nyquest

# Tail the metrics JSONL (host-side, since logs/ is mounted)
tail -f logs/nyquest_metrics.jsonl

# One-off inside the container
docker exec -it nyquest-engine sh
```

The metrics path (`logs/nyquest_metrics.jsonl`) inside the container maps
to `./logs/nyquest_metrics.jsonl` on the host via the volume mount, so
metrics survive container restarts.

---

## Restart / update / rollback

```bash
# Restart in place
docker compose restart nyquest

# Rebuild after a source change + restart
bash scripts/build.sh
docker compose up -d --force-recreate nyquest

# Rollback to a previous published tag (e.g. an earlier 3.2.x)
docker run --rm -p 5400:5400 nyquest:3.2.0
```

`docker compose up -d` is idempotent: same image + same env = no-op.

---

## Smoke test

```bash
bash scripts/smoke-test.sh
```

Five checks:

1. `/health` reports `"version":"3.2.0"`
2. `/metrics` returns HTTP 200
3. `/admin/v2/compression/cache` reports a positive capacity
4. `/admin/v2/compression/rules` returns 19 categories
5. `POST /v1/messages` runs the engine end-to-end on a verbose system prompt

A non-zero exit means at least one check failed; CI should treat it as a
deploy gate.

You can target a non-default host:

```bash
BASE=http://staging.internal:5400 bash scripts/smoke-test.sh
```

---

## Performance — host vs bridge network

Measured live on an internal Ubuntu 24.04 host, 2026-05-10, against this image:

| Network mode | Single-thread `ab` /health | Concurrent `ab` -c 20 /health |
|---|---|---|
| **host** (default in compose) | **8,266 req/s** | **18,287 req/s** |
| bridge (with `-p 5400:5400`) | 2,163 req/s | 13,980 req/s |
| systemd (no Docker, baseline) | 6,778 req/s | 17,716 req/s |

Compression results are byte-identical across all three. The bridge
overhead is `docker-proxy` sitting in the request path; host networking
bypasses it. For a compression proxy where real workloads are
kilobyte-sized POST bodies whose engine work dominates over network
overhead, the bridge mode is fine in absolute terms — host mode is just
strictly faster at the cost of less network isolation.

## Coexistence with a non-Docker deployment

Nothing in this Docker setup writes to `/etc/systemd/system/`. To run
Docker and a systemd unit side-by-side, bind them to different host
ports:

* For systemd, edit the YAML config (`port: 5401`).
* For Docker, drop `network_mode: host` and add `ports: ["5401:5400"]`.

---

## Troubleshooting

**Build is slow.** First build is ~3–6 minutes (full Rust compile + test).
Subsequent builds re-use Docker layer cache when only Cargo.lock-equivalent
inputs are unchanged. Use `bash scripts/build.sh --no-cache` only when you
suspect a stale dep.

**`/health` returns nothing.** Container is still starting (cold start ~1s).
Healthcheck has a 10s `start_period` for this reason. After that, check
`docker compose logs nyquest`.

**Cache shows 0 entries.** Either the process just started or
`NYQUEST_CACHE_ENABLED=false`. Hit `/admin/v2/compression/cache` after a
few requests — `entries`, `hits`, `misses`, `stores` should all climb.

**`tests fail during build`.** That's the intended gate. Read the
`cargo test` output the build prints, fix, rebuild. Don't add `|| true`
to the test step.

**Container exits immediately.** Inspect with `docker logs` — most
common cause is a malformed mounted `nyquest.yaml`.

---

## File map

```
Dockerfile                # multi-stage build (req #1–#3)
.dockerignore             # excludes target/, .git, secrets, node_modules
.env.example              # env-var documentation (copy to .env)
docker-compose.yml        # local + prod profile (req #6)
README_DOCKER.md          # this file (req #9)
scripts/
  build.sh                # build the image + tag (req #14)
  run-local.sh            # docker compose up wrapper (req #14)
  smoke-test.sh           # /health + /metrics + /admin + /v1/messages (req #14)
```
