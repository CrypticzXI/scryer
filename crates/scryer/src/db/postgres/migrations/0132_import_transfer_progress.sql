ALTER TABLE imports
    ADD COLUMN import_transfer_phase text;

ALTER TABLE imports
    ADD COLUMN import_transfer_bytes bigint;

ALTER TABLE imports
    ADD COLUMN import_transfer_total_bytes bigint;

ALTER TABLE imports
    ADD COLUMN import_transfer_started_at timestamp with time zone;

ALTER TABLE imports
    ADD COLUMN import_transfer_updated_at timestamp with time zone;
