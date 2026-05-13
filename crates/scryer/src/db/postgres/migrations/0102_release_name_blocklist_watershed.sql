DELETE FROM blocklist;

DELETE FROM release_download_attempts
WHERE outcome = 'failed';

DELETE FROM download_submissions
WHERE COALESCE(tracked_state, '') = 'failed'
  AND COALESCE(title_id, '') = ''
  AND COALESCE(facet, '') = ''
  AND COALESCE(BTRIM(COALESCE(source_title, '')), '') = ''
  AND COALESCE(BTRIM(COALESCE(source_hint, '')), '') = ''
  AND COALESCE(BTRIM(COALESCE(request_signature, '')), '') = ''
  AND episode_id IS NULL
  AND collection_id IS NULL;
