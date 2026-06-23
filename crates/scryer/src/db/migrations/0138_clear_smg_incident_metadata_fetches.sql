UPDATE titles
   SET metadata_fetched_at = NULL
 WHERE metadata_fetched_at IS NOT NULL
   AND datetime(metadata_fetched_at) >= datetime('2026-06-21T15:41:13Z')
   AND datetime(metadata_fetched_at) < datetime('2026-06-22T22:00:39Z');
