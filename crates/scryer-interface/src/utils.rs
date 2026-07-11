use async_graphql::Result as GqlResult;
use scryer_domain::{ExternalId, NewTitle};

use crate::types::{AddTitleInput, DownloadSourceKindValue, FillerPolicyValue, IntoApplication, RecapPolicyValue};

pub(crate) struct ResolvedTitleOptionsInput {
    pub quality_profile_id: Option<async_graphql::ID>,
    pub root_folder_id: Option<Option<String>>,
    pub monitor_type: Option<crate::types::MonitorTypeValue>,
    pub use_season_folders: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub filler_policy: Option<FillerPolicyValue>,
    pub recap_policy: Option<RecapPolicyValue>,
}

fn push_structured_tag(tags: &mut Vec<String>, prefix: &str, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let normalized = value.trim();
    if normalized.is_empty() {
        return;
    }
    tags.push(format!("{prefix}{normalized}"));
}

fn set_structured_tag(tags: &mut Vec<String>, prefix: &str, value: Option<String>) {
    tags.retain(|tag| !tag.starts_with(prefix));
    push_structured_tag(tags, prefix, value);
}

fn normalize_title_tag(tag: String) -> Option<String> {
    let trimmed = tag.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }

    Some(if trimmed.starts_with("scryer:") {
        trimmed
    } else {
        trimmed.to_lowercase()
    })
}

pub(crate) fn normalize_title_tags(tags: Vec<String>) -> Vec<String> {
    tags.into_iter().filter_map(normalize_title_tag).collect()
}

pub(crate) fn apply_title_options(tags: &mut Vec<String>, options: ResolvedTitleOptionsInput) {
    set_structured_tag(
        tags,
        "scryer:quality-profile:",
        options
            .quality_profile_id
            .map(|value| value.as_ref().trim().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:monitor-type:",
        options
            .monitor_type
            .map(|value| value.as_tag_value().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:filler-policy:",
        options.filler_policy.map(|value| value.as_app_str().to_string()),
    );
    set_structured_tag(
        tags,
        "scryer:recap-policy:",
        options.recap_policy.map(|value| value.as_app_str().to_string()),
    );

    if let Some(use_season_folders) = options.use_season_folders {
        set_structured_tag(
            tags,
            "scryer:season-folder:",
            Some(
                if use_season_folders {
                    "enabled"
                } else {
                    "disabled"
                }
                .to_string(),
            ),
        );
    }

    if let Some(monitor_specials) = options.monitor_specials {
        set_structured_tag(
            tags,
            "scryer:monitor-specials:",
            Some(if monitor_specials { "true" } else { "false" }.to_string()),
        );
    }

    if let Some(inter_season_movies) = options.inter_season_movies {
        set_structured_tag(
            tags,
            "scryer:inter-season-movies:",
            Some(if inter_season_movies { "true" } else { "false" }.to_string()),
        );
    }
}

pub(crate) fn merge_title_option_tags(
    mut tags: Vec<String>,
    options: ResolvedTitleOptionsInput,
) -> Vec<String> {
    apply_title_options(&mut tags, options);
    tags
}

pub(crate) fn map_add_input(
    input: AddTitleInput,
    resolved_options: Option<ResolvedTitleOptionsInput>,
) -> GqlResult<NewTitle> {
    let AddTitleInput {
        name,
        facet,
        library_id: _,
        monitored,
        mut tags,
        options: _,
        external_ids,
        source_hint: _,
        source_kind: _,
        source_title: _,
        min_availability,
        year,
        overview,
        sort_title,
        slug,
        runtime_minutes,
        language,
        content_status,
    } = input;

    let parsed_facet = facet.into_domain();
    tags = normalize_title_tags(tags);
    let root_folder_id = resolved_options
        .as_ref()
        .and_then(|options| options.root_folder_id.clone().flatten());
    if let Some(options) = resolved_options {
        apply_title_options(&mut tags, options);
    }

    Ok(NewTitle {
        name,
        facet: parsed_facet,
        monitored,
        tags,
        external_ids: external_ids
            .unwrap_or_default()
            .into_iter()
            .map(|item| ExternalId {
                source: item.source,
                value: item.value,
            })
            .collect(),
        root_folder_id,
        min_availability,
        poster_url: None,
        year,
        overview,
        sort_title,
        slug,
        runtime_minutes,
        language,
        content_status,
    })
}

pub(crate) fn parse_download_source_kind(
    raw: Option<DownloadSourceKindValue>,
) -> Option<scryer_application::DownloadSourceKind> {
    raw.map(DownloadSourceKindValue::into_application)
}

#[cfg(test)]
mod tests {
    use crate::types::MediaFacetValue;
    use scryer_domain::MediaFacet;

    #[test]
    fn media_facet_value_maps_series_to_series_domain() {
        assert_eq!(MediaFacetValue::Series.into_domain(), MediaFacet::Series);
    }
}
