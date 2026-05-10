#!/usr/bin/env bash
# Build the Nyquest engine Docker image.
#
# Usage:
#   bash scripts/build.sh                  # builds nyquest:3.2.0 + nyquest:latest
#   bash scripts/build.sh --no-cache       # force a clean rebuild
#   IMAGE_TAG=foo bash scripts/build.sh    # custom tag
#
# The image's own RUN layer runs `cargo test` before the final binary build
# (Dockerfile requirement #15) — failed tests fail the build.

set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
cd "$repo_root"

VERSION=$(grep -E '^version = ' Cargo.toml | head -1 | cut -d'"' -f2)
IMAGE_TAG=${IMAGE_TAG:-nyquest}

echo "Building $IMAGE_TAG:$VERSION  (also tagging $IMAGE_TAG:latest)"
docker build "$@" \
    --tag "$IMAGE_TAG:$VERSION" \
    --tag "$IMAGE_TAG:latest" \
    .

echo
echo "Image built:"
docker images "$IMAGE_TAG" --format 'table {{.Repository}}:{{.Tag}}\t{{.Size}}\t{{.CreatedAt}}'
