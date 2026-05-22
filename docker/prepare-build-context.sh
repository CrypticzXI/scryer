#!/bin/sh
set -eu

ARTIFACTS_DIR=${1:?artifacts directory is required}
CONTEXT_DIR=${2:?context directory is required}

rm -rf "$CONTEXT_DIR" docker-build

mkdir -p \
    "$CONTEXT_DIR/amd64" \
    "$CONTEXT_DIR/arm64" \
    docker-build/launcher-amd64 \
    docker-build/launcher-arm64 \
    docker-build/amd64-portable \
    docker-build/amd64-haswell \
    docker-build/arm64-portable \
    docker-build/arm64-optimized

tar -xzf "$ARTIFACTS_DIR/launcher-linux-amd64.tar.gz" -C docker-build/launcher-amd64
tar -xzf "$ARTIFACTS_DIR/launcher-linux-arm64.tar.gz" -C docker-build/launcher-arm64
tar -xzf "$ARTIFACTS_DIR/scryer-linux-x86_64-portable.tar.gz" -C docker-build/amd64-portable
tar -xzf "$ARTIFACTS_DIR/scryer-linux-x86_64-haswell.tar.gz" -C docker-build/amd64-haswell
tar -xzf "$ARTIFACTS_DIR/scryer-linux-arm64-portable.tar.gz" -C docker-build/arm64-portable
tar -xzf "$ARTIFACTS_DIR/scryer-linux-arm64-optimized.tar.gz" -C docker-build/arm64-optimized

install -m 0755 docker-build/launcher-amd64/scryer-launcher "$CONTEXT_DIR/amd64/scryer-launcher"
install -m 0755 docker-build/amd64-portable/scryer "$CONTEXT_DIR/amd64/scryer-portable"
install -m 0755 docker-build/amd64-haswell/scryer "$CONTEXT_DIR/amd64/scryer-haswell"
install -m 0755 docker-build/launcher-arm64/scryer-launcher "$CONTEXT_DIR/arm64/scryer-launcher"
install -m 0755 docker-build/arm64-portable/scryer "$CONTEXT_DIR/arm64/scryer-portable"
install -m 0755 docker-build/arm64-optimized/scryer "$CONTEXT_DIR/arm64/scryer-arm64-optimized"

test -x "$CONTEXT_DIR/amd64/scryer-launcher"
test -x "$CONTEXT_DIR/amd64/scryer-portable"
test -x "$CONTEXT_DIR/amd64/scryer-haswell"
test -x "$CONTEXT_DIR/arm64/scryer-launcher"
test -x "$CONTEXT_DIR/arm64/scryer-portable"
test -x "$CONTEXT_DIR/arm64/scryer-arm64-optimized"
