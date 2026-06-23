ALTER TABLE titles
    ALTER COLUMN root_folder_id SET NOT NULL;

CREATE INDEX idx_titles_root_folder_id
    ON titles(root_folder_id);
