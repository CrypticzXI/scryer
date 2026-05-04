-- Reset legacy failed-download blocklist state so release-name semantics can
-- start fresh with source_title as the canonical failed-release identity.

DELETE FROM blocklist;

DELETE FROM release_download_attempts
WHERE outcome = 'failed';

DELETE FROM download_submissions
WHERE COALESCE(tracked_state, '') = 'failed'
  AND COALESCE(title_id, '') = ''
  AND COALESCE(facet, '') = ''
  AND COALESCE(TRIM(COALESCE(source_title, '')), '') = ''
  AND COALESCE(TRIM(COALESCE(source_hint, '')), '') = ''
  AND COALESCE(TRIM(COALESCE(request_signature, '')), '') = ''
  AND episode_id IS NULL
  AND collection_id IS NULL;