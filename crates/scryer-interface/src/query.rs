use async_graphql::{Context, ID, MergedObject, Object, Result as GqlResult};

use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, DownloadImportFilter, ExternalImportArrSourceKind as AppArrSourceKind,
    ExternalImportMonitorWarmupStatus,
    ExternalImportSetupSecretDraft as AppExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftStatus, ExternalImportSetupSecretInstanceKind,
    ExternalImportSetupSecretOverrideDraft, JwtSessionScope, MediaRequestCounts,
    OAuthAuthorizationSource, PendingImportCounts, RuntimePathStyle, SCRYER_VERSION, SortDirection,
    TitleCatalogContentStatus, TitleCatalogFilter, TitleCatalogSort, TitleCatalogSortKey,
    TitleHistoryFilter, WantedItemsQuery, is_supported_title_history_event_type,
    supported_title_history_event_types,
};
use scryer_domain::{AppPermission, LibraryPermission, TitleHistoryEventType};
use scryer_interface_metadata::MetadataQueries;
use scryer_interface_settings::SettingsQueries;
use std::{fs, io, path::Path};

use crate::context::{
    actor_from_ctx, actor_has_any_library_permission, actor_has_app_permission, app_from_ctx,
    current_user_from_ctx, mfa_verification_from_ctx, require_app_permission,
    require_config_app_permission, to_gql_error,
};
use crate::mappers::{
    catalog_discovery_query_from_input, discovery_home_query_from_input,
    discovery_item_detail_query_from_input, discovery_items_query_from_input, from_activity_event,
    from_backup_info, from_catalog_discovery, from_collection, from_delete_preview,
    from_delete_titles_preview, from_discovery_home, from_discovery_item,
    from_discovery_items_result, from_discovery_sync_status, from_domain_event,
    from_download_queue_item, from_episode, from_external_import_monitor_warmup_progress,
    from_job_definition, from_job_run, from_library, from_library_scan_session,
    from_library_settings, from_linked_account, from_media_rename_plan, from_media_request,
    from_media_request_counts, from_pending_import_connection, from_pending_import_counts,
    from_pending_release, from_provider_type, from_runtime_path_style,
    from_smg_scryer_update_notice, from_smg_version_compatibility_notice, from_system_health,
    from_title, from_title_acquisition_diagnostics, from_title_history_page,
    from_title_release_blocklist_entry, from_user_with_auth_factor_status, from_wanted_item,
};
use crate::types::*;

fn browse_path_read_dir(path: &str) -> Result<fs::ReadDir, AppError> {
    let target = Path::new(path);
    if !target.is_absolute() {
        return Err(AppError::Validation("Path must be absolute.".to_string()));
    }

    let metadata = fs::metadata(target).map_err(|error| browse_path_io_error(path, error))?;
    if !metadata.is_dir() {
        return Err(AppError::Validation(format!(
            "Path is not a directory: {path}"
        )));
    }

    fs::read_dir(target).map_err(|error| browse_path_io_error(path, error))
}

fn library_root_path_is_valid(path: &str) -> bool {
    let target = Path::new(path.trim());
    if !target.is_absolute() {
        return false;
    }

    fs::read_dir(target).is_ok()
}

fn browse_path_io_error(path: &str, error: io::Error) -> AppError {
    let message = match error.kind() {
        io::ErrorKind::NotFound => format!("Directory does not exist: {path}"),
        io::ErrorKind::PermissionDenied => format!("Directory is not readable: {path}"),
        _ => format!("Directory cannot be opened: {path}"),
    };
    AppError::Validation(message)
}

async fn require_library_settings_permission(ctx: &Context<'_>) -> GqlResult<()> {
    let app = app_from_ctx(ctx)?;
    let actor = actor_from_ctx(ctx)?;
    app.require_library_settings_read_permission(&actor)
        .await
        .map_err(to_gql_error)
}

fn supported_title_history_values_message() -> String {
    supported_title_history_event_types()
        .iter()
        .map(TitleHistoryEventType::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_required_datetime(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AppError::Validation(format!("invalid {field} timestamp: {error}")))
}

fn parse_supported_title_history_event_types(
    event_types: Option<Vec<TitleHistoryEventTypeValue>>,
) -> GqlResult<Option<Vec<TitleHistoryEventType>>> {
    let Some(event_types) = event_types else {
        return Ok(None);
    };

    let supported_values = supported_title_history_values_message();
    let mut parsed = Vec::with_capacity(event_types.len());
    for value in event_types {
        let event_type = value.into_domain();
        if !is_supported_title_history_event_type(event_type) {
            return Err(to_gql_error(AppError::Validation(format!(
                "unsupported title history event type `{}`. Supported values: {supported_values}",
                event_type.as_str()
            ))));
        }
        parsed.push(event_type);
    }

    Ok(Some(parsed))
}

const TITLE_CATALOG_PAGE_SIZE: usize = 300;

fn title_catalog_page_limit(limit: Option<i32>) -> usize {
    limit
        .unwrap_or(TITLE_CATALOG_PAGE_SIZE as i32)
        .clamp(1, TITLE_CATALOG_PAGE_SIZE as i32) as usize
}

fn title_catalog_page_offset(offset: Option<i32>) -> usize {
    offset.unwrap_or(0).max(0) as usize
}

fn title_catalog_sort_from_input(sort: Option<TitleCatalogSortInput>) -> TitleCatalogSort {
    let Some(sort) = sort else {
        return TitleCatalogSort::default();
    };
    let key = match sort.key {
        TitleCatalogSortKeyValue::Title => TitleCatalogSortKey::Title,
        TitleCatalogSortKeyValue::Library => TitleCatalogSortKey::Library,
        TitleCatalogSortKeyValue::Monitored => TitleCatalogSortKey::Monitored,
        TitleCatalogSortKeyValue::Quality => TitleCatalogSortKey::Quality,
        TitleCatalogSortKeyValue::Episodes => TitleCatalogSortKey::Episodes,
        TitleCatalogSortKeyValue::Status => TitleCatalogSortKey::Status,
        TitleCatalogSortKeyValue::Size => TitleCatalogSortKey::Size,
        TitleCatalogSortKeyValue::Added => TitleCatalogSortKey::Added,
    };
    let direction = sort
        .direction
        .map(SortDirectionValue::into_application)
        .unwrap_or(SortDirection::Asc);
    TitleCatalogSort { key, direction }
}

fn title_catalog_filter_from_input(filter: Option<TitleCatalogFilterInput>) -> TitleCatalogFilter {
    let Some(filter) = filter else {
        return TitleCatalogFilter::default();
    };
    TitleCatalogFilter {
        monitored: filter.monitored,
        content_statuses: filter
            .content_statuses
            .unwrap_or_default()
            .into_iter()
            .map(|status| match status {
                TitleCatalogContentStatusValue::Continuing => TitleCatalogContentStatus::Continuing,
                TitleCatalogContentStatusValue::Ended => TitleCatalogContentStatus::Ended,
            })
            .collect(),
    }
}

fn usize_to_i32_saturating(value: usize) -> i32 {
    value.min(i32::MAX as usize) as i32
}

fn optional_ids_to_strings(ids: Option<Vec<ID>>) -> Option<Vec<String>> {
    ids.map(|ids| ids.into_iter().map(String::from).collect())
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
                client_id: client.client_id.into(),
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
        title_id: item.title_id.into(),
        title_name: item.title_name,
        title_slug: item.title_slug,
        title_facet: MediaFacetValue::from_domain(item.title_facet),
        library_id: item.library_id.into(),
        library_name: item.library_name,
        library_slug: item.library_slug,
        episode_id: item.episode_id.map(Into::into),
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
    let libraries = async {
        if selection.include_library_context {
            app.list_libraries_for_permission(actor, None, scryer_domain::LibraryPermission::View)
                .await
        } else {
            Ok(Vec::new())
        }
    };
    let summaries = async {
        if selection.include_quality_tier {
            app.list_primary_collection_summaries(actor, &title_ids)
                .await
        } else {
            Ok(Vec::new())
        }
    };
    let media_size_summaries = async {
        if selection.include_size_bytes {
            app.list_title_media_size_summaries(actor, &title_ids).await
        } else {
            Ok(Vec::new())
        }
    };
    let quality_summaries = async {
        if selection.include_current_quality_tier {
            app.list_title_quality_summaries(actor, &title_ids).await
        } else {
            Ok(Vec::new())
        }
    };
    let episode_progress_summaries = async {
        if selection.include_episode_progress {
            app.list_title_episode_progress_summaries(actor, &title_ids)
                .await
        } else {
            Ok(Vec::new())
        }
    };
    let collections_by_title_id = async {
        if selection.include_collections {
            app.list_collections_for_titles(actor, &titles).await
        } else {
            Ok(std::collections::HashMap::new())
        }
    };
    let (
        libraries,
        summaries,
        media_size_summaries,
        quality_summaries,
        episode_progress_summaries,
        collections_by_title_id,
    ) = tokio::try_join!(
        libraries,
        summaries,
        media_size_summaries,
        quality_summaries,
        episode_progress_summaries,
        collections_by_title_id,
    )
    .map_err(to_gql_error)?;
    let library_map: std::collections::HashMap<&str, (&String, &String)> = libraries
        .iter()
        .map(|library| (library.id.as_str(), (&library.name, &library.slug)))
        .collect();
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
            payload.size_bytes = media_size_map.get(id.as_str()).copied().map(Long::from);
            if let Some(summary) = episode_progress_map.get(id.as_str()) {
                payload.episodes_owned = Some(summary.owned_episodes);
                payload.episodes_monitored = Some(summary.monitored_episodes);
                payload.episodes_total = Some(summary.total_episodes);
            }
            if selection.include_collections {
                payload.preloaded_collections = Some(
                    collections_by_title_id
                        .get(id.as_str())
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(from_collection)
                        .collect(),
                );
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
    include_collections: bool,
}

impl TitlePayloadSelection {
    fn from_ctx(ctx: &Context<'_>) -> Self {
        let lookahead = ctx.look_ahead();
        let title_field_exists = |name: &str| {
            lookahead.field(name).exists() || lookahead.field("items").field(name).exists()
        };
        Self {
            include_external_ids: title_field_exists("externalIds"),
            include_library_context: title_field_exists("libraryName")
                || title_field_exists("librarySlug"),
            include_quality_tier: title_field_exists("qualityTier"),
            include_current_quality_tier: title_field_exists("currentQualityTier"),
            include_size_bytes: title_field_exists("sizeBytes"),
            include_episode_progress: title_field_exists("episodesOwned")
                || title_field_exists("episodesMonitored")
                || title_field_exists("episodesTotal"),
            include_collections: title_field_exists("collections"),
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
struct ExternalImportQueries;

#[derive(Default)]
struct UtilityQueries;

#[derive(Default)]
struct AccountQueries;

#[derive(MergedObject, Default)]
pub struct QueryRoot(
    CatalogQueries,
    ActivityQueries,
    JobAndDownloadQueries,
    SettingsQueries,
    SystemQueries,
    AcquisitionQueries,
    ExternalImportQueries,
    MetadataQueries,
    UtilityQueries,
    AccountQueries,
);

fn gql_secret_instance_kind_query(
    kind: ExternalImportSetupSecretInstanceKind,
) -> ExternalImportConnectionKind {
    match kind {
        ExternalImportSetupSecretInstanceKind::Sonarr => ExternalImportConnectionKind::Sonarr,
        ExternalImportSetupSecretInstanceKind::Radarr => ExternalImportConnectionKind::Radarr,
        ExternalImportSetupSecretInstanceKind::Prowlarr => ExternalImportConnectionKind::Prowlarr,
    }
}

fn api_key_override_payload_query(
    override_entry: ExternalImportSetupSecretOverrideDraft,
) -> ExternalImportSetupApiKeyOverridePayload {
    ExternalImportSetupApiKeyOverridePayload {
        dedup_key: override_entry.dedup_key,
        api_key: override_entry.secret,
    }
}

fn password_override_payload_query(
    override_entry: ExternalImportSetupSecretOverrideDraft,
) -> ExternalImportSetupPasswordOverridePayload {
    ExternalImportSetupPasswordOverridePayload {
        dedup_key: override_entry.dedup_key,
        password: override_entry.secret,
    }
}

fn external_import_setup_secret_draft_payload_query(
    draft: AppExternalImportSetupSecretDraft,
) -> ExternalImportSetupSecretDraftPayload {
    let secrets = draft.secrets;
    ExternalImportSetupSecretDraftPayload {
        instance_api_keys: secrets
            .instance_api_keys
            .into_iter()
            .map(|entry| ExternalImportSetupInstanceApiKeyPayload {
                instance_id: ID::from(entry.instance_id),
                kind: gql_secret_instance_kind_query(entry.kind),
                api_key: entry.api_key,
            })
            .collect(),
        download_client_api_key_overrides: secrets
            .download_client_api_key_overrides
            .into_iter()
            .map(api_key_override_payload_query)
            .collect(),
        download_client_password_overrides: secrets
            .download_client_password_overrides
            .into_iter()
            .map(password_override_payload_query)
            .collect(),
        indexer_api_key_overrides: secrets
            .indexer_api_key_overrides
            .into_iter()
            .map(api_key_override_payload_query)
            .collect(),
        updated_at: draft.updated_at,
    }
}

fn external_import_setup_secret_status_payload_query(
    status: ExternalImportSetupSecretDraftStatus,
) -> ExternalImportSetupSecretDraftStatusPayload {
    ExternalImportSetupSecretDraftStatusPayload {
        has_draft: status.has_draft,
        owned_by_current_user: status.owned_by_current_user,
        updated_at: status.updated_at,
    }
}

#[Object]
impl ExternalImportQueries {
    async fn external_import_setup_secret_draft(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<ExternalImportSetupSecretDraftPayload>> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.get_external_import_setup_secret_draft(&actor)
            .await
            .map(|draft| draft.map(external_import_setup_secret_draft_payload_query))
            .map_err(to_gql_error)
    }

    async fn external_import_setup_secret_draft_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<ExternalImportSetupSecretDraftStatusPayload> {
        let actor = require_config_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        app.external_import_setup_secret_draft_status(&actor)
            .await
            .map(external_import_setup_secret_status_payload_query)
            .map_err(to_gql_error)
    }
}

#[Object]
impl AccountQueries {
    async fn linked_accounts(
        &self,
        ctx: &Context<'_>,
        user_id: Option<ID>,
    ) -> GqlResult<Vec<LinkedAccountPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let user_id = user_id.map(String::from);
        app.list_linked_accounts(&actor, user_id.as_deref())
            .await
            .map(|accounts| accounts.into_iter().map(from_linked_account).collect())
            .map_err(to_gql_error)
    }

    async fn external_account_invites(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<LinkedAccountPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.list_external_account_invites(&actor)
            .await
            .map(|accounts| accounts.into_iter().map(from_linked_account).collect())
            .map_err(to_gql_error)
    }
}

#[allow(clippy::too_many_arguments)]
#[Object]
impl CatalogQueries {
    async fn titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        query: Option<String>,
        filter: Option<TitleCatalogFilterInput>,
        sort: Option<TitleCatalogSortInput>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> GqlResult<TitleCatalogPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let page = app
            .list_titles(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids),
                query,
                title_catalog_filter_from_input(filter),
                title_catalog_sort_from_input(sort),
                title_catalog_page_limit(limit),
                title_catalog_page_offset(offset),
                selection.include_external_ids,
            )
            .await
            .map_err(to_gql_error)?;
        let limit = page.limit;
        let offset = page.offset;
        let has_more = page.has_more;
        let total_count = page.total_count;
        let items = title_payloads_from_titles(&app, &actor, page.items, selection).await?;

        Ok(TitleCatalogPayload {
            items,
            limit: usize_to_i32_saturating(limit),
            offset: usize_to_i32_saturating(offset),
            has_more,
            total_count: usize_to_i32_saturating(total_count),
        })
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

    async fn catalog_has_valid_root(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
    ) -> GqlResult<bool> {
        require_library_settings_permission(ctx).await?;
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let libraries = app
            .list_libraries_for_permission(
                &actor,
                Some(facet.into_domain()),
                LibraryPermission::ManageLibrary,
            )
            .await
            .map_err(to_gql_error)?;
        let root_paths = libraries
            .into_iter()
            .flat_map(|library| library.roots.into_iter().map(|root| root.path))
            .collect::<Vec<_>>();
        let has_valid_root = tokio::task::spawn_blocking(move || {
            root_paths
                .iter()
                .any(|path| library_root_path_is_valid(path))
        })
        .await
        .map_err(|error| {
            to_gql_error(AppError::Repository(format!(
                "catalog root validation task failed: {error}"
            )))
        })?;
        Ok(has_valid_root)
    }

    async fn media_requests(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        status: Option<MediaRequestStatusValue>,
    ) -> GqlResult<Vec<MediaRequestPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let requests = app
            .list_media_requests(
                &actor,
                scryer_application::ListMediaRequestsInput {
                    facet: facet.map(MediaFacetValue::into_domain),
                    library_ids: optional_ids_to_strings(library_ids),
                    status: status.map(MediaRequestStatusValue::into_domain),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(requests.into_iter().map(from_media_request).collect())
    }

    async fn my_media_requests(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
        status: Option<MediaRequestStatusValue>,
    ) -> GqlResult<Vec<MediaRequestPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let requests = app
            .list_my_media_requests(
                &actor,
                scryer_application::ListMediaRequestsInput {
                    facet: facet.map(MediaFacetValue::into_domain),
                    library_ids: optional_ids_to_strings(library_ids),
                    status: status.map(MediaRequestStatusValue::into_domain),
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(requests.into_iter().map(from_media_request).collect())
    }

    async fn library_settings(
        &self,
        ctx: &Context<'_>,
        library_id: ID,
    ) -> GqlResult<LibrarySettingsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_id = String::from(library_id);
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

    async fn title(&self, ctx: &Context<'_>, id: ID) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let title = if selection.include_external_ids {
            app.get_title(&actor, id.as_ref()).await
        } else {
            app.get_title_without_external_ids(&actor, id.as_ref())
                .await
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
        library_id: Option<ID>,
        library_slug: Option<String>,
        slug: String,
    ) -> GqlResult<Option<TitlePayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let selection = TitlePayloadSelection::from_ctx(ctx);
        let Some(title) = app
            .get_title_by_slug(
                &actor,
                facet.into_domain(),
                library_id.map(String::from),
                library_slug,
                &slug,
            )
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
            let title_id = title_id.to_string();
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
        title_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_id = String::from(title_id);
        let preview = app
            .preview_delete_title_files(&actor, &title_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn delete_titles_preview(
        &self,
        ctx: &Context<'_>,
        input: DeleteTitlesPreviewInput,
    ) -> GqlResult<DeleteTitlesPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let title_ids = input
            .title_ids
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        let preview = app
            .preview_delete_titles_files(&actor, &title_ids)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_titles_preview(preview))
    }

    async fn delete_media_file_preview(
        &self,
        ctx: &Context<'_>,
        file_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let file_id = String::from(file_id);
        let preview = app
            .preview_delete_media_file(&actor, &file_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn delete_external_subtitle_preview(
        &self,
        ctx: &Context<'_>,
        external_subtitle_id: ID,
    ) -> GqlResult<DeletePreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let external_subtitle_id = String::from(external_subtitle_id);
        let preview = app
            .preview_delete_external_subtitle_file(&actor, &external_subtitle_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_delete_preview(preview))
    }

    async fn wanted_item(&self, ctx: &Context<'_>, id: ID) -> GqlResult<Option<WantedItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .get_wanted_item(&actor, id.as_ref())
            .await
            .map_err(to_gql_error)?
            .map(from_wanted_item)
            .transpose()
            .map_err(to_gql_error)?;
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
            series_movie_link_id,
            season,
            episode,
            limit,
        } = input;

        let safe_limit = limit.unwrap_or(50).clamp(1, 200) as usize;
        let title_id = title_id.to_string();
        let cancel_token = tokio_util::sync::CancellationToken::new();
        struct CancelOnDrop(tokio_util::sync::CancellationToken);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.cancel();
            }
        }
        let _cancel_on_drop = CancelOnDrop(cancel_token.clone());
        let results = match (series_movie_link_id, season, episode) {
            (Some(series_movie_link_id), None, None) => app
                .search_indexers_for_series_movie(
                    &actor,
                    title_id,
                    series_movie_link_id.to_string(),
                    cancel_token.clone(),
                )
                .await
                .map_err(to_gql_error)?,
            (None, Some(season), Some(episode)) => app
                .search_indexers_for_episode(
                    &actor,
                    title_id,
                    season,
                    episode,
                    cancel_token.clone(),
                )
                .await
                .map_err(to_gql_error)?,
            (None, None, None) => app
                .search_indexers_for_title(&actor, title_id, cancel_token.clone())
                .await
                .map_err(to_gql_error)?,
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err(to_gql_error(AppError::Validation(
                    "episode searches require both season and episode".to_string(),
                )));
            }
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err(to_gql_error(AppError::Validation(
                    "series movie searches cannot include season or episode".to_string(),
                )));
            }
        };

        Ok(results
            .into_iter()
            .take(safe_limit)
            .map(crate::mappers::from_search_result)
            .collect())
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
            title_ids: filter
                .title_ids
                .map(|ids| ids.into_iter().map(String::from).collect::<Vec<String>>()),
            library_ids: filter
                .library_ids
                .map(|ids| ids.into_iter().map(String::from).collect::<Vec<String>>()),
            title_search: filter.title_search,
            download_id: filter.download_id,
            episode_id: filter.episode_id.map(String::from),
            group_by_event: filter.group_by_event.unwrap_or(false),
            limit: filter.limit.unwrap_or(50).max(1) as usize,
            offset: filter.offset.unwrap_or(0).max(0) as usize,
        };

        let page = app
            .list_title_history(&actor, &f)
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_history_page(page).map_err(to_gql_error)?)
    }

    async fn title_release_blocklist(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
        limit: Option<i32>,
    ) -> GqlResult<Vec<TitleReleaseBlocklistEntryPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_title_release_blocklist(
                &actor,
                title_id.as_ref(),
                limit.unwrap_or(100).max(1) as usize,
            )
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

    async fn audit_log(
        &self,
        ctx: &Context<'_>,
        event_types: Option<Vec<DomainEventTypeValue>>,
        title_id: Option<ID>,
        facet: Option<MediaFacetValue>,
        after_sequence: Option<Long>,
        before_sequence: Option<Long>,
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
            title_id: title_id.map(String::from),
            facet: facet.map(MediaFacetValue::into_domain),
            after_sequence: after_sequence.map(|value| value.0),
            before_sequence: before_sequence.map(|value| value.0),
            limit: limit.unwrap_or(100).max(1) as usize,
        };
        let events = app.audit_log(&actor, &filter).await.map_err(to_gql_error)?;
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

    async fn library_scan_session(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
    ) -> GqlResult<Option<LibraryScanProgressPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let session = app
            .library_scan_session(&actor, session_id.as_str())
            .await
            .map_err(to_gql_error)?;
        Ok(session.map(from_library_scan_session))
    }

    async fn external_import_arr_source_warmup_status(
        &self,
        ctx: &Context<'_>,
        session_id: ID,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.maintain_external_import_arr_source_sessions(&actor)
            .await
            .map_err(to_gql_error)?;
        let session_id = String::from(session_id);
        let snapshot = app
            .get_external_import_monitor_warmup_status(&actor, &session_id)
            .await
            .map_err(to_gql_error)?;
        Ok(from_external_import_monitor_warmup_progress(snapshot))
    }

    async fn external_import_aggregate_warmup_progress(
        &self,
        ctx: &Context<'_>,
        input: ExternalImportAggregateWarmupProgressInput,
    ) -> GqlResult<ExternalImportAggregateWarmupProgressPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        app.maintain_external_import_arr_source_sessions(&actor)
            .await
            .map_err(to_gql_error)?;
        if input.source_warmup_session_ids.is_empty() {
            return Ok(ExternalImportAggregateWarmupProgressPayload {
                status: ExternalImportMonitorWarmupStatusValue::Completed,
                titles_total_known: true,
                titles_fetched: 0,
                titles_total: 0,
                error_message: None,
            });
        }

        let mut status = ExternalImportMonitorWarmupStatusValue::Completed;
        let mut titles_total_known = true;
        let mut titles_fetched = 0i32;
        let mut titles_total = 0i32;
        let mut error_message = None;

        for session_id in input.source_warmup_session_ids {
            let session_id_string = session_id.to_string();
            let snapshot = app
                .get_external_import_monitor_warmup_status(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let source = app
                .external_import_arr_source_warmup_result(&actor, &session_id_string)
                .await
                .map_err(to_gql_error)?;
            let (known, fetched, total) = match source.kind {
                AppArrSourceKind::Radarr => (
                    snapshot.movies_total_known,
                    snapshot.movies_progress.completed,
                    snapshot.movies_progress.total,
                ),
                AppArrSourceKind::Sonarr => (
                    snapshot.series_total_known,
                    snapshot.series_progress.completed,
                    snapshot.series_progress.total,
                ),
            };
            titles_total_known &= known;
            titles_fetched = titles_fetched.saturating_add(fetched);
            titles_total = titles_total.saturating_add(total);

            match snapshot.status {
                ExternalImportMonitorWarmupStatus::Failed => {
                    status = ExternalImportMonitorWarmupStatusValue::Failed;
                    error_message = snapshot.error_message;
                }
                ExternalImportMonitorWarmupStatus::Canceled
                    if status != ExternalImportMonitorWarmupStatusValue::Failed =>
                {
                    status = ExternalImportMonitorWarmupStatusValue::Canceled;
                    error_message = snapshot.error_message;
                }
                ExternalImportMonitorWarmupStatus::Queued
                | ExternalImportMonitorWarmupStatus::Running
                    if matches!(status, ExternalImportMonitorWarmupStatusValue::Completed) =>
                {
                    status = ExternalImportMonitorWarmupStatusValue::Running;
                }
                _ => {}
            }
        }

        Ok(ExternalImportAggregateWarmupProgressPayload {
            status,
            titles_total_known,
            titles_fetched,
            titles_total,
            error_message,
        })
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
        let can_manage_titles =
            actor_has_any_library_permission(ctx, LibraryPermission::ManageTitles).await?;
        let can_manage_system_settings =
            actor_has_app_permission(ctx, AppPermission::ManageSystemSettings).await?;

        let pending_import_counts = async {
            if can_resolve_imports {
                app.pending_import_counts(&actor).await
            } else {
                Ok(PendingImportCounts::default())
            }
        };
        let pending_media_request_counts = async {
            if can_manage_titles {
                app.pending_media_request_counts(&actor).await
            } else {
                Ok(MediaRequestCounts::default())
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

        let (
            pending_import_counts,
            pending_media_request_counts,
            activity_import_count,
            plugin_update_count,
        ) = tokio::try_join!(
            pending_import_counts,
            pending_media_request_counts,
            activity_import_count,
            plugin_update_count,
        )
        .map_err(to_gql_error)?;

        Ok(NavigationBadgeCountsPayload {
            pending_import_counts: from_pending_import_counts(pending_import_counts),
            pending_media_request_counts: from_media_request_counts(pending_media_request_counts),
            activity_import_count: activity_import_count as i32,
            plugin_update_count: plugin_update_count as i32,
        })
    }

    async fn pending_imports(
        &self,
        ctx: &Context<'_>,
        facet: MediaFacetValue,
        library_ids: Option<Vec<ID>>,
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
                optional_ids_to_strings(library_ids),
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
        pending_import_id: ID,
    ) -> GqlResult<PendingImportBindingPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let pending_import_id = String::from(pending_import_id);
        let preview = app
            .preview_title_bound_pending_import(&actor, &pending_import_id)
            .await
            .map_err(to_gql_error)?;
        Ok(PendingImportBindingPreviewPayload {
            title: from_title(preview.title),
            file: PendingImportBindingFilePreviewPayload {
                file_path: preview.file.file_path,
                file_name: preview.file.file_name,
                size_bytes: Long::from(preview.file.size_bytes),
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
                suggested_episode_ids: preview
                    .file
                    .suggested_episode_ids
                    .into_iter()
                    .map(Into::into)
                    .collect(),
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

    async fn discovery_home(
        &self,
        ctx: &Context<'_>,
        input: Option<DiscoveryHomeInput>,
    ) -> GqlResult<DiscoveryHomePayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .discovery_home(&actor, discovery_home_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_home(result))
    }

    async fn discovery_items(
        &self,
        ctx: &Context<'_>,
        input: Option<DiscoveryItemsInput>,
    ) -> GqlResult<DiscoveryItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .discovery_items(&actor, discovery_items_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_items_result(result))
    }

    async fn discovery_item_detail(
        &self,
        ctx: &Context<'_>,
        input: DiscoveryItemDetailInput,
    ) -> GqlResult<Option<DiscoveryItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let item = app
            .discovery_item_detail(&actor, discovery_item_detail_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(item.map(from_discovery_item))
    }

    async fn catalog_discovery(
        &self,
        ctx: &Context<'_>,
        input: CatalogDiscoveryInput,
    ) -> GqlResult<CatalogDiscoveryPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let result = app
            .catalog_discovery(&actor, catalog_discovery_query_from_input(input))
            .await
            .map_err(to_gql_error)?;
        Ok(from_catalog_discovery(result))
    }

    async fn discovery_sync_status(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<DiscoverySyncStatusPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let status = app
            .discovery_sync_status(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(from_discovery_sync_status(status))
    }

    async fn download_queue(
        &self,
        ctx: &Context<'_>,
        include_all_activity: Option<bool>,
        include_history_only: Option<bool>,
        include_import_activity: Option<bool>,
        title_id: Option<ID>,
        activity_filter: Option<DownloadActivityFilterValue>,
    ) -> GqlResult<Vec<DownloadQueueItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = match title_id {
            Some(title_id) => {
                app.list_download_queue_for_title(
                    &actor,
                    title_id.as_ref(),
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
        client_ids: Option<Vec<ID>>,
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
                client_ids.map(|ids| ids.into_iter().map(String::from).collect()),
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
    async fn runtime_info(&self, ctx: &Context<'_>) -> GqlResult<RuntimeInfoPayload> {
        let _actor = actor_from_ctx(ctx)?;
        Ok(RuntimeInfoPayload {
            runtime_path_style: from_runtime_path_style(RuntimePathStyle::current()),
        })
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

    async fn smg_scryer_update_notice(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Option<SmgScryerUpdateNoticePayload>> {
        let app = app_from_ctx(ctx)?;
        let _actor = actor_from_ctx(ctx)?;
        let notice = app.smg_scryer_update_notice().await.map_err(to_gql_error)?;
        Ok(notice.map(from_smg_scryer_update_notice))
    }

    async fn recycled_items(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 500)] limit: i32,
        #[graphql(default = 0)] offset: i32,
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<RecycledItemsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let library_ids = library_ids.map(|ids| ids.into_iter().map(|id| id.to_string()).collect());
        let all = app
            .list_recycled_items(&actor, library_ids)
            .await
            .map_err(to_gql_error)?;
        let total_count = all.len() as i32;
        let limit = limit.clamp(1, 500) as usize;
        let offset = offset.max(0) as usize;
        let items = all
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|item| {
                Ok(RecycledItemPayload {
                    id: ID::from(item.id),
                    original_path: item.original_path,
                    file_name: item.file_name,
                    size_bytes: Long::from_u64_saturating(item.size_bytes),
                    title_id: item.title_id.map(ID::from),
                    reason: item.reason,
                    recycled_at: parse_required_datetime(
                        &item.recycled_at,
                        "recycled item recycled_at",
                    )
                    .map_err(to_gql_error)?,
                    media_root: item.media_root,
                    library_id: ID::from(item.library_id),
                    library_name: item.library_name,
                })
            })
            .collect::<GqlResult<Vec<_>>>()?;
        Ok(RecycledItemsPayload { items, total_count })
    }

    async fn backups(&self, ctx: &Context<'_>) -> GqlResult<Vec<BackupInfoPayload>> {
        require_app_permission(ctx, AppPermission::ManageSystemSettings).await?;
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let backups = app.list_backups(&actor).await.map_err(to_gql_error)?;
        backups
            .into_iter()
            .map(from_backup_info)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| to_gql_error(AppError::Validation(error)))
    }

    async fn pending_releases(
        &self,
        ctx: &Context<'_>,
        filter: Option<PendingReleaseFilterInput>,
        #[graphql(default = 50)] limit: i32,
        #[graphql(default = 0)] offset: i32,
    ) -> GqlResult<PendingReleasesPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let mut releases = app
            .list_pending_releases(&actor)
            .await
            .map_err(to_gql_error)?;
        if let Some(filter) = filter {
            if let Some(title_id) = filter.title_id {
                let title_id = String::from(title_id);
                releases.retain(|release| release.title_id == title_id);
            }
            if let Some(wanted_item_id) = filter.wanted_item_id {
                let wanted_item_id = String::from(wanted_item_id);
                releases.retain(|release| release.wanted_item_id == wanted_item_id);
            }
            if let Some(statuses) = filter.statuses {
                let statuses = statuses
                    .into_iter()
                    .map(PendingReleaseStatusValue::into_application)
                    .collect::<Vec<_>>();
                releases.retain(|release| statuses.contains(&release.status));
            }
        }

        let total_count = releases.len();
        let limit = limit.clamp(1, 500) as usize;
        let offset = offset.max(0) as usize;
        let items = releases
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(from_pending_release)
            .collect::<Vec<_>>();
        let has_more = offset.saturating_add(items.len()) < total_count;
        Ok(PendingReleasesPayload {
            items,
            limit: usize_to_i32_saturating(limit),
            offset: usize_to_i32_saturating(offset),
            has_more,
            total_count: usize_to_i32_saturating(total_count),
        })
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
        input: PreviewManualImportInput,
    ) -> GqlResult<ManualImportPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let client_id = input.client_id.map(|id| id.to_string());
        let title_id = input.title_id.to_string();

        let preview = scryer_application::preview_manual_import(
            &app,
            &actor,
            client_id.as_deref(),
            &input.download_client_item_id,
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
                    size_bytes: Long::from(f.size_bytes),
                    quality: f.quality,
                    parsed_season: f.parsed_season.map(|v| v as i32),
                    parsed_episodes: f.parsed_episodes.into_iter().map(|v| v as i32).collect(),
                    suggested_episode_id: f.suggested_episode_id.map(Into::into),
                    suggested_episode_label: f.suggested_episode_label,
                })
                .collect(),
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(from_episode)
                .collect(),
            available_series_movies: preview
                .available_series_movies
                .into_iter()
                .map(|target| ManualImportSeriesMovieTargetPayload {
                    series_movie_link_id: target.series_movie_link_id,
                    movie_title: target.movie_title,
                    year: target.year,
                    runtime_minutes: target.runtime_minutes,
                })
                .collect(),
        })
    }

    async fn preview_manual_import_path(
        &self,
        ctx: &Context<'_>,
        input: PreviewManualImportPathInput,
    ) -> GqlResult<ManualImportPreviewPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;

        let preview = scryer_application::preview_manual_import_path(
            &app,
            &actor,
            &input.path,
            input.title_id.as_ref(),
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
                    size_bytes: Long::from(f.size_bytes),
                    quality: f.quality,
                    parsed_season: f.parsed_season.map(|v| v as i32),
                    parsed_episodes: f.parsed_episodes.into_iter().map(|v| v as i32).collect(),
                    suggested_episode_id: f.suggested_episode_id.map(Into::into),
                    suggested_episode_label: f.suggested_episode_label,
                })
                .collect(),
            available_episodes: preview
                .available_episodes
                .into_iter()
                .map(from_episode)
                .collect(),
            available_series_movies: preview
                .available_series_movies
                .into_iter()
                .map(|target| ManualImportSeriesMovieTargetPayload {
                    series_movie_link_id: target.series_movie_link_id,
                    movie_title: target.movie_title,
                    year: target.year,
                    runtime_minutes: target.runtime_minutes,
                })
                .collect(),
        })
    }

    async fn me(&self, ctx: &Context<'_>) -> GqlResult<Option<UserPayload>> {
        let auth_context = mfa_verification_from_ctx(ctx);
        if auth_context.session_scope == JwtSessionScope::MfaEnrollment {
            return Err(to_gql_error(AppError::MfaEnrollmentRequired(
                "MFA enrollment must be completed before accessing Scryer".into(),
            )));
        }

        match current_user_from_ctx(ctx) {
            Some(user) => {
                let app = app_from_ctx(ctx)?;
                let effective_authorization = user.authorization.clone();
                let mut user = app
                    .load_user_for_auth_payload(&user)
                    .await
                    .map_err(to_gql_error)?;
                if auth_context.oauth_authorization_source == OAuthAuthorizationSource::Authless {
                    user.username = "Anonymous".to_string();
                }
                user.authorization = effective_authorization;
                let auth_factor_status = app
                    .user_auth_factor_status(&user.id)
                    .await
                    .map_err(to_gql_error)?;
                Ok(Some(from_user_with_auth_factor_status(
                    user,
                    auth_factor_status,
                )))
            }
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
        title_id: Option<ID>,
        library_ids: Option<Vec<ID>>,
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
                    title_id: title_id.map(String::from),
                    library_ids: optional_ids_to_strings(library_ids).unwrap_or_default(),
                    title_search,
                    latest_decision_codes: latest_decision_codes.unwrap_or_default(),
                    limit,
                    offset,
                },
            )
            .await
            .map_err(to_gql_error)?;
        Ok(WantedItemsListPayload {
            items: items
                .into_iter()
                .map(from_wanted_item)
                .collect::<scryer_application::AppResult<Vec<_>>>()
                .map_err(to_gql_error)?,
            total,
        })
    }

    async fn cutoff_unmet_titles(
        &self,
        ctx: &Context<'_>,
        facet: Option<MediaFacetValue>,
        library_ids: Option<Vec<ID>>,
    ) -> GqlResult<Vec<CutoffUnmetItemPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let items = app
            .list_cutoff_unmet_titles(
                &actor,
                facet.map(MediaFacetValue::into_domain),
                optional_ids_to_strings(library_ids),
            )
            .await
            .map_err(to_gql_error)?;
        Ok(items.into_iter().map(from_cutoff_unmet_item).collect())
    }

    async fn title_acquisition_diagnostics(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
    ) -> GqlResult<TitleAcquisitionDiagnosticsPayload> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let diagnostics = app
            .title_acquisition_diagnostics(&actor, title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        Ok(from_title_acquisition_diagnostics(diagnostics).map_err(to_gql_error)?)
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
        script_id: ID,
        limit: Option<i32>,
    ) -> GqlResult<Vec<PostProcessingScriptRunPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let script_id = String::from(script_id);

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
            .map(|channel| {
                let fields = app.notification_provider_config_fields(channel.channel_type.as_str());
                crate::mappers::from_notification_channel_with_fields(channel, &fields)
            })
            .collect())
    }

    async fn notification_targets(
        &self,
        ctx: &Context<'_>,
    ) -> GqlResult<Vec<NotificationTargetPayload>> {
        let app = app_from_ctx(ctx)?;
        let actor = actor_from_ctx(ctx)?;
        let targets = app
            .list_notification_targets(&actor)
            .await
            .map_err(to_gql_error)?;
        Ok(targets
            .into_iter()
            .map(crate::mappers::from_notification_target)
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
        require_library_settings_permission(ctx).await?;
        let read_dir = browse_path_read_dir(&path).map_err(to_gql_error)?;
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
            generated_at: Utc::now(),
            lines,
            count,
        })
    }

    /// List external subtitles for a title.
    async fn external_subtitles(
        &self,
        ctx: &Context<'_>,
        title_id: ID,
    ) -> GqlResult<Vec<ExternalSubtitlePayload>> {
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        let downloads = app
            .list_external_subtitles_for_title(&actor, title_id.as_ref())
            .await
            .map_err(to_gql_error)?;
        downloads
            .into_iter()
            .map(|d| {
                Ok(ExternalSubtitlePayload {
                    id: d.id.into(),
                    media_file_id: d.media_file_id.into(),
                    title_id: d.title_id.into(),
                    episode_id: d.episode_id.map(Into::into),
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
                    downloaded_at: parse_required_datetime(
                        &d.downloaded_at,
                        "external subtitle downloaded_at",
                    )
                    .map_err(to_gql_error)?,
                })
            })
            .collect::<GqlResult<Vec<_>>>()
    }

    /// List external subtitle blocklist entries for a specific media file.
    async fn external_subtitle_blocklist_entries(
        &self,
        ctx: &Context<'_>,
        media_file_id: ID,
    ) -> GqlResult<Vec<ExternalSubtitleBlocklistEntryPayload>> {
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;
        let media_file_id = String::from(media_file_id);
        let entries = app
            .list_external_subtitle_blocklist_for_media_file(&actor, &media_file_id)
            .await
            .map_err(to_gql_error)?;
        entries
            .into_iter()
            .map(|entry| {
                Ok(ExternalSubtitleBlocklistEntryPayload {
                    id: entry.id.into(),
                    media_file_id: entry.media_file_id.into(),
                    provider: entry.provider,
                    provider_file_id: entry.provider_file_id,
                    language: entry.language,
                    reason: entry.reason,
                    created_at: parse_required_datetime(
                        &entry.created_at,
                        "external subtitle blocklist created_at",
                    )?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()
            .map_err(to_gql_error)
    }
}
