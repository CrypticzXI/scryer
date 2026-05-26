#!/bin/sh
set -eu

DOCKERFILE_PATH=${1:-docker/scryer.Dockerfile}
CONTEXT_SCRIPT_PATH=${2:-docker/prepare-build-context.sh}

assert_contains() {
    haystack=$1
    needle=$2
    message=$3

    case "$haystack" in
        *"$needle"*) ;;
        *)
            printf 'assertion failed: %s\nmissing: %s\n' "$message" "$needle" >&2
            exit 1
            ;;
    esac
}

assert_not_contains() {
    haystack=$1
    needle=$2
    message=$3

    case "$haystack" in
        *"$needle"*)
            printf 'assertion failed: %s\nunexpected: %s\n' "$message" "$needle" >&2
            exit 1
            ;;
        *) ;;
    esac
}

dockerfile_content=$(cat "$DOCKERFILE_PATH")
context_script_content=$(cat "$CONTEXT_SCRIPT_PATH")

assert_contains "$dockerfile_content" 'COPY ${TARGETARCH}/scryer ${TARGETARCH}/scryer-launcher /opt/scryer/' "Dockerfile should copy one architecture-specific scryer payload and launcher"
assert_contains "$dockerfile_content" "mkdir -p /config /data" "Dockerfile should create persistent data directories"
assert_contains "$dockerfile_content" "VOLUME /config" "Dockerfile should declare the /config volume"
assert_contains "$dockerfile_content" "ENV PUID=1000" "Dockerfile should default PUID"
assert_contains "$dockerfile_content" "ENV PGID=1000" "Dockerfile should default PGID"
assert_contains "$dockerfile_content" "ENV UMASK=022" "Dockerfile should default UMASK"
assert_contains "$dockerfile_content" "ENV SCRYER_DB_PATH=/config/scryer.db" "Dockerfile should store the database under /config"
assert_contains "$dockerfile_content" 'ENTRYPOINT ["/opt/scryer/scryer-launcher"]' "Dockerfile should use the launcher entrypoint"

assert_contains "$context_script_content" "launcher-linux-amd64.tar.gz" "build context should include the amd64 launcher"
assert_contains "$context_script_content" "launcher-linux-arm64.tar.gz" "build context should include the arm64 launcher"
assert_contains "$context_script_content" "scryer-linux-x86_64-portable.tar.gz" "build context should include the amd64 portable payload"
assert_contains "$context_script_content" "scryer-linux-arm64-portable.tar.gz" "build context should include the arm64 portable payload"
assert_contains "$context_script_content" 'install -m 0755 docker-build/amd64-portable/scryer "$CONTEXT_DIR/amd64/scryer"' "build context should install the amd64 payload at the Dockerfile path"
assert_contains "$context_script_content" 'install -m 0755 docker-build/arm64-portable/scryer "$CONTEXT_DIR/arm64/scryer"' "build context should install the arm64 payload at the Dockerfile path"
assert_not_contains "$context_script_content" "haswell" "build context should not require an extra amd64 optimized payload"
assert_not_contains "$context_script_content" "optimized" "build context should not require an extra arm64 optimized payload"

printf 'docker packaging contract check passed for %s and %s\n' "$DOCKERFILE_PATH" "$CONTEXT_SCRIPT_PATH"
