ALTER TABLE file_episode_map
ADD COLUMN role TEXT NOT NULL DEFAULT 'additional'
    CHECK (role IN ('primary', 'additional'));

UPDATE file_episode_map
SET role = CASE
    WHEN file_id = (
        SELECT candidate.file_id
        FROM file_episode_map AS candidate
        INNER JOIN media_files AS media_file ON media_file.id = candidate.file_id
        WHERE candidate.episode_id = file_episode_map.episode_id
        ORDER BY
            CASE WHEN media_file.role = 'primary' THEN 0 ELSE 1 END,
            media_file.created_at DESC,
            candidate.file_id ASC
        LIMIT 1
    ) THEN 'primary'
    ELSE 'additional'
END;

CREATE UNIQUE INDEX idx_file_episode_map_one_primary_per_episode
ON file_episode_map (episode_id)
WHERE role = 'primary';
