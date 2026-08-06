#!/bin/sh
set -eu

ARTIFACTS_DIR=${1:?artifacts directory is required}
CONTEXT_DIR=${2:?context directory is required}

find_artifact() {
    artifact_name=$1
    artifact_path=$(find "$ARTIFACTS_DIR" -type f -name "$artifact_name" -print)
    artifact_count=$(printf '%s\n' "$artifact_path" | awk 'NF { count += 1 } END { print count + 0 }')

    if [ "$artifact_count" -ne 1 ]; then
        printf 'Expected exactly one %s below %s; found %s\n' \
            "$artifact_name" "$ARTIFACTS_DIR" "$artifact_count" >&2
        return 1
    fi

    printf '%s\n' "$artifact_path"
}

rm -rf "$CONTEXT_DIR" docker-build

mkdir -p \
    "$CONTEXT_DIR/amd64" \
    "$CONTEXT_DIR/arm64" \
    docker-build/launcher-amd64 \
    docker-build/launcher-arm64 \
    docker-build/amd64-portable \
    docker-build/arm64-portable

tar -xzf "$(find_artifact launcher-linux-amd64.tar.gz)" -C docker-build/launcher-amd64
tar -xzf "$(find_artifact launcher-linux-arm64.tar.gz)" -C docker-build/launcher-arm64
tar -xzf "$(find_artifact scryer-linux-x86_64-portable.tar.gz)" -C docker-build/amd64-portable
tar -xzf "$(find_artifact scryer-linux-arm64-portable.tar.gz)" -C docker-build/arm64-portable

install -m 0755 docker-build/launcher-amd64/scryer-launcher "$CONTEXT_DIR/amd64/scryer-launcher"
install -m 0755 docker-build/amd64-portable/scryer "$CONTEXT_DIR/amd64/scryer"
install -m 0755 docker-build/launcher-arm64/scryer-launcher "$CONTEXT_DIR/arm64/scryer-launcher"
install -m 0755 docker-build/arm64-portable/scryer "$CONTEXT_DIR/arm64/scryer"

test -x "$CONTEXT_DIR/amd64/scryer-launcher"
test -x "$CONTEXT_DIR/amd64/scryer"
test -x "$CONTEXT_DIR/arm64/scryer-launcher"
test -x "$CONTEXT_DIR/arm64/scryer"
