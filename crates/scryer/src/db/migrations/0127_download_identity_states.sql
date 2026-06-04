CREATE TABLE IF NOT EXISTS download_identity_states (
    id TEXT PRIMARY KEY,
    identity_key TEXT NOT NULL UNIQUE,
    download_request_id TEXT,
    download_fingerprint TEXT,
    client_id TEXT,
    client_type TEXT,
    download_client_item_id TEXT,
    tracked_state TEXT NOT NULL,
    reason TEXT,
    detail TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        download_request_id IS NOT NULL
        OR download_fingerprint IS NOT NULL
    )
);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_request_id
    ON download_identity_states(download_request_id);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_fingerprint
    ON download_identity_states(download_fingerprint);
