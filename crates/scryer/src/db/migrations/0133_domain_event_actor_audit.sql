ALTER TABLE domain_events
    ADD COLUMN actor_kind TEXT NOT NULL DEFAULT 'system';

ALTER TABLE domain_events
    ADD COLUMN actor_display_name TEXT NOT NULL DEFAULT 'System';

UPDATE domain_events
SET actor_kind = CASE
        WHEN actor_user_id IS NULL THEN 'system'
        ELSE 'user'
    END,
    actor_display_name = CASE
        WHEN actor_user_id IS NULL THEN 'System'
        ELSE COALESCE(
            (
                SELECT COALESCE(NULLIF(users.display_name, ''), users.username)
                FROM users
                WHERE users.id = domain_events.actor_user_id
            ),
            actor_user_id,
            'System'
        )
    END;
