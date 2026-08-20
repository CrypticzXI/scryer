pub fn migration_key_from_version_and_desc(version: i64, description: &str) -> String {
    format!("{version:04}_{}", description.replace(' ', "_"))
}
