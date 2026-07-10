-- RFC 119: the wanted-row search scheduler is retired. What to search is the
-- derived target set gated by the scope_indexer_coverage ledger; wanted_items
-- remains only as the per-scope acquisition-state ledger (status, scores,
-- grabs, pauses, last search attempt). The per-item cadence columns go away.
DROP INDEX IF EXISTS idx_wanted_items_next_search;

ALTER TABLE wanted_items DROP COLUMN IF EXISTS search_phase;
ALTER TABLE wanted_items DROP COLUMN IF EXISTS next_search_at;
ALTER TABLE wanted_items DROP COLUMN IF EXISTS search_count;
ALTER TABLE wanted_items DROP COLUMN IF EXISTS baseline_date;
