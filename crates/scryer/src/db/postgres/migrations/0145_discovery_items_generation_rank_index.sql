-- RFC 121 SW4.4: fetch_personalized_items filters discovery_items by
-- (base_generation_id, tombstoned_at IS NULL, owned_in_input = FALSE). Index that
-- predicate so the hottest personalized-home read seeks the candidate set instead
-- of scanning the table. rank_score is deliberately NOT indexed: every caller
-- orders by COALESCE(rank_score, ...) plus joined discovery_titles columns, which
-- no discovery_items index can serve — the final sort stays, only smaller.
CREATE INDEX IF NOT EXISTS idx_discovery_items_generation_rank
    ON discovery_items USING btree (base_generation_id, tombstoned_at, owned_in_input);
