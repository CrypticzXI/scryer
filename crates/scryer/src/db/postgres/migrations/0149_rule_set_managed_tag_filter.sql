ALTER TABLE rule_sets
    ADD COLUMN IF NOT EXISTS managed_tag_filter text;
