ALTER TABLE titles ADD COLUMN root_folder_id TEXT;

CREATE TEMP TABLE _scryer_0136_matched_root_tags (
    title_id TEXT NOT NULL,
    tag_value TEXT NOT NULL,
    root_id TEXT NOT NULL,
    root_is_default INTEGER NOT NULL
);

INSERT INTO _scryer_0136_matched_root_tags (
    title_id,
    tag_value,
    root_id,
    root_is_default
)
SELECT titles.id,
       CAST(tag.value AS TEXT),
       roots.id,
       roots.is_default
  FROM titles
  JOIN json_each(
       CASE
           WHEN json_valid(COALESCE(titles.tags, '[]')) THEN COALESCE(titles.tags, '[]')
           ELSE '[]'
       END
   ) AS tag
  JOIN library_roots roots
    ON roots.library_id = COALESCE(
           titles.library_id,
           CASE titles.facet
               WHEN 'movie' THEN 'movie_default_library'
               WHEN 'series' THEN 'series_default_library'
               WHEN 'anime' THEN 'anime_default_library'
               ELSE 'movie_default_library'
           END
       )
   AND roots.normalized_path = lower(rtrim(
           substr(CAST(tag.value AS TEXT), length('scryer:root-folder:') + 1),
           '/'
       ))
 WHERE CAST(tag.value AS TEXT) LIKE 'scryer:root-folder:%';

UPDATE titles
   SET root_folder_id = (
       SELECT root_id
         FROM _scryer_0136_matched_root_tags matched
        WHERE matched.title_id = titles.id
          AND matched.root_is_default = 0
        ORDER BY root_id ASC
        LIMIT 1
   )
 WHERE root_folder_id IS NULL
   AND EXISTS (
       SELECT 1
         FROM _scryer_0136_matched_root_tags matched
        WHERE matched.title_id = titles.id
          AND matched.root_is_default = 0
   );

UPDATE titles
   SET tags = COALESCE((
       SELECT json_group_array(tag.value)
         FROM json_each(
              CASE
                  WHEN json_valid(COALESCE(titles.tags, '[]')) THEN COALESCE(titles.tags, '[]')
                  ELSE '[]'
              END
          ) AS tag
        WHERE NOT EXISTS (
              SELECT 1
                FROM _scryer_0136_matched_root_tags matched
               WHERE matched.title_id = titles.id
                 AND matched.tag_value = CAST(tag.value AS TEXT)
        )
   ), '[]')
 WHERE EXISTS (
       SELECT 1
         FROM _scryer_0136_matched_root_tags matched
        WHERE matched.title_id = titles.id
   );

DROP TABLE _scryer_0136_matched_root_tags;

CREATE INDEX idx_titles_root_folder_id
    ON titles(root_folder_id);
