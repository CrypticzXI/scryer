ALTER TABLE subtitle_blacklist RENAME TO subtitle_blocklist;

DROP INDEX IF EXISTS idx_subtitle_blacklist_media_file;
CREATE INDEX IF NOT EXISTS idx_subtitle_blocklist_media_file
    ON subtitle_blocklist(media_file_id);

ALTER TABLE subtitle_downloads
    ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'downloaded';
