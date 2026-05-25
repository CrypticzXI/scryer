CREATE TABLE media_requests (
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
    created_by_user_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE,
    FOREIGN KEY (created_by_user_id) REFERENCES users(id) ON DELETE CASCADE,
    CHECK (facet IN ('movie', 'series', 'anime')),
    CHECK (status IN ('pending'))
);

CREATE TABLE media_request_external_ids (
    request_id TEXT NOT NULL,
    library_id TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (request_id, source, external_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (library_id) REFERENCES libraries(id) ON DELETE CASCADE
);

CREATE TABLE media_request_requesters (
    request_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    PRIMARY KEY (request_id, user_id),
    FOREIGN KEY (request_id) REFERENCES media_requests(id) ON DELETE CASCADE,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_request_external_ids_lookup
    ON media_request_external_ids (library_id, source, external_id);

CREATE INDEX idx_media_request_requesters_user
    ON media_request_requesters (user_id);
