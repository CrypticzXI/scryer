-- Rolled up from migrations/0125_release_metadata_enum_canonicalization.sql
UPDATE media_files
SET source_type = CASE
    WHEN source_type IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
    ELSE source_type
END,
video_codec = CASE
    WHEN video_codec IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
    ELSE video_codec
END,
video_codec_parsed = CASE
    WHEN video_codec_parsed IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
    ELSE video_codec_parsed
END,
audio_codec_parsed = CASE
    WHEN audio_codec_parsed IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
    ELSE audio_codec_parsed
END;

CREATE TEMP TABLE quality_profile_source_allowlist_norm AS
SELECT profile_id, canonical_source AS source, MIN(created_at) AS created_at
FROM (
    SELECT profile_id, created_at, CASE
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
        ELSE NULL
    END AS canonical_source
    FROM quality_profile_source_allowlist
) normalized
WHERE canonical_source IS NOT NULL
GROUP BY profile_id, canonical_source;

DELETE FROM quality_profile_source_allowlist;
INSERT INTO quality_profile_source_allowlist (profile_id, source, created_at)
SELECT profile_id, source, created_at FROM quality_profile_source_allowlist_norm ORDER BY profile_id, source;
DROP TABLE quality_profile_source_allowlist_norm;

CREATE TEMP TABLE quality_profile_source_blocklist_norm AS
SELECT profile_id, canonical_source AS source, MIN(created_at) AS created_at
FROM (
    SELECT profile_id, created_at, CASE
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
        ELSE NULL
    END AS canonical_source
    FROM quality_profile_source_blocklist
) normalized
WHERE canonical_source IS NOT NULL
GROUP BY profile_id, canonical_source;

DELETE FROM quality_profile_source_blocklist;
INSERT INTO quality_profile_source_blocklist (profile_id, source, created_at)
SELECT profile_id, source, created_at FROM quality_profile_source_blocklist_norm ORDER BY profile_id, source;
DROP TABLE quality_profile_source_blocklist_norm;

CREATE TEMP TABLE quality_profile_audio_codec_allowlist_norm AS
SELECT profile_id, canonical_codec AS codec, MIN(created_at) AS created_at
FROM (
    SELECT profile_id, created_at, CASE
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
        ELSE NULL
    END AS canonical_codec
    FROM quality_profile_audio_codec_allowlist
) normalized
WHERE canonical_codec IS NOT NULL
GROUP BY profile_id, canonical_codec;

DELETE FROM quality_profile_audio_codec_allowlist;
INSERT INTO quality_profile_audio_codec_allowlist (profile_id, codec, created_at)
SELECT profile_id, codec, created_at FROM quality_profile_audio_codec_allowlist_norm ORDER BY profile_id, codec;
DROP TABLE quality_profile_audio_codec_allowlist_norm;

CREATE TEMP TABLE quality_profile_audio_codec_blocklist_norm AS
SELECT profile_id, canonical_codec AS codec, MIN(created_at) AS created_at
FROM (
    SELECT profile_id, created_at, CASE
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(TRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
        ELSE NULL
    END AS canonical_codec
    FROM quality_profile_audio_codec_blocklist
) normalized
WHERE canonical_codec IS NOT NULL
GROUP BY profile_id, canonical_codec;

DELETE FROM quality_profile_audio_codec_blocklist;
INSERT INTO quality_profile_audio_codec_blocklist (profile_id, codec, created_at)
SELECT profile_id, codec, created_at FROM quality_profile_audio_codec_blocklist_norm ORDER BY profile_id, codec;
DROP TABLE quality_profile_audio_codec_blocklist_norm;

-- Rolled up from migrations/0126_webauthn_passkeys.sql
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

-- Rolled up from migrations/0127_user_external_accounts.sql
ALTER TABLE notification_channels
    ADD COLUMN media_server_connection_id TEXT;

CREATE TABLE media_server_connections (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    login_enabled INTEGER NOT NULL DEFAULT 0,
    linking_enabled INTEGER NOT NULL DEFAULT 0,
    auto_add_enabled INTEGER NOT NULL DEFAULT 0,
    default_app_permissions INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (provider IN ('jellyfin', 'plex', 'emby'))
);

CREATE TABLE jellyfin_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE plex_media_server_details (
    connection_id TEXT PRIMARY KEY,
    machine_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE emby_media_server_details (
    connection_id TEXT PRIMARY KEY,
    api_key TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE media_server_path_mappings (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    source_path TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE
);

CREATE TABLE media_server_default_library_grants (
    connection_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    permissions INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, library_id),
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

CREATE INDEX idx_media_server_connections_provider
    ON media_server_connections (provider, enabled);

CREATE INDEX idx_media_server_path_mappings_connection
    ON media_server_path_mappings (connection_id, sort_order);

INSERT OR IGNORE INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    trim(json_extract(connection.value, '$.id')),
    'jellyfin',
    COALESCE(
        NULLIF(trim(json_extract(connection.value, '$.displayName')), ''),
        NULLIF(trim(json_extract(connection.value, '$.id')), ''),
        'Jellyfin'
    ),
    rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(trim(allowed_provider.value)) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(trim(login_provider.value)) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(trim(linking_provider.value)) = 'jellyfin'
    ),
    0,
    0,
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.jellyfin.connections'
  AND connection.type = 'object'
  AND NULLIF(trim(json_extract(connection.value, '$.id')), '') IS NOT NULL
  AND NULLIF(rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'), '') IS NOT NULL;

INSERT OR IGNORE INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    trim(json_extract(connection.value, '$.id')),
    'plex',
    COALESCE(
        NULLIF(trim(json_extract(connection.value, '$.displayName')), ''),
        NULLIF(trim(json_extract(connection.value, '$.id')), ''),
        'Plex'
    ),
    COALESCE(NULLIF(rtrim(trim(json_extract(connection.value, '$.baseUrl')), '/'), ''), 'https://plex.tv'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(trim(allowed_provider.value)) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(trim(login_provider.value)) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(trim(linking_provider.value)) = 'plex'
    ),
    0,
    0,
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND connection.type = 'object'
  AND NULLIF(trim(json_extract(connection.value, '$.id')), '') IS NOT NULL;

INSERT OR IGNORE INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT id, NULL, created_at, updated_at
FROM media_server_connections
WHERE provider = 'jellyfin';

INSERT OR IGNORE INTO plex_media_server_details (connection_id, machine_id, created_at, updated_at)
SELECT
    trim(json_extract(connection.value, '$.id')),
    NULLIF(trim(json_extract(connection.value, '$.machineId')), ''),
    datetime('now'),
    datetime('now')
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
JOIN json_each(CASE WHEN json_valid(value.value_json) THEN value.value_json ELSE '[]' END) connection
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND connection.type = 'object'
  AND EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.id = trim(json_extract(connection.value, '$.id'))
        AND existing.provider = 'plex'
  );

UPDATE media_server_connections
SET
    enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    ),
    login_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND EXISTS (
            SELECT 1
            FROM settings_values login_value
            JOIN settings_definitions login_definition
              ON login_definition.id = login_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(login_value.value_json) THEN login_value.value_json ELSE '[]' END) login_provider
            WHERE login_definition.key_name = 'auth.providers.login_enabled'
              AND lower(trim(login_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    ),
    linking_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(allowed_value.value_json) THEN allowed_value.value_json ELSE '[]' END) allowed_provider
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(trim(allowed_provider.value)) = media_server_connections.provider
        )
        AND EXISTS (
            SELECT 1
            FROM settings_values linking_value
            JOIN settings_definitions linking_definition
              ON linking_definition.id = linking_value.setting_definition_id
            JOIN json_each(CASE WHEN json_valid(linking_value.value_json) THEN linking_value.value_json ELSE '[]' END) linking_provider
            WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
              AND lower(trim(linking_provider.value)) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(trim(allowed_id.value), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                JOIN json_each(CASE WHEN json_valid(ids_value.value_json) THEN ids_value.value_json ELSE '[]' END) allowed_id
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND trim(allowed_id.value) = media_server_connections.id
            )
        )
    )
WHERE provider IN ('jellyfin', 'plex');

INSERT OR IGNORE INTO media_server_connections (
    id,
    provider,
    display_name,
    base_url,
    enabled,
    login_enabled,
    linking_enabled,
    auto_add_enabled,
    default_app_permissions,
    created_at,
    updated_at
)
SELECT
    'jellyfin-notification-' || channel.id,
    'jellyfin',
    COALESCE(NULLIF(trim(channel.name), ''), 'Jellyfin notifications'),
    rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/'),
    channel.is_enabled,
    0,
    0,
    0,
    0,
    channel.created_at,
    channel.updated_at
FROM notification_channels channel
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NULL
  AND json_valid(channel.config_json)
  AND NULLIF(rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/'), '') IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.provider = 'jellyfin'
        AND existing.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
  );

INSERT OR IGNORE INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT
    connection.id,
    NULLIF(trim(json_extract(channel.config_json, '$.api_key')), ''),
    connection.created_at,
    connection.updated_at
FROM notification_channels channel
JOIN media_server_connections connection
  ON connection.id = 'jellyfin-notification-' || channel.id
WHERE channel.channel_type = 'jellyfin'
  AND json_valid(channel.config_json);

UPDATE jellyfin_media_server_details
SET api_key = (
    SELECT NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '')
    FROM notification_channels channel
    JOIN media_server_connections connection
      ON connection.provider = 'jellyfin'
     AND connection.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
    WHERE connection.id = jellyfin_media_server_details.connection_id
      AND channel.channel_type = 'jellyfin'
      AND json_valid(channel.config_json)
      AND NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '') IS NOT NULL
    LIMIT 1
)
WHERE api_key IS NULL
  AND EXISTS (
      SELECT 1
      FROM notification_channels channel
      JOIN media_server_connections connection
        ON connection.provider = 'jellyfin'
       AND connection.base_url = rtrim(trim(json_extract(channel.config_json, '$.base_url')), '/')
      WHERE connection.id = jellyfin_media_server_details.connection_id
        AND channel.channel_type = 'jellyfin'
        AND json_valid(channel.config_json)
        AND NULLIF(trim(json_extract(channel.config_json, '$.api_key')), '') IS NOT NULL
  );

UPDATE notification_channels
SET media_server_connection_id = (
    SELECT connection.id
    FROM media_server_connections connection
    WHERE connection.provider = 'jellyfin'
      AND connection.base_url = rtrim(trim(json_extract(notification_channels.config_json, '$.base_url')), '/')
      AND connection.id <> 'jellyfin-notification-' || notification_channels.id
    ORDER BY connection.id
    LIMIT 1
)
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND json_valid(config_json)
  AND NULLIF(rtrim(trim(json_extract(config_json, '$.base_url')), '/'), '') IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.provider = 'jellyfin'
        AND connection.base_url = rtrim(trim(json_extract(notification_channels.config_json, '$.base_url')), '/')
        AND connection.id <> 'jellyfin-notification-' || notification_channels.id
  );

UPDATE notification_channels
SET media_server_connection_id = 'jellyfin-notification-' || id
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.id = 'jellyfin-notification-' || notification_channels.id
  );

WITH RECURSIVE mapping_lines(channel_id, connection_id, remaining, line, sort_order) AS (
    SELECT
        id,
        media_server_connection_id,
        replace(json_extract(config_json, '$.path_mappings'), char(13), '') || char(10),
        '',
        0
    FROM notification_channels
    WHERE channel_type = 'jellyfin'
      AND media_server_connection_id IS NOT NULL
      AND json_valid(config_json)
      AND json_type(CASE WHEN json_valid(config_json) THEN config_json ELSE '{}' END, '$.path_mappings') = 'text'
      AND NULLIF(trim(json_extract(config_json, '$.path_mappings')), '') IS NOT NULL
    UNION ALL
    SELECT
        channel_id,
        connection_id,
        substr(remaining, instr(remaining, char(10)) + 1),
        trim(substr(remaining, 1, instr(remaining, char(10)) - 1)),
        sort_order + 1
    FROM mapping_lines
    WHERE remaining <> ''
      AND instr(remaining, char(10)) > 0
)
INSERT OR IGNORE INTO media_server_path_mappings (
    id,
    connection_id,
    source_path,
    destination_path,
    sort_order
)
SELECT
    'notification-path-mapping-' || channel_id || '-' || sort_order,
    connection_id,
    trim(substr(line, 1, instr(line, '=>') - 1)),
    trim(substr(line, instr(line, '=>') + 2)),
    sort_order - 1
FROM mapping_lines
WHERE line <> ''
  AND instr(line, '=>') > 0
  AND NULLIF(trim(substr(line, 1, instr(line, '=>') - 1)), '') IS NOT NULL
  AND NULLIF(trim(substr(line, instr(line, '=>') + 2)), '') IS NOT NULL;

CREATE TABLE user_external_accounts (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    external_user_id TEXT,
    username TEXT NOT NULL,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL,
    verified_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (connection_id) REFERENCES media_server_connections(id),
    CHECK (provider IN ('plex', 'jellyfin')),
    CHECK (status IN ('pending_claim', 'active', 'disabled'))
);

CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
    ON user_external_accounts (provider, connection_id, external_user_id);

CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
    ON user_external_accounts (provider, connection_id, LOWER(username))
    WHERE status = 'pending_claim' AND external_user_id IS NULL;

CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
    ON user_external_accounts (user_id, provider, connection_id);

CREATE INDEX idx_user_external_accounts_user_status
    ON user_external_accounts (user_id, status);

-- Rolled up from migrations/0128_episode_image_url.sql
ALTER TABLE episodes ADD COLUMN image_url TEXT;

-- Rolled up from migrations/0129_media_requests.sql
CREATE TABLE media_requests (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending'))
);

CREATE TABLE media_request_external_ids (
    request_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, source, external_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

CREATE TABLE media_request_requesters (
    request_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    PRIMARY KEY (request_id, user_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_request_external_ids_lookup
    ON media_request_external_ids (library_id, source, external_id);

CREATE INDEX idx_media_request_requesters_user
    ON media_request_requesters (user_id);

-- Rolled up from migrations/0130_external_account_last_login.sql
ALTER TABLE user_external_accounts
    ADD COLUMN last_login_at TEXT;

UPDATE user_external_accounts
   SET last_login_at = verified_at
 WHERE status = 'active'
   AND verified_at IS NOT NULL
   AND last_login_at IS NULL;

-- Rolled up from migrations/0131_totp_credentials.sql
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

-- Rolled up from migrations/0132_media_request_lifecycle_profiles.sql
PRAGMA foreign_keys = OFF;

CREATE TABLE media_requests_new (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    requested_quality_profile_id TEXT,
    requested_quality_profile_name TEXT,
    resolved_by_user_id TEXT,
    resolved_at TEXT,
    created_title_id TEXT,
    approved_quality_profile_id TEXT,
    approved_quality_profile_name TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending', 'approved', 'rejected'))
);

INSERT INTO media_requests_new (
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    created_by_user_id, created_at, updated_at
)
SELECT
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    created_by_user_id, created_at, updated_at
FROM media_requests;

DROP TABLE media_requests;
ALTER TABLE media_requests_new RENAME TO media_requests;

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);

PRAGMA foreign_keys = ON;

-- Rolled up from migrations/0133_media_request_monitor_type.sql
ALTER TABLE media_requests
    ADD COLUMN requested_monitor_type TEXT
        CHECK (
            requested_monitor_type IS NULL
            OR requested_monitor_type IN (
                'monitored',
                'unmonitored',
                'futureepisodes',
                'missingandfutureepisodes',
                'allepisodes',
                'none'
            )
        );

-- Rolled up from migrations/0134_media_request_canceled_status.sql
PRAGMA foreign_keys = OFF;

CREATE TABLE media_requests_new (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    requested_quality_profile_id TEXT,
    requested_quality_profile_name TEXT,
    requested_monitor_type TEXT
        CHECK (
            requested_monitor_type IS NULL
            OR requested_monitor_type IN (
                'monitored',
                'unmonitored',
                'futureepisodes',
                'missingandfutureepisodes',
                'allepisodes',
                'none'
            )
        ),
    resolved_by_user_id TEXT,
    resolved_at TEXT,
    created_title_id TEXT,
    approved_quality_profile_id TEXT,
    approved_quality_profile_name TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'))
);

INSERT INTO media_requests_new (
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at
)
SELECT
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at
FROM media_requests;

DROP TABLE media_requests;
ALTER TABLE media_requests_new RENAME TO media_requests;

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);

PRAGMA foreign_keys = ON;

-- Rolled up from migrations/0135_user_account_kind.sql
ALTER TABLE users
    ADD COLUMN account_kind TEXT NOT NULL DEFAULT 'local'
        CHECK (account_kind IN ('local', 'external_auto_provisioned'));

UPDATE users
   SET account_kind = 'external_auto_provisioned'
 WHERE password_hash IS NULL
   AND EXISTS (
       SELECT 1
         FROM user_external_accounts account
        WHERE account.user_id = users.id
          AND account.status = 'active'
   );

-- Rolled up from migrations/0136_notification_subscription_targets.sql
DROP INDEX IF EXISTS idx_notification_subscriptions_channel_scope;

CREATE TABLE notification_subscriptions_next (
    id TEXT PRIMARY KEY,
    channel_id TEXT,
    target_kind TEXT NOT NULL DEFAULT 'plugin_channel',
    target_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    scope TEXT NOT NULL,
    scope_id TEXT,
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (target_kind IN ('plugin_channel', 'media_server_connection')),
    FOREIGN KEY (channel_id) REFERENCES notification_channels(id) ON DELETE CASCADE
);

INSERT INTO notification_subscriptions_next (
    id,
    channel_id,
    target_kind,
    target_id,
    event_type,
    scope,
    scope_id,
    is_enabled,
    created_at,
    updated_at
)
SELECT
    id,
    channel_id,
    'plugin_channel',
    channel_id,
    event_type,
    scope,
    scope_id,
    is_enabled,
    created_at,
    updated_at
FROM notification_subscriptions;

DROP TABLE notification_subscriptions;

ALTER TABLE notification_subscriptions_next RENAME TO notification_subscriptions;
