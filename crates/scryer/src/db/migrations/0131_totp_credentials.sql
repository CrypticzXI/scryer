CREATE TABLE totp_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL UNIQUE,
    secret_base32 TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    digits INTEGER NOT NULL,
    period_seconds INTEGER NOT NULL,
    last_accepted_step INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    CHECK (digits IN (6, 8)),
    CHECK (period_seconds > 0)
);

CREATE TABLE totp_enrollment_challenges (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    secret_base32 TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    digits INTEGER NOT NULL,
    period_seconds INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    CHECK (digits IN (6, 8)),
    CHECK (period_seconds > 0)
);

CREATE INDEX idx_totp_enrollment_challenges_expires_at
    ON totp_enrollment_challenges (expires_at);

CREATE INDEX idx_totp_enrollment_challenges_user_id
    ON totp_enrollment_challenges (user_id);

CREATE TABLE totp_recovery_codes (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    created_at TEXT NOT NULL,
    used_at TEXT,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_totp_recovery_codes_user_id
    ON totp_recovery_codes (user_id, used_at);

CREATE TABLE totp_failed_attempts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    attempted_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_totp_failed_attempts_user_id_attempted_at
    ON totp_failed_attempts (user_id, attempted_at);
