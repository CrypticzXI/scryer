ALTER TABLE media_files
    ADD COLUMN role TEXT NOT NULL DEFAULT 'primary';

ALTER TABLE download_submissions
    ADD COLUMN purpose TEXT NOT NULL DEFAULT 'standard';
