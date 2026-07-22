CREATE TABLE image_proxy_sources (
  token text PRIMARY KEY,
  upstream_url text,
  owner_type text,
  owner_id text,
  image_kind text NOT NULL,
  fallback_class text NOT NULL,
  last_seen_at timestamptz NOT NULL
);

CREATE INDEX idx_image_proxy_sources_last_seen_at
  ON image_proxy_sources(last_seen_at);

CREATE TABLE image_proxy_cache_entries (
  token text NOT NULL,
  variant text NOT NULL,
  content_type text NOT NULL,
  byte_size bigint NOT NULL,
  upstream_etag text,
  upstream_last_modified text,
  fetched_at timestamptz NOT NULL,
  last_accessed_at timestamptz NOT NULL,
  PRIMARY KEY (token, variant),
  FOREIGN KEY (token) REFERENCES image_proxy_sources(token) ON DELETE CASCADE
);

CREATE INDEX idx_image_proxy_cache_entries_last_accessed_at
  ON image_proxy_cache_entries(last_accessed_at);
