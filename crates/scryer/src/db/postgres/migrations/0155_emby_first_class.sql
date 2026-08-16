ALTER TABLE emby_media_server_details ADD COLUMN server_id TEXT;
ALTER TABLE emby_media_server_details ADD COLUMN connect_enabled BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE user_external_accounts
    DROP CONSTRAINT user_external_accounts_provider_check;
ALTER TABLE user_external_accounts
    ADD CONSTRAINT user_external_accounts_provider_check
    CHECK (provider IN ('plex', 'jellyfin', 'emby'));
