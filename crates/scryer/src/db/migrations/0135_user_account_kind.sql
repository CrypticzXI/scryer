ALTER TABLE users
    ADD COLUMN account_kind TEXT NOT NULL DEFAULT 'local'
        CHECK (account_kind IN ('local', 'external_auto_provisioned'));

UPDATE users
   SET account_kind = 'external_auto_provisioned'
 WHERE password_hash IS NULL
   AND EXISTS (
       SELECT 1
         FROM user_external_accounts account
        WHERE account.user_id = users.id
          AND account.status = 'active'
   );
