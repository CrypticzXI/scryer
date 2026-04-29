pub use scryer_plugin_sdk::*;

pub(crate) fn config_field_to_domain(field: &ConfigFieldDef) -> scryer_domain::ConfigFieldDef {
    scryer_domain::ConfigFieldDef {
        key: field.key.clone(),
        label: field.label.clone(),
        field_type: match field.field_type {
            ConfigFieldType::String => scryer_domain::ConfigFieldType::String,
            ConfigFieldType::Password => scryer_domain::ConfigFieldType::Password,
            ConfigFieldType::Multiline => scryer_domain::ConfigFieldType::Multiline,
            ConfigFieldType::Bool => scryer_domain::ConfigFieldType::Bool,
            ConfigFieldType::Select => scryer_domain::ConfigFieldType::Select,
            ConfigFieldType::Number => scryer_domain::ConfigFieldType::Number,
        },
        required: field.required,
        default_value: field.default_value.clone(),
        value_source: match field.value_source {
            ConfigFieldValueSource::User => scryer_domain::ConfigFieldValueSource::User,
            ConfigFieldValueSource::HostBinding => {
                scryer_domain::ConfigFieldValueSource::HostBinding
            }
        },
        host_binding: field.host_binding.map(host_binding_to_domain),
        options: field
            .options
            .iter()
            .map(|option| scryer_domain::ConfigFieldOption {
                value: option.value.clone(),
                label: option.label.clone(),
            })
            .collect(),
        help_text: field.help_text.clone(),
    }
}

pub(crate) fn config_fields_to_domain(
    fields: &[ConfigFieldDef],
) -> Vec<scryer_domain::ConfigFieldDef> {
    fields.iter().map(config_field_to_domain).collect()
}

pub(crate) fn host_binding_to_domain(
    binding: PluginHostBindingId,
) -> scryer_domain::PluginHostBindingId {
    match binding {
        PluginHostBindingId::SmgOpenSubtitlesApiKey => {
            scryer_domain::PluginHostBindingId::SmgOpenSubtitlesApiKey
        }
    }
}

pub(crate) fn indexer_capabilities_to_domain(
    capabilities: &IndexerCapabilities,
) -> scryer_domain::IndexerProviderCapabilities {
    scryer_domain::IndexerProviderCapabilities {
        rss: capabilities.rss,
        supported_ids: capabilities.supported_ids.clone(),
        deduplicates_aliases: capabilities.deduplicates_aliases,
        season_param: capabilities.season_param.clone(),
        episode_param: capabilities.episode_param.clone(),
        query_param: capabilities.query_param.clone(),
        search: capabilities.search,
        imdb_search: capabilities.imdb_search,
        tvdb_search: capabilities.tvdb_search,
        anidb_search: capabilities.anidb_search,
    }
}

pub(crate) fn tagged_alias_to_sdk(alias: scryer_domain::TaggedAlias) -> TaggedAlias {
    TaggedAlias {
        name: alias.name,
        language: alias.language,
    }
}

pub(crate) fn decode_plugin_result<T>(
    output: &str,
    context: &str,
) -> scryer_application::AppResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let envelope: PluginResult<T> = serde_json::from_str(output).map_err(|error| {
        scryer_application::AppError::Repository(format!(
            "{context}: plugin returned invalid result envelope: {error}"
        ))
    })?;

    match envelope {
        PluginResult::Ok(value) => Ok(value),
        PluginResult::Err(error) => Err(scryer_application::AppError::Repository(format!(
            "{context}: plugin error {:?}: {}",
            error.code, error.public_message
        ))),
    }
}
