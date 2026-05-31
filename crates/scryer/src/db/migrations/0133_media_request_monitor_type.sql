ALTER TABLE media_requests
    ADD COLUMN requested_monitor_type TEXT
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
