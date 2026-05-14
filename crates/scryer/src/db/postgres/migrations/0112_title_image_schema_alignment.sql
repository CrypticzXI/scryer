ALTER TABLE titles
    ADD COLUMN IF NOT EXISTS poster_local_path TEXT,
    ADD COLUMN IF NOT EXISTS banner_local_path TEXT,
    ADD COLUMN IF NOT EXISTS background_local_path TEXT;

CREATE TABLE IF NOT EXISTS title_images (
    id TEXT PRIMARY KEY,
    title_id TEXT,
    provider TEXT,
    provider_image_id TEXT,
    kind TEXT,
    source_url TEXT,
    source_etag TEXT,
    source_last_modified TEXT,
    source_format TEXT,
    source_width BIGINT,
    source_height BIGINT,
    storage_mode TEXT,
    master_path TEXT,
    master_format TEXT,
    master_sha256 TEXT,
    master_width BIGINT,
    master_height BIGINT,
    bytes BYTEA,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (title_id, kind)
);

CREATE TABLE IF NOT EXISTS title_image_variants (
    id TEXT PRIMARY KEY,
    title_image_id TEXT,
    variant_key TEXT,
    path TEXT,
    format TEXT,
    width BIGINT,
    height BIGINT,
    bytes BYTEA,
    sha256 TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (title_image_id, variant_key)
);
