UPDATE titles
   SET metadata_fetched_at = NULL
 WHERE metadata_fetched_at IS NOT NULL
   AND metadata_fetched_at >= TIMESTAMPTZ '2026-06-21T15:41:13Z'
   AND metadata_fetched_at < TIMESTAMPTZ '2026-06-22T22:00:39Z';
