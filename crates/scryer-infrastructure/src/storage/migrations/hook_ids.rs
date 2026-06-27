pub(crate) fn is_known_migration_hook_id(hook_id: &str) -> bool {
    match hook_id {
        "migrate_jellyfin_notification_channels_to_media_server_targets" => true,
        "migrate_title_root_folder_ids" => true,
        "migrate_title_catalog_sort_keys" => true,
        #[cfg(test)]
        "test_insert_hook_marker" => true,
        _ => false,
    }
}

pub(crate) fn validate_migration_hook_id(hook_id: &str) -> Result<(), String> {
    if is_known_migration_hook_id(hook_id) {
        Ok(())
    } else {
        Err(format!("unknown migration hook id '{hook_id}'"))
    }
}
