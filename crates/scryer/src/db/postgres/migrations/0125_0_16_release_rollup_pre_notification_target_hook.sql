-- Rolled up from postgres/migrations/0125_release_metadata_enum_canonicalization.sql
UPDATE media_files
SET source_type = CASE
    WHEN source_type IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source_type)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
    ELSE source_type
END,
video_codec = CASE
    WHEN video_codec IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
    ELSE video_codec
END,
video_codec_parsed = CASE
    WHEN video_codec_parsed IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(video_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
    ELSE video_codec_parsed
END,
audio_codec_parsed = CASE
    WHEN audio_codec_parsed IS NULL THEN NULL
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
    WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(audio_codec_parsed)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
    ELSE audio_codec_parsed
END;

CREATE TEMP TABLE quality_profile_source_allowlist_norm AS
SELECT profile_id, canonical_source AS source, MIN(created_at) AS created_at
FROM (
    SELECT profile_id, created_at, CASE
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
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
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBDL', 'WEB') THEN 'WEB-DL'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WEBRIP', 'WEBRI') THEN 'WEBRip'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BLURAY', 'BLU', 'BD', 'UHD', 'BDRIP', 'BRRIP', 'BDREMUX', 'BDRIO') THEN 'BluRay'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('BRDISK', 'BDMV', 'BDISO', 'BD25', 'BD50', 'BD66', 'BD100') THEN 'BRDISK'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVD', 'DVDRIP') THEN 'DVD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HDTV', 'RAWHD') THEN 'HDTV'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('CAM', 'HQCAM') THEN 'CAM'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELESYNC', 'TS') THEN 'TELESYNC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('TELECINE', 'TC') THEN 'TELECINE'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('DVDSCR', 'DVDSCREENER') THEN 'DVDSCR'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(source)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('WORKPRINT', 'WP') THEN 'WORKPRINT'
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
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
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
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DDP', 'DD+', 'DDPLUS', 'DOLBYDIGITALPLUS', 'DOLBYDIGITAL+') THEN 'DDP'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('EAC3', 'EAC', 'EC3') THEN 'EAC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AC3', 'DD', 'DOLBYDIGITAL') THEN 'AC3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('AAC', 'AACLC', 'HEAAC') THEN 'AAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('TRUEHD', 'DOLBYTRUEHD') THEN 'TRUEHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('DTSMA', 'DTSHDMA', 'DTSHDMASTER', 'DTSHDMASTERAUDIO') THEN 'DTSMA'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSX' THEN 'DTSX'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTSHD' THEN 'DTSHD'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'DTS' THEN 'DTS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'FLAC' THEN 'FLAC'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'OPUS' THEN 'OPUS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') = 'VORBIS' THEN 'VORBIS'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('MP3', 'MPEG3', 'MPEGAUDIOLAYER3') THEN 'MP3'
        WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), ':', '') IN ('PCM', 'LPCM') THEN 'PCM'
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

-- Rolled up from postgres/migrations/0126_webauthn_passkeys.sql
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

-- Rolled up from postgres/migrations/0127_user_external_accounts.sql
ALTER TABLE notification_channels
    ADD COLUMN IF NOT EXISTS media_server_connection_id text;

CREATE TABLE media_server_connections (
    id text PRIMARY KEY,
    provider text NOT NULL CHECK (provider IN ('jellyfin', 'plex', 'emby')),
    display_name text NOT NULL,
    base_url text NOT NULL,
    enabled boolean NOT NULL DEFAULT true,
    login_enabled boolean NOT NULL DEFAULT false,
    linking_enabled boolean NOT NULL DEFAULT false,
    auto_add_enabled boolean NOT NULL DEFAULT false,
    default_app_permissions bigint NOT NULL DEFAULT 0,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE jellyfin_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE plex_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    machine_id text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE emby_media_server_details (
    connection_id text PRIMARY KEY REFERENCES media_server_connections(id) ON DELETE CASCADE,
    api_key text,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE media_server_path_mappings (
    id text PRIMARY KEY,
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    source_path text NOT NULL,
    destination_path text NOT NULL,
    sort_order bigint NOT NULL DEFAULT 0
);

CREATE TABLE media_server_default_library_grants (
    connection_id text NOT NULL REFERENCES media_server_connections(id) ON DELETE CASCADE,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    permissions bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (connection_id, library_id)
);

CREATE INDEX idx_media_server_connections_provider
    ON media_server_connections (provider, enabled);

CREATE INDEX idx_media_server_path_mappings_connection
    ON media_server_path_mappings (connection_id, sort_order);

INSERT INTO media_server_connections (
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
    btrim(connection.value ->> 'id'),
    'jellyfin',
    COALESCE(
        NULLIF(btrim(connection.value ->> 'displayName'), ''),
        NULLIF(btrim(connection.value ->> 'id'), ''),
        'Jellyfin'
    ),
    rtrim(btrim(connection.value ->> 'baseUrl'), '/'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                THEN allowed_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS allowed_provider(value)
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(btrim(allowed_provider.value #>> '{}')) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = 'jellyfin'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = 'jellyfin'
    ),
    false,
    0,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.jellyfin.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND NULLIF(btrim(connection.value ->> 'id'), '') IS NOT NULL
  AND NULLIF(rtrim(btrim(connection.value ->> 'baseUrl'), '/'), '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

INSERT INTO media_server_connections (
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
    btrim(connection.value ->> 'id'),
    'plex',
    COALESCE(
        NULLIF(btrim(connection.value ->> 'displayName'), ''),
        NULLIF(btrim(connection.value ->> 'id'), ''),
        'Plex'
    ),
    COALESCE(NULLIF(rtrim(btrim(connection.value ->> 'baseUrl'), '/'), ''), 'https://plex.tv'),
    EXISTS (
        SELECT 1
        FROM settings_values allowed_value
        JOIN settings_definitions allowed_definition
          ON allowed_definition.id = allowed_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                THEN allowed_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS allowed_provider(value)
        WHERE allowed_definition.key_name = 'auth.providers.allowed'
          AND lower(btrim(allowed_provider.value #>> '{}')) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = 'plex'
    ),
    EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = 'plex'
    ),
    false,
    0,
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND NULLIF(btrim(connection.value ->> 'id'), '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT id, NULL, created_at, updated_at
FROM media_server_connections
WHERE provider = 'jellyfin'
ON CONFLICT (connection_id) DO NOTHING;

INSERT INTO plex_media_server_details (connection_id, machine_id, created_at, updated_at)
SELECT
    btrim(connection.value ->> 'id'),
    NULLIF(btrim(connection.value ->> 'machineId'), ''),
    CURRENT_TIMESTAMP,
    CURRENT_TIMESTAMP
FROM settings_values value
JOIN settings_definitions definition
  ON definition.id = value.setting_definition_id
CROSS JOIN LATERAL jsonb_array_elements(
    CASE
        WHEN jsonb_typeof(value.value_json::jsonb) = 'array'
        THEN value.value_json::jsonb
        ELSE '[]'::jsonb
    END
) AS connection(value)
WHERE definition.key_name = 'auth.providers.plex.connections'
  AND jsonb_typeof(connection.value) = 'object'
  AND EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.id = btrim(connection.value ->> 'id')
        AND existing.provider = 'plex'
  )
ON CONFLICT (connection_id) DO NOTHING;

UPDATE media_server_connections
SET
    enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values allowed_value
            JOIN settings_definitions allowed_definition
              ON allowed_definition.id = allowed_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(allowed_value.value_json::jsonb) = 'array'
                    THEN allowed_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS allowed_provider(value)
            WHERE allowed_definition.key_name = 'auth.providers.allowed'
              AND lower(btrim(allowed_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND (
            NOT EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                CROSS JOIN LATERAL jsonb_array_elements(
                    CASE
                        WHEN jsonb_typeof(ids_value.value_json::jsonb) = 'array'
                        THEN ids_value.value_json::jsonb
                        ELSE '[]'::jsonb
                    END
                ) AS allowed_id(value)
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND NULLIF(btrim(allowed_id.value #>> '{}'), '') IS NOT NULL
            )
            OR EXISTS (
                SELECT 1
                FROM settings_values ids_value
                JOIN settings_definitions ids_definition
                  ON ids_definition.id = ids_value.setting_definition_id
                CROSS JOIN LATERAL jsonb_array_elements(
                    CASE
                        WHEN jsonb_typeof(ids_value.value_json::jsonb) = 'array'
                        THEN ids_value.value_json::jsonb
                        ELSE '[]'::jsonb
                    END
                ) AS allowed_id(value)
                WHERE ids_definition.key_name = CASE media_server_connections.provider
                    WHEN 'jellyfin' THEN 'auth.providers.jellyfin.allowed_connection_ids'
                    WHEN 'plex' THEN 'auth.providers.plex.allowed_connection_ids'
                END
                  AND btrim(allowed_id.value #>> '{}') = media_server_connections.id
            )
        )
    ),
    login_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values login_value
            JOIN settings_definitions login_definition
              ON login_definition.id = login_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                    THEN login_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS login_provider(value)
            WHERE login_definition.key_name = 'auth.providers.login_enabled'
              AND lower(btrim(login_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND enabled
    ),
    linking_enabled = (
        EXISTS (
            SELECT 1
            FROM settings_values linking_value
            JOIN settings_definitions linking_definition
              ON linking_definition.id = linking_value.setting_definition_id
            CROSS JOIN LATERAL jsonb_array_elements(
                CASE
                    WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                    THEN linking_value.value_json::jsonb
                    ELSE '[]'::jsonb
                END
            ) AS linking_provider(value)
            WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
              AND lower(btrim(linking_provider.value #>> '{}')) = media_server_connections.provider
        )
        AND enabled
    )
WHERE provider IN ('jellyfin', 'plex');

UPDATE media_server_connections
SET
    login_enabled = enabled AND EXISTS (
        SELECT 1
        FROM settings_values login_value
        JOIN settings_definitions login_definition
          ON login_definition.id = login_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(login_value.value_json::jsonb) = 'array'
                THEN login_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS login_provider(value)
        WHERE login_definition.key_name = 'auth.providers.login_enabled'
          AND lower(btrim(login_provider.value #>> '{}')) = media_server_connections.provider
    ),
    linking_enabled = enabled AND EXISTS (
        SELECT 1
        FROM settings_values linking_value
        JOIN settings_definitions linking_definition
          ON linking_definition.id = linking_value.setting_definition_id
        CROSS JOIN LATERAL jsonb_array_elements(
            CASE
                WHEN jsonb_typeof(linking_value.value_json::jsonb) = 'array'
                THEN linking_value.value_json::jsonb
                ELSE '[]'::jsonb
            END
        ) AS linking_provider(value)
        WHERE linking_definition.key_name = 'auth.providers.linking_enabled'
          AND lower(btrim(linking_provider.value #>> '{}')) = media_server_connections.provider
    )
WHERE provider IN ('jellyfin', 'plex');

CREATE OR REPLACE FUNCTION pg_temp.scryer_try_jsonb(value text)
RETURNS jsonb
LANGUAGE plpgsql
AS $$
BEGIN
    RETURN value::jsonb;
EXCEPTION WHEN others THEN
    RETURN NULL;
END
$$;

INSERT INTO media_server_connections (
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
    COALESCE(NULLIF(btrim(channel.name), ''), 'Jellyfin notifications'),
    rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/'),
    channel.is_enabled,
    false,
    false,
    false,
    0,
    channel.created_at,
    channel.updated_at
FROM notification_channels channel
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NULL
  AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
  AND NULLIF(rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/'), '') IS NOT NULL
  AND NOT EXISTS (
      SELECT 1
      FROM media_server_connections existing
      WHERE existing.provider = 'jellyfin'
        AND existing.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
  )
ON CONFLICT (id) DO NOTHING;

INSERT INTO jellyfin_media_server_details (connection_id, api_key, created_at, updated_at)
SELECT
    connection.id,
    NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), ''),
    connection.created_at,
    connection.updated_at
FROM notification_channels channel
JOIN media_server_connections connection
  ON connection.id = 'jellyfin-notification-' || channel.id
WHERE channel.channel_type = 'jellyfin'
ON CONFLICT (connection_id) DO NOTHING;

UPDATE jellyfin_media_server_details
SET api_key = (
    SELECT NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '')
    FROM notification_channels channel
    JOIN media_server_connections connection
      ON connection.provider = 'jellyfin'
     AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
    WHERE connection.id = jellyfin_media_server_details.connection_id
      AND channel.channel_type = 'jellyfin'
      AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
      AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '') IS NOT NULL
    LIMIT 1
)
WHERE api_key IS NULL
  AND EXISTS (
      SELECT 1
      FROM notification_channels channel
      JOIN media_server_connections connection
        ON connection.provider = 'jellyfin'
       AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'base_url'), '/')
      WHERE connection.id = jellyfin_media_server_details.connection_id
        AND channel.channel_type = 'jellyfin'
        AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
        AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'api_key'), '') IS NOT NULL
  );

UPDATE notification_channels
SET media_server_connection_id = (
    SELECT connection.id
    FROM media_server_connections connection
    WHERE connection.provider = 'jellyfin'
      AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(notification_channels.config_json) ->> 'base_url'), '/')
      AND connection.id <> 'jellyfin-notification-' || notification_channels.id
    ORDER BY connection.id
    LIMIT 1
)
WHERE channel_type = 'jellyfin'
  AND media_server_connection_id IS NULL
  AND pg_temp.scryer_try_jsonb(config_json) IS NOT NULL
  AND NULLIF(rtrim(btrim(pg_temp.scryer_try_jsonb(config_json) ->> 'base_url'), '/'), '') IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM media_server_connections connection
      WHERE connection.provider = 'jellyfin'
        AND connection.base_url = rtrim(btrim(pg_temp.scryer_try_jsonb(notification_channels.config_json) ->> 'base_url'), '/')
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

INSERT INTO media_server_path_mappings (
    id,
    connection_id,
    source_path,
    destination_path,
    sort_order
)
SELECT
    'notification-path-mapping-' || channel.id || '-' || mapping.ordinality,
    channel.media_server_connection_id,
    parsed.source_path,
    parsed.destination_path,
    mapping.ordinality - 1
FROM notification_channels channel
CROSS JOIN LATERAL regexp_split_to_table(
    replace(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'path_mappings', E'\r', ''),
    E'\n'
) WITH ORDINALITY AS mapping(line, ordinality)
CROSS JOIN LATERAL (
    SELECT
        btrim(substr(mapping.line, 1, position('=>' in mapping.line) - 1)) AS source_path,
        btrim(substr(mapping.line, position('=>' in mapping.line) + 2)) AS destination_path
) AS parsed
WHERE channel.channel_type = 'jellyfin'
  AND channel.media_server_connection_id IS NOT NULL
  AND pg_temp.scryer_try_jsonb(channel.config_json) IS NOT NULL
  AND NULLIF(btrim(pg_temp.scryer_try_jsonb(channel.config_json) ->> 'path_mappings'), '') IS NOT NULL
  AND position('=>' in mapping.line) > 0
  AND NULLIF(parsed.source_path, '') IS NOT NULL
  AND NULLIF(parsed.destination_path, '') IS NOT NULL
ON CONFLICT (id) DO NOTHING;

CREATE TABLE user_external_accounts (
    id text PRIMARY KEY,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider text NOT NULL CHECK (provider IN ('plex', 'jellyfin')),
    connection_id text NOT NULL REFERENCES media_server_connections(id),
    external_user_id text,
    username text NOT NULL,
    display_name text,
    avatar_url text,
    status text NOT NULL CHECK (status IN ('pending_claim', 'active', 'disabled')),
    verified_at timestamp with time zone,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
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

-- Rolled up from postgres/migrations/0128_episode_image_url.sql
ALTER TABLE episodes ADD COLUMN image_url text;

-- Rolled up from postgres/migrations/0129_media_requests.sql
CREATE TABLE media_requests (
    id text PRIMARY KEY,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    facet text NOT NULL CHECK (facet IN ('movie', 'series', 'anime')),
    status text NOT NULL CHECK (status IN ('pending')),
    identity_fingerprint text NOT NULL,
    title text NOT NULL,
    sort_title text,
    slug text,
    poster_url text,
    year integer,
    overview text,
    runtime_minutes integer,
    language text,
    content_status text,
    created_by_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE media_request_external_ids (
    request_id text NOT NULL REFERENCES media_requests(id) ON DELETE CASCADE,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    PRIMARY KEY (request_id, source, external_id)
);

CREATE TABLE media_request_requesters (
    request_id text NOT NULL REFERENCES media_requests(id) ON DELETE CASCADE,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_at timestamp with time zone NOT NULL,
    PRIMARY KEY (request_id, user_id)
);

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_request_external_ids_lookup
    ON media_request_external_ids (library_id, source, external_id);

CREATE INDEX idx_media_request_requesters_user
    ON media_request_requesters (user_id);

-- Rolled up from postgres/migrations/0130_external_account_last_login.sql
ALTER TABLE user_external_accounts
    ADD COLUMN last_login_at timestamp with time zone;

UPDATE user_external_accounts
   SET last_login_at = verified_at
 WHERE status = 'active'
   AND verified_at IS NOT NULL
   AND last_login_at IS NULL;

-- Rolled up from postgres/migrations/0131_totp_credentials.sql
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

-- Rolled up from postgres/migrations/0132_media_request_lifecycle_profiles.sql
ALTER TABLE media_requests
    DROP CONSTRAINT IF EXISTS media_requests_status_check;

ALTER TABLE media_requests
    ADD COLUMN requested_quality_profile_id text,
    ADD COLUMN requested_quality_profile_name text,
    ADD COLUMN resolved_by_user_id text REFERENCES users(id) ON DELETE SET NULL,
    ADD COLUMN resolved_at timestamp with time zone,
    ADD COLUMN created_title_id text REFERENCES titles(id) ON DELETE SET NULL,
    ADD COLUMN approved_quality_profile_id text,
    ADD COLUMN approved_quality_profile_name text,
    ADD CONSTRAINT media_requests_status_check CHECK (status IN ('pending', 'approved', 'rejected'));

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);

-- Rolled up from postgres/migrations/0133_media_request_monitor_type.sql
ALTER TABLE media_requests
    ADD COLUMN requested_monitor_type text,
    ADD CONSTRAINT media_requests_requested_monitor_type_check
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

-- Rolled up from postgres/migrations/0134_media_request_canceled_status.sql
ALTER TABLE media_requests
    DROP CONSTRAINT IF EXISTS media_requests_status_check;

ALTER TABLE media_requests
    ADD CONSTRAINT media_requests_status_check
        CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'));

-- Rolled up from postgres/migrations/0135_user_account_kind.sql
ALTER TABLE users
    ADD COLUMN account_kind text NOT NULL DEFAULT 'local',
    ADD CONSTRAINT users_account_kind_check
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

-- Rolled up from postgres/migrations/0136_notification_subscription_targets.sql
DROP INDEX IF EXISTS idx_notification_subscriptions_channel_scope;

ALTER TABLE notification_subscriptions
    ADD COLUMN target_kind TEXT NOT NULL DEFAULT 'plugin_channel';

ALTER TABLE notification_subscriptions
    ADD COLUMN target_id TEXT;

UPDATE notification_subscriptions
   SET target_id = channel_id
 WHERE target_id IS NULL;

ALTER TABLE notification_subscriptions
    ALTER COLUMN target_id SET NOT NULL;

ALTER TABLE notification_subscriptions
    ALTER COLUMN channel_id DROP NOT NULL;

ALTER TABLE notification_subscriptions
    ADD CONSTRAINT notification_subscriptions_target_kind_check
    CHECK (target_kind IN ('plugin_channel', 'media_server_connection'));
