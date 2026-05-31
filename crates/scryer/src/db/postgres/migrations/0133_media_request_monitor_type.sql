ALTER TABLE media_requests
    ADD COLUMN requested_monitor_type text,
    ADD CONSTRAINT media_requests_requested_monitor_type_check
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
        );
