CREATE TABLE webauthn_credentials (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    credential_id text NOT NULL UNIQUE,
    credential_json text NOT NULL,
    friendly_name text,
    created_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone
);

CREATE INDEX idx_webauthn_credentials_user_id_created_at
    ON webauthn_credentials (user_id, created_at DESC);

CREATE TABLE webauthn_challenges (
    id text PRIMARY KEY,
    user_id text REFERENCES users(id) ON DELETE CASCADE,
    challenge_type text NOT NULL CHECK (challenge_type IN ('registration', 'authentication')),
    state_json text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

CREATE INDEX idx_webauthn_challenges_expires_at
    ON webauthn_challenges (expires_at);

CREATE INDEX idx_webauthn_challenges_user_id
    ON webauthn_challenges (user_id);
