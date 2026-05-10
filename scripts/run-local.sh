#!/usr/bin/env bash
# Bring up the Nyquest engine locally via docker compose.
# Idempotent: rebuilds only on Dockerfile/source change.
#
# Usage:
#   bash scripts/run-local.sh           # foreground (Ctrl-C stops)
#   bash scripts/run-local.sh -d        # detached
#   bash scripts/run-local.sh --build   # force rebuild before up
#
# After: bash scripts/smoke-test.sh

set -euo pipefail
cd "$(dirname "$0")/.."

# Create .env from .env.example if missing — keeps compose env_file happy.
if [ ! -f .env ] && [ -f .env.example ]; then
    echo "(.env not found — copying from .env.example)"
    cp .env.example .env
fi

# Ensure logs/ exists on the host (compose mounts it; missing host path = mount failure)
mkdir -p logs

exec docker compose up "$@"
