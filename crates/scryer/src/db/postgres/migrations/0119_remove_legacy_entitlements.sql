DROP TABLE IF EXISTS user_entitlements;
DROP TABLE IF EXISTS entitlements;

ALTER TABLE users
    DROP COLUMN IF EXISTS entitlements;
