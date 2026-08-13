ALTER TABLE emby_media_server_details ADD COLUMN server_id TEXT;
ALTER TABLE emby_media_server_details ADD COLUMN connect_enabled INTEGER NOT NULL DEFAULT 0 CHECK (connect_enabled IN (0, 1));
