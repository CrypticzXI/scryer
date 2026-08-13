ALTER TABLE indexers
    ADD COLUMN download_client_id TEXT
    REFERENCES download_clients(id)
    ON DELETE SET NULL;

CREATE INDEX idx_indexers_download_client_id
    ON indexers(download_client_id);

ALTER TABLE pending_releases
    ADD COLUMN indexer_id TEXT;

CREATE INDEX idx_pending_releases_indexer_id
    ON pending_releases(indexer_id);

UPDATE pending_releases
SET indexer_id = (
    SELECT indexers.id
    FROM indexers
    WHERE indexers.name = pending_releases.indexer_source
)
WHERE pending_releases.indexer_source IS NOT NULL
  AND (
      SELECT COUNT(*)
      FROM indexers
      WHERE indexers.name = pending_releases.indexer_source
  ) = 1;
