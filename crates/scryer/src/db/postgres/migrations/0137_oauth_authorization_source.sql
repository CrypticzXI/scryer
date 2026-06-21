ALTER TABLE oauth_authorization_codes
    ADD COLUMN authorization_source TEXT NOT NULL DEFAULT 'authenticated';

ALTER TABLE oauth_refresh_grants
    ADD COLUMN authorization_source TEXT NOT NULL DEFAULT 'authenticated';

CREATE INDEX IF NOT EXISTS idx_oauth_refresh_grants_authorization_source
    ON oauth_refresh_grants(authorization_source);
