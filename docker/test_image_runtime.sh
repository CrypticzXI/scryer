#!/bin/sh
set -eu

IMAGE_TAG=${1:?image tag is required}
PLATFORM=${2:?platform is required}

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

run_version() {
    docker run --rm --platform "$PLATFORM" "$IMAGE_TAG" --version
}

reown_volume() {
    volume_name=$1
    owner=$2

    docker run --rm \
        -v "$volume_name:/target" \
        alpine:3.22 \
        chown "$owner" /target >/dev/null
}

volume_owner() {
    volume_name=$1

    docker run --rm \
        -v "$volume_name:/target" \
        alpine:3.22 \
        stat -c '%u:%g' /target
}

current_uid=$(id -u)
current_gid=$(id -g)
config_volume=scryer-test-config-$$
rootfs_container_id=
tmpdir=$(mktemp -d)
rootfs_dir=$tmpdir/rootfs
mkdir -p "$rootfs_dir"

cleanup() {
    if docker volume inspect "$config_volume" >/dev/null 2>&1; then
        docker volume rm -f "$config_volume" >/dev/null 2>&1 || true
    fi
    if [ -n "$rootfs_container_id" ]; then
        docker rm -f "$rootfs_container_id" >/dev/null 2>&1 || true
    fi
    rm -rf "$tmpdir"
}

trap cleanup EXIT INT TERM

entrypoint=$(docker image inspect --format '{{json .Config.Entrypoint}}' "$IMAGE_TAG")
assert_contains "$entrypoint" "/opt/scryer/scryer-launcher" "image should use the Rust launcher entrypoint"

output=$(run_version)
assert_contains "$output" "scryer" "image should proxy --version output"

output=$(docker run --rm --platform "$PLATFORM" --read-only "$IMAGE_TAG" --version)
assert_contains "$output" "scryer" "read-only rootfs should still start"

output=$(
    docker run --rm --platform "$PLATFORM" \
        --read-only \
        --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m \
        "$IMAGE_TAG" \
        --version
)
assert_contains "$output" "scryer" "noexec tmpfs should still start"

docker volume create "$config_volume" >/dev/null

reown_volume "$config_volume" "65534:65534"
docker run --rm --platform "$PLATFORM" \
    -e PUID="$current_uid" \
    -e PGID="$current_gid" \
    -v "$config_volume:/config" \
    "$IMAGE_TAG" \
    --version >/dev/null

owner=$(volume_owner "$config_volume")
[ "$owner" = "$current_uid:$current_gid" ] || {
    printf 'assertion failed: root entrypoint path should chown /config\nexpected: %s\nactual: %s\n' \
        "$current_uid:$current_gid" "$owner" >&2
    exit 1
}

reown_volume "$config_volume" "65534:65534"
docker run --rm --platform "$PLATFORM" \
    --user "$current_uid:$current_gid" \
    -e PUID=12345 \
    -e PGID=12345 \
    -v "$config_volume:/config" \
    "$IMAGE_TAG" \
    --version >/dev/null

owner=$(volume_owner "$config_volume")
[ "$owner" = "65534:65534" ] || {
    printf 'assertion failed: non-root entrypoint path should skip chown\nexpected: 65534:65534\nactual: %s\n' "$owner" >&2
    exit 1
}

rootfs_container_id=$(docker create --platform "$PLATFORM" "$IMAGE_TAG" --version)
docker export "$rootfs_container_id" | tar -C "$rootfs_dir" -xf -

[ -f "$rootfs_dir/etc/ssl/certs/ca-certificates.crt" ] || {
    printf 'assertion failed: runtime image should include a CA bundle at /etc/ssl/certs/ca-certificates.crt\n' >&2
    exit 1
}

[ -f "$rootfs_dir/usr/share/zoneinfo/Etc/UTC" ] || {
    printf 'assertion failed: runtime image should include zoneinfo at /usr/share/zoneinfo\n' >&2
    exit 1
}

utc_offset=$(
    docker run --rm --platform "$PLATFORM" \
        -e TZ=Etc/UTC \
        -v "$rootfs_dir/usr/share/zoneinfo:/usr/share/zoneinfo:ro" \
        debian:bookworm-slim \
        date +%z
)
[ "$utc_offset" = "+0000" ] || {
    printf 'assertion failed: mounted runtime zoneinfo should resolve TZ=Etc/UTC\nexpected: +0000\nactual: %s\n' \
        "$utc_offset" >&2
    exit 1
}

denver_offset=$(
    docker run --rm --platform "$PLATFORM" \
        -e TZ=America/Denver \
        -v "$rootfs_dir/usr/share/zoneinfo:/usr/share/zoneinfo:ro" \
        debian:bookworm-slim \
        date +%z
)
[ "$denver_offset" != "+0000" ] || {
    printf 'assertion failed: mounted runtime zoneinfo should resolve TZ=America/Denver to a non-UTC offset\n' >&2
    exit 1
}

docker run --rm --platform "$PLATFORM" \
    -v "$rootfs_dir/etc/ssl/certs/ca-certificates.crt:/runtime-ca-certificates.crt:ro" \
    curlimages/curl:8.13.0 \
    --fail --silent --show-error --location \
    --cacert /runtime-ca-certificates.crt \
    https://example.com >/dev/null

printf 'docker launcher image smoke tests passed for %s\n' "$PLATFORM"
