ALTER TABLE settings_values
    DROP CONSTRAINT IF EXISTS settings_values_setting_definition_id_fkey;

UPDATE settings_values
SET setting_definition_id = settings_definitions.category || ':' || settings_definitions.scope || ':' || settings_definitions.key_name
FROM settings_definitions
WHERE settings_values.setting_definition_id = settings_definitions.id;

UPDATE settings_definitions
SET id = category || ':' || scope || ':' || key_name;

ALTER TABLE settings_values
    ADD CONSTRAINT settings_values_setting_definition_id_fkey
    FOREIGN KEY (setting_definition_id) REFERENCES settings_definitions(id) ON DELETE CASCADE;

DROP TABLE IF EXISTS user_entitlements;
DROP TABLE IF EXISTS entitlements;

CREATE TABLE entitlements (
    code TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    category TEXT NOT NULL
);

INSERT INTO entitlements (code, description, category)
VALUES
    ('manage_config', 'Manage instance configuration', 'system'),
    ('manage_title', 'Create and edit catalog entities', 'media'),
    ('manage_users', 'Manage users and security settings', 'system'),
    ('view_catalog', 'Read access to title and media catalog', 'media')
ON CONFLICT (code) DO NOTHING;

CREATE TABLE user_entitlements (
    user_id TEXT NOT NULL,
    entitlement_code TEXT NOT NULL,
    granted_by_user_id TEXT,
    granted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, entitlement_code),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
    FOREIGN KEY (entitlement_code) REFERENCES entitlements(code) ON DELETE CASCADE,
    FOREIGN KEY (granted_by_user_id) REFERENCES users(id) ON DELETE SET NULL
);

INSERT INTO user_entitlements (user_id, entitlement_code, granted_by_user_id, granted_at, expires_at)
SELECT users.id, entitlement_codes.code, NULL, NOW(), NULL
FROM users
CROSS JOIN LATERAL jsonb_array_elements_text(users.entitlements) AS entitlement_codes(code)
JOIN entitlements ON entitlements.code = entitlement_codes.code
ON CONFLICT (user_id, entitlement_code) DO NOTHING;

CREATE INDEX IF NOT EXISTS idx_user_entitlements_user
    ON user_entitlements (user_id);

DROP TABLE IF EXISTS quality_profile_quality_tiers;

CREATE TABLE quality_profile_quality_tiers (
    profile_id TEXT NOT NULL,
    quality_tier TEXT NOT NULL,
    sort_order BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (profile_id, quality_tier),
    FOREIGN KEY (profile_id) REFERENCES quality_profiles(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_quality_profile_quality_tiers_profile
    ON quality_profile_quality_tiers (profile_id, sort_order);

ALTER TABLE titles
    ALTER COLUMN updated_at DROP NOT NULL;

ALTER TABLE title_external_ids
    ALTER COLUMN facet DROP NOT NULL,
    ALTER COLUMN provenance SET DEFAULT 'metadata',
    ALTER COLUMN source_scope SET DEFAULT '',
    ALTER COLUMN updated_at DROP NOT NULL;
