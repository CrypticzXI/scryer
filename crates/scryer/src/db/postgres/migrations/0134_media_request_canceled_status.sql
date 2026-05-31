ALTER TABLE media_requests
    DROP CONSTRAINT IF EXISTS media_requests_status_check;

ALTER TABLE media_requests
    ADD CONSTRAINT media_requests_status_check
        CHECK (status IN ('pending', 'approved', 'rejected', 'canceled'));
