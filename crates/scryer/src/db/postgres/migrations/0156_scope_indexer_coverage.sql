-- RFC 119: per-(scope, indexer, fingerprint) active-search convergence ledger.
-- A row records that an acquisition scope's catalog was actively searched on an
-- indexer under a given search-criteria fingerprint. When every routed indexer
-- has a current-fingerprint row the scope is "converged" and drops to RSS-only.
CREATE TABLE IF NOT EXISTS scope_indexer_coverage (
    scope_key TEXT NOT NULL,
    facet TEXT NOT NULL,
    indexer_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    searched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (scope_key, facet, indexer_id)
);

CREATE INDEX IF NOT EXISTS idx_scope_indexer_coverage_indexer
    ON scope_indexer_coverage(indexer_id);

CREATE INDEX IF NOT EXISTS idx_scope_indexer_coverage_searched_at
    ON scope_indexer_coverage(searched_at);
