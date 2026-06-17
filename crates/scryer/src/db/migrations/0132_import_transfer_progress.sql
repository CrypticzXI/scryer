ALTER TABLE imports
    ADD COLUMN import_transfer_phase TEXT;

ALTER TABLE imports
    ADD COLUMN import_transfer_bytes INTEGER;

ALTER TABLE imports
    ADD COLUMN import_transfer_total_bytes INTEGER;

ALTER TABLE imports
    ADD COLUMN import_transfer_started_at TEXT;

ALTER TABLE imports
    ADD COLUMN import_transfer_updated_at TEXT;
