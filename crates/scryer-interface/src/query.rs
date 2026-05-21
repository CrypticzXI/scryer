use async_graphql::{Context, Error, MergedObject, Object, Result as GqlResult};

use chrono::Utc;
use scryer_application::{
    DownloadImportFilter, PendingImportCounts, SCRYER_VERSION, TitleHistoryFilter,
    WantedItemsQuery, is_supported_title_history_event_type, supported_title_history_event_types,
};
use scryer_domain::{AppPermission, LibraryPermission, TitleHistoryEventType};
use scryer_interface_metadata::MetadataQueries;
use scryer_interface_settings::SettingsQueries;

use crate::context::{
    actor_from_ctx, actor_has_any_library_permission, actor_has_app_permission, app_from_ctx,
    current_user_from_ctx, require_app_permission, to_gql_error,
};
use crate::mappers::{
    from_activity_event, from_backup_info, from_delete_preview, from_domain_event,
    from_download_queue_item, from_episode, from_external_import_monitor_warmup_progress,
    from_job_definition, from_job_run, from_library, from_library_scan_session,
    from_library_settings, from_media_rename_plan, from_pending_import_connection,
    from_pending_import_counts, from_pending_release, from_provider_type,
    from_smg_version_compatibility_notice, from_system_health, from_title,
    from_title_acquisition_diagnostics, from_title_history_page, from_title_history_record,
    from_title_release_blocklist_entry, from_user, from_wanted_item,
};
use crate::types::*;

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

fn from_cutoff_unmet_item(item: scryer_application::CutoffUnmetItem) -> CutoffUnmetItemPayload {
    CutoffUnmetItemPayload {
        title_id: item.title_id,
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: MediaFacetValue::from_domain(item.title_facet),
        library_id: item.library_id,
        library_name: item.library_name,
        library_slug: item.library_slug,
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
    selection: TitlePayloadSelection,
) -> GqlResult<Vec<TitlePayload>> {
    if titles.is_empty() {
        return Ok(Vec::new());
    }

    let title_ids: Vec<String> = titles.iter().map(|t| t.id.clone()).collect();
    let libraries = if selection.include_library_context {
        app.list_libraries_for_permission(actor, None, scryer_domain::LibraryPermission::View)
            .await
            .map_err(to_gql_error)?
    } else {
        Vec::new()
    };
    let library_map: std::collections::HashMap<&str, (&String, &String)> = libraries
        .iter()
        .map(|library| (library.id.as_str(), (&library.name, &library.slug)))
        .collect();
    let summaries = if selection.include_quality_tier {
        app.list_primary_collection_summaries(actor, &title_ids)
            .await
            .map_err(to_gql_error)?
    } else {
        Vec::new()
    };
    let media_size_summaries = if selection.include_size_bytes {
        app.list_title_media_size_summaries(actor, &title_ids)
            .await
            .map_err(to_gql_error)?
    } else {
        Vec::new()
    };
    let quality_summaries = if selection.include_current_quality_tier {
        app.list_title_quality_summaries(actor, &title_ids)
            .await
            .map_err(to_gql_error)?
    } else {
        Vec::new()
    };
    let episode_progress_summaries = if selection.include_episode_progress {
        app.list_title_episode_progress_summaries(actor, &title_ids)
            .await
            .map_err(to_gql_error)?
    } else {
        Vec::new()
    };
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
            let library_id = t.library_id.clone();
            let mut payload = from_title(t);
            if let Some((library_name, library_slug)) = library_map.get(library_id.as_str()) {
                payload.library_name = Some((*library_name).clone());
                payload.library_slug = Some((*library_slug).clone());
            }
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

#[derive(Clone, Copy)]
struct TitlePayloadSelection {
    include_external_ids: bool,
    include_library_context: bool,
    include_quality_tier: bool,
    include_current_quality_tier: bool,
    include_size_bytes: bool,
    include_episode_progress: bool,
}

impl TitlePayloadSelection {
    fn from_ctx(ctx: &Context<'_>) -> Self {
        let lookahead = ctx.look_ahead();
        Self {
            include_external_ids: lookahead.field("externalIds").exists(),
            include_library_context: lookahead.field("libraryName").exists()
                || lookahead.field("librarySlug").exists(),
            include_quality_tier: lookahead.field("qualityTier").exists(),
            include_current_quality_tier: lookahead.field("currentQualityTier").exists(),
            include_size_bytes: lookahead.field("sizeBytes").exists(),
            include_episode_progress: lookahead.field("episodesOwned").exists()
                || lookahead.field("episodesMonitored").exists()
                || lookahead.field("episodesTotal").exists(),
        }
    }
}

#[derive(Default)]
struct CatalogQueries;

#[derive(Default)]
struct ActivityQueries;

#[derive(Default)]
struct JobAndDownloadQueries;

#[derive(Default)]
struct SystemQueries;

#[derive(Default)]
struct AcquisitionQueries;

#[derive(Default)]
struct UtilityQueries;

#[derive(MergedObject, Default)]
pub struct QueryRoot(
    CatalogQueries,
    ActivityQueries,
    JobAndDownloadQueries,
    SettingsQueries,
    SystemQueries,
    AcquisitionQueries,
    MetadataQueries,
    UtilityQueries,
);

#[allow(clippy::too_many_arguments)]
#[Object]
impl CatalogQueries {
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<String>>,
        query: Option<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let parsed_facet = facet.map(MediaFacetValue::into_domain);
        let titles = if selection.include_external_ids {
            app.list_titles(&actor, parsed_facet, library_ids, query)
                .await
        } else {
            app.list_titles_without_external_ids(&actor, parsed_facet, library_ids, query)
                .await
        }
        .map_err(to_gql_error)?;

        title_payloads_from_titles(&app, &actor, titles, selection).await
    }

    async fn libraries(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        permission: Option<LibraryPermissionValue>,
    ) -> GqlResult<Vec<LibraryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let libraries = app
            .list_libraries_for_permission(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                permission
                    .map(LibraryPermissionValue::into_domain)
                    .unwrap_or(scryer_domain::LibraryPermission::View),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(libraries.into_iter().map(from_library).collect())
    }

    async fn library_settings(
        &self,
        ctx: &Context<'_>,
        library_id: String,
    ) -> GqlResult<LibrarySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let settings = app
            .get_library_settings(&actor, &library_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_library_settings(settings))
    }

    async fn titles_by_external_ids(
        &self,
        ctx: &Context<'_>,
        source: String,
        values: Vec<String>,
    ) -> GqlResult<Vec<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let titles = app
            .list_titles_by_external_ids(&actor, &source, &values)
            .await
            .map_err(to_gql_error)?;

        title_payloads_from_titles(&app, &actor, titles, selection).await
    }

    async fn title(&self, ctx: &Context<'_>, id: String) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let title = if selection.include_external_ids {
            app.get_title(&actor, &id).await
        } else {
            app.get_title_without_external_ids(&actor, &id).await
        }
        .map_err(to_gql_error)?;
        let Some(title) = title else {
            return Ok(None);
        };
        let mut payloads = title_payloads_from_titles(&app, &actor, vec![title], selection).await?;
        Ok(payloads.pop())
    }

    async fn title_by_slug(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        library_id: Option<String>,
        library_slug: Option<String>,
        slug: String,
    ) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let Some(title) = app
            .get_title_by_slug(&actor, facet.into_domain(), library_id, library_slug, &slug)
            .await
            .map_err(to_gql_error)?
        else {
            return Ok(None);
        };
        let mut payloads = title_payloads_from_titles(&app, &actor, vec![title], selection).await?;
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
                library_ids: None,
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
            library_ids: filter.library_ids,
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
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl ActivityQueries {
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

    async fn external_import_monitor_warmup_status(
        &self,
        ctx: &Context<'_>,
        session_id: String,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let snapshot = app
            .get_external_import_monitor_warmup_status(&actor, &session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_external_import_monitor_warmup_progress(snapshot))
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

        let can_resolve_imports =
            actor_has_any_library_permission(ctx, LibraryPermission::ResolveImports).await?;
        let can_manage_system_settings =
            actor_has_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let pending_import_counts = async {
            if can_resolve_imports {
                app.pending_import_counts(&actor).await
            } else {
                Ok(PendingImportCounts::default())
            }
        };
        let activity_import_count = async {
            if can_resolve_imports {
                app.count_download_import_items(&actor, DownloadImportFilter::All)
                    .await
            } else {
                Ok(0)
            }
        };
        let plugin_update_count = async {
            if can_manage_system_settings {
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
        library_ids: Option<Vec<String>>,
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
                library_ids,
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
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl JobAndDownloadQueries {
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
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl SystemQueries {
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

    async fn backups(&self, ctx: &Context<'_>) -> GqlResult<Vec<BackupInfoPayload>> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let backups = app.list_backups(&actor).await.map_err(to_gql_error)?;
        Ok(backups.into_iter().map(from_backup_info).collect())
    }

    async fn pending_releases(&self, ctx: &Context<'_>) -> GqlResult<Vec<PendingReleasePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let releases = app
            .list_pending_releases(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(releases.into_iter().map(from_pending_release).collect())
    }

    async fn import_history(
        &self,
        ctx: &Context<'_>,
        limit: Option<i32>,
    ) -> GqlResult<Vec<ImportRecordPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
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

        let preview = scryer_application::preview_manual_import(
            &app,
            &actor,
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
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl AcquisitionQueries {
    async fn wanted_items(
        &self,
        ctx: &Context<'_>,
        statuses: Option<Vec<WantedStatusValue>>,
        media_types: Option<Vec<WantedMediaTypeValue>>,
        title_id: Option<String>,
        library_ids: Option<Vec<String>>,
        title_search: Option<String>,
        latest_decision_codes: Option<Vec<String>>,
        #[graphql(default = 50)] limit: i64,
        #[graphql(default = 0)] offset: i64,
    ) -> GqlResult<WantedItemsListPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let (items, total) = app
            .list_wanted_items(
                &actor,
                WantedItemsQuery {
                    statuses: statuses
                        .unwrap_or_default()
                        .into_iter()
                        .map(|value| value.as_str().to_string())
                        .collect(),
                    media_types: media_types
                        .unwrap_or_default()
                        .into_iter()
                        .map(|value| value.as_str().to_string())
                        .collect(),
                    title_id,
                    library_ids: library_ids.unwrap_or_default(),
                    title_search,
                    latest_decision_codes: latest_decision_codes.unwrap_or_default(),
                    limit,
                    offset,
                },
            )
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
        library_ids: Option<Vec<String>>,
    ) -> GqlResult<Vec<CutoffUnmetItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_cutoff_unmet_titles(&actor, facet.map(MediaFacetValue::into_domain), library_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_cutoff_unmet_item).collect())
    }

    async fn title_acquisition_diagnostics(
        &self,
        ctx: &Context<'_>,
        title_id: String,
    ) -> GqlResult<TitleAcquisitionDiagnosticsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let diagnostics = app
            .title_acquisition_diagnostics(&actor, &title_id)
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

    // ── Post-Processing Scripts ──────────────────────────────────────────

    async fn post_processing_scripts(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<PostProcessingScriptPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let scripts = app
            .list_post_processing_scripts(&actor)
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

        let limit = limit.unwrap_or(50).clamp(1, 500) as usize;
        let runs = app
            .list_post_processing_script_runs(&actor, &script_id, limit)
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
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_indexer_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(
                    pt,
                    name,
                    fields,
                    default_base_url,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }

    async fn download_client_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_download_client_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|(pt, name, fields, default_base_url)| {
                from_provider_type(
                    pt,
                    name,
                    fields,
                    default_base_url,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }

    async fn subtitle_provider_types(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<ProviderTypePayload>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
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
                    Vec::new(),
                    false,
                )
            })
            .collect())
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl UtilityQueries {
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
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let provider_types = app.available_notification_provider_types();
        Ok(provider_types
            .into_iter()
            .map(|pt| {
                let name = app
                    .notification_provider_name(&pt)
                    .unwrap_or_else(|| pt.clone());
                let fields = app.notification_provider_config_fields(&pt);
                let supported_events = app
                    .notification_provider_supported_events(&pt)
                    .into_iter()
                    .map(|event| event.as_str().to_string())
                    .collect();
                let supports_test = app.notification_provider_supports_test(&pt);
                from_provider_type(
                    pt,
                    name,
                    fields,
                    None,
                    Vec::new(),
                    Vec::new(),
                    supported_events,
                    supports_test,
                )
            })
            .collect())
    }

    async fn notification_event_types(&self, ctx: &Context<'_>) -> GqlResult<Vec<String>> {
        let app = app_from_ctx(ctx)?;
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
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
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
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
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
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
        let app = app_from_ctx(ctx)?;
        let downloads = app
            .list_external_subtitles_for_title(&actor, &title_id)
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
        let app = app_from_ctx(ctx)?;
        let entries = app
            .list_external_subtitle_blocklist_for_media_file(&actor, &media_file_id)
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
