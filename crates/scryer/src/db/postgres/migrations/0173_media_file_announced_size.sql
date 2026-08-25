-- Option c of the grab-vs-import size decision: when a landed file is within the
-- normal transfer overhead of what its release announced (landed >= 0.85 x
-- announced), the import scores the size term on the announced size, so grab
-- and import agree. The incumbent bar is re-derived from the stored row (I7),
-- so the row has to remember the announced size or the bar could not reproduce
-- the import score. Nullable, no backfill: a row without it scores on its real
-- size, exactly as before.
ALTER TABLE media_files
    ADD COLUMN IF NOT EXISTS announced_size_bytes BIGINT;
