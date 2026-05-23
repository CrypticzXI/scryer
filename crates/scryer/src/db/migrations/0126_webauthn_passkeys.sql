CREATE TABLE webauthn_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    credential_id TEXT NOT NULL,
    credential_json TEXT NOT NULL,
    friendly_name TEXT,
    created_at TEXT NOT NULL,
    last_used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_webauthn_credentials_credential_id
    ON webauthn_credentials (credential_id);

CREATE INDEX idx_webauthn_credentials_user_id_created_at
    ON webauthn_credentials (user_id, created_at DESC);

CREATE TABLE webauthn_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    challenge_type TEXT NOT NULL,
    state_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (challenge_type IN ('registration', 'authentication'))
);

CREATE INDEX idx_webauthn_challenges_expires_at
    ON webauthn_challenges (expires_at);

CREATE INDEX idx_webauthn_challenges_user_id
    ON webauthn_challenges (user_id);
