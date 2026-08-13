ALTER TABLE emby_media_server_details ADD COLUMN server_id TEXT;
ALTER TABLE emby_media_server_details ADD COLUMN connect_enabled BOOLEAN NOT NULL DEFAULT FALSE;
