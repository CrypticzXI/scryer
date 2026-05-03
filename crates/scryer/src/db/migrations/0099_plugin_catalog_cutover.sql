ALTER TABLE plugin_installations ADD COLUMN support_tier TEXT NOT NULL DEFAULT 'official';
ALTER TABLE plugin_installations ADD COLUMN publisher TEXT;
ALTER TABLE plugin_installations ADD COLUMN docs_url TEXT;
ALTER TABLE plugin_installations ADD COLUMN source_repo TEXT;
ALTER TABLE plugin_installations ADD COLUMN manifest_url TEXT;
ALTER TABLE plugin_installations ADD COLUMN wasm_digest TEXT;
ALTER TABLE plugin_installations ADD COLUMN artifact_digest TEXT;

CREATE TABLE IF NOT EXISTS plugin_catalog_sources (
    source_key      TEXT PRIMARY KEY,
    source_kind     TEXT NOT NULL,
    source_url      TEXT NOT NULL,
    github_repo     TEXT,
    support_tier    TEXT NOT NULL DEFAULT 'official',
    catalog_json    TEXT,
    last_success_at TEXT,
    last_error      TEXT,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_catalog_status (
    status_key      TEXT PRIMARY KEY,
    status_json     TEXT NOT NULL,
    checked_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plugin_catalog_sources_kind
    ON plugin_catalog_sources(source_kind);
