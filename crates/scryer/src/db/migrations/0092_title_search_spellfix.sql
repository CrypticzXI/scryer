CREATE TABLE IF NOT EXISTS title_search_terms (
    term_id INTEGER PRIMARY KEY,
    title_id TEXT NOT NULL REFERENCES titles(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    term_kind TEXT NOT NULL,
    raw_term TEXT NOT NULL,
    normalized_term TEXT NOT NULL,
    weight INTEGER NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_title_search_terms_title_kind_normalized
    ON title_search_terms(title_id, term_kind, normalized_term);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_title_id
    ON title_search_terms(title_id);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_facet_normalized_term
    ON title_search_terms(facet, normalized_term);

CREATE INDEX IF NOT EXISTS idx_title_search_terms_normalized_term
    ON title_search_terms(normalized_term);

CREATE VIRTUAL TABLE IF NOT EXISTS title_search_spellfix USING spellfix1;
