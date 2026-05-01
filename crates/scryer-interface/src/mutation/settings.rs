use async_graphql::{Context, Error, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{
    AcquisitionSettings as AppAcquisitionSettings, QualityProfile, QualityProfileCriteria,
    SecuritySettings as AppSecuritySettings, UpdateGeneralSettings as AppUpdateGeneralSettings,
    UpdateSecuritySettings as AppUpdateSecuritySettings,
    UpdateSubtitleSettings as AppUpdateSubtitleSettings,
};
use scryer_domain::Entitlement;

use crate::context::{actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, to_gql_error};
use crate::mappers::{
    from_download_client_routing_entry, from_indexer_routing_entry, from_library_paths_settings,
    from_media_settings, from_quality_profile_settings, from_service_settings,
    from_tvdb_scan_operation, from_user,
};
use crate::types::*;

#[derive(Default)]
pub(crate) struct SettingsMutations;

fn from_subtitle_settings(
    settings: scryer_application::SubtitleSettings,
) -> SubtitleSettingsPayload {
    SubtitleSettingsPayload {
        enabled: settings.enabled,
        languages: settings
            .languages
            .into_iter()
            .map(|language| SubtitleLanguagePreferencePayload {
                code: language.code,
                hearing_impaired: language.hearing_impaired,
                forced: language.forced,
            })
            .collect(),
        auto_download_on_import: settings.auto_download_on_import,
        minimum_score_series: settings.minimum_score_series,
        minimum_score_movie: settings.minimum_score_movie,
        search_interval_hours: settings.search_interval_hours,
        include_ai_translated: settings.include_ai_translated,
        include_machine_translated: settings.include_machine_translated,
        sync_enabled: settings.sync_enabled,
        sync_threshold_series: settings.sync_threshold_series,
        sync_threshold_movie: settings.sync_threshold_movie,
        sync_max_offset_seconds: settings.sync_max_offset_seconds,
    }
}

fn from_acquisition_settings(
    settings: scryer_application::AcquisitionSettings,
) -> AcquisitionSettingsPayload {
    AcquisitionSettingsPayload {
        enabled: settings.enabled,
        upgrade_cooldown_hours: settings.upgrade_cooldown_hours,
        same_tier_min_delta: settings.same_tier_min_delta,
        cross_tier_min_delta: settings.cross_tier_min_delta,
        forced_upgrade_delta_bypass: settings.forced_upgrade_delta_bypass,
        poll_interval_seconds: settings.poll_interval_seconds,
        sync_interval_seconds: settings.sync_interval_seconds,
        batch_size: settings.batch_size,
    }
}

fn from_general_settings(settings: scryer_application::GeneralSettings) -> GeneralSettingsPayload {
    GeneralSettingsPayload {
        keep_history_forever: settings.keep_history_forever,
        history_retention_days: settings.history_retention_days,
    }
}

fn from_security_settings(
    settings: AppSecuritySettings,
    auth_runtime: &crate::context::AuthRuntimeStateSnapshot,
) -> SecuritySettingsPayload {
    SecuritySettingsPayload {
        form_login_enabled: settings.form_login_enabled,
        skip_login_for_local_ips: settings.skip_login_for_local_ips,
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        env_override_active: auth_runtime.env_override_active,
        env_override_description: auth_runtime.env_override_description.clone(),
    }
}

fn from_delay_profile(profile: scryer_application::DelayProfile) -> DelayProfilePayload {
    DelayProfilePayload {
        id: profile.id,
        name: profile.name,
        usenet_delay_minutes: profile.usenet_delay_minutes as i32,
        torrent_delay_minutes: profile.torrent_delay_minutes as i32,
        preferred_protocol: DelayProfilePreferredProtocolValue::from_application(
            profile.preferred_protocol,
        ),
        min_age_minutes: profile.min_age_minutes as i32,
        bypass_score_threshold: profile.bypass_score_threshold,
        applies_to_facets: profile
            .applies_to_facets
            .into_iter()
            .filter_map(|facet| MediaFacetValue::parse(&facet))
            .collect(),
        tags: profile.tags,
        priority: profile.priority,
        enabled: profile.enabled,
    }
}

fn normalize_quality_profile(profile: QualityProfile) -> QualityProfile {
    let normalize_list = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.to_ascii_lowercase()))
            .collect::<Vec<_>>()
    };

    let normalize_quality_tiers = |values: Vec<String>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .map(|value| value.trim().to_ascii_uppercase())
            .filter(|value| !value.is_empty())
            .filter(|value| seen.insert(value.clone()))
            .collect::<Vec<_>>()
    };

    let criteria = profile.criteria;
    let mut facet_persona_overrides = std::collections::HashMap::new();
    for (scope, persona) in criteria.facet_persona_overrides {
        if let Some(scope) = ContentScopeValue::parse(&scope) {
            facet_persona_overrides.insert(scope.as_scope_id().to_string(), persona);
        }
    }

    QualityProfile {
        id: profile.id.trim().to_string(),
        name: profile.name.trim().to_string(),
        criteria: QualityProfileCriteria {
            quality_tiers: normalize_quality_tiers(criteria.quality_tiers),
            archival_quality: criteria
                .archival_quality
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: normalize_list(criteria.source_allowlist),
            source_blocklist: normalize_list(criteria.source_blocklist),
            video_codec_allowlist: normalize_list(criteria.video_codec_allowlist),
            video_codec_blocklist: normalize_list(criteria.video_codec_blocklist),
            audio_codec_allowlist: normalize_list(criteria.audio_codec_allowlist),
            audio_codec_blocklist: normalize_list(criteria.audio_codec_blocklist),
            atmos_preferred: criteria.atmos_preferred,
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: criteria.prefer_dual_audio,
            required_audio_languages: normalize_list(criteria.required_audio_languages),
            scoring_persona: criteria.scoring_persona,
            scoring_overrides: criteria.scoring_overrides,
            cutoff_tier: criteria
                .cutoff_tier
                .map(|value| value.trim().to_ascii_uppercase())
                .filter(|value| !value.is_empty()),
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides,
        },
    }
}

fn quality_profile_from_input(
    input: QualityProfileInput,
    existing: Option<&QualityProfile>,
) -> GqlResult<QualityProfile> {
    let criteria = input.criteria;

    let profile = normalize_quality_profile(QualityProfile {
        id: input.id,
        name: input.name,
        criteria: QualityProfileCriteria {
            quality_tiers: criteria.quality_tiers,
            archival_quality: criteria.archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist: criteria.source_allowlist,
            source_blocklist: criteria.source_blocklist,
            video_codec_allowlist: criteria.video_codec_allowlist,
            video_codec_blocklist: criteria.video_codec_blocklist,
            audio_codec_allowlist: criteria.audio_codec_allowlist,
            audio_codec_blocklist: criteria.audio_codec_blocklist,
            atmos_preferred: existing
                .map(|profile| profile.criteria.atmos_preferred)
                .unwrap_or(false),
            dolby_vision_allowed: criteria.dolby_vision_allowed,
            detected_hdr_allowed: criteria.detected_hdr_allowed,
            prefer_remux: criteria.prefer_remux,
            allow_bd_disk: criteria.allow_bd_disk,
            allow_upgrades: criteria.allow_upgrades,
            prefer_dual_audio: false,
            required_audio_languages: Vec::new(),
            scoring_persona: scryer_application::ScoringPersona::Balanced,
            scoring_overrides: criteria.scoring_overrides.into_application(),
            cutoff_tier: criteria.cutoff_tier,
            min_score_to_grab: criteria.min_score_to_grab,
            facet_persona_overrides: std::collections::HashMap::new(),
        },
    });

    if profile.id.is_empty() {
        return Err(Error::new("quality profile id is required"));
    }
    if profile.name.is_empty() {
        return Err(Error::new("quality profile name is required"));
    }
    if profile.criteria.quality_tiers.is_empty() {
        return Err(Error::new(
            "quality profile must include at least one quality tier",
        ));
    }

    Ok(profile)
}

#[Object]
impl SettingsMutations {
    async fn update_subtitle_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateSubtitleSettingsInput,
    ) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_subtitle_settings(
                &actor,
                AppUpdateSubtitleSettings {
                    enabled: input.enabled,
                    languages: input
                        .languages
                        .into_iter()
                        .map(|language| {
                            scryer_application::subtitles::wanted::SubtitleLanguagePref {
                                code: language.code,
                                hearing_impaired: language.hearing_impaired.unwrap_or(false),
                                forced: language.forced.unwrap_or(false),
                            }
                        })
                        .collect(),
                    auto_download_on_import: input.auto_download_on_import,
                    minimum_score_series: input.minimum_score_series,
                    minimum_score_movie: input.minimum_score_movie,
                    search_interval_hours: input.search_interval_hours,
                    include_ai_translated: input.include_ai_translated,
                    include_machine_translated: input.include_machine_translated,
                    sync_enabled: input.sync_enabled,
                    sync_threshold_series: input.sync_threshold_series,
                    sync_threshold_movie: input.sync_threshold_movie,
                    sync_max_offset_seconds: input.sync_max_offset_seconds,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_subtitle_settings(settings))
    }

    async fn update_acquisition_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateAcquisitionSettingsInput,
    ) -> GqlResult<AcquisitionSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_acquisition_settings(
                &actor,
                AppAcquisitionSettings {
                    enabled: input.enabled,
                    upgrade_cooldown_hours: input.upgrade_cooldown_hours,
                    same_tier_min_delta: input.same_tier_min_delta,
                    cross_tier_min_delta: input.cross_tier_min_delta,
                    forced_upgrade_delta_bypass: input.forced_upgrade_delta_bypass,
                    poll_interval_seconds: input.poll_interval_seconds,
                    sync_interval_seconds: input.sync_interval_seconds,
                    batch_size: input.batch_size,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_acquisition_settings(settings))
    }

    async fn update_general_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateGeneralSettingsInput,
    ) -> GqlResult<GeneralSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_general_settings(
                &actor,
                AppUpdateGeneralSettings {
                    keep_history_forever: input.keep_history_forever,
                    history_retention_days: input.history_retention_days,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_general_settings(settings))
    }

    async fn update_security_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateSecuritySettingsInput,
    ) -> GqlResult<SecuritySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);

        let settings = app
            .update_security_settings(
                &actor,
                AppUpdateSecuritySettings {
                    form_login_enabled: input.form_login_enabled,
                    skip_login_for_local_ips: input.skip_login_for_local_ips,
                },
            )
            .await
            .map_err(to_gql_error)?;
        let snapshot = auth_runtime.apply_saved_security_settings(
            settings.form_login_enabled,
            settings.skip_login_for_local_ips,
        );

        Ok(from_security_settings(settings, &snapshot))
    }

    async fn upsert_delay_profile(
        &self,
        ctx: &Context<'_>,
        input: DelayProfileInput,
    ) -> GqlResult<DelayProfilePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let profile = app
            .upsert_delay_profile(
                &actor,
                scryer_application::DelayProfile {
                    id: input.id,
                    name: input.name,
                    usenet_delay_minutes: input.usenet_delay_minutes as i64,
                    torrent_delay_minutes: input.torrent_delay_minutes as i64,
                    preferred_protocol: input.preferred_protocol.into_application(),
                    min_age_minutes: input.min_age_minutes as i64,
                    bypass_score_threshold: input.bypass_score_threshold,
                    applies_to_facets: input
                        .applies_to_facets
                        .into_iter()
                        .map(|facet| facet.into_domain().as_str().to_string())
                        .collect(),
                    tags: input.tags,
                    priority: input.priority,
                    enabled: input.enabled,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_delay_profile(profile))
    }

    async fn delete_delay_profile(
        &self,
        ctx: &Context<'_>,
        input: DeleteDelayProfileInput,
    ) -> GqlResult<DelayProfileDeletionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let id = app
            .delete_delay_profile(&actor, &input.id)
            .await
            .map_err(to_gql_error)?;
        Ok(DelayProfileDeletionPayload { id })
    }

    async fn update_media_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateMediaSettingsInput,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let scope = input.scope;
        app.update_media_settings(
            &actor,
            scope.into_media_facet(),
            scryer_application::UpdateMediaSettings {
                library_path: input.library_path,
                root_folders: input.root_folders.map(|entries| {
                    entries
                        .into_iter()
                        .map(|entry| scryer_domain::RootFolderEntry {
                            path: entry.path,
                            is_default: entry.is_default,
                        })
                        .collect()
                }),
                required_audio_languages: input.required_audio_languages,
                rename_template: input.rename_template,
                rename_collision_policy: input.rename_collision_policy,
                rename_missing_metadata_policy: input.rename_missing_metadata_policy,
                filler_policy: input.filler_policy,
                recap_policy: input.recap_policy,
                monitor_specials: input.monitor_specials,
                inter_season_movies: input.inter_season_movies,
                monitor_filler_movies: input.monitor_filler_movies,
                nfo_write_on_import: input.nfo_write_on_import,
                plexmatch_write_on_import: input.plexmatch_write_on_import,
            },
        )
        .await
        .map(|settings| from_media_settings(scope, settings))
        .map_err(to_gql_error)
    }

    async fn update_library_paths(
        &self,
        ctx: &Context<'_>,
        input: UpdateLibraryPathsInput,
    ) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.update_library_paths(
            &actor,
            scryer_application::UpdateLibraryPaths {
                movie_path: input.movie_path,
                series_path: input.series_path,
                anime_path: input.anime_path,
            },
        )
        .await
        .map(from_library_paths_settings)
        .map_err(to_gql_error)
    }

    async fn update_service_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateServiceSettingsInput,
    ) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.update_service_settings(
            &actor,
            scryer_application::UpdateServiceSettings {
                tls_cert_path: input.tls_cert_path,
                tls_key_path: input.tls_key_path,
            },
        )
        .await
        .map(from_service_settings)
        .map_err(to_gql_error)
    }

    async fn save_quality_profile_settings(
        &self,
        ctx: &Context<'_>,
        input: SaveQualityProfileSettingsInput,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let current = app
            .get_quality_profile_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        let existing_by_id = current
            .profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile))
            .collect::<std::collections::HashMap<_, _>>();

        let profiles = input
            .profiles
            .into_iter()
            .map(|profile| {
                let existing = existing_by_id.get(profile.id.as_str()).copied();
                quality_profile_from_input(profile, existing)
            })
            .collect::<GqlResult<Vec<_>>>()?;
        app.save_quality_profile_settings(
            &actor,
            scryer_application::SaveQualityProfileSettings {
                profiles,
                replace_existing: input.replace_existing,
                global_profile_id: input.global_profile_id,
                category_selections: input
                    .category_selections
                    .into_iter()
                    .map(
                        |selection| scryer_application::UpdateQualityProfileSelection {
                            facet: selection.scope.into_media_facet(),
                            inherit_global: selection.inherit_global,
                            profile_id: selection.profile_id,
                        },
                    )
                    .collect(),
                global_scoring_persona: input
                    .global_scoring_persona
                    .map(ScoringPersonaValue::into_application),
                category_persona_selections: input
                    .category_persona_selections
                    .into_iter()
                    .map(
                        |selection| scryer_application::UpdateFacetScoringPersonaSelection {
                            facet: selection.scope.into_media_facet(),
                            inherit_global: selection.inherit_global,
                            persona: selection.persona.map(ScoringPersonaValue::into_application),
                        },
                    )
                    .collect(),
            },
        )
        .await
        .map(from_quality_profile_settings)
        .map_err(to_gql_error)
    }

    async fn update_download_client_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateDownloadClientRoutingInput,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let scope = input.scope;
        app.update_download_client_routing(
            &actor,
            scope.as_scope_id(),
            input
                .entries
                .into_iter()
                .map(
                    |entry| scryer_application::DownloadClientRoutingSettingsEntry {
                        client_id: entry.client_id,
                        enabled: entry.enabled,
                        category: entry.category,
                        recent_queue_priority: entry.recent_queue_priority,
                        older_queue_priority: entry.older_queue_priority,
                        remove_completed: entry.remove_completed,
                        remove_failed: entry.remove_failed,
                    },
                )
                .collect(),
        )
        .await
        .map(|entries| {
            entries
                .into_iter()
                .map(from_download_client_routing_entry)
                .collect()
        })
        .map_err(to_gql_error)
    }

    async fn update_indexer_routing(
        &self,
        ctx: &Context<'_>,
        input: UpdateIndexerRoutingInput,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let scope = input.scope;
        app.update_indexer_routing(
            &actor,
            scope.as_scope_id(),
            input
                .entries
                .into_iter()
                .map(|entry| scryer_application::IndexerRoutingSettingsEntry {
                    indexer_id: entry.indexer_id,
                    enabled: entry.enabled,
                    categories: entry.categories,
                    priority: entry.priority,
                })
                .collect(),
        )
        .await
        .map(|entries| {
            entries
                .into_iter()
                .map(from_indexer_routing_entry)
                .collect()
        })
        .map_err(to_gql_error)
    }

    async fn delete_quality_profile(
        &self,
        ctx: &Context<'_>,
        input: DeleteQualityProfileInput,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.delete_quality_profile(&actor, &input.profile_id)
            .await
            .map(from_quality_profile_settings)
            .map_err(to_gql_error)
    }

    async fn queue_tvdb_movies_scan(
        &self,
        ctx: &Context<'_>,
        input: QueueTvdbMoviesScanInput,
    ) -> GqlResult<TvdbScanOperationPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let source = input.source.trim().to_string();
        let operation = app
            .queue_tvdb_movies_scan(&actor, input.limit, &source)
            .await
            .map_err(to_gql_error)?;

        Ok(from_tvdb_scan_operation(operation, input.limit, source))
    }

    async fn login(&self, ctx: &Context<'_>, input: LoginInput) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .authenticate_credentials(&input.username, &input.password)
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

    /// Issue a JWT for the default admin user without credentials.
    /// Retained for compatibility when authentication is disabled.
    async fn dev_auto_login(&self, ctx: &Context<'_>) -> GqlResult<LoginPayload> {
        let auth_runtime = auth_runtime_from_ctx(ctx);
        if auth_runtime.snapshot().effective_form_login_enabled {
            return Err(Error::new("authentication is enabled"));
        }
        let app = app_from_ctx(ctx)?;
        let user = app
            .find_or_create_default_user()
            .await
            .map_err(to_gql_error)?;
        let token = app.issue_access_token(&user).map_err(to_gql_error)?;
        let expires_at =
            (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
        Ok(LoginPayload {
            token,
            user: from_user(user),
            expires_at,
        })
    }

    /// Mark the setup wizard as complete.
    async fn complete_setup(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.complete_setup(&actor).await.map_err(to_gql_error)
    }
}
