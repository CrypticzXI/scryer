CREATE TABLE IF NOT EXISTS download_submission_episode_links (
    download_client_id TEXT NOT NULL DEFAULT '',
    download_client_type TEXT NOT NULL,
    download_client_item_id TEXT NOT NULL,
    episode_id TEXT NOT NULL,
    PRIMARY KEY (
        download_client_id,
        download_client_type,
        download_client_item_id,
        episode_id
    )
);

CREATE INDEX IF NOT EXISTS idx_download_submission_episode_links_episode
ON download_submission_episode_links(episode_id);
