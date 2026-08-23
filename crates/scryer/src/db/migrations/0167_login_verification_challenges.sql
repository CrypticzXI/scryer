-- Durable, short-lived state for password-login verification. These rows are
-- intentionally excluded from backups and are deleted after a successful
-- verification or expiry.
CREATE TABLE login_verification_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    login_method TEXT NOT NULL,
    persist_session INTEGER NOT NULL,
    allow_passkey INTEGER NOT NULL,
    allow_totp INTEGER NOT NULL,
    auth_session_version TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (login_method IN ('local_password', 'jellyfin', 'emby')),
    CHECK (persist_session IN (0, 1)),
    CHECK (allow_passkey IN (0, 1)),
    CHECK (allow_totp IN (0, 1))
);

CREATE INDEX idx_login_verification_challenges_user_id
    ON login_verification_challenges (user_id);

CREATE INDEX idx_login_verification_challenges_expires_at
    ON login_verification_challenges (expires_at);

-- Existing WebAuthn ceremonies are ephemeral. Invalidating them makes the
-- purpose boundary below unambiguous during upgrade.
DELETE FROM webauthn_challenges;

ALTER TABLE webauthn_challenges
    ADD COLUMN purpose TEXT NOT NULL DEFAULT 'standalone_authentication';

ALTER TABLE webauthn_challenges
    ADD COLUMN login_verification_challenge_id TEXT
    REFERENCES login_verification_challenges(id) ON DELETE CASCADE;

CREATE INDEX idx_webauthn_challenges_login_verification
    ON webauthn_challenges (login_verification_challenge_id);
