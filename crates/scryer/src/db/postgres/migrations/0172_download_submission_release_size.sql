-- D18 needs the size a queued release announced, so an in-flight submission can
-- be scored the way the candidate beside it is scored. Without it the queued
-- pseudo-incumbent carries no size term and any candidate in a larger size band
-- reads as an upgrade over an identical release already downloading.
-- Nullable, no backfill: a row written before this column compares size-less.
ALTER TABLE download_submissions
    ADD COLUMN IF NOT EXISTS release_size_bytes BIGINT;
