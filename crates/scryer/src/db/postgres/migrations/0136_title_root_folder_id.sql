ALTER TABLE titles ADD COLUMN root_folder_id text;

CREATE TEMP TABLE _scryer_0136_matched_root_tags ON COMMIT DROP AS
SELECT t.id AS title_id,
       tag.value AS tag_value,
       roots.id AS root_id,
       roots.is_default AS root_is_default
  FROM titles AS t
  JOIN jsonb_array_elements_text(COALESCE(t.tags, '[]'::jsonb)) AS tag(value)
    ON true
  JOIN library_roots roots
    ON roots.library_id = COALESCE(
           t.library_id,
           CASE t.facet
               WHEN 'movie' THEN 'movie_default_library'
               WHEN 'series' THEN 'series_default_library'
               WHEN 'anime' THEN 'anime_default_library'
               ELSE 'movie_default_library'
           END
       )
   AND roots.normalized_path = lower(regexp_replace(
           substr(tag.value, length('scryer:root-folder:') + 1),
           '/+$',
           ''
       ))
 WHERE tag.value LIKE 'scryer:root-folder:%';

UPDATE titles AS t
   SET root_folder_id = matched.root_id
  FROM (
       SELECT DISTINCT ON (title_id)
              title_id,
              root_id
         FROM _scryer_0136_matched_root_tags
        WHERE root_is_default = false
        ORDER BY title_id, root_id ASC
   ) AS matched
 WHERE t.id = matched.title_id
   AND t.root_folder_id IS NULL;

UPDATE titles AS t
   SET tags = COALESCE((
       SELECT jsonb_agg(tag.value)
         FROM jsonb_array_elements_text(COALESCE(t.tags, '[]'::jsonb)) AS tag(value)
        WHERE NOT EXISTS (
              SELECT 1
                FROM _scryer_0136_matched_root_tags matched
               WHERE matched.title_id = t.id
                 AND matched.tag_value = tag.value
        )
   ), '[]'::jsonb)
 WHERE EXISTS (
       SELECT 1
         FROM _scryer_0136_matched_root_tags matched
        WHERE matched.title_id = t.id
   );

DROP TABLE _scryer_0136_matched_root_tags;

CREATE INDEX idx_titles_root_folder_id
    ON titles(root_folder_id);
