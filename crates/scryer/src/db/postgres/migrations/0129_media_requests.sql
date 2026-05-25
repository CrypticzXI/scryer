CREATE TABLE media_requests (
    id text PRIMARY KEY,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    facet text NOT NULL CHECK (facet IN ('movie', 'series', 'anime')),
    status text NOT NULL CHECK (status IN ('pending')),
    identity_fingerprint text NOT NULL,
    title text NOT NULL,
    sort_title text,
    slug text,
    poster_url text,
    year integer,
    overview text,
    runtime_minutes integer,
    language text,
    content_status text,
    created_by_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamp with time zone NOT NULL,
    updated_at timestamp with time zone NOT NULL
);

CREATE TABLE media_request_external_ids (
    request_id text NOT NULL REFERENCES media_requests(id) ON DELETE CASCADE,
    library_id text NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    source text NOT NULL,
    external_id text NOT NULL,
    created_at timestamp with time zone NOT NULL,
    PRIMARY KEY (request_id, source, external_id)
);

CREATE TABLE media_request_requesters (
    request_id text NOT NULL REFERENCES media_requests(id) ON DELETE CASCADE,
    user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_at timestamp with time zone NOT NULL,
    PRIMARY KEY (request_id, user_id)
);

CREATE INDEX idx_media_requests_library_facet_status
    ON media_requests (library_id, facet, status);

CREATE INDEX idx_media_requests_status_updated
    ON media_requests (status, updated_at);

CREATE INDEX idx_media_request_external_ids_lookup
    ON media_request_external_ids (library_id, source, external_id);

CREATE INDEX idx_media_request_requesters_user
    ON media_request_requesters (user_id);
