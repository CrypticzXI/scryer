-- Remove historical imported episode grabs that were recorded with legacy
-- title-level scope. Keep the cleanup conservative: only series/anime rows with
-- no persisted episode/collection scope and clearly episode-shaped source titles
-- are pruned.
DELETE FROM download_submissions
WHERE COALESCE(tracked_state, '') = 'imported'
  AND COALESCE(title_id, '') <> ''
  AND episode_id IS NULL
  AND collection_id IS NULL
  AND lower(facet) IN ('series', 'anime')
  AND source_title IS NOT NULL
  AND (
    upper(source_title) GLOB '*S[0-9][0-9]E[0-9][0-9]*'
    OR upper(source_title) GLOB '*[0-9]X[0-9][0-9]*'
  );