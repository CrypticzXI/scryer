-- SQLite already maintains the richer title_search_terms projection in the
-- historical migration path. This step exists so migration 0106 explicitly
-- treats SQLite while the PostgreSQL step brings its baseline table to parity.

