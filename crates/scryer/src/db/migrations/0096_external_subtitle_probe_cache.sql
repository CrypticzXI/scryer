CREATE TABLE IF NOT EXISTS external_subtitle_probe_cache (
    media_file_id TEXT NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    modified_at TEXT,
    language TEXT,
    hearing_impaired INTEGER,
    detection_source_language TEXT NOT NULL,
    detection_source_hi TEXT NOT NULL,
    probe_version INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (media_file_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_external_subtitle_probe_cache_media_file
    ON external_subtitle_probe_cache(media_file_id);

CREATE INDEX IF NOT EXISTS idx_external_subtitle_probe_cache_file_path
    ON external_subtitle_probe_cache(file_path);
