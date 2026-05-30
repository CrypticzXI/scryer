mod mutation;

use async_graphql::{Context, Object, Result as GqlResult};
use scryer_interface_core::{
    AuthRuntimeStateSnapshot, actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, to_gql_error,
};
use scryer_interface_media::mappers::{
    from_download_client_config, from_download_client_routing_entry,
    from_indexer_config_with_fields, from_indexer_routing_entry, from_library_paths_settings,
    from_media_settings, from_quality_profile_settings, from_service_settings,
    from_subtitle_provider_config, from_user,
};
use scryer_interface_media::types::*;

pub use mutation::SettingsMutations;

#[derive(Default)]
pub struct SettingsQueries;

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

fn from_recycle_bin_settings(
    settings: scryer_application::RecycleBinSettings,
) -> RecycleBinSettingsPayload {
    RecycleBinSettingsPayload {
        enabled: settings.enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_auth_provider_settings_redacts_connection_internals() {
        let payload =
            from_auth_provider_runtime_settings(scryer_application::AuthProviderSettings {
                allowed_providers: vec![scryer_domain::ExternalAccountProvider::Jellyfin],
                provider_login_enabled: vec![scryer_domain::ExternalAccountProvider::Jellyfin],
                provider_linking_enabled: vec![scryer_domain::ExternalAccountProvider::Jellyfin],
                allowed_jellyfin_connection_ids: vec!["jellyfin-main".to_string()],
                allowed_plex_connection_ids: vec![],
                allowed_jellyfin_connections: vec![scryer_application::AuthProviderConnection {
                    id: "jellyfin-main".to_string(),
                    display_name: "Main Jellyfin".to_string(),
                    base_url: Some("https://jellyfin.example.test".to_string()),
                    machine_id: None,
                }],
                allowed_plex_connections: vec![scryer_application::AuthProviderConnection {
                    id: "plex-main".to_string(),
                    display_name: "Main Plex".to_string(),
                    base_url: None,
                    machine_id: Some("machine-1".to_string()),
                }],
            });

        assert_eq!(payload.allowed_jellyfin_connections[0].id, "jellyfin-main");
        assert_eq!(
            payload.allowed_jellyfin_connections[0].display_name,
            "Main Jellyfin"
        );
        assert!(
            payload.allowed_jellyfin_connections[0]
                .user_visible_url
                .is_none()
        );
        assert!(payload.allowed_jellyfin_connections[0].base_url.is_none());
        assert!(payload.allowed_plex_connections[0].machine_id.is_none());
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
    settings: scryer_application::SecuritySettings,
    auth_runtime: &AuthRuntimeStateSnapshot,
) -> SecuritySettingsPayload {
    SecuritySettingsPayload {
        form_login_enabled: settings.form_login_enabled,
        skip_login_for_local_ips: settings.skip_login_for_local_ips,
        totp_require_config_step_up: settings.totp_require_config_step_up,
        totp_require_jellyfin_login: settings.totp_require_jellyfin_login,
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

fn runtime_auth_provider_connections(
    connections: Vec<scryer_application::AuthProviderConnection>,
) -> Vec<AuthProviderConnectionPayload> {
    connections
        .into_iter()
        .map(|connection| AuthProviderConnectionPayload {
            id: connection.id,
            display_name: connection.display_name,
            user_visible_url: None,
            base_url: None,
            machine_id: None,
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

fn from_auth_provider_runtime_settings(
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
        allowed_jellyfin_connections: runtime_auth_provider_connections(
            settings.allowed_jellyfin_connections,
        ),
        allowed_plex_connections: runtime_auth_provider_connections(
            settings.allowed_plex_connections,
        ),
        allowed_jellyfin_connection_ids,
        allowed_plex_connection_ids,
    }
}

fn from_auth_runtime_state(auth_runtime: &AuthRuntimeStateSnapshot) -> AuthRuntimeStatePayload {
    AuthRuntimeStatePayload {
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        skip_login_for_local_ips: auth_runtime.skip_login_for_local_ips,
        passkey_enabled: auth_runtime.passkey_enabled,
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

fn from_totp_status(status: scryer_application::TotpStatus) -> TotpStatusPayload {
    TotpStatusPayload {
        enabled: status.enabled,
        created_at: status.created_at,
        last_used_at: status.last_used_at,
        recovery_codes_remaining: status.recovery_codes_remaining,
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

#[allow(clippy::too_many_arguments)]
#[Object]
impl SettingsQueries {
    async fn subtitle_settings(&self, ctx: &Context<'_>) -> GqlResult<SubtitleSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_subtitle_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_subtitle_settings(settings))
    }

    async fn acquisition_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AcquisitionSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_acquisition_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_acquisition_settings(settings))
    }

    async fn general_settings(&self, ctx: &Context<'_>) -> GqlResult<GeneralSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_general_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_general_settings(settings))
    }

    async fn recycle_bin_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<RecycleBinSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_recycle_bin_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_recycle_bin_settings(settings))
    }

    async fn auto_backup_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AutoBackupSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_auto_backup_settings(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_auto_backup_settings(settings))
    }

    async fn security_settings(&self, ctx: &Context<'_>) -> GqlResult<SecuritySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        let settings = app
            .get_security_settings(&actor)
            .await
            .map_err(to_gql_error)?;

        Ok(from_security_settings(settings, &auth_runtime.snapshot()))
    }

    async fn auth_provider_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AuthProviderSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_auth_provider_settings(&actor)
            .await
            .map(from_auth_provider_settings)
            .map_err(to_gql_error)
    }

    async fn auth_provider_runtime_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<AuthProviderSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        app.get_auth_provider_runtime_settings()
            .await
            .map(from_auth_provider_runtime_settings)
            .map_err(to_gql_error)
    }

    async fn auth_runtime_state(&self, ctx: &Context<'_>) -> GqlResult<AuthRuntimeStatePayload> {
        let auth_runtime = auth_runtime_from_ctx(ctx);
        Ok(from_auth_runtime_state(&auth_runtime.snapshot()))
    }

    async fn my_passkeys(&self, ctx: &Context<'_>) -> GqlResult<Vec<PasskeySummaryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let auth_runtime = auth_runtime_from_ctx(ctx);
        app.list_my_passkeys(&actor, auth_runtime.snapshot().effective_form_login_enabled)
            .await
            .map(|passkeys| passkeys.into_iter().map(from_passkey_summary).collect())
            .map_err(to_gql_error)
    }

    async fn my_totp(&self, ctx: &Context<'_>) -> GqlResult<TotpStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.totp_status(&actor)
            .await
            .map(from_totp_status)
            .map_err(to_gql_error)
    }

    async fn delay_profiles(&self, ctx: &Context<'_>) -> GqlResult<Vec<DelayProfilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let profiles = app.get_delay_profiles(&actor).await.map_err(to_gql_error)?;
        Ok(profiles.into_iter().map(from_delay_profile).collect())
    }

    async fn media_settings(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<MediaSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_media_settings(&actor, scope.into_media_facet())
            .await
            .map(|settings| from_media_settings(scope, settings))
            .map_err(to_gql_error)
    }

    async fn library_paths(&self, ctx: &Context<'_>) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_library_paths(&actor)
            .await
            .map(from_library_paths_settings)
            .map_err(to_gql_error)
    }

    async fn service_settings(&self, ctx: &Context<'_>) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_service_settings(&actor)
            .await
            .map(from_service_settings)
            .map_err(to_gql_error)
    }

    async fn quality_profile_settings(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<QualityProfileSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_quality_profile_settings(&actor)
            .await
            .map(from_quality_profile_settings)
            .map_err(to_gql_error)
    }

    async fn download_client_routing(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<DownloadClientRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_download_client_routing(&actor, scope.as_scope_id())
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(from_download_client_routing_entry)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    async fn indexer_routing(
        &self,
        ctx: &Context<'_>,
        scope: ContentScopeValue,
    ) -> GqlResult<Vec<IndexerRoutingEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.get_indexer_routing(&actor, scope.as_scope_id())
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .map(from_indexer_routing_entry)
                    .collect()
            })
            .map_err(to_gql_error)
    }

    async fn indexers(
        &self,
        ctx: &Context<'_>,
        provider_type: Option<String>,
    ) -> GqlResult<Vec<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_indexer_configs(&actor, provider_type)
            .await
            .map_err(to_gql_error)?;
        let stats = app
            .indexer_query_stats(&actor)
            .await
            .map_err(to_gql_error)?;
        let mut payloads = Vec::with_capacity(configs.len());
        for config in configs {
            let config_fields = app
                .indexer_config_fields_for_provider_type(&config.provider_type)
                .unwrap_or_default();
            payloads.push(from_indexer_config_with_fields(config, &config_fields));
        }
        for payload in &mut payloads {
            if let Some(s) = stats.iter().find(|s| s.indexer_id == payload.id) {
                payload.last_query_at = s.last_query_at.clone();
            }
        }
        Ok(payloads)
    }

    async fn root_folders(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
    ) -> GqlResult<Vec<RootFolderPayload>> {
        let app = app_from_ctx(ctx)?;
        let media_facet = facet.into_domain();
        let entries = app
            .root_folders_for_facet(&media_facet)
            .await
            .map_err(to_gql_error)?;
        Ok(entries
            .into_iter()
            .map(|e| RootFolderPayload {
                path: e.path,
                is_default: e.is_default,
            })
            .collect())
    }

    async fn download_client_configs(
        &self,
        ctx: &Context<'_>,
        client_type: Option<String>,
    ) -> GqlResult<Vec<DownloadClientConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_download_client_configs(&actor, client_type)
            .await
            .map_err(to_gql_error)?;
        Ok(configs
            .into_iter()
            .map(from_download_client_config)
            .collect())
    }

    async fn subtitle_provider_configs(
        &self,
        ctx: &Context<'_>,
        provider_type: Option<String>,
    ) -> GqlResult<Vec<SubtitleProviderConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let configs = app
            .list_subtitle_provider_configs(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(configs
            .into_iter()
            .filter(|config| {
                provider_type.as_ref().is_none_or(|provider_type| {
                    config.provider_type.eq_ignore_ascii_case(provider_type)
                })
            })
            .map(|config| {
                let config_fields = app.subtitle_provider_config_fields(&config.provider_type);
                from_subtitle_provider_config(config, &config_fields)
            })
            .collect())
    }

    async fn users(&self, ctx: &Context<'_>) -> GqlResult<Vec<UserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let users = app.list_users(&actor).await.map_err(to_gql_error)?;
        let mut payloads = Vec::with_capacity(users.len());
        for user in users {
            let user = app
                .attach_user_authorization(user)
                .await
                .map_err(to_gql_error)?;
            payloads.push(from_user(user));
        }
        Ok(payloads)
    }
}
