CREATE TABLE totp_credentials (
    id text PRIMARY KEY,
    user_id text NOT NULL UNIQUE REFERENCES users(id) ON DELETE CASCADE,
    secret_base32 text NOT NULL,
    algorithm text NOT NULL CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    digits integer NOT NULL CHECK (digits IN (6, 8)),
    period_seconds integer NOT NULL CHECK (period_seconds > 0),
    last_accepted_step bigint,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL,
    last_used_at timestamp with time zone
);

CREATE TABLE totp_enrollment_challenges (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    secret_base32 text NOT NULL,
    algorithm text NOT NULL CHECK (algorithm IN ('SHA1', 'SHA256', 'SHA512')),
    digits integer NOT NULL CHECK (digits IN (6, 8)),
    period_seconds integer NOT NULL CHECK (period_seconds > 0),
    created_at timestamp with time zone NOT NULL,
    expires_at timestamp with time zone NOT NULL
);

CREATE INDEX idx_totp_enrollment_challenges_expires_at
    ON totp_enrollment_challenges (expires_at);

CREATE INDEX idx_totp_enrollment_challenges_user_id
    ON totp_enrollment_challenges (user_id);

CREATE TABLE totp_recovery_codes (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone
);

CREATE INDEX idx_totp_recovery_codes_user_id
    ON totp_recovery_codes (user_id, used_at);

CREATE TABLE totp_failed_attempts (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    attempted_at timestamp with time zone NOT NULL
);

CREATE INDEX idx_totp_failed_attempts_user_id_attempted_at
    ON totp_failed_attempts (user_id, attempted_at);
