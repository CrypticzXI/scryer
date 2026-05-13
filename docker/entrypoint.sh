#!/bin/sh
set -e

print_kv() {
    printf '  %-18s %s\n' "$1" "$2"
}

cpuinfo_field() {
    key="$1"
    awk -F': *' -v key="$key" '$1 == key { print $2; exit }' /proc/cpuinfo 2>/dev/null
}

resolved_db_path() {
    db_path="${SCRYER_DB_PATH:-/config/scryer.db}"
    db_path="${db_path#sqlite://}"
    db_path="${db_path%%\?*}"
    printf '%s\n' "$db_path"
}

cpu_count() {
    if command -v getconf >/dev/null 2>&1; then
        getconf _NPROCESSORS_ONLN 2>/dev/null && return 0
    fi
    if command -v nproc >/dev/null 2>&1; then
        nproc 2>/dev/null && return 0
    fi
    if [ -r /proc/cpuinfo ]; then
        grep -c '^processor' /proc/cpuinfo 2>/dev/null && return 0
    fi
    printf 'unknown\n'
}

log_startup_diagnostics() {
    launch_mode="$1"
    kernel="$(uname -sr 2>/dev/null || printf 'unknown')"
    machine="$(uname -m 2>/dev/null || printf 'unknown')"
    os_pretty='unknown'

    if [ -r /etc/os-release ]; then
        os_pretty="$(awk -F= '/^PRETTY_NAME=/{gsub(/^"|"$/, "", $2); print $2; exit}' /etc/os-release 2>/dev/null)"
        [ -n "$os_pretty" ] || os_pretty='unknown'
    fi

    echo "  Startup diagnostics:"
    print_kv 'Launch mode:' "$launch_mode"
    print_kv 'Kernel:' "$kernel"
    print_kv 'OS:' "$os_pretty"
    print_kv 'Machine:' "$machine"
    print_kv 'Entrypoint UID:' "$(id -u 2>/dev/null || printf 'unknown')"
    print_kv 'Entrypoint GID:' "$(id -g 2>/dev/null || printf 'unknown')"
    print_kv 'Target UID:' "${PUID:-unknown}"
    print_kv 'Target GID:' "${PGID:-unknown}"
    print_kv 'CPU count:' "$(cpu_count)"

    if [ -x /usr/local/bin/scryer ]; then
        print_kv 'Binary size:' "$(wc -c < /usr/local/bin/scryer 2>/dev/null | awk '{print $1}') bytes"
        if command -v sha256sum >/dev/null 2>&1; then
            print_kv 'Binary sha256:' "$(sha256sum /usr/local/bin/scryer 2>/dev/null | awk '{print $1}')"
        fi
    else
        print_kv 'Binary status:' 'missing or not executable'
    fi

    if [ -r /proc/cpuinfo ]; then
        vendor="$(cpuinfo_field 'vendor_id')"
        family="$(cpuinfo_field 'cpu family')"
        model="$(cpuinfo_field 'model')"
        stepping="$(cpuinfo_field 'stepping')"
        model_name="$(cpuinfo_field 'model name')"
        architecture="$(cpuinfo_field 'CPU architecture')"
        part="$(cpuinfo_field 'CPU part')"
        revision="$(cpuinfo_field 'CPU revision')"
        flags="$(cpuinfo_field 'flags')"
        features="$(cpuinfo_field 'Features')"

        [ -n "$vendor" ] && print_kv 'CPU vendor:' "$vendor"
        [ -n "$family" ] && print_kv 'CPU family:' "$family"
        [ -n "$model" ] && print_kv 'CPU model:' "$model"
        [ -n "$stepping" ] && print_kv 'CPU stepping:' "$stepping"
        [ -n "$model_name" ] && print_kv 'CPU model name:' "$model_name"
        [ -n "$architecture" ] && print_kv 'CPU arch level:' "$architecture"
        [ -n "$part" ] && print_kv 'CPU part:' "$part"
        [ -n "$revision" ] && print_kv 'CPU revision:' "$revision"
        [ -n "$flags" ] && print_kv 'CPU flags:' "$flags"
        [ -n "$features" ] && print_kv 'CPU features:' "$features"
    else
        print_kv 'CPU info:' 'unavailable'
    fi

    return 0
}

# If not running as root (e.g. --user flag), skip privilege setup
# and just exec the binary directly.
if [ "$(id -u)" -ne 0 ]; then
    PUID="$(id -u)"
    PGID="$(id -g)"
    log_startup_diagnostics 'non-root'
    exec /usr/local/bin/scryer --data-dir /config "$@"
fi

PUID=${PUID:-1000}
PGID=${PGID:-1000}

# ── Migrate from /data to /config ────────────────────────────────────────────
# Previous images stored the database in /data. If the user hasn't overridden
# SCRYER_DB_PATH and the old database exists but the new location doesn't,
# move it automatically so existing installs upgrade seamlessly.
if [ "${SCRYER_DB_PATH}" = "/config/scryer.db" ] || [ -z "${SCRYER_DB_PATH}" ]; then
    if [ -f /data/scryer.db ] && [ ! -f /config/scryer.db ]; then
        echo "  Migrating legacy database into the config volume"
        mkdir -p /config
        cp /data/scryer.db /config/scryer.db
        # Copy WAL/SHM if present
        [ -f /data/scryer.db-wal ] && cp /data/scryer.db-wal /config/scryer.db-wal
        [ -f /data/scryer.db-shm ] && cp /data/scryer.db-shm /config/scryer.db-shm
        echo "  Migration complete. The legacy database can be removed after verifying."
    fi
fi

# Derive the database directory from SCRYER_DB_PATH so we chown the right
# location regardless of whether the user overrides the default path.
DB_PATH="$(resolved_db_path)"
DB_DIR="$(dirname "$DB_PATH")"

# Ensure /config and the database directory are owned by the requested user.
mkdir -p /config
chown -R "$PUID":"$PGID" /config
if [ -d "$DB_DIR" ] && [ "$DB_DIR" != "/config" ]; then
    chown -R "$PUID":"$PGID" "$DB_DIR"
fi

echo "
───────────────────────────────────
  scryer
  User UID:  $PUID
  User GID:  $PGID
$(log_startup_diagnostics root)
───────────────────────────────────
"

exec su-exec "$PUID":"$PGID" /usr/local/bin/scryer --data-dir /config "$@"
