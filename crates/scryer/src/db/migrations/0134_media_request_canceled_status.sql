PRAGMA foreign_keys = OFF;

CREATE TABLE media_requests_new (
    id TEXT PRIMARY KEY,
    library_id TEXT NOT NULL,
    facet TEXT NOT NULL,
    status TEXT NOT NULL,
    identity_fingerprint TEXT NOT NULL,
    title TEXT NOT NULL,
    sort_title TEXT,
    slug TEXT,
    poster_url TEXT,
    year INTEGER,
    overview TEXT,
    runtime_minutes INTEGER,
    language TEXT,
    content_status TEXT,
    requested_quality_profile_id TEXT,
    requested_quality_profile_name TEXT,
    requested_monitor_type TEXT
        CHECK (
            requested_monitor_type IS NULL
            OR requested_monitor_type IN (
                'monitored',
                'unmonitored',
                'futureepisodes',
                'missingandfutureepisodes',
                'allepisodes',
                'none'
            )
        ),
    resolved_by_user_id TEXT,
    resolved_at TEXT,
    created_title_id TEXT,
    approved_quality_profile_id TEXT,
    approved_quality_profile_name TEXT,
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (resolved_by_user_id) REFERENCES users(id) ON DELETE SET NULL,
    FOREIGN KEY (created_title_id) REFERENCES titles(id) ON DELETE SET NULL,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'))
);

INSERT INTO media_requests_new (
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at
)
SELECT
    id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
    poster_url, year, overview, runtime_minutes, language, content_status,
    requested_quality_profile_id, requested_quality_profile_name, requested_monitor_type,
    resolved_by_user_id, resolved_at, created_title_id,
    approved_quality_profile_id, approved_quality_profile_name,
    created_by_user_id, created_at, updated_at
FROM media_requests;

DROP TABLE media_requests;
ALTER TABLE media_requests_new RENAME TO media_requests;

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_requests_created_title
    ON media_requests (created_title_id);

PRAGMA foreign_keys = ON;
