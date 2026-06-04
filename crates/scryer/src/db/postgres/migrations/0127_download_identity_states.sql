CREATE TABLE IF NOT EXISTS download_identity_states (
    id text PRIMARY KEY,
    identity_key text NOT NULL UNIQUE,
    download_request_id text,
    download_fingerprint text,
    client_id text,
    client_type text,
    download_client_item_id text,
    tracked_state text NOT NULL,
    reason text,
    detail text,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    CHECK (
        download_request_id IS NOT NULL
        OR download_fingerprint IS NOT NULL
    )
);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_request_id
    ON download_identity_states(download_request_id);

CREATE INDEX IF NOT EXISTS idx_download_identity_states_fingerprint
    ON download_identity_states(download_fingerprint);
