PRAGMA foreign_keys = OFF;

DROP TABLE IF EXISTS user_entitlements;
DROP TABLE IF EXISTS entitlements;

CREATE TABLE users_without_legacy_entitlements (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT NOT NULL UNIQUE,
    display_name TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    password_hash TEXT,
    passkey_public_key TEXT,
    locale TEXT,
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    last_login_at TEXT
);

INSERT INTO users_without_legacy_entitlements (
    id,
    username,
    display_name,
    status,
    password_hash,
    passkey_public_key,
    locale,
    created_at,
    updated_at,
    last_login_at
)
SELECT
    id,
    username,
    display_name,
    status,
    password_hash,
    passkey_public_key,
    locale,
    created_at,
    updated_at,
    last_login_at
FROM users;

DROP TABLE users;
ALTER TABLE users_without_legacy_entitlements RENAME TO users;

PRAGMA foreign_keys = ON;
