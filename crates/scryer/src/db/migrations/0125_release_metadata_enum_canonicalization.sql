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
