ALTER TABLE user_external_accounts
    ADD COLUMN last_login_at timestamp with time zone;

UPDATE user_external_accounts
   SET last_login_at = verified_at
 WHERE status = 'active'
   AND verified_at IS NOT NULL
   AND last_login_at IS NULL;
