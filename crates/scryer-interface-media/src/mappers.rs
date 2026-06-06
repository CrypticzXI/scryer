use crate::types::*;
use scryer_application::stored_paths::stored_path_to_path_buf;
use scryer_application::{
    ActivityEvent, BackupInfo, DeletePreview, DownloadClientRoutingSettingsEntry,
    FacetScoringPersonaSelection, IgnorePendingImportResult, IndexerRoutingSettingsEntry,
    IndexerSearchResult, JobDefinition, JobRun, LibraryPathsSettings, LibraryScanSummary,
    LibrarySettings, ManualPluginPreview, MediaRequestCounts, MediaSettings, ParsedEpisodeMetadata,
    ParsedReleaseMetadata, PendingImportConnection, PendingImportCounts, PendingImportItem,
    PendingImportSearchAttempt, PendingRelease, PluginCatalogStatus, QualityProfile,
    QualityProfileCriteria, QualityProfileDecision, QualityProfileSelection,
    QualityProfileSettings, RegistryPlugin, RenameApplyItemResult, RenameApplyResult, RenamePlan,
    RenamePlanItem, ResolvePendingImportResult, RssSyncReport, ScoringEntry, ScoringSource,
    ServiceSettings, SmgVersionCompatibilityNotice, SubmissionScope, SystemHealth,
    TitleHistoryPage, TitleReleaseBlocklistEntry,
};
use scryer_domain::{
    CalendarEpisode, Collection, ConfigFieldDef, ConfigFieldType, DomainEvent,
    DownloadClientConfig, DownloadQueueItem, Episode, IndexerConfig, Library, MediaFacet,
    MediaRequest, PluginInstallation, PluginSupportTier, RuleSet, SubtitleProviderConfig, Title,
    TitleHistoryRecord, User,
};
use scryer_rules;
use serde_json::Value;
use std::fs;

fn support_tier_label(value: PluginSupportTier) -> String {
    match value {
        PluginSupportTier::Official => "official".to_string(),
        PluginSupportTier::VerifiedCommunity => "verified_community".to_string(),
        PluginSupportTier::Unverified => "unverified".to_string(),
    }
}

fn import_facet_from_payload(payload: &Value) -> Option<MediaFacetValue> {
    let parameters = payload.get("parameters")?.as_array()?;
    for parameter in parameters {
        let (key, value) = match parameter {
            Value::Array(values) => (
                values.first().and_then(Value::as_str),
                values.get(1).and_then(Value::as_str),
            ),
            Value::Object(_) => (
                parameter.get("key").and_then(Value::as_str),
                parameter.get("value").and_then(Value::as_str),
            ),
            _ => (None, None),
        };
        let Some(key) = key else {
            continue;
        };
        if key != "*scryer_facet" {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        return match value.trim().to_ascii_lowercase().as_str() {
            "movie" => Some(MediaFacetValue::Movie),
            "series" => Some(MediaFacetValue::Series),
            "anime" => Some(MediaFacetValue::Anime),
            _ => None,
        };
    }
    None
}

fn path_basename(path: &str) -> Option<String> {
    let path = stored_path_to_path_buf(path.trim());
    let display = path.to_string_lossy();
    let trimmed = display.trim().trim_end_matches(std::path::MAIN_SEPARATOR);
    if trimmed.is_empty() {
        return None;
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn looks_like_weaver_job_id(title: &str, source_ref: &str) -> bool {
    let trimmed = title.trim();
    !trimmed.is_empty()
        && (trimmed == source_ref
            || (trimmed.len() >= 4 && trimmed.chars().all(|ch| ch.is_ascii_digit())))
}

fn import_source_title_from_payload(
    payload: &Value,
    source_system: &str,
    source_ref: &str,
    source_path: Option<&str>,
) -> Option<String> {
    let payload_title = payload
        .get("source_title")
        .and_then(Value::as_str)
        .or_else(|| payload.get("name").and_then(Value::as_str))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToString::to_string);

    let fallback_path_title = source_path.and_then(path_basename).or_else(|| {
        payload
            .get("dest_dir")
            .and_then(Value::as_str)
            .and_then(path_basename)
    });

    if source_system.eq_ignore_ascii_case("weaver")
        && payload_title
            .as_deref()
            .is_some_and(|title| looks_like_weaver_job_id(title, source_ref))
    {
        return fallback_path_title.or(payload_title);
    }

    payload_title.or(fallback_path_title)
}

pub fn from_scoring_overrides(
    overrides: scryer_application::ScoringOverrides,
) -> ScoringOverridesPayload {
    ScoringOverridesPayload {
        allow_x265_non4k: overrides.allow_x265_non4k,
        block_dv_without_fallback: overrides.block_dv_without_fallback,
        prefer_compact_encodes: overrides.prefer_compact_encodes,
        prefer_lossless_audio: overrides.prefer_lossless_audio,
        block_upscaled: overrides.block_upscaled,
    }
}

pub fn from_quality_profile_criteria(
    criteria: QualityProfileCriteria,
) -> QualityProfileCriteriaPayload {
    QualityProfileCriteriaPayload {
        quality_tiers: criteria.quality_tiers,
        archival_quality: criteria.archival_quality,
        allow_unknown_quality: criteria.allow_unknown_quality,
        source_allowlist: criteria
            .source_allowlist
            .into_iter()
            .map(|source| source.to_string())
            .collect(),
        source_blocklist: criteria
            .source_blocklist
            .into_iter()
            .map(|source| source.to_string())
            .collect(),
        video_codec_allowlist: criteria
            .video_codec_allowlist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        video_codec_blocklist: criteria
            .video_codec_blocklist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        audio_codec_allowlist: criteria
            .audio_codec_allowlist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        audio_codec_blocklist: criteria
            .audio_codec_blocklist
            .into_iter()
            .map(|codec| codec.to_string())
            .collect(),
        dolby_vision_allowed: criteria.dolby_vision_allowed,
        detected_hdr_allowed: criteria.detected_hdr_allowed,
        prefer_remux: criteria.prefer_remux,
        allow_bd_disk: criteria.allow_bd_disk,
        allow_upgrades: criteria.allow_upgrades,
        scoring_overrides: from_scoring_overrides(criteria.scoring_overrides),
        cutoff_tier: criteria.cutoff_tier,
        min_score_to_grab: criteria.min_score_to_grab,
    }
}

pub fn from_quality_profile(profile: QualityProfile) -> QualityProfilePayload {
    QualityProfilePayload {
        id: profile.id,
        name: profile.name,
        criteria: from_quality_profile_criteria(profile.criteria),
    }
}

pub fn from_library_paths_settings(settings: LibraryPathsSettings) -> LibraryPathsPayload {
    LibraryPathsPayload {
        movie_path: settings.movie_path,
        series_path: settings.series_path,
        anime_path: settings.anime_path,
    }
}

pub fn from_service_settings(settings: ServiceSettings) -> ServiceSettingsPayload {
    ServiceSettingsPayload {
        tls_cert_path: settings.tls_cert_path,
        tls_key_path: settings.tls_key_path,
    }
}

pub fn from_download_client_routing_entry(
    entry: DownloadClientRoutingSettingsEntry,
) -> DownloadClientRoutingEntryPayload {
    DownloadClientRoutingEntryPayload {
        client_id: entry.client_id,
        enabled: entry.enabled,
        category: entry.category,
        recent_queue_priority: entry.recent_queue_priority,
        older_queue_priority: entry.older_queue_priority,
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
    }
}

pub fn from_indexer_routing_entry(
    entry: IndexerRoutingSettingsEntry,
) -> IndexerRoutingEntryPayload {
    IndexerRoutingEntryPayload {
        indexer_id: entry.indexer_id,
        enabled: entry.enabled,
        categories: entry.categories,
        priority: entry.priority,
    }
}

pub fn from_library_settings(settings: LibrarySettings) -> LibrarySettingsPayload {
    LibrarySettingsPayload {
        required_audio_languages_override: settings.required_audio_languages_override,
        required_audio_languages: settings.required_audio_languages,
        quality_profile_id_override: settings.quality_profile_id_override,
        quality_profile_id: settings.quality_profile_id,
        request_quality_profile_ids_override: settings.request_quality_profile_ids_override,
        request_quality_profile_ids: settings.request_quality_profile_ids,
        request_quality_profile_default_id: settings.request_quality_profile_default_id,
        scoring_persona_override: settings
            .scoring_persona_override
            .map(ScoringPersonaValue::from_application),
        scoring_persona: ScoringPersonaValue::from_application(settings.scoring_persona),
        filler_policy_override: settings.filler_policy_override,
        filler_policy: settings.filler_policy,
        recap_policy_override: settings.recap_policy_override,
        recap_policy: settings.recap_policy,
        monitor_specials_override: settings.monitor_specials_override,
        monitor_specials: settings.monitor_specials,
        inter_season_movies_override: settings.inter_season_movies_override,
        inter_season_movies: settings.inter_season_movies,
        monitor_filler_movies_override: settings.monitor_filler_movies_override,
        monitor_filler_movies: settings.monitor_filler_movies,
        nfo_write_on_import_override: settings.nfo_write_on_import_override,
        nfo_write_on_import: settings.nfo_write_on_import,
        plexmatch_write_on_import_override: settings.plexmatch_write_on_import_override,
        plexmatch_write_on_import: settings.plexmatch_write_on_import,
        import_mode_override: settings
            .import_mode_override
            .map(|mode| mode.as_str().to_string()),
        import_mode: settings.import_mode.as_str().to_string(),
        indexer_routing_override: settings.indexer_routing_override.map(|entries| {
            entries
                .into_iter()
                .map(from_indexer_routing_entry)
                .collect()
        }),
        download_client_routing_override: settings.download_client_routing_override.map(
            |entries| {
                entries
                    .into_iter()
                    .map(from_download_client_routing_entry)
                    .collect()
            },
        ),
    }
}

fn from_quality_scope(facet: MediaFacet) -> ContentScopeValue {
    match facet {
        MediaFacet::Movie => ContentScopeValue::Movie,
        MediaFacet::Series => ContentScopeValue::Series,
        MediaFacet::Anime => ContentScopeValue::Anime,
    }
}

fn from_quality_profile_selection(
    selection: QualityProfileSelection,
) -> QualityProfileSelectionPayload {
    QualityProfileSelectionPayload {
        scope: from_quality_scope(selection.facet),
        override_profile_id: selection.override_profile_id,
        effective_profile_id: selection.effective_profile_id,
        inherits_global: selection.inherits_global,
    }
}

fn from_facet_scoring_persona_selection(
    selection: FacetScoringPersonaSelection,
) -> FacetScoringPersonaSelectionPayload {
    FacetScoringPersonaSelectionPayload {
        scope: from_quality_scope(selection.facet),
        override_persona: selection
            .override_persona
            .map(ScoringPersonaValue::from_application),
        effective_persona: ScoringPersonaValue::from_application(selection.effective_persona),
        inherits_global: selection.inherits_global,
    }
}

pub fn from_media_settings(
    scope: ContentScopeValue,
    settings: MediaSettings,
) -> MediaSettingsPayload {
    MediaSettingsPayload {
        scope,
        library_path: settings.library_path,
        root_folders: settings
            .root_folders
            .into_iter()
            .map(|entry| RootFolderPayload {
                path: entry.path,
                is_default: entry.is_default,
            })
            .collect(),
        required_audio_languages: settings.required_audio_languages,
        folder_template: settings.folder_template,
        rename_template: settings.rename_template,
        rename_collision_policy: settings.rename_collision_policy,
        rename_missing_metadata_policy: settings.rename_missing_metadata_policy,
        filler_policy: settings.filler_policy,
        recap_policy: settings.recap_policy,
        monitor_specials: settings.monitor_specials,
        inter_season_movies: settings.inter_season_movies,
        monitor_filler_movies: settings.monitor_filler_movies,
        nfo_write_on_import: settings.nfo_write_on_import,
        plexmatch_write_on_import: settings.plexmatch_write_on_import,
        import_mode: settings.import_mode.as_str().to_string(),
    }
}

pub fn from_quality_profile_settings(
    settings: QualityProfileSettings,
) -> QualityProfileSettingsPayload {
    QualityProfileSettingsPayload {
        profiles: settings
            .profiles
            .into_iter()
            .map(from_quality_profile)
            .collect(),
        global_profile_id: settings.global_profile_id,
        global_scoring_persona: ScoringPersonaValue::from_application(
            settings.global_scoring_persona,
        ),
        category_selections: settings
            .category_selections
            .into_iter()
            .map(from_quality_profile_selection)
            .collect(),
        category_persona_selections: settings
            .category_persona_selections
            .into_iter()
            .map(from_facet_scoring_persona_selection)
            .collect(),
    }
}

pub fn from_delete_preview(preview: DeletePreview) -> DeletePreviewPayload {
    DeletePreviewPayload {
        fingerprint: preview.fingerprint,
        total_file_count: preview.total_file_count,
        media_count: preview.media_count,
        subtitle_count: preview.subtitle_count,
        image_count: preview.image_count,
        other_count: preview.other_count,
        directory_count: preview.directory_count,
        requires_typed_confirmation: preview.requires_typed_confirmation,
        typed_confirmation_prompt: preview.typed_confirmation_prompt,
        target_label: preview.target_label,
        sample_paths: preview.sample_paths,
    }
}

pub fn from_delete_titles_preview(
    preview: scryer_application::DeleteTitlesPreview,
) -> DeleteTitlesPreviewPayload {
    let failed_count = preview
        .items
        .iter()
        .filter(|item| item.error.is_some())
        .count() as i32;
    DeleteTitlesPreviewPayload {
        preview: from_delete_preview(preview.preview),
        items: preview
            .items
            .into_iter()
            .map(|item| DeleteTitlePreviewResultPayload {
                title_id: item.title_id,
                preview: item.preview.map(from_delete_preview),
                error: item.error,
            })
            .collect(),
        failed_count,
    }
}

pub fn from_search_result(result: IndexerSearchResult) -> IndexerSearchResultPayload {
    let seeders = result
        .extra
        .get("seeders")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let peers = result
        .extra
        .get("peers")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let info_hash = result
        .extra
        .get("info_hash")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());
    let freeleech = result.extra.get("freeleech").and_then(|v| v.as_bool());
    let download_volume_factor = result
        .extra
        .get("downloadvolumefactor")
        .and_then(|v| v.as_f64());

    IndexerSearchResultPayload {
        source: result.source,
        title: result.title,
        link: result.link,
        download_url: result.download_url,
        source_kind: result
            .source_kind
            .map(DownloadSourceKindValue::from_application),
        size_bytes: result.size_bytes,
        published_at: result.published_at,
        thumbs_up: result.thumbs_up,
        thumbs_down: result.thumbs_down,
        parsed_release: result.parsed_release_metadata.map(from_parsed_release),
        quality_profile_decision: result
            .quality_profile_decision
            .map(from_quality_profile_decision),
        seeders,
        peers,
        info_hash,
        freeleech,
        download_volume_factor,
        candidate_token: result.candidate_token,
        queue_scope: result.queue_scope.map(from_submission_scope),
        auto_eligible: result.auto_eligible,
        auto_decision_code: result.auto_decision_code,
        auto_decision_summary: result.auto_decision_summary,
    }
}

pub fn from_submission_scope(scope: SubmissionScope) -> QueueDownloadScopePayload {
    match scope {
        SubmissionScope::Episode { episode_id } => QueueDownloadScopePayload {
            kind: "episode".to_string(),
            episode_id: Some(episode_id),
            episode_ids: Vec::new(),
            collection_id: None,
        },
        SubmissionScope::EpisodeSet { episode_ids } => QueueDownloadScopePayload {
            kind: "episode_set".to_string(),
            episode_id: None,
            episode_ids,
            collection_id: None,
        },
        SubmissionScope::Collection { collection_id } => QueueDownloadScopePayload {
            kind: "collection".to_string(),
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: Some(collection_id),
        },
        SubmissionScope::Title => QueueDownloadScopePayload {
            kind: "title".to_string(),
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: None,
        },
        SubmissionScope::Orphan => QueueDownloadScopePayload {
            kind: "orphan".to_string(),
            episode_id: None,
            episode_ids: Vec::new(),
            collection_id: None,
        },
    }
}

pub fn from_title_release_blocklist_entry(
    entry: TitleReleaseBlocklistEntry,
) -> TitleReleaseBlocklistEntryPayload {
    TitleReleaseBlocklistEntryPayload {
        id: entry.id,
        source_hint: entry.source_hint,
        source_title: entry.source_title,
        error_message: entry.error_message,
        attempted_at: entry.attempted_at,
        episode_ids: entry.episode_ids,
    }
}

pub fn from_quality_profile_decision(
    decision: QualityProfileDecision,
) -> QualityProfileDecisionPayload {
    QualityProfileDecisionPayload {
        allowed: decision.allowed,
        block_codes: decision.block_codes,
        release_score: decision.release_score,
        preference_score: decision.preference_score,
        scoring_log: decision
            .scoring_log
            .into_iter()
            .map(|e: ScoringEntry| {
                let (source, rule_set_name) = match e.source {
                    ScoringSource::Builtin => ("builtin".to_string(), None),
                    ScoringSource::UserRule { id, name } => (format!("user:{id}"), Some(name)),
                    ScoringSource::SystemRule { id, name } => (format!("system:{id}"), Some(name)),
                };
                ScoringEntryPayload {
                    code: e.code,
                    delta: e.delta,
                    source,
                    rule_set_name,
                }
            })
            .collect(),
    }
}

pub fn from_parsed_release(result: ParsedReleaseMetadata) -> ParsedReleasePayload {
    ParsedReleasePayload {
        raw_title: result.raw_title,
        normalized_title: result.normalized_title,
        release_group: result.release_group,
        languages_audio: result.languages_audio,
        languages_subtitles: result.languages_subtitles,
        year: result.year,
        quality: result.quality,
        source: result.source.map(|source| source.to_string()),
        video_codec: result.video_codec.map(|codec| codec.to_string()),
        video_encoding: result.video_encoding,
        audio: result.audio.map(|codec| codec.to_string()),
        audio_channels: result.audio_channels,
        is_dual_audio: result.is_dual_audio,
        is_atmos: result.is_atmos,
        is_dolby_vision: result.is_dolby_vision,
        detected_hdr: result.detected_hdr,
        fps: result.fps,
        is_proper_upload: result.is_proper_upload,
        is_remux: result.is_remux,
        is_bd_disk: result.is_bd_disk,
        is_ai_enhanced: result.is_ai_enhanced,
        parser_version: result.parser_version.to_string(),
        parse_confidence: result.parse_confidence,
        missing_fields: result.missing_fields,
        parse_hints: result.parse_hints,
        episode: result.episode.map(from_parsed_episode),
    }
}

pub fn from_parsed_episode(episode: ParsedEpisodeMetadata) -> ParsedEpisodePayload {
    ParsedEpisodePayload {
        season: episode.season.map(|value| value as i32),
        episode_numbers: episode
            .episode_numbers
            .into_iter()
            .map(|value| value as i32)
            .collect(),
        absolute_episode: episode.absolute_episode.map(|value| value as i32),
        raw: episode.raw,
    }
}

pub fn from_indexer_config_with_fields(
    config: IndexerConfig,
    config_fields: &[ConfigFieldDef],
) -> IndexerConfigPayload {
    let is_managed = config.managed_parent_config_id.is_some();
    let managed_parent_config_id = config.managed_parent_config_id.clone();
    let supports_managed_children_sync = config.provider_type.eq_ignore_ascii_case("prowlarr");
    let (config_json, stored_secret_keys) =
        redact_indexer_config_json(config.config_json, config_fields);
    let has_api_key = stored_secret_keys.iter().any(|key| key == "api_key")
        || config
            .api_key_encrypted
            .as_ref()
            .is_some_and(|value| !value.is_empty());
    IndexerConfigPayload {
        id: config.id,
        name: config.name,
        provider_type: config.provider_type,
        base_url: config.base_url,
        has_api_key,
        is_managed,
        managed_parent_config_id,
        supports_managed_children_sync,
        stored_secret_keys,
        rate_limit_seconds: config.rate_limit_seconds,
        rate_limit_burst: config.rate_limit_burst,
        disabled_until: config.disabled_until.map(|value| value.to_rfc3339()),
        is_enabled: config.is_enabled,
        enable_interactive_search: config.enable_interactive_search,
        enable_auto_search: config.enable_auto_search,
        last_health_status: config.last_health_status,
        last_error_at: config.last_error_at.map(|value| value.to_rfc3339()),
        last_query_at: None,
        config_json,
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

pub fn from_indexer_config_sync_result(
    result: scryer_application::IndexerConfigSyncResult,
) -> IndexerConfigSyncPayload {
    IndexerConfigSyncPayload {
        parent_config_id: result.parent_config_id,
        created_ids: result.created_ids,
        updated_ids: result.updated_ids,
        deleted_ids: result.deleted_ids,
    }
}

fn redact_indexer_config_json(
    config_json: Option<String>,
    config_fields: &[ConfigFieldDef],
) -> (Option<String>, Vec<String>) {
    let Some(raw) = config_json else {
        return (None, Vec::new());
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return (Some(raw), Vec::new());
    };
    let Some(object) = value.as_object_mut() else {
        return (Some(raw), Vec::new());
    };

    let configured_secret_keys = config_fields
        .iter()
        .filter(|field| field.field_type == ConfigFieldType::Password)
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let mut stored_secret_keys = object
        .iter()
        .filter_map(|(key, value)| {
            let is_secret = if configured_secret_keys.is_empty() {
                indexer_config_key_is_secret(key)
            } else {
                configured_secret_keys.contains(key.as_str())
            };
            if !is_secret {
                return None;
            }
            match value {
                serde_json::Value::String(value) if !value.trim().is_empty() => Some(key.clone()),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    stored_secret_keys.sort();
    stored_secret_keys.dedup();

    for key in &stored_secret_keys {
        object.remove(key);
    }

    let redacted = serde_json::to_string(&value).ok();
    (redacted, stored_secret_keys)
}

fn indexer_config_key_is_secret(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    normalized.contains("apikey")
        || normalized.contains("password")
        || normalized.contains("secret")
        || normalized.ends_with("token")
}

#[expect(
    clippy::too_many_arguments,
    reason = "provider type payload is assembled from discrete application fields"
)]
pub fn from_provider_type(
    provider_type: String,
    name: String,
    config_fields: Vec<scryer_domain::ConfigFieldDef>,
    default_base_url: Option<String>,
    available_host_bindings: Vec<String>,
    recommended_facets: Vec<String>,
    supported_events: Vec<String>,
    supports_test: bool,
) -> ProviderTypePayload {
    ProviderTypePayload {
        provider_type,
        name,
        default_base_url,
        available_host_bindings,
        recommended_facets: recommended_facets
            .into_iter()
            .filter_map(|facet| MediaFacetValue::parse(&facet))
            .collect(),
        supported_events,
        supports_test,
        config_fields: config_fields
            .into_iter()
            .map(|f| PluginConfigFieldPayload {
                key: f.key,
                label: f.label,
                field_type: f.field_type.as_str().to_string(),
                required: f.required,
                default_value: f.default_value,
                value_source: match f.value_source {
                    scryer_domain::ConfigFieldValueSource::User => "user",
                    scryer_domain::ConfigFieldValueSource::HostBinding => "host_binding",
                }
                .to_string(),
                role: f.role.map(|role| role.as_str().to_string()),
                host_binding: f.host_binding.map(|binding| binding.as_str().to_string()),
                options: f
                    .options
                    .into_iter()
                    .map(|o| PluginConfigFieldOptionPayload {
                        value: o.value,
                        label: o.label,
                    })
                    .collect(),
                help_text: f.help_text,
            })
            .collect(),
    }
}

pub fn from_download_client_config(config: DownloadClientConfig) -> DownloadClientConfigPayload {
    let base_url =
        scryer_application::resolve_download_client_base_url_from_config_json(&config.config_json);
    DownloadClientConfigPayload {
        id: config.id,
        name: config.name,
        client_type: config.client_type,
        base_url,
        config_json: config.config_json,
        is_enabled: config.is_enabled,
        status: config.status.as_str().to_string(),
        last_error: config.last_error,
        last_seen_at: config.last_seen_at.map(|value| value.to_rfc3339()),
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

pub fn from_subtitle_provider_config(
    config: SubtitleProviderConfig,
    config_fields: &[scryer_domain::ConfigFieldDef],
) -> SubtitleProviderConfigPayload {
    let secret_keys = config_fields
        .iter()
        .filter(|field| matches!(field.field_type, scryer_domain::ConfigFieldType::Password))
        .map(|field| field.key.as_str())
        .collect::<std::collections::HashSet<_>>();

    let has_config = serde_json::from_str::<Value>(&config.config_json)
        .ok()
        .is_some_and(|value| match value {
            Value::Object(map) => !map.is_empty(),
            Value::Null => false,
            _ => true,
        });

    let stored_secret_keys = serde_json::from_str::<Value>(&config.config_json)
        .ok()
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .map(|map| {
            map.iter()
                .filter_map(|(key, value)| {
                    let is_secret =
                        secret_keys.contains(key.as_str()) || looks_like_secret_config_key(key);
                    if is_secret
                        && !value.is_null()
                        && value.as_str().is_none_or(|value| !value.trim().is_empty())
                    {
                        Some(key.clone())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    SubtitleProviderConfigPayload {
        id: config.id,
        name: config.name,
        provider_type: config.provider_type,
        has_config,
        stored_secret_keys,
        enabled_facets: config
            .enabled_facets
            .iter()
            .filter_map(|facet| MediaFacetValue::parse(facet))
            .collect(),
        is_enabled: config.is_enabled,
        last_health_status: config.last_health_status,
        last_error: config.last_error,
        last_error_at: config.last_error_at.map(|value| value.to_rfc3339()),
        disabled_until: config.disabled_until.map(|value| value.to_rfc3339()),
        created_at: config.created_at.to_rfc3339(),
        updated_at: config.updated_at.to_rfc3339(),
    }
}

fn looks_like_secret_config_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized == "api_key"
        || normalized == "apikey"
        || normalized.contains("api_key")
}

pub fn from_download_queue_item(item: DownloadQueueItem) -> DownloadQueueItemPayload {
    let display_state = DownloadDisplayStateValue::from_application(
        scryer_application::derive_download_queue_display_state(&item),
    );
    DownloadQueueItemPayload {
        id: item.id,
        title_id: item.title_id,
        episode_id: item.episode_id,
        title_name: item.title_name,
        facet: item.facet.as_deref().and_then(MediaFacetValue::parse),
        is_scryer_origin: item.is_scryer_origin,
        tracked_state: item
            .tracked_state
            .map(TrackedDownloadStateValue::from_domain),
        tracked_status: item
            .tracked_status
            .map(TrackedDownloadStatusValue::from_domain),
        tracked_status_messages: item.tracked_status_messages,
        tracked_match_type: item
            .tracked_match_type
            .map(TitleMatchTypeValue::from_domain),
        client_id: item.client_id,
        client_name: item.client_name,
        client_type: item.client_type,
        state: DownloadQueueStateValue::from_domain(item.state),
        display_state,
        progress_percent: i32::from(item.progress_percent),
        size_bytes: item.size_bytes.map(|value| value.to_string()),
        remaining_seconds: item
            .remaining_seconds
            .and_then(|value| i32::try_from(value).ok()),
        queued_at: item.queued_at,
        last_updated_at: item.last_updated_at,
        attention_required: item.attention_required,
        attention_reason: item.attention_reason,
        download_client_item_id: item.download_client_item_id,
        download_id: item.download_id,
        import_status: item.import_status.map(ImportStatusValue::from_domain),
        import_error_code: item
            .import_error_code
            .map(ImportErrorCodeValue::from_domain),
        import_error_message: item.import_error_message,
        imported_at: item.imported_at,
        delete_status: item
            .delete_status
            .map(DownloadQueueDeleteStatusValue::from_domain),
        delete_error_message: item.delete_error_message,
    }
}

fn extract_tag_string(tags: &[String], prefix: &str) -> Option<String> {
    tags.iter().find_map(|tag| {
        tag.strip_prefix(prefix).and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    })
}

fn extract_tag_bool(tags: &[String], prefix: &str) -> Option<bool> {
    tags.iter()
        .find_map(|tag| tag.strip_prefix(prefix))
        .map(|value| !value.trim().eq_ignore_ascii_case("false"))
}

pub fn from_title(title: Title) -> TitlePayload {
    let quality_profile_id = extract_tag_string(&title.tags, "scryer:quality-profile:");
    let root_folder_path = extract_tag_string(&title.tags, "scryer:root-folder:");
    let monitor_type = extract_tag_string(&title.tags, "scryer:monitor-type:")
        .as_deref()
        .and_then(MonitorTypeValue::from_tag_value);
    let use_season_folders = extract_tag_string(&title.tags, "scryer:season-folder:")
        .map(|value| !value.eq_ignore_ascii_case("disabled"));
    let monitor_specials = extract_tag_bool(&title.tags, "scryer:monitor-specials:");
    let inter_season_movies = extract_tag_bool(&title.tags, "scryer:inter-season-movies:");
    let filler_policy = extract_tag_string(&title.tags, "scryer:filler-policy:");
    let recap_policy = extract_tag_string(&title.tags, "scryer:recap-policy:");

    TitlePayload {
        id: title.id,
        library_id: title.library_id,
        library_name: None,
        library_slug: None,
        name: title.name,
        facet: MediaFacetValue::from_domain(title.facet),
        monitored: title.monitored,
        tags: title.tags,
        external_ids: title
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        created_by: title.created_by,
        created_at: title.created_at.to_rfc3339(),
        year: title.year,
        overview: title.overview,
        poster_url: title.poster_url,
        poster_source_url: title.poster_source_url,
        background_url: title.background_url,
        background_source_url: title.background_source_url,
        sort_title: title.sort_title,
        slug: title.slug,
        imdb_id: title.imdb_id,
        runtime_minutes: title.runtime_minutes,
        genres: title.genres,
        content_status: title.content_status,
        language: title.language,
        first_aired: title.first_aired,
        network: title.network,
        studio: title.studio,
        country: title.country,
        aliases: title.aliases,
        metadata_language: title.metadata_language,
        metadata_fetched_at: title.metadata_fetched_at.map(|dt| dt.to_rfc3339()),
        min_availability: title.min_availability,
        digital_release_date: title.digital_release_date,
        quality_profile_id,
        root_folder_path,
        monitor_type,
        use_season_folders,
        monitor_specials,
        inter_season_movies,
        filler_policy,
        recap_policy,
        quality_tier: None,
        current_quality_tier: None,
        size_bytes: None,
        episodes_owned: None,
        episodes_monitored: None,
        episodes_total: None,
    }
}

pub fn from_media_request(request: MediaRequest) -> MediaRequestPayload {
    MediaRequestPayload {
        id: request.id,
        library_id: request.library_id,
        facet: MediaFacetValue::from_domain(request.facet),
        status: MediaRequestStatusValue::from_domain(request.status),
        identity_fingerprint: request.identity_fingerprint,
        title: request.title,
        sort_title: request.sort_title,
        slug: request.slug,
        poster_url: request.poster_url,
        year: request.year,
        overview: request.overview,
        runtime_minutes: request.runtime_minutes,
        language: request.language,
        content_status: request.content_status,
        requested_quality_profile_id: request.requested_quality_profile_id,
        requested_quality_profile_name: request.requested_quality_profile_name,
        requested_monitor_type: request.requested_monitor_type,
        resolved_by_user_id: request.resolved_by_user_id,
        resolved_at: request.resolved_at.map(|value| value.to_rfc3339()),
        created_title_id: request.created_title_id,
        approved_quality_profile_id: request.approved_quality_profile_id,
        approved_quality_profile_name: request.approved_quality_profile_name,
        external_ids: request
            .external_ids
            .into_iter()
            .map(|id| ExternalIdPayload {
                source: id.source,
                value: id.value,
            })
            .collect(),
        requesters: request
            .requesters
            .into_iter()
            .map(|requester| MediaRequestRequesterPayload {
                user_id: requester.user_id,
                username: requester.username,
                avatar_url: requester.avatar_url,
                requested_at: requester.requested_at.to_rfc3339(),
            })
            .collect(),
        created_by_user_id: request.created_by_user_id,
        created_at: request.created_at.to_rfc3339(),
        updated_at: request.updated_at.to_rfc3339(),
    }
}

pub fn from_library(library: Library) -> LibraryPayload {
    LibraryPayload {
        id: library.id,
        facet: MediaFacetValue::from_domain(library.facet),
        name: library.name,
        slug: library.slug,
        is_default: library.is_default,
        roots: library
            .roots
            .into_iter()
            .map(|root| LibraryRootPayload {
                id: root.id,
                path: root.path,
                is_default: root.is_default,
            })
            .collect(),
    }
}

pub fn from_library_scan_summary(summary: LibraryScanSummary) -> LibraryScanSummaryPayload {
    LibraryScanSummaryPayload {
        scanned: summary.scanned as i32,
        matched: summary.matched as i32,
        imported: summary.imported as i32,
        skipped: summary.skipped as i32,
        unmatched: summary.unmatched as i32,
    }
}

pub fn from_pending_import_counts(counts: PendingImportCounts) -> PendingImportCountsPayload {
    PendingImportCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

pub fn from_media_request_counts(counts: MediaRequestCounts) -> MediaRequestCountsPayload {
    MediaRequestCountsPayload {
        movie: counts.movie as i32,
        series: counts.series as i32,
        anime: counts.anime as i32,
    }
}

fn from_pending_import_search_attempt(
    attempt: PendingImportSearchAttempt,
) -> PendingImportSearchAttemptPayload {
    PendingImportSearchAttemptPayload {
        query: attempt.query,
        result_count: attempt.result_count as i32,
        top_results: attempt.top_results,
        summary: attempt.summary,
    }
}

pub fn from_pending_import_item(item: PendingImportItem) -> PendingImportItemPayload {
    PendingImportItemPayload {
        id: item.id,
        library_id: item.library_id,
        library_slug: item.library_slug,
        facet: MediaFacetValue::from_domain(item.facet),
        status: PendingImportStatusValue::from_application(item.status),
        title_id: item.title_id,
        title_name: item.title_name,
        title_slug: item.title_slug,
        display_name: item.display_name,
        path: item.path,
        folder_path: item.folder_path,
        query: item.query,
        year_hint: item.year_hint,
        reason: item.reason,
        search_attempts: item
            .search_attempts
            .into_iter()
            .map(from_pending_import_search_attempt)
            .collect(),
    }
}

pub fn from_pending_import_connection(
    connection: PendingImportConnection,
) -> PendingImportConnectionPayload {
    PendingImportConnectionPayload {
        total: connection.total as i32,
        items: connection
            .items
            .into_iter()
            .map(from_pending_import_item)
            .collect(),
    }
}

pub fn from_resolve_pending_import_result(
    result: ResolvePendingImportResult,
) -> ResolvePendingImportPayload {
    ResolvePendingImportPayload {
        title: from_title(result.title),
        created: result.created,
        library_scan: from_library_scan_summary(result.library_scan),
    }
}

pub fn from_ignore_pending_import_result(
    result: IgnorePendingImportResult,
) -> IgnorePendingImportPayload {
    IgnorePendingImportPayload {
        id: result.id,
        status: PendingImportStatusValue::from_application(result.status),
    }
}

pub fn from_cancel_library_scan_result(
    result: scryer_application::CancelLibraryScanResult,
) -> CancelLibraryScanPayload {
    CancelLibraryScanPayload {
        session_id: result.session_id,
        accepted: result.accepted,
    }
}

pub fn from_library_scan_phase_progress(
    progress: scryer_application::LibraryScanPhaseProgress,
) -> LibraryScanPhaseProgressPayload {
    LibraryScanPhaseProgressPayload {
        total: progress.total as i32,
        completed: progress.completed as i32,
        failed: progress.failed as i32,
    }
}

pub fn from_library_scan_session(
    session: scryer_application::LibraryScanSession,
) -> LibraryScanProgressPayload {
    LibraryScanProgressPayload {
        session_id: session.session_id,
        facet: MediaFacetValue::from_domain(session.facet),
        library_id: session.library_id,
        mode: LibraryScanModeValue::from_application(session.mode),
        status: LibraryScanStatusValue::from_application(session.status),
        started_at: session.started_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        found_titles: session.found_titles as i32,
        title_match_total_known: session.title_match_total_known,
        title_match_progress: from_library_scan_phase_progress(session.title_match_progress),
        hydration_total_known: session.metadata_total_known,
        hydration_progress: from_library_scan_phase_progress(session.metadata_progress.clone()),
        media_analysis_total_known: session.file_total_known,
        media_analysis_progress: from_library_scan_phase_progress(session.file_progress.clone()),
        metadata_total_known: session.metadata_total_known,
        file_total_known: session.file_total_known,
        metadata_progress: from_library_scan_phase_progress(session.metadata_progress),
        file_progress: from_library_scan_phase_progress(session.file_progress),
        summary: session.summary.map(from_library_scan_summary),
    }
}

pub fn from_job_definition(definition: JobDefinition) -> JobDefinitionPayload {
    JobDefinitionPayload {
        key: JobKeyValue::from_application(definition.key),
        display_name: definition.display_name,
        description: definition.description,
        category: JobCategoryValue::from_application(definition.category),
        section: JobSectionValue::from_application(definition.section),
        manual_trigger_allowed: definition.manual_trigger_allowed,
        uses_library_scan_progress: definition.uses_library_scan_progress,
        schedule: JobScheduleInfoPayload {
            kind: JobScheduleKindValue::from_application(definition.schedule.kind),
            description: definition.schedule.description,
            interval_seconds: definition
                .schedule
                .interval_seconds
                .map(|value| value as i32),
            initial_delay_seconds: definition
                .schedule
                .initial_delay_seconds
                .map(|value| value as i32),
            next_run_at: definition
                .schedule
                .next_run_at
                .map(|value| value.to_rfc3339()),
        },
    }
}

pub fn from_job_run(run: JobRun) -> JobRunPayload {
    JobRunPayload {
        id: run.id,
        job_key: JobKeyValue::from_application(run.job_key),
        display_name: run.display_name,
        category: JobCategoryValue::from_application(run.category),
        section: JobSectionValue::from_application(run.section),
        status: JobRunStatusValue::from_application(run.status),
        trigger_source: JobTriggerSourceValue::from_application(run.trigger_source),
        started_at: run.started_at.to_rfc3339(),
        completed_at: run.completed_at.map(|value| value.to_rfc3339()),
        summary_json: run.summary_json,
        summary_text: run.summary_text,
        error_text: run.error_text,
        progress_json: run.progress_json,
        library_scan_progress: run.library_scan_progress.map(from_library_scan_session),
    }
}

pub fn from_media_rename_plan(plan: RenamePlan) -> MediaRenamePlanPayload {
    MediaRenamePlanPayload {
        facet: MediaFacetValue::from_domain(plan.facet),
        title_id: plan.title_id,
        template: plan.template,
        collision_policy: plan.collision_policy.as_str().to_string(),
        missing_metadata_policy: plan.missing_metadata_policy.as_str().to_string(),
        fingerprint: plan.fingerprint,
        total: plan.total as i32,
        renamable: plan.renamable as i32,
        noop: plan.noop as i32,
        conflicts: plan.conflicts as i32,
        errors: plan.errors as i32,
        items: plan
            .items
            .into_iter()
            .map(from_media_rename_plan_item)
            .collect(),
    }
}

fn from_media_rename_plan_item(item: RenamePlanItem) -> MediaRenamePlanItemPayload {
    MediaRenamePlanItemPayload {
        collection_id: item.collection_id,
        media_file_id: item.media_file_id,
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        normalized_filename: item.normalized_filename,
        collision: item.collision,
        reason_code: item.reason_code,
        write_action: item.write_action.as_str().to_string(),
        source_size_bytes: item.source_size_bytes.map(|value| value.to_string()),
        source_mtime_unix_ms: item.source_mtime_unix_ms.map(|value| value.to_string()),
    }
}

pub fn from_media_rename_apply(result: RenameApplyResult) -> MediaRenameApplyPayload {
    MediaRenameApplyPayload {
        plan_fingerprint: result.plan_fingerprint,
        total: result.total as i32,
        applied: result.applied as i32,
        skipped: result.skipped as i32,
        failed: result.failed as i32,
        items: result
            .items
            .into_iter()
            .map(from_media_rename_apply_item)
            .collect(),
    }
}

fn from_media_rename_apply_item(item: RenameApplyItemResult) -> MediaRenameApplyItemPayload {
    MediaRenameApplyItemPayload {
        collection_id: item.collection_id,
        media_file_id: item.media_file_id,
        current_path: item.current_path,
        proposed_path: item.proposed_path,
        final_path: item.final_path,
        write_action: item.write_action.as_str().to_string(),
        status: item.status.as_str().to_string(),
        reason_code: item.reason_code,
        error_message: item.error_message,
    }
}

pub fn from_collection(collection: Collection) -> CollectionPayload {
    let file_size_bytes = file_size_bytes_for_path(collection.ordered_path.as_deref());
    let map_movie =
        |movie: scryer_domain::InterstitialMovieMetadata| InterstitialMovieMetadataPayload {
            tvdb_id: movie.tvdb_id,
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            content_status: movie.content_status,
            overview: movie.overview,
            poster_url: movie.poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            genres: movie.genres,
            studio: movie.studio,
            digital_release_date: movie.digital_release_date,
            association_confidence: movie.association_confidence,
            continuity_status: movie.continuity_status,
            movie_form: movie.movie_form,
            confidence: movie.confidence,
            signal_summary: movie.signal_summary,
            placement: movie.placement,
            movie_tmdb_id: movie.movie_tmdb_id,
            movie_mal_id: movie.movie_mal_id,
        };
    CollectionPayload {
        id: collection.id,
        title_id: collection.title_id,
        collection_type: collection.collection_type.as_str().to_string(),
        collection_index: collection.collection_index,
        label: collection.label,
        ordered_path: collection.ordered_path,
        narrative_order: collection.narrative_order,
        file_size_bytes,
        first_episode_number: collection.first_episode_number,
        last_episode_number: collection.last_episode_number,
        interstitial_movie: collection.interstitial_movie.map(map_movie),
        interstitial_season_episode: collection.interstitial_season_episode,
        specials_movies: collection
            .specials_movies
            .into_iter()
            .map(map_movie)
            .collect(),
        monitored: collection.monitored,
        created_at: collection.created_at.to_rfc3339(),
    }
}

pub fn file_size_bytes_for_path(ordered_path: Option<&str>) -> Option<i64> {
    let path = stored_path_to_path_buf(ordered_path?);
    fs::metadata(&path).ok().and_then(|metadata| {
        if metadata.is_file() {
            Some(metadata.len() as i64)
        } else {
            None
        }
    })
}

pub fn from_episode(episode: Episode) -> EpisodePayload {
    EpisodePayload {
        id: episode.id,
        title_id: episode.title_id,
        collection_id: episode.collection_id,
        episode_type: episode.episode_type.as_str().to_string(),
        episode_number: episode.episode_number,
        season_number: episode.season_number,
        episode_label: episode.episode_label,
        title: episode.title,
        overview: episode.overview,
        air_date: episode.air_date,
        duration_seconds: episode.duration_seconds,
        has_multi_audio: episode.has_multi_audio,
        has_subtitle: episode.has_subtitle,
        is_filler: episode.is_filler,
        is_recap: episode.is_recap,
        absolute_number: episode.absolute_number,
        tvdb_id: episode.tvdb_id,
        image_url: episode.image_url,
        monitored: episode.monitored,
        created_at: episode.created_at.to_rfc3339(),
    }
}

pub fn from_calendar_episode(ep: CalendarEpisode) -> CalendarEpisodePayload {
    CalendarEpisodePayload {
        id: ep.id,
        title_id: ep.title_id,
        library_id: ep.library_id,
        library_name: ep.library_name,
        library_slug: ep.library_slug,
        title_name: ep.title_name,
        title_slug: ep.title_slug,
        title_facet: ep.title_facet,
        season_number: ep.season_number,
        episode_number: ep.episode_number,
        episode_title: ep.episode_title,
        air_date: ep.air_date,
        monitored: ep.monitored,
    }
}

pub fn from_title_media_file(file: scryer_application::TitleMediaFile) -> TitleMediaFilePayload {
    TitleMediaFilePayload {
        id: file.id,
        title_id: file.title_id,
        episode_id: file.episode_id,
        file_path: file.file_path,
        size_bytes: file.size_bytes.to_string(),
        quality_label: file.quality_label,
        scan_status: file.scan_status,
        created_at: file.created_at,
        video_codec: file.video_codec.map(|codec| codec.to_string()),
        video_width: file.video_width,
        video_height: file.video_height,
        video_bitrate_kbps: file.video_bitrate_kbps,
        video_bit_depth: file.video_bit_depth,
        video_hdr_format: file.video_hdr_format,
        video_frame_rate: file.video_frame_rate,
        video_profile: file.video_profile,
        audio_codec: file.audio_codec,
        audio_profile: file.audio_profile,
        audio_channels: file.audio_channels,
        audio_bitrate_kbps: file.audio_bitrate_kbps,
        audio_languages: file.audio_languages,
        audio_streams: file
            .audio_streams
            .into_iter()
            .map(|s| crate::types::AudioStreamDetailPayload {
                codec: s.codec,
                profile: s.profile,
                channels: s.channels,
                language: s.language,
                bitrate_kbps: s.bitrate_kbps,
            })
            .collect(),
        subtitle_languages: file.subtitle_languages,
        subtitle_codecs: file.subtitle_codecs,
        subtitle_streams: file
            .subtitle_streams
            .into_iter()
            .map(|s| crate::types::SubtitleStreamDetailPayload {
                codec: s.codec,
                language: s.language,
                name: s.name,
                forced: s.forced,
                default: s.default,
            })
            .collect(),
        has_multiaudio: file.has_multiaudio,
        duration_seconds: file.duration_seconds,
        num_chapters: file.num_chapters,
        container_format: file.container_format,
        scene_name: file.scene_name,
        release_group: file.release_group,
        source_type: file.source_type,
        resolution: file.resolution,
        video_codec_parsed: file.video_codec_parsed.map(|codec| codec.to_string()),
        audio_codec_parsed: file.audio_codec_parsed,
        audio_channels_parsed: file.audio_channels_parsed,
        acquisition_score: file.acquisition_score,
        scoring_log: file.scoring_log,
        indexer_source: file.indexer_source,
        grabbed_release_title: file.grabbed_release_title,
        grabbed_at: file.grabbed_at,
        edition: file.edition,
        original_file_path: file.original_file_path,
        release_hash: file.release_hash,
    }
}

pub fn from_user(user: User) -> UserPayload {
    from_user_with_auth_factor_status(user, scryer_application::UserAuthFactorStatus::default())
}

pub fn from_user_with_auth_factor_status(
    user: User,
    auth_factor_status: scryer_application::UserAuthFactorStatus,
) -> UserPayload {
    let User {
        id,
        username,
        password_hash,
        account_kind,
        authorization,
        ..
    } = user;

    let app_permissions = authorization
        .app
        .to_permissions()
        .into_iter()
        .map(AppPermissionValue::from_domain)
        .collect();
    let mut library_permissions = authorization
        .libraries
        .into_iter()
        .map(
            |(library_id, permissions)| UserLibraryPermissionGrantPayload {
                library_id,
                permissions: permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
    library_permissions.sort_by(|left, right| left.library_id.cmp(&right.library_id));

    UserPayload {
        id,
        username,
        has_password: password_hash.is_some(),
        has_mfa: auth_factor_status.has_mfa,
        has_passkey: auth_factor_status.has_passkey,
        account_kind: UserAccountKindValue::from_domain(account_kind),
        app_permissions,
        library_permissions,
    }
}

pub fn from_linked_account(account: scryer_domain::UserExternalAccount) -> LinkedAccountPayload {
    LinkedAccountPayload {
        id: account.id,
        user_id: account.user_id,
        provider: ExternalAccountProviderValue::from_domain(account.provider),
        connection_id: account.connection_id,
        external_user_id: account.external_user_id,
        username: account.username,
        display_name: account.display_name,
        avatar_url: account.avatar_url,
        status: ExternalAccountStatusValue::from_domain(account.status),
        verified_at: account.verified_at.map(|value| value.to_rfc3339()),
        last_login_at: account.last_login_at.map(|value| value.to_rfc3339()),
        created_at: account.created_at.to_rfc3339(),
        updated_at: account.updated_at.to_rfc3339(),
    }
}

pub fn from_media_server_connection(
    connection: scryer_domain::MediaServerConnection,
) -> MediaServerConnectionPayload {
    let api_key_present = connection.api_key_present();
    let machine_id_present = connection.machine_id.is_some();
    MediaServerConnectionPayload {
        id: connection.id,
        provider: MediaServerProviderValue::from_domain(connection.provider),
        display_name: connection.display_name,
        base_url: connection.base_url,
        enabled: connection.enabled,
        login_enabled: connection.login_enabled,
        linking_enabled: connection.linking_enabled,
        auto_add_enabled: connection.auto_add_enabled,
        default_app_permissions: connection
            .default_app_permissions
            .to_permissions()
            .into_iter()
            .map(AppPermissionValue::from_domain)
            .collect(),
        default_library_grants: connection
            .default_library_grants
            .into_iter()
            .map(|grant| MediaServerDefaultLibraryGrantPayload {
                library_id: grant.library_id,
                permissions: grant
                    .permissions
                    .with_request_shadowing()
                    .to_permissions()
                    .into_iter()
                    .map(LibraryPermissionValue::from_domain)
                    .collect(),
            })
            .collect(),
        machine_id_present,
        api_key_present,
        path_mappings: connection
            .path_mappings
            .into_iter()
            .map(|mapping| MediaServerPathMappingPayload {
                source_path: mapping.source_path,
                destination_path: mapping.destination_path,
            })
            .collect(),
        created_at: connection.created_at.to_rfc3339(),
        updated_at: connection.updated_at.to_rfc3339(),
    }
}

pub fn from_jellyfin_server_user(
    user: scryer_application::JellyfinServerUser,
) -> JellyfinServerUserPayload {
    JellyfinServerUserPayload {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        avatar_url: user.avatar_url,
    }
}

pub fn from_plex_server_discovery(
    server: scryer_application::PlexServerDiscovery,
) -> PlexServerDiscoveryPayload {
    PlexServerDiscoveryPayload {
        id: server.id,
        name: server.name,
    }
}

pub fn from_activity_event(event: ActivityEvent) -> ActivityEventPayload {
    ActivityEventPayload {
        id: event.id,
        kind: ActivityKindValue::from_application(event.kind),
        severity: ActivitySeverityValue::from_application(event.severity),
        channels: event
            .channels
            .into_iter()
            .map(ActivityChannelValue::from_application)
            .collect(),
        actor_user_id: event.actor_user_id,
        title_id: event.title_id,
        facet: event.facet.as_deref().and_then(MediaFacetValue::parse),
        message: event.message,
        occurred_at: event.occurred_at.to_rfc3339(),
    }
}

pub fn from_import_record(record: scryer_domain::ImportRecord) -> ImportRecordPayload {
    // Deserialize result_json to extract structured fields
    let (error_message, decision, skip_reason, title_id, source_path, dest_path) =
        if let Some(ref result_json) = record.result_json {
            if let Ok(result) = serde_json::from_str::<scryer_domain::ImportResult>(result_json) {
                (
                    result.error_message,
                    Some(ImportDecisionValue::from_domain(result.decision)),
                    result.skip_reason.map(ImportSkipReasonValue::from_domain),
                    result.title_id,
                    Some(result.source_path),
                    result.dest_path,
                )
            } else {
                (None, None, None, None, None, None)
            }
        } else {
            (None, None, None, None, None, None)
        };

    let payload = serde_json::from_str::<serde_json::Value>(&record.payload_json).ok();
    let source_title = payload.as_ref().and_then(|payload| {
        import_source_title_from_payload(
            payload,
            &record.source_system,
            &record.source_ref,
            source_path.as_deref(),
        )
    });
    let facet = payload.as_ref().and_then(import_facet_from_payload);

    ImportRecordPayload {
        id: record.id,
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_title,
        facet,
        import_type: ImportTypeValue::from_domain(record.import_type),
        status: ImportStatusValue::from_domain(record.status),
        error_message,
        decision,
        skip_reason,
        title_id,
        source_path,
        dest_path,
        started_at: record.started_at,
        finished_at: record.finished_at,
        created_at: record.created_at,
    }
}

pub fn from_wanted_item(item: scryer_application::WantedItem) -> WantedItemPayload {
    WantedItemPayload {
        id: item.id,
        title_id: item.title_id,
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: item.title_facet,
        library_id: item.library_id,
        library_name: item.library_name,
        library_slug: item.library_slug,
        episode_id: item.episode_id,
        collection_id: item.collection_id,
        season_number: item.season_number,
        episode_number: item.episode_number,
        media_type: WantedMediaTypeValue::parse(&item.media_type)
            .expect("wanted item media_type should map to GraphQL enum"),
        search_phase: WantedSearchPhaseValue::parse(&item.search_phase)
            .expect("wanted item search_phase should map to GraphQL enum"),
        next_search_at: item.next_search_at,
        last_search_at: item.last_search_at,
        search_count: item.search_count,
        baseline_date: item.baseline_date,
        status: WantedStatusValue::from_application(item.status),
        grabbed_release: item.grabbed_release,
        current_score: item.current_score,
        latest_release_decision: item.latest_release_decision.map(from_release_decision),
        mismatch_recovery_eligible: item.mismatch_recovery_eligible,
        created_at: item.created_at,
        updated_at: item.updated_at,
    }
}

pub fn from_release_decision(
    decision: scryer_application::ReleaseDecision,
) -> ReleaseDecisionPayload {
    ReleaseDecisionPayload {
        id: decision.id,
        wanted_item_id: decision.wanted_item_id,
        title_id: decision.title_id,
        release_title: decision.release_title,
        release_url: decision.release_url,
        release_size_bytes: decision.release_size_bytes,
        decision_code: decision.decision_code,
        candidate_score: decision.candidate_score,
        current_score: decision.current_score,
        score_delta: decision.score_delta,
        explanation_json: decision.explanation_json,
        created_at: decision.created_at,
    }
}

pub fn from_decision_code_count(
    item: scryer_application::DecisionCodeCount,
) -> DecisionCodeCountPayload {
    DecisionCodeCountPayload {
        code: item.code,
        count: item.count,
    }
}

pub fn from_wanted_status_count(
    item: scryer_application::WantedStatusCount,
) -> WantedStatusCountPayload {
    WantedStatusCountPayload {
        status: scryer_application::WantedStatus::parse(&item.status)
            .map(WantedStatusValue::from_application)
            .unwrap_or(WantedStatusValue::Wanted),
        count: item.count,
    }
}

pub fn from_pending_release_status_count(
    item: scryer_application::PendingReleaseStatusCount,
) -> PendingReleaseStatusCountPayload {
    PendingReleaseStatusCountPayload {
        status: scryer_application::PendingReleaseStatus::parse(&item.status)
            .map(PendingReleaseStatusValue::from_application)
            .unwrap_or(PendingReleaseStatusValue::Waiting),
        count: item.count,
    }
}

pub fn from_title_acquisition_diagnostics(
    value: scryer_application::TitleAcquisitionDiagnostics,
) -> TitleAcquisitionDiagnosticsPayload {
    TitleAcquisitionDiagnosticsPayload {
        recent_decisions: value
            .recent_decisions
            .into_iter()
            .map(from_release_decision)
            .collect(),
        decision_counts: value
            .decision_counts
            .into_iter()
            .map(from_decision_code_count)
            .collect(),
        wanted_status_counts: value
            .wanted_status_counts
            .into_iter()
            .map(from_wanted_status_count)
            .collect(),
        pending_release_counts: value
            .pending_release_counts
            .into_iter()
            .map(from_pending_release_status_count)
            .collect(),
        mismatch_recovery_eligible_count: value.mismatch_recovery_eligible_count,
        latest_decision_at: value.latest_decision_at,
        latest_wanted_search_at: value.latest_wanted_search_at,
    }
}

pub fn from_system_health(health: SystemHealth) -> SystemHealthPayload {
    SystemHealthPayload {
        service_ready: health.service_ready,
        db_path: health.db_path,
        datastore_engine: health.datastore_engine,
        datastore_migration_key: health.datastore_migration_key,
        total_titles: health.total_titles as i32,
        monitored_titles: health.monitored_titles as i32,
        total_users: health.total_users as i32,
        titles_movie: health.titles_movie as i32,
        titles_series: health.titles_series as i32,
        titles_anime: health.titles_anime as i32,
        titles_other: health.titles_other as i32,
        recent_events: health.recent_events as i32,
        recent_event_preview: health.recent_event_preview,
        db_migration_version: health.db_migration_version,
        indexer_stats: health
            .indexer_stats
            .into_iter()
            .map(|s| IndexerQueryStatsPayload {
                indexer_id: s.indexer_id,
                indexer_name: s.indexer_name,
                queries_last_24h: s.queries_last_24h as i32,
                successful_last_24h: s.successful_last_24h as i32,
                failed_last_24h: s.failed_last_24h as i32,
                last_query_at: s.last_query_at,
                api_current: s.api_current.map(|v| v as i32),
                api_max: s.api_max.map(|v| v as i32),
                grab_current: s.grab_current.map(|v| v as i32),
                grab_max: s.grab_max.map(|v| v as i32),
            })
            .collect(),
    }
}

pub fn from_smg_version_compatibility_notice(
    notice: SmgVersionCompatibilityNotice,
) -> SmgVersionCompatibilityNoticePayload {
    SmgVersionCompatibilityNoticePayload {
        status: notice.status,
        minimum_version: notice.minimum_version,
        your_version: notice.your_version,
        message: notice.message,
        upgrade_deadline: notice.upgrade_deadline,
    }
}

pub fn from_rule_set(rs: RuleSet) -> RuleSetPayload {
    RuleSetPayload {
        id: rs.id,
        name: rs.name,
        description: rs.description,
        rego_source: scryer_rules::strip_editor_source(&rs.rego_source),
        enabled: rs.enabled,
        priority: rs.priority,
        applied_facets: rs
            .applied_facets
            .iter()
            .map(|f| format!("{:?}", f).to_lowercase())
            .collect(),
        is_managed: rs.is_managed,
        managed_key: rs.managed_key,
        created_at: rs.created_at.to_rfc3339(),
        updated_at: rs.updated_at.to_rfc3339(),
    }
}

pub fn from_registry_plugin(p: RegistryPlugin) -> RegistryPluginPayload {
    RegistryPluginPayload {
        id: p.id,
        name: p.name,
        description: p.description,
        version: p.version,
        latest_version: p.latest_version,
        plugin_type: p.plugin_type,
        provider_type: p.provider_type,
        author: p.author,
        official: p.official,
        publisher: p.publisher,
        support_tier: support_tier_label(p.support_tier),
        status: p.status,
        docs_url: p.docs_url,
        source_repo: p.source_repo,
        builtin: p.builtin,
        source_url: p.source_url,
        source_kind: p.source_kind,
        blocked_reason: p.blocked_reason,
        bytes: p.bytes.and_then(|value| i64::try_from(value).ok()),
        is_installed: p.is_installed,
        is_enabled: p.is_enabled,
        installed_version: p.installed_version,
        update_available: p.update_available,
        install_in_progress: p.install_in_progress,
        default_base_url: p.default_base_url,
    }
}

pub fn from_plugin_install_progress(
    snapshot: scryer_application::PluginInstallProgressSnapshot,
) -> PluginInstallProgressPayload {
    PluginInstallProgressPayload {
        plugin_id: snapshot.plugin_id,
        operation_kind: match snapshot.operation_kind {
            scryer_application::PluginInstallOperationKind::Install => {
                PluginInstallOperationKindValue::Install
            }
            scryer_application::PluginInstallOperationKind::Upgrade => {
                PluginInstallOperationKindValue::Upgrade
            }
        },
        state: match snapshot.state {
            scryer_application::PluginInstallState::Downloading => {
                PluginInstallStateValue::Downloading
            }
            scryer_application::PluginInstallState::Verifying => PluginInstallStateValue::Verifying,
            scryer_application::PluginInstallState::Installing => {
                PluginInstallStateValue::Installing
            }
            scryer_application::PluginInstallState::Succeeded => PluginInstallStateValue::Succeeded,
            scryer_application::PluginInstallState::Failed => PluginInstallStateValue::Failed,
        },
        label: snapshot.label,
        step_index: snapshot.step_index,
        step_count: snapshot.step_count,
        message: snapshot.message,
        error: snapshot.error,
    }
}

pub fn from_external_import_monitor_warmup_progress(
    snapshot: scryer_application::ExternalImportMonitorWarmupProgressSnapshot,
) -> ExternalImportMonitorWarmupProgressPayload {
    let map_phase_progress =
        |progress: scryer_application::ExternalImportMonitorWarmupPhaseProgress| {
            LibraryScanPhaseProgressPayload {
                total: progress.total,
                completed: progress.completed,
                failed: progress.failed,
            }
        };

    ExternalImportMonitorWarmupProgressPayload {
        session_id: snapshot.session_id,
        status: match snapshot.status {
            scryer_application::ExternalImportMonitorWarmupStatus::Queued => {
                ExternalImportMonitorWarmupStatusValue::Queued
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Running => {
                ExternalImportMonitorWarmupStatusValue::Running
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Completed => {
                ExternalImportMonitorWarmupStatusValue::Completed
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Canceled => {
                ExternalImportMonitorWarmupStatusValue::Canceled
            }
            scryer_application::ExternalImportMonitorWarmupStatus::Failed => {
                ExternalImportMonitorWarmupStatusValue::Failed
            }
        },
        phase: match snapshot.phase {
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingMovies => {
                ExternalImportMonitorWarmupPhaseValue::LoadingMovies
            }
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingSeries => {
                ExternalImportMonitorWarmupPhaseValue::LoadingSeries
            }
            scryer_application::ExternalImportMonitorWarmupPhase::LoadingEpisodes => {
                ExternalImportMonitorWarmupPhaseValue::LoadingEpisodes
            }
            scryer_application::ExternalImportMonitorWarmupPhase::BuildingSnapshot => {
                ExternalImportMonitorWarmupPhaseValue::BuildingSnapshot
            }
            scryer_application::ExternalImportMonitorWarmupPhase::Ready => {
                ExternalImportMonitorWarmupPhaseValue::Ready
            }
        },
        started_at: snapshot.started_at,
        updated_at: snapshot.updated_at,
        overall_total_known: snapshot.overall_total_known,
        overall_progress: map_phase_progress(snapshot.overall_progress),
        movies_total_known: snapshot.movies_total_known,
        movies_progress: map_phase_progress(snapshot.movies_progress),
        series_total_known: snapshot.series_total_known,
        series_progress: map_phase_progress(snapshot.series_progress),
        episode_fetch_total_known: snapshot.episode_fetch_total_known,
        episode_fetch_expected_total: snapshot.episode_fetch_expected_total,
        episode_fetch_expected_monitored_total: snapshot.episode_fetch_expected_monitored_total,
        episode_fetch_progress: map_phase_progress(snapshot.episode_fetch_progress),
        snapshot_build_total_known: snapshot.snapshot_build_total_known,
        snapshot_build_progress: map_phase_progress(snapshot.snapshot_build_progress),
        matched_movie_count: snapshot.matched_movie_count,
        matched_series_count: snapshot.matched_series_count,
        unmatched_movie_count: snapshot.unmatched_movie_count,
        unmatched_series_count: snapshot.unmatched_series_count,
        ambiguous_movie_count: snapshot.ambiguous_movie_count,
        ambiguous_series_count: snapshot.ambiguous_series_count,
        error_message: snapshot.error_message,
    }
}

pub fn from_notification_channel(
    ch: scryer_domain::NotificationChannelConfig,
) -> NotificationChannelPayload {
    NotificationChannelPayload {
        id: ch.id,
        name: ch.name,
        channel_type: ch.channel_type.as_str().to_string(),
        config_json: ch.config_json,
        media_server_connection_id: ch.media_server_connection_id,
        is_enabled: ch.is_enabled,
        created_at: ch.created_at.to_rfc3339(),
        updated_at: ch.updated_at.to_rfc3339(),
    }
}

pub fn from_notification_subscription(
    sub: scryer_domain::NotificationSubscription,
) -> NotificationSubscriptionPayload {
    NotificationSubscriptionPayload {
        id: sub.id,
        channel_id: sub.channel_id,
        target_kind: sub.target_kind.as_str().to_string(),
        target_id: sub.target_id,
        event_type: sub.event_type.as_str().to_string(),
        scope: sub.scope,
        scope_id: sub.scope_id,
        is_enabled: sub.is_enabled,
        created_at: sub.created_at.to_rfc3339(),
        updated_at: sub.updated_at.to_rfc3339(),
    }
}

pub fn from_notification_target(
    target: scryer_domain::NotificationTarget,
) -> NotificationTargetPayload {
    NotificationTargetPayload {
        id: target.id,
        target_kind: target.target_kind.as_str().to_string(),
        name: target.name,
        provider_type: target.provider_type,
        media_server_provider: target
            .media_server_provider
            .map(MediaServerProviderValue::from_domain),
        media_server_connection_id: target.media_server_connection_id,
        is_enabled: target.is_enabled,
    }
}

pub fn from_domain_event(event: DomainEvent) -> DomainEventEnvelopePayload {
    let (stream_kind, stream_id) = match event.stream {
        scryer_domain::DomainEventStream::Global => ("global".to_string(), None),
        scryer_domain::DomainEventStream::Title { title_id } => {
            ("title".to_string(), Some(title_id))
        }
        scryer_domain::DomainEventStream::LibraryScan { session_id } => {
            ("library_scan".to_string(), Some(session_id))
        }
        scryer_domain::DomainEventStream::JobRun { run_id } => {
            ("job_run".to_string(), Some(run_id))
        }
        scryer_domain::DomainEventStream::DownloadQueueItem { item_id } => {
            ("download_queue_item".to_string(), Some(item_id))
        }
    };

    DomainEventEnvelopePayload {
        sequence: event.sequence,
        event_id: event.event_id,
        occurred_at: event.occurred_at.to_rfc3339(),
        actor_user_id: event.actor_user_id,
        title_id: event.title_id,
        facet: event.facet.map(MediaFacetValue::from_domain),
        event_type: DomainEventTypeValue::from_domain(event.payload.event_type()),
        stream_kind,
        stream_id,
        payload_json: async_graphql::Json(
            serde_json::to_value(event.payload).unwrap_or(serde_json::Value::Null),
        ),
    }
}

pub fn from_plugin_installation(inst: PluginInstallation) -> PluginInstallationPayload {
    PluginInstallationPayload {
        id: inst.id,
        plugin_id: inst.plugin_id,
        name: inst.name,
        description: inst.description,
        version: inst.version,
        sdk_version: inst.sdk_version,
        sdk_constraint: inst.sdk_constraint,
        plugin_type: inst.plugin_type,
        provider_type: inst.provider_type,
        is_enabled: inst.is_enabled,
        is_builtin: inst.is_builtin,
        source_kind: match inst.source_kind {
            scryer_domain::PluginSourceKind::Bundled => "bundled".to_string(),
            scryer_domain::PluginSourceKind::Downloaded => "downloaded".to_string(),
            scryer_domain::PluginSourceKind::Community => "community".to_string(),
            scryer_domain::PluginSourceKind::Manual => "manual".to_string(),
        },
        source_url: inst.source_url,
        publisher: inst.publisher,
        support_tier: support_tier_label(inst.support_tier),
        docs_url: inst.docs_url,
        source_repo: inst.source_repo,
        manifest_url: inst.manifest_url,
        wasm_digest: inst.wasm_digest,
        artifact_digest: inst.artifact_digest,
        installed_at: inst.installed_at.to_rfc3339(),
        updated_at: inst.updated_at.to_rfc3339(),
    }
}

pub fn from_plugin_catalog_status(status: PluginCatalogStatus) -> PluginCatalogStatusPayload {
    PluginCatalogStatusPayload {
        refresh_state: status.refresh_state,
        github_available: status.github_available,
        last_checked_at: status.last_checked_at,
        outage_message: status.outage_message,
        blocked_actions: status.blocked_actions,
        restore_warnings: status.restore_warnings,
        last_error: status.last_error,
    }
}

pub fn from_manual_plugin_preview(preview: ManualPluginPreview) -> ManualPluginPreviewPayload {
    ManualPluginPreviewPayload {
        github_repo_url: preview.github_repo_url,
        plugin: from_registry_plugin(preview.plugin),
    }
}

pub fn from_backup_info(info: BackupInfo) -> BackupInfoPayload {
    BackupInfoPayload {
        filename: info.filename,
        size_bytes: info.size_bytes.to_string(),
        created_at: info.created_at,
        format_version: info.format_version,
        source_engine: info.source_engine,
        source_migration_key: info.source_migration_key,
        encrypted: info.encrypted,
        row_counts: info
            .row_counts
            .into_iter()
            .map(|(table, row_count)| BackupRowCountPayload {
                table,
                row_count: row_count.to_string(),
            })
            .collect(),
        trigger: info.trigger.as_str().to_string(),
        status: info.status.as_str().to_string(),
        error_message: info.error_message,
    }
}

pub fn from_rss_sync_report(report: RssSyncReport) -> RssSyncReportPayload {
    RssSyncReportPayload {
        releases_fetched: report.releases_fetched as i32,
        releases_matched: report.releases_matched as i32,
        releases_grabbed: report.releases_grabbed as i32,
        releases_held: report.releases_held as i32,
    }
}

pub fn from_pending_release(pr: PendingRelease) -> PendingReleasePayload {
    PendingReleasePayload {
        id: pr.id,
        wanted_item_id: pr.wanted_item_id,
        title_id: pr.title_id,
        release_title: pr.release_title,
        release_url: pr.release_url,
        release_size_bytes: pr.release_size_bytes.map(|v| v.to_string()),
        release_score: pr.release_score,
        scoring_log_json: pr.scoring_log_json,
        indexer_source: pr.indexer_source,
        added_at: pr.added_at,
        delay_until: pr.delay_until,
        status: PendingReleaseStatusValue::from_application(pr.status),
    }
}

pub fn from_pp_script(s: scryer_domain::PostProcessingScript) -> PostProcessingScriptPayload {
    PostProcessingScriptPayload {
        id: s.id,
        name: s.name,
        description: s.description,
        script_type: s.script_type.as_str().to_string(),
        script_content: s.script_content,
        applied_facets: s.applied_facets,
        execution_mode: s.execution_mode.as_str().to_string(),
        timeout_secs: s.timeout_secs as i32,
        priority: s.priority,
        enabled: s.enabled,
        debug: s.debug,
        created_at: s.created_at.to_rfc3339(),
        updated_at: s.updated_at.to_rfc3339(),
    }
}

pub fn from_pp_script_run(
    r: scryer_domain::PostProcessingScriptRun,
) -> PostProcessingScriptRunPayload {
    PostProcessingScriptRunPayload {
        id: r.id,
        script_id: r.script_id,
        script_name: r.script_name,
        title_id: r.title_id,
        title_name: r.title_name,
        facet: r.facet.as_deref().and_then(MediaFacetValue::parse),
        file_path: r.file_path,
        status: r.status.as_str().to_string(),
        exit_code: r.exit_code,
        stdout_tail: r.stdout_tail,
        stderr_tail: r.stderr_tail,
        duration_ms: r.duration_ms.map(|v| v as i32),
        env_payload_json: r.env_payload_json,
        started_at: r.started_at,
        completed_at: r.completed_at,
    }
}

pub fn from_title_history_record(record: TitleHistoryRecord) -> TitleHistoryEventPayload {
    TitleHistoryEventPayload {
        id: record.id,
        title_id: record.title_id,
        title_name: record.title_name,
        facet: record.facet.map(MediaFacetValue::from_domain),
        episode_id: record.episode_id,
        episode_ids: record.episode_ids,
        collection_id: record.collection_id,
        event_type: record.event_type.as_str().to_string(),
        source_title: record.source_title,
        display_title: record.display_title,
        source_system: record.source_system,
        source_ref: record.source_ref,
        source_hint: record.source_hint,
        quality: record.quality,
        download_id: record.download_id,
        client_id: record.client_id,
        client_name: record.client_name,
        import_id: record.import_id,
        skip_reason: record.skip_reason,
        retry_requires_password: record.retry_requires_password,
        failure_reason: record.failure_reason,
        blocklist_reason: record.blocklist_reason,
        source_path: record.source_path,
        dest_path: record.dest_path,
        data_json: record.data_json,
        occurred_at: record.occurred_at,
        created_at: record.created_at,
    }
}

pub fn from_title_history_page(page: TitleHistoryPage) -> TitleHistoryPagePayload {
    TitleHistoryPagePayload {
        records: page
            .records
            .into_iter()
            .map(from_title_history_record)
            .collect(),
        total_count: page.total_count,
    }
}

#[cfg(test)]
mod tests {
    use super::from_import_record;
    use crate::types::MediaFacetValue;
    use scryer_domain::{CompletedDownload, ImportRecord, ImportStatus, ImportType};

    #[test]
    fn from_import_record_uses_release_folder_for_numeric_weaver_job_name() {
        let payload = CompletedDownload {
            client_type: "weaver".to_string(),
            client_id: String::new(),
            download_client_item_id: "10495".to_string(),
            name: "10495".to_string(),
            dest_dir: "/downloads/Example.Show.S01E01.1080p.WEB-DL".to_string(),
            category: Some("anime".to_string()),
            size_bytes: None,
            completed_at: None,
            parameters: vec![("*scryer_facet".to_string(), "anime".to_string())],
        };
        let record = ImportRecord {
            id: "import-1".to_string(),
            source_client_id: None,
            source_system: "weaver".to_string(),
            source_ref: "10495".to_string(),
            import_type: ImportType::SeriesDownload,
            status: ImportStatus::Completed,
            payload_json: serde_json::to_string(&payload).expect("serialize completed download"),
            result_json: None,
            started_at: None,
            finished_at: None,
            created_at: "2026-04-27T20:17:00Z".to_string(),
            updated_at: "2026-04-27T20:17:00Z".to_string(),
        };

        let mapped = from_import_record(record);
        assert_eq!(
            mapped.source_title.as_deref(),
            Some("Example.Show.S01E01.1080p.WEB-DL")
        );
        assert!(matches!(mapped.facet, Some(MediaFacetValue::Anime)));
    }
}
