CREATE TABLE image_proxy_sources (
  token TEXT PRIMARY KEY,
  upstream_url TEXT,
  owner_type TEXT,
  owner_id TEXT,
  image_kind TEXT NOT NULL,
  fallback_class TEXT NOT NULL,
  last_seen_at TEXT NOT NULL
);

CREATE INDEX idx_image_proxy_sources_last_seen_at
  ON image_proxy_sources(last_seen_at);

CREATE TABLE image_proxy_cache_entries (
  token TEXT NOT NULL,
  variant TEXT NOT NULL,
  content_type TEXT NOT NULL,
  byte_size INTEGER NOT NULL,
  upstream_etag TEXT,
  upstream_last_modified TEXT,
  fetched_at TEXT NOT NULL,
  last_accessed_at TEXT NOT NULL,
  PRIMARY KEY (token, variant),
  FOREIGN KEY (token) REFERENCES image_proxy_sources(token) ON DELETE CASCADE
);

CREATE INDEX idx_image_proxy_cache_entries_last_accessed_at
  ON image_proxy_cache_entries(last_accessed_at);
