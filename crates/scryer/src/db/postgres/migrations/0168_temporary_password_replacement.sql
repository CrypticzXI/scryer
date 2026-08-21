-- Administrator-provided local passwords are temporary until the subscriber
-- replaces them after authenticating through the local password flow.
ALTER TABLE users
    ADD COLUMN password_change_required BOOLEAN NOT NULL DEFAULT FALSE;
