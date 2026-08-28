use std::sync::Arc;

use scryer_application::{
    IndexerConfigRepository, IndexerConfigUpdate, PluginInstallationRepository,
};
use serde_json::Value;

fn legacy_profile_id(provider_type: &str) -> Option<&'static str> {
    if provider_type.eq_ignore_ascii_case("nzbgeek") {
        Some("nzbgeek")
    } else if provider_type.eq_ignore_ascii_case("dognzb") {
        Some("dognzb")
    } else {
        None
    }
}

fn migrated_config_json(
    provider_type: &str,
    config_json: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(profile_id) = legacy_profile_id(provider_type) else {
        return Ok(None);
    };
    let mut value: Value = serde_json::from_str(config_json.unwrap_or("{}")).map_err(|error| {
        format!("legacy {provider_type} configuration is invalid JSON: {error}")
    })?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| format!("legacy {provider_type} configuration must be a JSON object"))?;
    object.insert(
        "profile_id".to_string(),
        Value::String(profile_id.to_string()),
    );
    Ok(Some(value.to_string()))
}

fn legacy_plugin_name(name: &str, plugin_id: &str) -> String {
    let trimmed = name.trim();
    let base = if trimmed.is_empty() {
        plugin_id.trim()
    } else {
        trimmed
    };
    if base.to_ascii_lowercase().ends_with(" - legacy") {
        base.to_string()
    } else {
        format!("{base} - Legacy")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub indexer_configs: u64,
    pub plugin_installations: u64,
}

pub async fn migrate(
    indexer_configs: Arc<dyn IndexerConfigRepository>,
    plugin_installations: &dyn PluginInstallationRepository,
) -> Result<MigrationReport, String> {
    let configs = indexer_configs
        .list(None)
        .await
        .map_err(|error| format!("failed to list indexer configurations: {error}"))?;
    let mut report = MigrationReport::default();
    for config in configs {
        let Some(config_json) =
            migrated_config_json(&config.provider_type, config.config_json.as_deref())?
        else {
            continue;
        };
        indexer_configs
            .update(IndexerConfigUpdate {
                id: config.id,
                provider_type: Some("newznab".to_string()),
                config_json: Some(config_json),
                ..IndexerConfigUpdate::default()
            })
            .await
            .map_err(|error| format!("failed to migrate legacy indexer configuration: {error}"))?;
        report.indexer_configs += 1;
    }

    let installations = plugin_installations
        .list_plugin_installations()
        .await
        .map_err(|error| format!("failed to list plugin installations: {error}"))?;
    for mut installation in installations {
        if legacy_profile_id(&installation.plugin_id).is_none()
            && legacy_profile_id(&installation.provider_type).is_none()
        {
            continue;
        }
        let name = legacy_plugin_name(&installation.name, &installation.plugin_id);
        if !installation.is_enabled && installation.name == name {
            continue;
        }
        installation.name = name;
        installation.is_enabled = false;
        plugin_installations
            .update_plugin_installation(&installation, None)
            .await
            .map_err(|error| format!("failed to retire legacy plugin installation: {error}"))?;
        report.plugin_installations += 1;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_wrappers_map_to_their_newznab_profiles_and_preserve_overrides() {
        for (provider_type, expected_profile) in [("nzbgeek", "nzbgeek"), ("DogNZB", "dognzb")] {
            let migrated = migrated_config_json(
                provider_type,
                Some(
                    r#"{"additional_parameters":"attrs=poster","api_key":"secret","base_url":"https://custom.example.test","request_interval_ms":750}"#,
                ),
            )
            .expect("legacy configuration should migrate")
            .expect("legacy provider should produce an update");
            let value: Value =
                serde_json::from_str(&migrated).expect("migrated configuration should be JSON");
            assert_eq!(value["profile_id"], expected_profile);
            assert_eq!(value["api_key"], "secret");
            assert_eq!(value["base_url"], "https://custom.example.test");
            assert_eq!(value["additional_parameters"], "attrs=poster");
            assert_eq!(value["request_interval_ms"], 750);
        }
    }

    #[test]
    fn legacy_plugin_names_gain_one_suffix() {
        assert_eq!(legacy_plugin_name("NZBGeek", "nzbgeek"), "NZBGeek - Legacy");
        assert_eq!(
            legacy_plugin_name("DogNZB - Legacy", "dognzb"),
            "DogNZB - Legacy"
        );
    }

    #[test]
    fn migration_ignores_nonlegacy_providers() {
        assert_eq!(
            migrated_config_json("newznab", Some(r#"{"profile_id":"nzbgeek"}"#))
                .expect("generic Newznab configuration should be accepted"),
            None
        );
    }

    #[test]
    fn migration_rejects_invalid_legacy_configuration_without_replacing_it() {
        assert!(migrated_config_json("nzbgeek", Some("not-json")).is_err());
    }
}
