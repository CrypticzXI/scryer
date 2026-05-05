use async_graphql::{ComplexObject, Context, Error, Object, Result as GqlResult};

use chrono::Utc;
use scryer_application::{
    DownloadImportFilter, PendingImportCounts, ReleaseDecisionsQuery, SCRYER_VERSION,
    TitleHistoryFilter, WantedItemsQuery, is_supported_title_history_event_type,
    supported_title_history_event_types,
};
use scryer_domain::{Entitlement, PolicyInput, TitleHistoryEventType};

use crate::context::{
    actor_from_ctx, app_from_ctx, auth_runtime_from_ctx, current_user_from_ctx, to_gql_error,
};
use crate::mappers::{
    from_activity_event, from_backup_info, from_calendar_episode, from_collection,
    from_delete_preview, from_disk_space, from_domain_event, from_download_client_config,
    from_download_client_routing_entry, from_download_queue_item, from_episode,
    from_health_check_result, from_indexer_config, from_indexer_routing_entry, from_job_definition,
    from_job_run, from_library_paths_settings, from_library_scan_session, from_media_rename_plan,
    from_media_settings, from_pending_import_connection, from_pending_import_counts,
    from_pending_release, from_provider_type, from_quality_profile_settings, from_release_decision,
    from_service_settings, from_smg_version_compatibility_notice, from_submission_scope,
    from_subtitle_provider_config, from_system_health, from_title,
    from_title_acquisition_diagnostics, from_title_history_page, from_title_history_record,
    from_title_media_file, from_title_release_blocklist_entry, from_user, from_wanted_item,
};
use crate::types::*;

fn title_scope_from_facet(facet: MediaFacetValue) -> ContentScopeValue {
    match facet {
        MediaFacetValue::Movie => ContentScopeValue::Movie,
        MediaFacetValue::Series => ContentScopeValue::Series,
        MediaFacetValue::Anime => ContentScopeValue::Anime,
    }
}

fn supported_title_history_values_message() -> String {
    supported_title_history_event_types()
        .iter()
        .map(TitleHistoryEventType::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_supported_title_history_event_types(
    event_types: Option<Vec<String>>,
) -> GqlResult<Option<Vec<TitleHistoryEventType>>> {
    let Some(event_types) = event_types else {
        return Ok(None);
    };

    let supported_values = supported_title_history_values_message();
    let mut parsed = Vec::with_capacity(event_types.len());
    for raw in event_types {
        let Some(event_type) = TitleHistoryEventType::parse(&raw) else {
            return Err(Error::new(format!(
                "invalid title history event type `{raw}`. Supported values: {supported_values}"
            )));
        };
        if !is_supported_title_history_event_type(event_type) {
            return Err(Error::new(format!(
                "unsupported title history event type `{raw}`. Supported values: {supported_values}"
            )));
        }
        parsed.push(event_type);
    }

    Ok(Some(parsed))
}

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
    settings: scryer_application::SecuritySettings,
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

fn from_auth_runtime_state(
    auth_runtime: &crate::context::AuthRuntimeStateSnapshot,
) -> AuthRuntimeStatePayload {
    AuthRuntimeStatePayload {
        effective_form_login_enabled: auth_runtime.effective_form_login_enabled,
        skip_login_for_local_ips: auth_runtime.skip_login_for_local_ips,
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

fn from_download_history_page(
    page: scryer_application::DownloadHistoryPage,
) -> DownloadHistoryPagePayload {
    DownloadHistoryPagePayload {
        items: page
            .items
            .into_iter()
            .map(from_download_queue_item)
            .collect(),
        has_more: page.has_more,
        total_count: page.total_count as i32,
        available_clients: page
            .available_clients
            .into_iter()
            .map(|client| DownloadClientFilterOptionPayload {
                client_id: client.client_id,
                client_name: client.client_name,
                client_type: client.client_type,
            })
            .collect(),
    }
}

fn from_download_import_page(
    page: scryer_application::DownloadImportPage,
) -> DownloadImportPagePayload {
    DownloadImportPagePayload {
        items: page
            .items
            .into_iter()
            .map(from_download_queue_item)
            .collect(),
        has_more: page.has_more,
        total_count: page.total_count as i32,
    }
}

fn from_metadata_search_item(
    item: scryer_application::RichMetadataSearchItem,
) -> MetadataSearchItemPayload {
    MetadataSearchItemPayload {
        tvdb_id: item.tvdb_id,
        name: item.name,
        imdb_id: item.imdb_id,
        slug: item.slug,
        type_hint: item.type_hint,
        year: item.year,
        status: item.status,
        overview: item.overview,
        popularity: item.popularity,
        poster_url: item.poster_url,
        language: item.language,
        runtime_minutes: item.runtime_minutes,
        sort_title: item.sort_title,
    }
}

fn from_cutoff_unmet_item(item: scryer_application::CutoffUnmetItem) -> CutoffUnmetItemPayload {
    CutoffUnmetItemPayload {
        title_id: item.title_id,
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: MediaFacetValue::from_domain(item.title_facet),
        episode_id: item.episode_id,
        season_number: item.season_number,
        episode_number: item.episode_number,
        current_tier: item.current_tier,
        target_tier: item.target_tier,
    }
}

async fn title_payloads_from_titles(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    titles: Vec<scryer_domain::Title>,
) -> GqlResult<Vec<TitlePayload>> {
    let title_ids: Vec<String> = titles.iter().map(|t| t.id.clone()).collect();
    let summaries = app
        .list_primary_collection_summaries(actor, &title_ids)
        .await
        .map_err(to_gql_error)?;
    let media_size_summaries = app
        .list_title_media_size_summaries(actor, &title_ids)
        .await
        .map_err(to_gql_error)?;
    let quality_summaries = app
        .list_title_quality_summaries(actor, &title_ids)
        .await
        .map_err(to_gql_error)?;
    let episode_progress_summaries = app
        .list_title_episode_progress_summaries(actor, &title_ids)
        .await
        .map_err(to_gql_error)?;
    let summary_map: std::collections::HashMap<&str, _> =
        summaries.iter().map(|s| (s.title_id.as_str(), s)).collect();
    let media_size_map: std::collections::HashMap<&str, i64> = media_size_summaries
        .iter()
        .map(|summary| (summary.title_id.as_str(), summary.total_size_bytes))
        .collect();
    let quality_map: std::collections::HashMap<&str, &String> = quality_summaries
        .iter()
        .map(|summary| (summary.title_id.as_str(), &summary.quality_tier))
        .collect();
    let episode_progress_map: std::collections::HashMap<&str, _> = episode_progress_summaries
        .iter()
        .map(|summary| (summary.title_id.as_str(), summary))
        .collect();

    Ok(titles
        .into_iter()
        .map(|t| {
            let id = t.id.clone();
            let mut payload = from_title(t);
            if let Some(s) = summary_map.get(id.as_str()) {
                payload.quality_tier = s.label.clone();
            }
            if let Some(quality_tier) = quality_map.get(id.as_str()) {
                payload.current_quality_tier = Some((*quality_tier).clone());
            }
            payload.size_bytes = media_size_map.get(id.as_str()).copied();
            if let Some(summary) = episode_progress_map.get(id.as_str()) {
                payload.episodes_owned = Some(summary.owned_episodes);
                payload.episodes_monitored = Some(summary.monitored_episodes);
                payload.episodes_total = Some(summary.total_episodes);
            }
            payload
        })
        .collect())
}

#[derive(Copy, Clone)]
pub struct QueryRoot;

#[expect(
    clippy::too_many_arguments,
    reason = "async-graphql's Object macro generates resolver wrappers that exceed clippy's argument threshold"
)]
#[Object]
impl QueryRoot {
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        query: Option<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let parsed_facet = facet.map(MediaFacetValue::into_domain);
        let titles = app
            .list_titles(&actor, parsed_facet, query)
            .await
            .map_err(to_gql_error)?;

        title_payloads_from_titles(&app, &actor, titles).await
    }

    async fn titles_by_external_ids(
        &self,
        ctx: &Context<'_>,
        source: String,
        values: Vec<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let titles = app
            .list_titles_by_external_ids(&actor, &source, &values)
            .await
            .map_err(to_gql_error)?;

        title_payloads_from_titles(&app, &actor, titles).await
    }

    async fn title(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let Some(title) = app.get_title(&actor, &id).await.map_err(to_gql_error)? else {
            return Ok(None);
        };
        let mut payloads = title_payloads_from_titles(&app, &actor, vec![title]).await?;
        Ok(payloads.pop())
    }

    async fn title_by_slug(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        slug: String,
    ) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let Some(title) = app
            .get_title_by_slug(&actor, facet.into_domain(), &slug)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(None);
        };
        let mut payloads = title_payloads_from_titles(&app, &actor, vec![title]).await?;
        Ok(payloads.pop())
    }

    async fn media_rename_preview(
        &self,
        ctx: &Context<'_>,
        input: MediaRenamePreviewInput,
    ) -> GqlResult<MediaRenamePlanPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let _ = input.dry_run;
        let facet = input.facet.into_domain();
        let plan = if let Some(title_id) = input.title_id {
            app.preview_rename_for_title(&actor, &title_id, facet)
                .await
                .map_err(to_gql_error)?
        } else {
            app.preview_rename_for_facet(&actor, facet)
                .await
                .map_err(to_gql_error)?
        };

        Ok(from_media_rename_plan(plan))
    }

    async fn delete_title_preview(
        &self,
        ctx: &Context<'_>,
        input: DeleteTitlePreviewInput,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_delete_title_files(&actor, &input.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn delete_media_file_preview(
        &self,
        ctx: &Context<'_>,
        input: DeleteMediaFilePreviewInput,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_delete_media_file(&actor, &input.file_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn delete_external_subtitle_preview(
        &self,
        ctx: &Context<'_>,
        input: DeleteExternalSubtitlePreviewInput,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_delete_external_subtitle_file(&actor, &input.external_subtitle_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn collection(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn episode(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }

    async fn wanted_item(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(item)
    }

    async fn policy_preview(
        &self,
        ctx: &Context<'_>,
        input: PolicyInputPayload,
    ) -> GqlResult<PolicyOutputPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let decision = app
            .evaluate_policy(
                &actor,
                PolicyInput {
                    title_id: input.title_id,
                    facet: input.facet.into_domain(),
                    has_existing_file: input.has_existing_file,
                    candidate_quality: input.candidate_quality,
                    requested_mode: scryer_domain::RequestedMode::parse(&input.requested_mode)
                        .ok_or_else(|| Error::new("invalid requestedMode for policyPreview"))?,
                    release_title: None,
                    quality_profile_id: None,
                    category: None,
                    tags: vec![],
                    is_anime: false,
                },
            )
            .await
            .map_err(to_gql_error)?;

        Ok(crate::mappers::from_policy(decision))
    }

    async fn search_releases(
        &self,
        ctx: &Context<'_>,
        input: SearchReleasesInput,
    ) -> GqlResult<Vec<IndexerSearchResultPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let SearchReleasesInput {
            title_id,
            collection_id,
            season,
            episode,
            limit,
        } = input;

        let safe_limit = limit.unwrap_or(50).clamp(1, 200) as usize;
        let results = match (collection_id, season, episode) {
            (Some(collection_id), None, None) => app
                .search_indexers_for_interstitial_movie(&actor, title_id, collection_id)
                .await
                .map_err(to_gql_error)?,
            (None, Some(season), Some(episode)) => app
                .search_indexers_for_episode(&actor, title_id, season, episode)
                .await
                .map_err(to_gql_error)?,
            (None, None, None) => app
                .search_indexers_for_title(&actor, title_id)
                .await
                .map_err(to_gql_error)?,
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err(Error::new(
                    "episode searches require both season and episode",
                ));
            }
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(Error::new(
                    "collection searches cannot include season or episode",
                ));
            }
        };

        Ok(results
            .into_iter()
            .take(safe_limit)
            .map(crate::mappers::from_search_result)
            .collect())
    }

    async fn title_events(
        &self,
        ctx: &Context<'_>,
        title_id: Option<String>,
        event_types: Option<Vec<String>>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<TitleHistoryEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let parsed_types = parse_supported_title_history_event_types(event_types)?;

        if let Some(ref tid) = title_id {
            let page = app
                .list_title_history_for_title(
                    &actor,
                    tid,
                    parsed_types.as_deref(),
                    limit.unwrap_or(100).max(1) as usize,
                    offset.unwrap_or(0).max(0) as usize,
                )
                .await
                .map_err(to_gql_error)?;
            Ok(page
                .records
                .into_iter()
                .map(from_title_history_record)
                .collect())
        } else {
            let filter = TitleHistoryFilter {
                event_types: parsed_types,
                title_ids: None,
                title_search: None,
                download_id: None,
                episode_id: None,
                group_by_event: false,
                limit: limit.unwrap_or(100).max(1) as usize,
                offset: offset.unwrap_or(0).max(0) as usize,
            };
            let page = app
                .list_title_history(&actor, &filter)
                .await
                .map_err(to_gql_error)?;
            Ok(page
                .records
                .into_iter()
                .map(from_title_history_record)
                .collect())
        }
    }

    async fn title_history(
        &self,
        ctx: &Context<'_>,
        filter: TitleHistoryFilterInput,
    ) -> GqlResult<TitleHistoryPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let parsed_types = parse_supported_title_history_event_types(filter.event_types)?;

        let f = TitleHistoryFilter {
            event_types: parsed_types,
            title_ids: filter.title_ids,
            title_search: filter.title_search,
            download_id: filter.download_id,
            episode_id: filter.episode_id,
            group_by_event: filter.group_by_event.unwrap_or(false),
            limit: filter.limit.unwrap_or(50).max(1) as usize,
            offset: filter.offset.unwrap_or(0).max(0) as usize,
        };

        let page = app
            .list_title_history(&actor, &f)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_history_page(page))
    }

    async fn episode_history(
        &self,
        ctx: &Context<'_>,
        episode_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleHistoryEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let records = app
            .list_title_history_for_episode(
                &actor,
                &episode_id,
                limit.unwrap_or(50).max(1) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(records.into_iter().map(from_title_history_record).collect())
    }

    async fn title_release_blocklist(
        &self,
        ctx: &Context<'_>,
        title_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleReleaseBlocklistEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_title_release_blocklist(&actor, &title_id, limit.unwrap_or(100).max(1) as usize)
            .await
            .map_err(to_gql_error)?;
        Ok(items
            .into_iter()
            .map(from_title_release_blocklist_entry)
            .collect())
    }

    async fn activity_events(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<Vec<ActivityEventPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let events = app
            .recent_activity(
                &actor,
                limit.unwrap_or(100) as i64,
                offset.unwrap_or(0) as i64,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_activity_event).collect())
    }

    async fn domain_events(
        &self,
        ctx: &Context<'_>,
        event_types: Option<Vec<DomainEventTypeValue>>,
        title_id: Option<String>,
        facet: Option<MediaFacetValue>,
        after_sequence: Option<i64>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<DomainEventEnvelopePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let filter = scryer_domain::DomainEventFilter {
            event_types: event_types.map(|types| {
                types
                    .into_iter()
                    .map(DomainEventTypeValue::into_domain)
                    .collect()
            }),
            title_id,
            facet: facet.map(MediaFacetValue::into_domain),
            after_sequence,
            before_sequence: None,
            limit: limit.unwrap_or(100).max(1) as usize,
        };
        let events = app
            .list_domain_events(&actor, &filter)
            .await
            .map_err(to_gql_error)?;
        Ok(events.into_iter().map(from_domain_event).collect())
    }

    async fn active_library_scans(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<LibraryScanProgressPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let sessions = app
            .active_library_scans(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(sessions
            .into_iter()
            .map(from_library_scan_session)
            .collect())
    }

    async fn pending_import_counts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<PendingImportCountsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let counts = app
            .pending_import_counts(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_pending_import_counts(counts))
    }

    async fn navigation_badge_counts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<NavigationBadgeCountsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let pending_import_counts = async {
            if actor.has_entitlement(&Entitlement::ManageTitle) {
                app.pending_import_counts(&actor).await
            } else {
                Ok(PendingImportCounts::default())
            }
        };
        let activity_import_count = async {
            if actor.has_entitlement(&Entitlement::ManageTitle) {
                app.count_download_import_items(&actor, DownloadImportFilter::All)
                    .await
            } else {
                Ok(0)
            }
        };
        let plugin_update_count = async {
            if actor.has_entitlement(&Entitlement::ManageConfig) {
                app.plugin_update_count(&actor).await
            } else {
                Ok(0)
            }
        };

        let (pending_import_counts, activity_import_count, plugin_update_count) = tokio::try_join!(
            pending_import_counts,
            activity_import_count,
            plugin_update_count,
        )
        .map_err(to_gql_error)?;

        Ok(NavigationBadgeCountsPayload {
            pending_import_counts: from_pending_import_counts(pending_import_counts),
            activity_import_count: activity_import_count as i32,
            plugin_update_count: plugin_update_count as i32,
        })
    }

    async fn pending_imports(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        status: PendingImportStatusValue,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
    ) -> GqlResult<PendingImportConnectionPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let connection = app
            .pending_imports(
                &actor,
                facet.into_domain(),
                status.into_application(),
                limit,
                offset,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_pending_import_connection(connection))
    }

    async fn pending_import_binding_preview(
        &self,
        ctx: &Context<'_>,
        pending_import_id: String,
    ) -> GqlResult<PendingImportBindingPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let preview = app
            .preview_title_bound_pending_import(&actor, &pending_import_id)
            .await
            .map_err(to_gql_error)?;
        Ok(PendingImportBindingPreviewPayload {
            title: from_title(preview.title),
            file: PendingImportBindingFilePreviewPayload {
                file_path: preview.file.file_path,
                file_name: preview.file.file_name,
                size_bytes: preview.file.size_bytes.to_string(),
                parsed_season: preview.file.parsed_season.map(|value| value as i32),
                parsed_episodes: preview
                    .file
                    .parsed_episodes
                    .into_iter()
                    .map(|value| value as i32)
                    .collect(),
                parsed_absolute_numbers: preview
                    .file
                    .parsed_absolute_numbers
                    .into_iter()
                    .map(|value| value as i32)
                    .collect(),
                suggested_episode_ids: preview.file.suggested_episode_ids,
            },
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(from_episode)
                .collect(),
        })
    }

    async fn jobs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobDefinitionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let jobs = app.list_jobs(&actor).await.map_err(to_gql_error)?;
        Ok(jobs.into_iter().map(from_job_definition).collect())
    }

    async fn active_job_runs(&self, ctx: &Context<'_>) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app.active_job_runs(&actor).await.map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    async fn job_runs(
        &self,
        ctx: &Context<'_>,
        job_key: JobKeyValue,
        limit: Option<i32>,
    ) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app
            .list_job_runs(
                &actor,
                job_key.into_application(),
                limit.unwrap_or(10).max(1) as usize,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    async fn recent_job_runs(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<JobRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let runs = app
            .list_recent_job_runs(&actor, limit.unwrap_or(50).max(1) as usize)
            .await
            .map_err(to_gql_error)?;
        Ok(runs.into_iter().map(from_job_run).collect())
    }

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        title_id: Option<String>,
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let can_view_title_progress = actor.has_entitlement(&Entitlement::ViewCatalog)
            || actor.has_entitlement(&Entitlement::ManageTitle);
        if title_id.is_some() {
            if !can_view_title_progress {
                return Err(Error::new("insufficient entitlements"));
            }
        } else if !actor.has_entitlement(&Entitlement::ManageTitle) {
            return Err(Error::new("insufficient entitlements"));
        }
        let items = match title_id {
            Some(title_id) => {
                app.list_download_queue_for_title(
                    &actor,
                    &title_id,
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
            }
            None => {
                app.list_download_queue(
                    &actor,
                    include_all_activity.unwrap_or(false),
                    include_history_only.unwrap_or(false),
                    include_import_activity.unwrap_or(false),
                    activity_filter
                        .unwrap_or(DownloadActivityFilterValue::All)
                        .into_application(),
                )
                .await
            }
        }
        .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_download_queue_item).collect())
    }

    async fn download_import(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
        filter: Option<DownloadImportFilterValue>,
    ) -> GqlResult<DownloadImportPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(50).clamp(1, 100) as usize;
        let offset = offset.unwrap_or(0).max(0) as usize;
        let page = app
            .list_download_import_page(
                &actor,
                limit,
                offset,
                filter
                    .unwrap_or(DownloadImportFilterValue::All)
                    .into_application(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_download_import_page(page))
    }

    async fn download_history(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
        offset: Option<i32>,
        filters: Option<Vec<DownloadHistoryFilterValue>>,
        client_ids: Option<Vec<String>>,
        scryer_submitted_only: Option<bool>,
        sort_key: Option<DownloadHistorySortKeyValue>,
        sort_direction: Option<SortDirectionValue>,
    ) -> GqlResult<DownloadHistoryPagePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let limit = limit.unwrap_or(50).clamp(1, 50) as usize;
        let offset = offset.unwrap_or(0).max(0) as usize;
        let sort = sort_key.map(|key| scryer_application::DownloadHistorySort {
            key: key.into_application(),
            direction: sort_direction
                .unwrap_or(SortDirectionValue::Asc)
                .into_application(),
        });
        let page = app
            .list_download_history_page(
                &actor,
                limit,
                offset,
                filters.map(|filters| {
                    filters
                        .into_iter()
                        .map(DownloadHistoryFilterValue::into_application)
                        .collect()
                }),
                client_ids,
                scryer_submitted_only.unwrap_or(false),
                sort,
            )
            .await
            .map_err(to_gql_error)?;
        Ok(from_download_history_page(page))
    }

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

    async fn auth_runtime_state(&self, ctx: &Context<'_>) -> GqlResult<AuthRuntimeStatePayload> {
        let auth_runtime = auth_runtime_from_ctx(ctx);
        Ok(from_auth_runtime_state(&auth_runtime.snapshot()))
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
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.get_media_settings(&actor, scope.into_media_facet())
            .await
            .map(|settings| from_media_settings(scope, settings))
            .map_err(to_gql_error)
    }

    async fn library_paths(&self, ctx: &Context<'_>) -> GqlResult<LibraryPathsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        app.get_library_paths(&actor)
            .await
            .map(from_library_paths_settings)
            .map_err(to_gql_error)
    }

    async fn service_settings(&self, ctx: &Context<'_>) -> GqlResult<ServiceSettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
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
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
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
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
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
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
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
        let stats = app.indexer_query_stats(&actor).map_err(to_gql_error)?;
        let mut payloads: Vec<IndexerConfigPayload> =
            configs.into_iter().map(from_indexer_config).collect();
        for payload in &mut payloads {
            if let Some(s) = stats.iter().find(|s| s.indexer_id == payload.id) {
                payload.last_query_at = s.last_query_at.clone();
            }
        }
        Ok(payloads)
    }

    async fn indexer(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<IndexerConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mut payload = app
            .get_indexer_config(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_indexer_config);
        if let Some(ref mut p) = payload {
            let stats = app.indexer_query_stats(&actor).map_err(to_gql_error)?;
            if let Some(s) = stats.iter().find(|s| s.indexer_id == p.id) {
                p.last_query_at = s.last_query_at.clone();
            }
        }
        Ok(payload)
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

    async fn download_client_config(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<DownloadClientConfigPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let config = app
            .get_download_client_config(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_download_client_config);
        Ok(config)
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
        Ok(users.into_iter().map(from_user).collect())
    }

    async fn user(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<UserPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user = app.get_user(&actor, &id).await.map_err(to_gql_error)?;
        Ok(user.map(from_user))
    }

    async fn system_health(&self, ctx: &Context<'_>) -> GqlResult<SystemHealthPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let health = app.system_health(&actor).await.map_err(to_gql_error)?;
        Ok(from_system_health(health))
    }

    async fn scryer_version(&self, ctx: &Context<'_>) -> GqlResult<String> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(SCRYER_VERSION.to_string())
    }

    async fn smg_version_compatibility_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<SmgVersionCompatibilityNoticePayload>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let notice = app
            .smg_version_compatibility_notice()
            .await
            .map_err(to_gql_error)?;
        Ok(notice.map(from_smg_version_compatibility_notice))
    }

    async fn recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<RecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let all = app
            .list_recycled_items(&actor)
            .await
            .map_err(to_gql_error)?;
        let total_count = all.len() as i32;
        let limit = limit.clamp(1, 500) as usize;
        let offset = offset.max(0) as usize;
        let items = all
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|entry| {
                let file_name = std::path::Path::new(&entry.manifest.original_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                RecycledItemPayload {
                    id: entry.entry_id,
                    original_path: entry.manifest.original_path,
                    file_name,
                    size_bytes: entry.manifest.size_bytes as i64,
                    title_id: entry.manifest.title_id,
                    reason: entry.manifest.reason,
                    recycled_at: entry.manifest.recycled_at,
                    media_root: entry.media_root,
                }
            })
            .collect();
        Ok(RecycledItemsPayload { items, total_count })
    }

    async fn health_checks(&self, ctx: &Context<'_>) -> GqlResult<Vec<HealthCheckPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let results = app
            .cached_health_check_results(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(results
            .iter()
            .cloned()
            .map(from_health_check_result)
            .collect())
    }

    async fn disk_space(&self, ctx: &Context<'_>) -> GqlResult<Vec<DiskSpacePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let info = app.disk_space(&actor).await.map_err(to_gql_error)?;
        Ok(info.into_iter().map(from_disk_space).collect())
    }

    async fn backups(&self, ctx: &Context<'_>) -> GqlResult<Vec<BackupInfoPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let backups = app.list_backups(&actor).await.map_err(to_gql_error)?;
        Ok(backups.into_iter().map(from_backup_info).collect())
    }

    async fn pending_releases(&self, ctx: &Context<'_>) -> GqlResult<Vec<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let releases = app.list_pending_releases().await.map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
    }

    async fn pending_release(
        &self,
        ctx: &Context<'_>,
        id: String,
    ) -> GqlResult<Option<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let release = app
            .get_pending_release(&actor, &id)
            .await
            .map_err(to_gql_error)?
            .map(from_pending_release);
        Ok(release)
    }

    async fn import_history(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<ImportRecordPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageTitle) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let records = app
            .list_import_history(&actor, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(records
            .into_iter()
            .map(crate::mappers::from_import_record)
            .collect())
    }

    async fn preview_manual_import(
        &self,
        ctx: &Context<'_>,
        client_id: Option<String>,
        download_client_item_id: String,
        title_id: String,
    ) -> GqlResult<ManualImportPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageTitle) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

        let preview = scryer_application::preview_manual_import(
            &app,
            client_id.as_deref(),
            &download_client_item_id,
            &title_id,
        )
        .await
        .map_err(to_gql_error)?;

        Ok(ManualImportPreviewPayload {
            files: preview
                .files
                .into_iter()
                .map(|f| ManualImportFilePreviewPayload {
                    file_path: f.file_path,
                    file_name: f.file_name,
                    size_bytes: f.size_bytes.to_string(),
                    quality: f.quality,
                    parsed_season: f.parsed_season.map(|v| v as i32),
                    parsed_episodes: f.parsed_episodes.into_iter().map(|v| v as i32).collect(),
                    suggested_episode_id: f.suggested_episode_id,
                    suggested_episode_label: f.suggested_episode_label,
                })
                .collect(),
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(from_episode)
                .collect(),
        })
    }

    async fn me(&self, ctx: &Context<'_>) -> GqlResult<Option<UserPayload>> {
        match current_user_from_ctx(ctx) {
            Some(user) => Ok(Some(from_user(user))),
            None => Ok(None),
        }
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<WantedStatusValue>,
        media_type: Option<WantedMediaTypeValue>,
        title_id: Option<String>,
        title_search: Option<String>,
        latest_decision_code: Option<String>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
    ) -> GqlResult<WantedItemsListPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let (items, total) = app
            .list_wanted_items(WantedItemsQuery {
                status: status.map(|value| value.as_str().to_string()),
                media_type: media_type.map(|value| value.as_str().to_string()),
                title_id,
                title_search,
                latest_decision_code,
                limit,
                offset,
            })
            .await
            .map_err(to_gql_error)?;
        Ok(WantedItemsListPayload {
            items: items.into_iter().map(from_wanted_item).collect(),
            total,
        })
    }

    async fn cutoff_unmet_titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
    ) -> GqlResult<Vec<CutoffUnmetItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_cutoff_unmet_titles(&actor, facet.map(MediaFacetValue::into_domain))
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_cutoff_unmet_item).collect())
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        wanted_item_id: Option<String>,
        title_id: Option<String>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(ReleaseDecisionsQuery {
                wanted_item_id,
                title_id,
                limit,
            })
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    async fn title_acquisition_diagnostics(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<TitleAcquisitionDiagnosticsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let diagnostics = app
            .title_acquisition_diagnostics(&title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_acquisition_diagnostics(diagnostics))
    }

    // ── Rule Sets ──────────────────────────────────────────────────────

    async fn rule_sets(&self, ctx: &Context<'_>) -> GqlResult<Vec<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_sets = app.list_rule_sets(&actor).await.map_err(to_gql_error)?;
        Ok(rule_sets
            .into_iter()
            .map(crate::mappers::from_rule_set)
            .collect())
    }

    async fn rule_set(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<RuleSetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let rule_set = app.get_rule_set(&actor, &id).await.map_err(to_gql_error)?;
        Ok(rule_set.map(crate::mappers::from_rule_set))
    }

    // ── Post-Processing Scripts ──────────────────────────────────────────

    async fn post_processing_scripts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<PostProcessingScriptPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }

        let scripts = app
            .list_post_processing_scripts()
            .await
            .map_err(to_gql_error)?;
        Ok(scripts
            .into_iter()
            .map(crate::mappers::from_pp_script)
            .collect())
    }

    async fn post_processing_script_runs(
        &self,
        ctx: &Context<'_>,
        script_id: String,
        limit: Option<i32>,
    ) -> GqlResult<Vec<PostProcessingScriptRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }

        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let runs = app
            .list_post_processing_script_runs(&script_id, limit)
            .await
            .map_err(to_gql_error)?;
        Ok(runs
            .into_iter()
            .map(crate::mappers::from_pp_script_run)
            .collect())
    }

    // ── Plugins ──────────────────────────────────────────────────────────

    async fn plugins(&self, ctx: &Context<'_>) -> GqlResult<Vec<RegistryPluginPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let plugins = app
            .list_available_plugins(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(plugins
            .into_iter()
            .map(crate::mappers::from_registry_plugin)
            .collect())
    }

    async fn plugin_catalog_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<PluginCatalogStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let status = app
            .plugin_catalog_status(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(crate::mappers::from_plugin_catalog_status(status))
    }

    /// List community rule packs from the plugin registry.
    async fn rule_pack_registry(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<RulePackRegistryEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let packs = app
            .list_rule_pack_registry(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(packs
            .into_iter()
            .map(|p| RulePackRegistryEntryPayload {
                id: p.id,
                name: p.name,
                description: p.description,
                author: p.author,
                version: p.version,
            })
            .collect())
    }

    /// Fetch templates from a community rule pack by its registry ID.
    async fn rule_pack_templates(
        &self,
        ctx: &Context<'_>,
        pack_id: String,
    ) -> GqlResult<Vec<RulePackTemplatePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let templates = app
            .fetch_rule_pack_templates(&actor, &pack_id)
            .await
            .map_err(to_gql_error)?;
        Ok(templates
            .into_iter()
            .map(|t| RulePackTemplatePayload {
                id: t.id,
                title: t.title,
                description: t.description,
                category: t.category,
                rego_source: t.rego_source,
                applied_facets: t.applied_facets,
            })
            .collect())
    }

    /// Returns all available indexer provider types from loaded plugins,
    /// with their config field schemas for dynamic form rendering.
    async fn indexer_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_indexer_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(pt, name, fields, default_base_url, Vec::new(), Vec::new())
            })
            .collect())
    }

    async fn download_client_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_download_client_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(pt, name, fields, default_base_url, Vec::new(), Vec::new())
            })
            .collect())
    }

    async fn subtitle_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let available_host_bindings = app
            .subtitle_host_bindings()
            .await
            .map_err(to_gql_error)?
            .into_keys()
            .map(|binding| binding.as_str().to_string())
            .collect::<Vec<_>>();
        let provider_types = app.available_subtitle_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|provider_type| {
                let name = app
                    .subtitle_provider_name(&provider_type)
                    .unwrap_or_else(|| provider_type.clone());
                let fields = app.subtitle_provider_config_fields(&provider_type);
                let recommended_facets = app.subtitle_provider_recommended_facets(&provider_type);
                from_provider_type(
                    provider_type,
                    name,
                    fields,
                    None,
                    available_host_bindings.clone(),
                    recommended_facets,
                )
            })
            .collect())
    }

    // ── Metadata Gateway (proxied from SMG) ──────────────────────────────

    async fn search_metadata(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(name = "type")] type_hint: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
        year: Option<i32>,
    ) -> GqlResult<Vec<MetadataSearchItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.clamp(1, 100);
        let results = app
            .search_metadata(&actor, &query, &type_hint, limit, &language, year)
            .await
            .map_err(to_gql_error)?;
        Ok(results.into_iter().map(from_metadata_search_item).collect())
    }

    async fn search_metadata_multi(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 25)] limit: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSearchMultiPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let limit = limit.clamp(1, 100);
        let result = app
            .search_metadata_multi(&actor, &query, limit, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSearchMultiPayload {
            movies: result
                .movies
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
            series: result
                .series
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
            anime: result
                .anime
                .into_iter()
                .map(from_metadata_search_item)
                .collect(),
        })
    }

    async fn metadata_movie(
        &self,
        ctx: &Context<'_>,
        tvdb_id: i32,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataMoviePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let movie = app
            .get_metadata_movie(&actor, tvdb_id as i64, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataMoviePayload {
            tvdb_id: movie.tvdb_id.to_string(),
            name: movie.name,
            slug: movie.slug,
            year: movie.year,
            status: movie.content_status,
            overview: movie.overview,
            poster_url: movie.poster_url,
            language: movie.language,
            runtime_minutes: movie.runtime_minutes,
            sort_title: movie.sort_title,
            imdb_id: movie.imdb_id,
            genres: movie.genres,
            studio: movie.studio,
            tmdb_release_date: movie.tmdb_release_date,
        })
    }

    async fn metadata_series(
        &self,
        ctx: &Context<'_>,
        id: String,
        #[graphql(default = true)] include_episodes: bool,
        #[graphql(default_with = "\"eng\".to_string()")] language: String,
    ) -> GqlResult<MetadataSeriesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let tvdb_id: i64 = id.parse().map_err(|_| Error::new("invalid tvdb id"))?;
        let series = app
            .get_metadata_series(&actor, tvdb_id, &language)
            .await
            .map_err(to_gql_error)?;
        Ok(MetadataSeriesPayload {
            tvdb_id: series.tvdb_id.to_string(),
            name: series.name,
            sort_name: series.sort_name,
            slug: series.slug,
            year: series.year,
            status: series.content_status,
            first_aired: series.first_aired,
            overview: series.overview,
            network: series.network,
            runtime_minutes: series.runtime_minutes,
            poster_url: series.poster_url,
            country: series.country,
            genres: series.genres,
            aliases: series.aliases,
            seasons: series
                .seasons
                .into_iter()
                .map(|s| MetadataSeasonPayload {
                    tvdb_id: s.tvdb_id.to_string(),
                    number: s.number,
                    label: s.label,
                    episode_type: s.episode_type,
                })
                .collect(),
            episodes: if include_episodes {
                series
                    .episodes
                    .into_iter()
                    .map(|e| MetadataEpisodePayload {
                        tvdb_id: e.tvdb_id.to_string(),
                        episode_number: e.episode_number,
                        season_number: e.season_number,
                        name: e.name,
                        aired: e.aired,
                        runtime_minutes: e.runtime_minutes,
                        is_filler: e.is_filler,
                    })
                    .collect()
            } else {
                vec![]
            },
        })
    }

    // ── Calendar ──────────────────────────────────────────────────────

    async fn calendar_episodes(
        &self,
        ctx: &Context<'_>,
        start_date: String,
        end_date: String,
    ) -> GqlResult<Vec<CalendarEpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_calendar_episodes(&actor, &start_date, &end_date)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_calendar_episode).collect())
    }

    // ── Notifications ────────────────────────────────────────────────────

    async fn notification_channels(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationChannelPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let channels = app
            .list_notification_channels(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(channels
            .into_iter()
            .map(crate::mappers::from_notification_channel)
            .collect())
    }

    async fn notification_subscriptions(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationSubscriptionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let subs = app
            .list_notification_subscriptions(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(subs
            .into_iter()
            .map(crate::mappers::from_notification_subscription)
            .collect())
    }

    async fn notification_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let provider_types = app.available_notification_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|pt| {
                let name = app
                    .notification_provider_name(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = app.notification_provider_config_fields(&pt);
                from_provider_type(pt, name, fields, None, Vec::new(), Vec::new())
            })
            .collect())
    }

    async fn notification_event_types(&self, ctx: &Context<'_>) -> GqlResult<Vec<String>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        Ok(app
            .subscribable_notification_event_types()
            .iter()
            .map(|e| e.as_str().to_string())
            .collect())
    }

    // ── Service Logs ────────────────────────────────────────────────────

    async fn setup_status(&self, ctx: &Context<'_>) -> GqlResult<SetupStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let setup_complete = app.setup_complete().await.map_err(to_gql_error)?;

        let has_download_clients = !app
            .list_download_client_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        let has_indexers = !app
            .list_indexer_configs(&actor, None)
            .await
            .map_err(to_gql_error)?
            .is_empty();

        Ok(SetupStatusPayload {
            setup_complete,
            has_download_clients,
            has_indexers,
        })
    }

    async fn browse_path(
        &self,
        ctx: &Context<'_>,
        #[graphql(default_with = "String::from(\"/\")")] path: String,
    ) -> GqlResult<Vec<DirectoryEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let target = std::path::Path::new(&path);
        if !target.is_absolute() {
            return Err(Error::new("path must be absolute"));
        }
        let read_dir = std::fs::read_dir(target)
            .map_err(|e| Error::new(format!("cannot read directory: {e}")))?;
        let mut entries: Vec<DirectoryEntryPayload> = Vec::new();
        for entry in read_dir.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if !ft.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let full_path = entry.path().to_string_lossy().into_owned();
            entries.push(DirectoryEntryPayload {
                name,
                path: full_path,
            });
        }
        entries.sort_by_key(|a| a.name.to_lowercase());
        Ok(entries)
    }

    async fn service_logs(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 250)] limit: i32,
    ) -> GqlResult<ServiceLogsPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(Error::new("insufficient entitlements"));
        }
        let safe_limit = (limit.clamp(1, 2000)) as usize;
        let lines = match ctx.data_opt::<crate::context::LogBuffer>() {
            Some(buf) => buf.snapshot(safe_limit),
            None => vec![],
        };
        let count = lines.len() as i32;
        Ok(ServiceLogsPayload {
            generated_at: Utc::now().to_rfc3339(),
            lines,
            count,
        })
    }

    /// List external subtitles for a title.
    async fn external_subtitles(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<Vec<ExternalSubtitlePayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let app = app_from_ctx(ctx)?;
        let downloads = app
            .list_external_subtitles_for_title(&title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(downloads
            .into_iter()
            .map(|d| ExternalSubtitlePayload {
                id: d.id,
                media_file_id: d.media_file_id,
                title_id: d.title_id,
                episode_id: d.episode_id,
                source_kind: d.source_kind.as_str().to_string(),
                language: d.language,
                provider: d.provider,
                provider_file_id: d.provider_file_id,
                file_path: d.file_path,
                score: d.score,
                hearing_impaired: d.hearing_impaired,
                forced: d.forced,
                ai_translated: d.ai_translated,
                machine_translated: d.machine_translated,
                uploader: d.uploader,
                release_info: d.release_info,
                synced: d.synced,
                downloaded_at: d.downloaded_at,
            })
            .collect())
    }

    /// List external subtitle blocklist entries for a specific media file.
    async fn external_subtitle_blocklist_entries(
        &self,
        ctx: &Context<'_>,
        media_file_id: String,
    ) -> GqlResult<Vec<ExternalSubtitleBlocklistEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ViewCatalog) {
            return Err(Error::new("insufficient entitlements"));
        }
        let app = app_from_ctx(ctx)?;
        let entries = app
            .list_external_subtitle_blocklist_for_media_file(&media_file_id)
            .await
            .map_err(to_gql_error)?;
        Ok(entries
            .into_iter()
            .map(|entry| ExternalSubtitleBlocklistEntryPayload {
                id: entry.id,
                media_file_id: entry.media_file_id,
                provider: entry.provider,
                provider_file_id: entry.provider_file_id,
                language: entry.language,
                reason: entry.reason,
                created_at: entry.created_at,
            })
            .collect())
    }
}

#[ComplexObject]
impl TitlePayload {
    async fn required_audio_languages_override(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<Vec<String>>> {
        let app = app_from_ctx(ctx)?;
        app.load_title_required_audio_override(&self.id)
            .await
            .map_err(to_gql_error)
    }

    async fn effective_required_audio_languages(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<String>> {
        let app = app_from_ctx(ctx)?;
        if let Some(languages) = app
            .load_title_required_audio_override(&self.id)
            .await
            .map_err(to_gql_error)?
        {
            return Ok(languages);
        }
        app.load_facet_required_audio_languages(title_scope_from_facet(self.facet).as_scope_id())
            .await
            .map_err(to_gql_error)
    }

    async fn inherits_required_audio_languages(&self, ctx: &Context<'_>) -> GqlResult<bool> {
        let app = app_from_ctx(ctx)?;
        Ok(app
            .load_title_required_audio_override(&self.id)
            .await?
            .is_none())
    }

    async fn collections(&self, ctx: &Context<'_>) -> GqlResult<Vec<CollectionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collections = app
            .list_collections(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(collections.into_iter().map(from_collection).collect())
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let files = app
            .list_title_media_files(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(files.into_iter().map(from_title_media_file).collect())
    }

    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        status: Option<String>,
    ) -> GqlResult<Vec<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let (items, _) = app
            .list_wanted_items(WantedItemsQuery {
                status,
                media_type: None,
                title_id: Some(self.id.clone()),
                title_search: None,
                latest_decision_code: None,
                limit: 500,
                offset: 0,
            })
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_wanted_item).collect())
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(ReleaseDecisionsQuery {
                wanted_item_id: None,
                title_id: Some(self.id.clone()),
                limit,
            })
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    async fn download_queue_items(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_download_queue_for_title(
                &actor,
                &self.id,
                include_all_activity.unwrap_or(false),
                include_history_only.unwrap_or(false),
                include_import_activity.unwrap_or(false),
                activity_filter
                    .unwrap_or(DownloadActivityFilterValue::All)
                    .into_application(),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_download_queue_item).collect())
    }
}

#[ComplexObject]
impl CollectionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episodes(&self, ctx: &Context<'_>) -> GqlResult<Vec<EpisodePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episodes = app
            .list_episodes(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(episodes.into_iter().map(from_episode).collect())
    }
}

#[ComplexObject]
impl EpisodePayload {
    async fn parent_title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let wanted_item = app
            .get_title_wanted_item(&actor, &self.title_id, Some(&self.id))
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(wanted_item)
    }

    async fn media_files(&self, ctx: &Context<'_>) -> GqlResult<Vec<TitleMediaFilePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let files = app
            .list_title_media_files(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(files
            .into_iter()
            .filter(|file| file.episode_id.as_deref() == Some(self.id.as_str()))
            .map(from_title_media_file)
            .collect())
    }
}

#[ComplexObject]
impl TitleMediaFilePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }
}

#[ComplexObject]
impl WantedItemPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn collection(&self, ctx: &Context<'_>) -> GqlResult<Option<CollectionPayload>> {
        let Some(collection_id) = self.collection_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let collection = app
            .get_collection(&actor, collection_id)
            .await
            .map_err(to_gql_error)?
            .map(from_collection);
        Ok(collection)
    }

    async fn episode(&self, ctx: &Context<'_>) -> GqlResult<Option<EpisodePayload>> {
        let Some(episode_id) = self.episode_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let episode = app
            .get_episode(&actor, episode_id)
            .await
            .map_err(to_gql_error)?
            .map(from_episode);
        Ok(episode)
    }

    async fn release_decisions(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 50)] limit: i64,
    ) -> GqlResult<Vec<ReleaseDecisionPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ViewCatalog) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let decisions = app
            .list_release_decisions(ReleaseDecisionsQuery {
                wanted_item_id: Some(self.id.clone()),
                title_id: None,
                limit,
            })
            .await
            .map_err(to_gql_error)?;
        Ok(decisions.into_iter().map(from_release_decision).collect())
    }

    async fn pending_releases(&self, ctx: &Context<'_>) -> GqlResult<Vec<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let releases = app
            .list_pending_releases_for_wanted_item(&actor, &self.id)
            .await
            .map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
    }
}

#[ComplexObject]
impl ReleaseDecisionPayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title = app
            .get_title(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, &self.wanted_item_id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(item)
    }
}

#[ComplexObject]
impl DownloadQueueItemPayload {
    async fn queue_scope(&self, ctx: &Context<'_>) -> GqlResult<Option<QueueDownloadScopePayload>> {
        let client_type = self.client_type.trim();
        let download_client_item_id = self.download_client_item_id.trim();
        if client_type.is_empty() || download_client_item_id.is_empty() {
            return Ok(self
                .episode_id
                .as_ref()
                .map(|episode_id| QueueDownloadScopePayload {
                    kind: "episode".to_string(),
                    episode_id: Some(episode_id.clone()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                }));
        }

        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_id = self.client_id.trim();
        let client_id = if client_id.is_empty() {
            None
        } else {
            Some(client_id)
        };
        let scope = app
            .find_download_queue_scope(&actor, client_id, client_type, download_client_item_id)
            .await
            .map_err(to_gql_error)?;

        Ok(scope.map(from_submission_scope).or_else(|| {
            self.episode_id
                .as_ref()
                .map(|episode_id| QueueDownloadScopePayload {
                    kind: "episode".to_string(),
                    episode_id: Some(episode_id.clone()),
                    episode_ids: Vec::new(),
                    collection_id: None,
                })
        }))
    }

    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let Some(title_id) = self.title_id.as_deref() else {
            return Ok(None);
        };
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let title = app
            .get_title_for_management(&actor, title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }
}

#[ComplexObject]
impl PendingReleasePayload {
    async fn title(&self, ctx: &Context<'_>) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let title = app
            .get_title_for_management(&actor, &self.title_id)
            .await
            .map_err(to_gql_error)?
            .map(from_title);
        Ok(title)
    }

    async fn wanted_item(&self, ctx: &Context<'_>) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&scryer_domain::Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }
        let wanted_item = app
            .get_wanted_item_for_management(&actor, &self.wanted_item_id)
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item);
        Ok(wanted_item)
    }
}
