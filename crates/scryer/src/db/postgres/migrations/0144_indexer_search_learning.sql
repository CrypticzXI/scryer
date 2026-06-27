CREATE TABLE IF NOT EXISTS indexer_search_learning (
    indexer_id TEXT NOT NULL,
    title_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    strategy_key TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    empty_successes INTEGER NOT NULL DEFAULT 0,
    usable_successes INTEGER NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ,
    last_usable_at TIMESTAMPTZ,
    suppressed BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
);

CREATE INDEX IF NOT EXISTS idx_indexer_search_learning_title
    ON indexer_search_learning (indexer_id, title_id, facet);
