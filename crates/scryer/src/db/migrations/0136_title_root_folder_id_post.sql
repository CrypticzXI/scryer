CREATE INDEX idx_titles_root_folder_id
    ON titles(root_folder_id);

CREATE TRIGGER trg_titles_root_folder_id_required_insert
BEFORE INSERT ON titles
FOR EACH ROW
WHEN NEW.root_folder_id IS NULL OR trim(NEW.root_folder_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'title root_folder_id is required');
END;

CREATE TRIGGER trg_titles_root_folder_id_required_update
BEFORE UPDATE OF root_folder_id ON titles
FOR EACH ROW
WHEN NEW.root_folder_id IS NULL OR trim(NEW.root_folder_id) = ''
BEGIN
    SELECT RAISE(ABORT, 'title root_folder_id is required');
END;
