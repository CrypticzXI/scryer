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
    docker-build/amd64-docker-cpu \
    docker-build/arm64-docker-cpu

tar -xzf "$ARTIFACTS_DIR/launcher-linux-amd64.tar.gz" -C docker-build/launcher-amd64
tar -xzf "$ARTIFACTS_DIR/launcher-linux-arm64.tar.gz" -C docker-build/launcher-arm64
tar -xzf "$ARTIFACTS_DIR/docker-scryer-linux-x86_64-cpu.tar.gz" -C docker-build/amd64-docker-cpu
tar -xzf "$ARTIFACTS_DIR/docker-scryer-linux-arm64-cpu.tar.gz" -C docker-build/arm64-docker-cpu

install -m 0755 docker-build/launcher-amd64/scryer-launcher "$CONTEXT_DIR/amd64/scryer-launcher"
install -m 0755 docker-build/amd64-docker-cpu/scryer "$CONTEXT_DIR/amd64/scryer"
install -m 0755 docker-build/launcher-arm64/scryer-launcher "$CONTEXT_DIR/arm64/scryer-launcher"
install -m 0755 docker-build/arm64-docker-cpu/scryer "$CONTEXT_DIR/arm64/scryer"

test -x "$CONTEXT_DIR/amd64/scryer-launcher"
test -x "$CONTEXT_DIR/amd64/scryer"
test -x "$CONTEXT_DIR/arm64/scryer-launcher"
test -x "$CONTEXT_DIR/arm64/scryer"
