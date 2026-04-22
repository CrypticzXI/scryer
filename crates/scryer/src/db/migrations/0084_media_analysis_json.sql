ALTER TABLE media_files RENAME COLUMN ffprobe_json TO analysis_json;

UPDATE media_files
SET analysis_json = CASE
    WHEN analysis_json IS NULL
         AND audio_languages_json IS NULL
         AND audio_streams_json IS NULL
         AND subtitle_languages_json IS NULL
         AND subtitle_codecs_json IS NULL
         AND subtitle_streams_json IS NULL
         AND video_codec IS NULL
         AND video_width IS NULL
         AND video_height IS NULL
         AND video_bitrate_kbps IS NULL
         AND video_bit_depth IS NULL
         AND video_hdr_format IS NULL
         AND video_frame_rate IS NULL
         AND video_profile IS NULL
         AND audio_codec IS NULL
         AND audio_profile IS NULL
         AND audio_channels IS NULL
         AND audio_bitrate_kbps IS NULL
         AND duration_seconds IS NULL
         AND num_chapters IS NULL
         AND container_format IS NULL
    THEN NULL
    ELSE json_object(
        'video_codec', video_codec,
        'video_width', video_width,
        'video_height', video_height,
        'video_bitrate_kbps', video_bitrate_kbps,
        'video_bit_depth', video_bit_depth,
        'video_hdr_format', video_hdr_format,
        'video_frame_rate', video_frame_rate,
        'video_profile', video_profile,
        'audio_codec', audio_codec,
        'audio_profile', audio_profile,
        'audio_channels', audio_channels,
        'audio_bitrate_kbps', audio_bitrate_kbps,
        'audio_languages', json(COALESCE(audio_languages_json, '[]')),
        'audio_streams', json(COALESCE(audio_streams_json, '[]')),
        'subtitle_languages', json(COALESCE(subtitle_languages_json, '[]')),
        'subtitle_codecs', json(COALESCE(subtitle_codecs_json, '[]')),
        'subtitle_streams', json(COALESCE(subtitle_streams_json, '[]')),
        'has_multiaudio', json(CASE WHEN has_multiaudio != 0 THEN 'true' ELSE 'false' END),
        'duration_seconds', duration_seconds,
        'num_chapters', num_chapters,
        'container_format', container_format
    )
END;

ALTER TABLE media_files DROP COLUMN audio_languages_json;
ALTER TABLE media_files DROP COLUMN subtitle_languages_json;
ALTER TABLE media_files DROP COLUMN audio_streams_json;
ALTER TABLE media_files DROP COLUMN subtitle_codecs_json;
ALTER TABLE media_files DROP COLUMN subtitle_streams_json;
