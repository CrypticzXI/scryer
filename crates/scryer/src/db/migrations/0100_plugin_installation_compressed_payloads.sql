ALTER TABLE plugin_installations RENAME TO plugin_installations_old;

CREATE TABLE plugin_installations (
    id               TEXT PRIMARY KEY,
    plugin_id        TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    version          TEXT NOT NULL,
    sdk_version      TEXT NOT NULL DEFAULT '',
    sdk_constraint   TEXT NOT NULL DEFAULT '',
    scryer_constraint TEXT,
    plugin_type      TEXT NOT NULL DEFAULT 'indexer',
    provider_type    TEXT NOT NULL,
    is_enabled       INTEGER NOT NULL DEFAULT 1,
    is_builtin       INTEGER NOT NULL DEFAULT 0,
    source_kind      TEXT NOT NULL DEFAULT 'downloaded',
    wasm_bytes       BLOB,
    wasm_encoding    TEXT NOT NULL DEFAULT 'identity',
    wasm_digest_algo TEXT,
    source_url       TEXT,
    support_tier     TEXT NOT NULL DEFAULT 'official',
    publisher        TEXT,
    docs_url         TEXT,
    source_repo      TEXT,
    manifest_url     TEXT,
    wasm_digest      TEXT,
    artifact_digest  TEXT,
    installed_at     TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);

INSERT INTO plugin_installations (
    id, plugin_id, name, description, version, sdk_version, sdk_constraint,
    scryer_constraint, plugin_type, provider_type, is_enabled, is_builtin,
    source_kind, wasm_bytes, wasm_encoding, wasm_digest_algo, source_url, support_tier,
    publisher, docs_url, source_repo, manifest_url, wasm_digest, artifact_digest,
    installed_at, updated_at
)
SELECT
    id,
    plugin_id,
    name,
    description,
    version,
    COALESCE(sdk_version, ''),
    COALESCE(sdk_constraint, ''),
    scryer_constraint,
    plugin_type,
    provider_type,
    is_enabled,
    is_builtin,
    COALESCE(source_kind, 'downloaded'),
    wasm_bytes,
    'identity',
    NULL,
    source_url,
    COALESCE(support_tier, 'official'),
    publisher,
    docs_url,
    source_repo,
    manifest_url,
    wasm_digest,
    artifact_digest,
    installed_at,
    updated_at
FROM plugin_installations_old;

DROP TABLE plugin_installations_old;
