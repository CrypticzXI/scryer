UPDATE media_files
SET video_codec = CASE
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
END;

UPDATE media_files
SET analysis_json = jsonb_set(
    analysis_json::jsonb,
    '{video_codec}',
    to_jsonb((
        CASE
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(analysis_json::jsonb ->> 'video_codec')), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
            ELSE analysis_json::jsonb ->> 'video_codec'
        END
    )::text),
    true
)::text
WHERE analysis_json IS NOT NULL
  AND BTRIM(analysis_json) <> ''
  AND jsonb_typeof(analysis_json::jsonb -> 'video_codec') = 'string';

CREATE TEMP TABLE quality_profile_video_codec_allowlist_norm AS
SELECT
    profile_id,
    canonical_codec AS codec,
    MIN(created_at) AS created_at
FROM (
    SELECT
        profile_id,
        created_at,
        CASE
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
            ELSE codec
        END AS canonical_codec
    FROM quality_profile_video_codec_allowlist
) normalized
GROUP BY profile_id, canonical_codec;

DELETE FROM quality_profile_video_codec_allowlist;

INSERT INTO quality_profile_video_codec_allowlist (profile_id, codec, created_at)
SELECT profile_id, codec, created_at
FROM quality_profile_video_codec_allowlist_norm
ORDER BY profile_id, codec;

DROP TABLE quality_profile_video_codec_allowlist_norm;

CREATE TEMP TABLE quality_profile_video_codec_blocklist_norm AS
SELECT
    profile_id,
    canonical_codec AS codec,
    MIN(created_at) AS created_at
FROM (
    SELECT
        profile_id,
        created_at,
        CASE
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AVC', 'AVC1', 'H264', 'X264') THEN 'H.264'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('HEVC', 'HEV1', 'H265', 'HVC1', 'X265') THEN 'H.265'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('AV1', 'AV01') THEN 'AV1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VP9' THEN 'VP9'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'VC1' THEN 'VC1'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'MPEG2' THEN 'MPEG2'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('MPEG4', 'MP4V') THEN 'MPEG4'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'XVID' THEN 'XVID'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') = 'DIVX' THEN 'DIVX'
            WHEN REPLACE(REPLACE(REPLACE(REPLACE(REPLACE(UPPER(BTRIM(codec)), '.', ''), '-', ''), '_', ''), ' ', ''), '/', '') IN ('VVC', 'H266') THEN 'VVC'
            ELSE codec
        END AS canonical_codec
    FROM quality_profile_video_codec_blocklist
) normalized
GROUP BY profile_id, canonical_codec;

DELETE FROM quality_profile_video_codec_blocklist;

INSERT INTO quality_profile_video_codec_blocklist (profile_id, codec, created_at)
SELECT profile_id, codec, created_at
FROM quality_profile_video_codec_blocklist_norm
ORDER BY profile_id, codec;

DROP TABLE quality_profile_video_codec_blocklist_norm;
