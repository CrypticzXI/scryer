use async_graphql::{Context, Error, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::{
    AcquisitionSettings as AppAcquisitionSettings, QualityProfile, QualityProfileCriteria,
    SecuritySettings as AppSecuritySettings,
    UpdateAutoBackupSettings as AppUpdateAutoBackupSettings,
    UpdateGeneralSettings as AppUpdateGeneralSettings,
    UpdateSecuritySettings as AppUpdateSecuritySettings,
    UpdateSubtitleSettings as AppUpdateSubtitleSettings,
};

use scryer_interface_core::{actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, to_gql_error};
use scryer_interface_media::mappers::{
    from_download_client_routing_entry, from_indexer_routing_entry, from_library_paths_settings,
    from_media_settings, from_quality_profile_settings, from_service_settings, from_user,
};
use scryer_interface_media::types::*;

#[derive(Default)]
pub struct SettingsMutations;

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
        plugin_http_ca_bundle_pem: settings.plugin_http_ca_bundle_pem,
        plugin_http_trusted_certificates: settings
            .plugin_http_trusted_certificates
            .into_iter()
            .map(|certificate| PluginHttpTrustedCertificatePayload {
                fingerprint_sha256: certificate.fingerprint_sha256,
                pem: certificate.pem,
            })
            .collect(),
    }
}

fn from_auto_backup_settings(
    settings: scryer_application::AutoBackupSettings,
) -> AutoBackupSettingsPayload {
    AutoBackupSettingsPayload {
        enabled: settings.enabled,
        daily_time_local: settings.daily_time_local,
        auto_backup_key_present: settings.auto_backup_key_present,
        next_run_at: settings.next_run_at,
    }
}

fn from_security_settings(
    settings: AppSecuritySettings,
    auth_runtime: &scryer_interface_core::AuthRuntimeStateSnapshot,
) -> SecuritySettingsPayload {
    SecuritySettingsPayload {
        form_login_enabled: settings.form_login_enabled,
        skip_login_for_local_ips: settings.skip_login_for_local_ips,
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        env_override_active: auth_runtime.env_override_active,
        env_override_description: auth_runtime.env_override_description.clone(),
    }
}

fn auth_provider_connections(
    connections: Vec<scryer_application::AuthProviderConnection>,
) -> Vec<AuthProviderConnectionPayload> {
    connections
        .into_iter()
        .map(|connection| AuthProviderConnectionPayload {
            user_visible_url: connection
                .base_url
                .clone()
                .or_else(|| connection.machine_id.clone()),
            id: connection.id,
            display_name: connection.display_name,
            base_url: connection.base_url,
            machine_id: connection.machine_id,
        })
        .collect()
}

fn from_auth_provider_settings(
    settings: scryer_application::AuthProviderSettings,
) -> AuthProviderSettingsPayload {
    let allowed_jellyfin_connection_ids = settings.allowed_jellyfin_connection_ids;
    let allowed_plex_connection_ids = settings.allowed_plex_connection_ids;

    AuthProviderSettingsPayload {
        allowed_providers: settings
            .allowed_providers
            .into_iter()
            .map(ExternalAccountProviderValue::from_domain)
            .collect(),
        provider_login_enabled: settings
            .provider_login_enabled
            .into_iter()
            .map(ExternalAccountProviderValue::from_domain)
            .collect(),
        provider_linking_enabled: settings
            .provider_linking_enabled
            .into_iter()
            .map(ExternalAccountProviderValue::from_domain)
            .collect(),
        allowed_jellyfin_connections: auth_provider_connections(
            settings.allowed_jellyfin_connections,
        ),
        allowed_plex_connections: auth_provider_connections(settings.allowed_plex_connections),
        allowed_jellyfin_connection_ids,
        allowed_plex_connection_ids,
    }
}

fn app_auth_provider_connections(
    connections: Option<Vec<AuthProviderConnectionInput>>,
) -> Vec<scryer_application::AuthProviderConnection> {
    connections
        .unwrap_or_default()
        .into_iter()
        .map(|connection| scryer_application::AuthProviderConnection {
            id: connection.id,
            display_name: connection.display_name.unwrap_or_default(),
            base_url: connection.base_url,
            machine_id: connection.machine_id,
        })
        .collect()
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

fn from_webauthn_challenge_start(
    challenge: scryer_application::WebauthnChallengeStart,
) -> WebauthnChallengePayload {
    WebauthnChallengePayload {
        challenge_id: challenge.challenge_id,
        options_json: challenge.options_json,
    }
}

fn from_passkey_summary(summary: scryer_application::PasskeySummary) -> PasskeySummaryPayload {
    PasskeySummaryPayload {
        id: summary.id,
        friendly_name: summary.friendly_name,
        created_at: summary.created_at,
        last_used_at: summary.last_used_at,
    }
}

async fn login_payload_from_user(
    app: &scryer_application::AppUseCase,
    user: scryer_domain::User,
) -> Result<LoginPayload, Error> {
    let user = app
        .attach_user_authorization(user)
        .await
        .map_err(to_gql_error)?;
    let token = app.issue_access_token(&user).await.map_err(to_gql_error)?;
    let expires_at = (Utc::now() + chrono::Duration::seconds(app.token_lifetime())).to_rfc3339();
    Ok(LoginPayload {
        token,
        user: from_user(user),
        expires_at,
    })
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

    let normalize_video_codec_list = |values: Vec<scryer_application::VideoCodec>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|codec| seen.insert(codec.to_string()))
            .collect::<Vec<_>>()
    };
    let normalize_source_list = |values: Vec<scryer_application::ReleaseSource>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|source| seen.insert(source.to_string()))
            .collect::<Vec<_>>()
    };
    let normalize_audio_codec_list = |values: Vec<scryer_application::AudioCodec>| {
        let mut seen = std::collections::HashSet::new();
        values
            .into_iter()
            .filter(|codec| seen.insert(codec.to_string()))
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
            source_allowlist: normalize_source_list(criteria.source_allowlist),
            source_blocklist: normalize_source_list(criteria.source_blocklist),
            video_codec_allowlist: normalize_video_codec_list(criteria.video_codec_allowlist),
            video_codec_blocklist: normalize_video_codec_list(criteria.video_codec_blocklist),
            audio_codec_allowlist: normalize_audio_codec_list(criteria.audio_codec_allowlist),
            audio_codec_blocklist: normalize_audio_codec_list(criteria.audio_codec_blocklist),
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
    let source_allowlist =
        parse_source_values(criteria.source_allowlist, "criteria.source_allowlist")?;
    let source_blocklist =
        parse_source_values(criteria.source_blocklist, "criteria.source_blocklist")?;
    let video_codec_allowlist = parse_video_codec_values(
        criteria.video_codec_allowlist,
        "criteria.video_codec_allowlist",
    )?;
    let video_codec_blocklist = parse_video_codec_values(
        criteria.video_codec_blocklist,
        "criteria.video_codec_blocklist",
    )?;
    let audio_codec_allowlist = parse_audio_codec_values(
        criteria.audio_codec_allowlist,
        "criteria.audio_codec_allowlist",
    )?;
    let audio_codec_blocklist = parse_audio_codec_values(
        criteria.audio_codec_blocklist,
        "criteria.audio_codec_blocklist",
    )?;

    let profile = normalize_quality_profile(QualityProfile {
        id: input.id,
        name: input.name,
        criteria: QualityProfileCriteria {
            quality_tiers: criteria.quality_tiers,
            archival_quality: criteria.archival_quality,
            allow_unknown_quality: criteria.allow_unknown_quality,
            source_allowlist,
            source_blocklist,
            video_codec_allowlist,
            video_codec_blocklist,
            audio_codec_allowlist,
            audio_codec_blocklist,
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

fn parse_video_codec_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::VideoCodec>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::VideoCodec::parse(trimmed.as_str()).ok_or_else(|| {
                async_graphql::Error::new(format!("invalid value {trimmed:?} for {field}"))
            })
        })
        .collect()
}

fn parse_source_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::ReleaseSource>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::ReleaseSource::parse(trimmed.as_str()).ok_or_else(|| {
                async_graphql::Error::new(format!("invalid value {trimmed:?} for {field}"))
            })
        })
        .collect()
}

fn parse_audio_codec_values(
    values: Vec<String>,
    field: &str,
) -> GqlResult<Vec<scryer_application::AudioCodec>> {
    values
        .into_iter()
        .map(|value| {
            let trimmed = value.trim().to_string();
            scryer_application::AudioCodec::parse(trimmed.as_str()).ok_or_else(|| {
                async_graphql::Error::new(format!("invalid value {trimmed:?} for {field}"))
            })
        })
        .collect()
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
                    plugin_http_ca_bundle_pem: input.plugin_http_ca_bundle_pem,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_general_settings(settings))
    }

    async fn update_auto_backup_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateAutoBackupSettingsInput,
    ) -> GqlResult<AutoBackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let settings = app
            .update_auto_backup_settings(
                &actor,
                AppUpdateAutoBackupSettings {
                    enabled: input.enabled,
                    daily_time_local: input.daily_time_local,
                    set_auto_backup_key: input.set_auto_backup_key,
                    clear_auto_backup_key: input.clear_auto_backup_key,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(from_auto_backup_settings(settings))
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

    async fn update_auth_provider_settings(
        &self,
        ctx: &Context<'_>,
        input: UpdateAuthProviderSettingsInput,
    ) -> GqlResult<AuthProviderSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let allowed_jellyfin_connections =
            app_auth_provider_connections(input.allowed_jellyfin_connections);
        let allowed_plex_connections =
            app_auth_provider_connections(input.allowed_plex_connections);
        app.update_auth_provider_settings(
            &actor,
            scryer_application::UpdateAuthProviderSettings {
                allowed_providers: input
                    .allowed_providers
                    .into_iter()
                    .map(ExternalAccountProviderValue::into_domain)
                    .collect(),
                provider_login_enabled: input
                    .provider_login_enabled
                    .into_iter()
                    .map(ExternalAccountProviderValue::into_domain)
                    .collect(),
                provider_linking_enabled: input
                    .provider_linking_enabled
                    .into_iter()
                    .map(ExternalAccountProviderValue::into_domain)
                    .collect(),
                allowed_jellyfin_connection_ids: input.allowed_jellyfin_connection_ids,
                allowed_plex_connection_ids: input.allowed_plex_connection_ids,
                allowed_jellyfin_connections,
                allowed_plex_connections,
            },
        )
        .await
        .map(from_auth_provider_settings)
        .map_err(to_gql_error)
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
                folder_template: input.folder_template,
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
        app.delete_quality_profile(&actor, &input.profile_id)
            .await
            .map(from_quality_profile_settings)
            .map_err(to_gql_error)
    }

    async fn webauthn_register_start(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_start(&actor, auth_runtime.snapshot().effective_form_login_enabled)
            .await
            .map(from_webauthn_challenge_start)
            .map_err(to_gql_error)
    }

    async fn webauthn_register_complete(
        &self,
        ctx: &Context<'_>,
        input: WebauthnRegisterCompleteInput,
    ) -> GqlResult<PasskeySummaryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_register_complete(
            &actor,
            &input.challenge_id,
            &input.response_json,
            input.friendly_name,
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map(from_passkey_summary)
        .map_err(to_gql_error)
    }

    async fn webauthn_authenticate_start(
        &self,
        ctx: &Context<'_>,
        username: Option<String>,
    ) -> GqlResult<WebauthnChallengePayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.webauthn_authenticate_start(
            username.as_deref(),
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map(from_webauthn_challenge_start)
        .map_err(to_gql_error)
    }

    async fn webauthn_authenticate_complete(
        &self,
        ctx: &Context<'_>,
        input: WebauthnCompleteInput,
    ) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let user = app
            .webauthn_authenticate_complete(
                &input.challenge_id,
                &input.response_json,
                auth_runtime.snapshot().effective_form_login_enabled,
            )
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(&app, user).await
    }

    async fn delete_my_passkey(&self, ctx: &Context<'_>, id: String) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.delete_my_passkey(
            &actor,
            &id,
            auth_runtime.snapshot().effective_form_login_enabled,
        )
        .await
        .map(|_| true)
        .map_err(to_gql_error)
    }

    async fn login(&self, ctx: &Context<'_>, input: LoginInput) -> GqlResult<LoginPayload> {
        let app = app_from_ctx(ctx)?;
        let user = app
            .authenticate_credentials(&input.username, &input.password)
            .await
            .map_err(to_gql_error)?;
        login_payload_from_user(&app, user).await
    }

    /// Mark the setup wizard as complete.
    async fn complete_setup(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.complete_setup(&actor).await.map_err(to_gql_error)
    }
}
