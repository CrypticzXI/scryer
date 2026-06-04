use std::collections::{HashMap, HashSet};

use async_graphql::{Context, Object, Result as GqlResult};
use chrono::Utc;
use scryer_application::external_import::{
    self, ArrDownloadClient, ArrEpisode, ArrIndexer, ArrMovie, ArrSeries, DetectedProwlarrIndexer,
    ExternalArrClient,
};
use scryer_application::{
    AppError, ExternalIdHint, ExternalIdProvider, ExternalImportLibraryPathsSelection,
    ExternalImportMonitorEpisodeEntry, ExternalImportMonitorMovieEntry,
    ExternalImportMonitorSeasonEntry, ExternalImportMonitorSeriesEntry,
    ExternalImportMonitorSnapshotChunk, ExternalImportMonitorSnapshotEntryKind,
    ExternalImportMonitorWarmupPhase, ExternalImportMonitorWarmupProgressSnapshot,
    ExternalImportMonitorWarmupStatus, IndexerConfigUpdate, LibraryScanHint, LibraryScanHintFacet,
    LibraryScanHintSet, LibraryScanHintSource, library_scan_file_leaf_key,
    library_scan_folder_leaf_key,
};
use scryer_domain::{AppPermission, MediaFacet, NewDownloadClientConfig, NewIndexerConfig};
use serde::Serialize;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::context::{actor_from_ctx, app_from_ctx, require_app_permission};
use crate::mappers::from_external_import_monitor_warmup_progress;
use crate::types::*;

#[derive(Default)]
pub(crate) struct ExternalImportMutations;

const SONARR_EPISODE_FETCH_CONCURRENCY: usize = 32;
const SNAPSHOT_CHUNK_FLUSH_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct ExternalImportWarmupConnections {
    sonarr: Option<ExternalImportConnectionInput>,
    radarr: Option<ExternalImportConnectionInput>,
}

struct SnapshotChunkWriter {
    app: scryer_application::AppUseCase,
    actor: scryer_domain::User,
    facet: MediaFacet,
    entry_kind: ExternalImportMonitorSnapshotEntryKind,
    chunk_index: i32,
    buffered_ndjson: String,
}

impl SnapshotChunkWriter {
    fn new(
        app: scryer_application::AppUseCase,
        actor: scryer_domain::User,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
    ) -> Self {
        Self {
            app,
            actor,
            facet,
            entry_kind,
            chunk_index: 0,
            buffered_ndjson: String::new(),
        }
    }

    async fn push<T: Serialize>(&mut self, value: &T) -> scryer_application::AppResult<()> {
        let line = serde_json::to_string(value).map_err(|err| {
            AppError::Repository(format!("failed to serialize snapshot entry: {err}"))
        })?;
        self.buffered_ndjson.push_str(&line);
        self.buffered_ndjson.push('\n');

        if self.buffered_ndjson.len() >= SNAPSHOT_CHUNK_FLUSH_BYTES {
            self.flush().await?;
        }

        Ok(())
    }

    async fn flush(&mut self) -> scryer_application::AppResult<()> {
        if self.buffered_ndjson.is_empty() {
            return Ok(());
        }

        let payload_ndjson = std::mem::take(&mut self.buffered_ndjson);
        let chunk = ExternalImportMonitorSnapshotChunk {
            facet: self.facet.clone(),
            entry_kind: self.entry_kind.clone(),
            chunk_index: self.chunk_index,
            payload_ndjson,
            created_at: Utc::now().to_rfc3339(),
        };
        self.app
            .append_external_import_monitor_snapshot_chunk(&self.actor, chunk)
            .await?;

        self.chunk_index += 1;
        Ok(())
    }

    async fn finish(&mut self) -> scryer_application::AppResult<()> {
        self.flush().await?;
        Ok(())
    }
}

async fn clear_external_import_monitor_apply_target(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    facet: MediaFacet,
) -> scryer_application::AppResult<()> {
    app.clear_external_import_monitor_snapshot_chunks(actor, facet)
        .await
}

async fn clear_external_import_monitor_apply_targets(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
) -> scryer_application::AppResult<()> {
    for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
        clear_external_import_monitor_apply_target(app, actor, facet).await?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ProwlarrImportGroup {
    base_url: String,
    sources: Vec<String>,
    child_names: Vec<String>,
    api_key: Option<String>,
    api_key_conflict: bool,
}

impl ProwlarrImportGroup {
    fn new(detected: DetectedProwlarrIndexer, source: &str) -> Self {
        let mut group = Self {
            base_url: detected.base_url.clone(),
            sources: Vec::new(),
            child_names: Vec::new(),
            api_key: None,
            api_key_conflict: false,
        };
        group.merge(detected, source);
        group
    }

    fn merge(&mut self, detected: DetectedProwlarrIndexer, source: &str) {
        push_unique(&mut self.sources, source.to_string());
        push_unique(&mut self.child_names, detected.child_name);
        if let Some(api_key) = detected.api_key {
            match self.api_key.as_deref() {
                Some(existing) if existing != api_key => {
                    self.api_key = None;
                    self.api_key_conflict = true;
                }
                None if !self.api_key_conflict => {
                    self.api_key = Some(api_key);
                }
                _ => {}
            }
        }
    }

    fn requires_api_key_override(&self) -> bool {
        self.api_key_conflict || self.api_key.is_none()
    }

    fn dedup_key(&self) -> String {
        prowlarr_dedup_key(&self.base_url)
    }

    fn to_payload(&self) -> ExternalImportIndexerPayload {
        ExternalImportIndexerPayload {
            sources: self.sources.clone(),
            name: prowlarr_display_name(&self.base_url),
            implementation: "Prowlarr".to_string(),
            scryer_provider_type: Some("prowlarr".to_string()),
            base_url: Some(self.base_url.clone()),
            api_key: self.api_key.clone(),
            dedup_key: self.dedup_key(),
            supported: true,
            child_count: i32::try_from(self.child_names.len()).unwrap_or(i32::MAX),
            child_names: self.child_names.clone(),
            requires_api_key_override: self.requires_api_key_override(),
            api_key_help_url: prowlarr_api_key_help_url(&self.base_url),
        }
    }
}

fn merge_direct_prowlarr_group(
    groups: &mut HashMap<String, ProwlarrImportGroup>,
    base_url: &str,
    api_key: &str,
    child_names: &[String],
) {
    let normalized_base_url = base_url.trim().trim_end_matches('/').to_string();
    let dedup_key = prowlarr_dedup_key(&normalized_base_url);
    let group = groups
        .entry(dedup_key)
        .or_insert_with(|| ProwlarrImportGroup {
            base_url: normalized_base_url.clone(),
            sources: Vec::new(),
            child_names: Vec::new(),
            api_key: None,
            api_key_conflict: false,
        });

    push_unique(&mut group.sources, "prowlarr".to_string());
    for child_name in child_names {
        push_unique(&mut group.child_names, child_name.clone());
    }
    group.api_key_conflict = false;
    group.api_key = Some(api_key.trim().to_string());
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}

fn prowlarr_dedup_key(base_url: &str) -> String {
    format!("prowlarr:{}", base_url.trim().trim_end_matches('/'))
}

fn prowlarr_display_name(base_url: &str) -> String {
    let host = url::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| base_url.trim().trim_end_matches('/').to_string());
    format!("Prowlarr ({host})")
}

fn prowlarr_api_key_help_url(base_url: &str) -> Option<String> {
    let normalized = base_url.trim().trim_end_matches('/');
    url::Url::parse(normalized).ok()?;
    Some(format!("{normalized}/settings/general"))
}

fn prowlarr_parent_config_json(base_url: &str, api_key: &str) -> String {
    serde_json::json!({
        "base_url": base_url.trim().trim_end_matches('/'),
        "api_key": api_key.trim(),
    })
    .to_string()
}

fn indexer_config_base_url(config_json: Option<&str>) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(config_json?).ok()?;
    value
        .get("base_url")
        .or_else(|| value.get("baseUrl"))?
        .as_str()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

fn merge_prowlarr_group(
    groups: &mut HashMap<String, ProwlarrImportGroup>,
    detected: DetectedProwlarrIndexer,
    source: &str,
) {
    let dedup_key = prowlarr_dedup_key(&detected.base_url);
    if let Some(group) = groups.get_mut(&dedup_key) {
        group.merge(detected, source);
    } else {
        groups.insert(dedup_key, ProwlarrImportGroup::new(detected, source));
    }
}

fn detect_imported_prowlarr_proxy_indexer(
    indexer: &ArrIndexer,
    linked_prowlarr_base_url: Option<&str>,
) -> Option<DetectedProwlarrIndexer> {
    if let Some(linked_prowlarr_base_url) = linked_prowlarr_base_url {
        external_import::detect_linked_prowlarr_proxy_indexer(indexer, linked_prowlarr_base_url)
    } else {
        external_import::detect_prowlarr_proxy_indexer(indexer)
    }
}

fn version_from_validation_result(
    result: &scryer_application::IndexerValidationResult,
) -> Option<String> {
    let message = result.message.as_deref()?.trim();
    message
        .strip_prefix("Connected to Prowlarr ")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn same_base_url(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

fn imported_indexer_config_json(
    fields: &[scryer_domain::ConfigFieldDef],
    base_url: &str,
    api_key: Option<&str>,
    api_path: Option<&str>,
) -> String {
    let mut object = serde_json::Map::new();
    if let Some(connection_field) = fields
        .iter()
        .find(|field| field.role == Some(scryer_domain::ConfigFieldRole::ConnectionUrl))
        && !base_url.trim().is_empty()
    {
        object.insert(
            connection_field.key.clone(),
            serde_json::Value::String(base_url.trim().to_string()),
        );
    }
    if let Some(api_key) = api_key.map(str::trim).filter(|value| !value.is_empty())
        && let Some(api_key_field) = fields.iter().find(|field| {
            field.key == "api_key"
                || (field.field_type == scryer_domain::ConfigFieldType::Password
                    && field.key.to_ascii_lowercase().contains("api"))
        })
    {
        object.insert(
            api_key_field.key.clone(),
            serde_json::Value::String(api_key.to_string()),
        );
    }
    if let Some(api_path) = api_path.map(str::trim).filter(|value| !value.is_empty())
        && let Some(api_path_field) = fields.iter().find(|field| field.key == "api_path")
    {
        object.insert(
            api_path_field.key.clone(),
            serde_json::Value::String(api_path.to_string()),
        );
    }

    serde_json::Value::Object(object).to_string()
}

#[Object]
impl ExternalImportMutations {
    /// Connect to Sonarr and/or Radarr, fetch their configs, return a preview.
    async fn preview_external_import(
        &self,
        ctx: &Context<'_>,
        input: PreviewExternalImportInput,
    ) -> GqlResult<ExternalImportPreviewPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;

        if input.sonarr.is_none() && input.radarr.is_none() && input.prowlarr.is_none() {
            return Err(async_graphql::Error::new(
                "at least one of sonarr, radarr, or prowlarr must be provided",
            ));
        }

        let mut payload = ExternalImportPreviewPayload {
            sonarr_connected: false,
            radarr_connected: false,
            prowlarr_connected: false,
            sonarr_version: None,
            radarr_version: None,
            prowlarr_version: None,
            sonarr_error: None,
            radarr_error: None,
            prowlarr_error: None,
            root_folders: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        };

        // Map from dedup_key → index in payload vecs, so duplicates merge sources.
        let mut dc_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut idx_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut prowlarr_groups: HashMap<String, ProwlarrImportGroup> = HashMap::new();
        let linked_prowlarr_base_url = input.prowlarr.as_ref().map(|conn| conn.base_url.as_str());

        if let Some(conn) = &input.prowlarr {
            let config_json = prowlarr_parent_config_json(&conn.base_url, &conn.api_key);
            match app
                .preview_managed_indexer_children(&actor, "prowlarr", Some(&config_json))
                .await
            {
                Ok((validation, plan)) => {
                    payload.prowlarr_connected = true;
                    payload.prowlarr_version = version_from_validation_result(&validation);
                    let child_names = plan
                        .children
                        .into_iter()
                        .map(|child| child.name.trim().to_string())
                        .filter(|name| !name.is_empty())
                        .collect::<Vec<_>>();
                    merge_direct_prowlarr_group(
                        &mut prowlarr_groups,
                        &conn.base_url,
                        &conn.api_key,
                        &child_names,
                    );
                }
                Err(error) => {
                    payload.prowlarr_error = Some(error.to_string());
                }
            }
        }

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = if source == "sonarr" {
                ExternalArrClient::for_sonarr_v4(conn.base_url.clone(), conn.api_key.clone())
            } else {
                ExternalArrClient::for_radarr_v6(conn.base_url.clone(), conn.api_key.clone())
            };
            match client.test_connection().await {
                Ok((_app_name, version)) => {
                    if source == "sonarr" {
                        payload.sonarr_connected = true;
                        payload.sonarr_version = Some(version);
                    } else {
                        payload.radarr_connected = true;
                        payload.radarr_version = Some(version);
                    }

                    if let Ok(folders) = client.list_root_folders().await {
                        for folder in folders {
                            payload.root_folders.push(ExternalImportRootFolderPayload {
                                source: source.to_string(),
                                path: folder.path,
                            });
                        }
                    }

                    if let Ok(clients) = client.list_download_clients().await {
                        for dc in clients {
                            let mapped = map_download_client(&dc, source);
                            if let Some(&existing) = dc_key_idx.get(&mapped.dedup_key) {
                                payload.download_clients[existing]
                                    .sources
                                    .push(source.to_string());
                            } else {
                                dc_key_idx.insert(
                                    mapped.dedup_key.clone(),
                                    payload.download_clients.len(),
                                );
                                payload.download_clients.push(mapped);
                            }
                        }
                    }

                    if let Ok(indexers) = client.list_indexers().await {
                        for idx in indexers {
                            if external_import::should_skip_imported_indexer(&idx) {
                                continue;
                            }
                            if let Some(detected) = detect_imported_prowlarr_proxy_indexer(
                                &idx,
                                linked_prowlarr_base_url,
                            ) {
                                merge_prowlarr_group(&mut prowlarr_groups, detected, source);
                                continue;
                            }

                            let mapped = map_indexer(&idx, source);
                            if let Some(&existing) = idx_key_idx.get(&mapped.dedup_key) {
                                push_unique(
                                    &mut payload.indexers[existing].sources,
                                    source.to_string(),
                                );
                            } else {
                                idx_key_idx
                                    .insert(mapped.dedup_key.clone(), payload.indexers.len());
                                payload.indexers.push(mapped);
                            }
                        }
                    }
                }
                Err(error) => {
                    if source == "sonarr" {
                        payload.sonarr_error = Some(error.to_string());
                    } else {
                        payload.radarr_error = Some(error.to_string());
                    }
                }
            }
        }

        let mut prowlarr_payloads = prowlarr_groups
            .into_values()
            .map(|group| group.to_payload())
            .collect::<Vec<_>>();
        prowlarr_payloads.sort_by(|left, right| left.dedup_key.cmp(&right.dedup_key));
        payload.indexers.extend(prowlarr_payloads);

        Ok(payload)
    }

    async fn start_external_import_monitor_warmup(
        &self,
        ctx: &Context<'_>,
        input: StartExternalImportMonitorWarmupInput,
    ) -> GqlResult<ExternalImportMonitorWarmupProgressPayload> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;

        let fingerprint =
            external_import_connection_fingerprint(input.sonarr.as_ref(), input.radarr.as_ref());
        let begin = app
            .begin_external_import_monitor_warmup(&actor, &fingerprint)
            .await?;

        if begin.created {
            let session_id = begin.snapshot.session_id.clone();
            let connections = ExternalImportWarmupConnections {
                sonarr: input.sonarr,
                radarr: input.radarr,
            };
            let app_for_task = app.clone();
            let actor_for_task = actor.clone();
            let snapshot_for_task = begin.snapshot.clone();
            tokio::spawn(async move {
                run_external_import_monitor_warmup_job(
                    app_for_task,
                    actor_for_task,
                    session_id,
                    connections,
                    begin.cancel_token,
                    snapshot_for_task,
                )
                .await;
            });
        }

        Ok(from_external_import_monitor_warmup_progress(begin.snapshot))
    }

    async fn cancel_external_import_monitor_warmup(
        &self,
        ctx: &Context<'_>,
        input: CancelExternalImportMonitorWarmupInput,
    ) -> GqlResult<bool> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;

        let canceled = app
            .cancel_external_import_monitor_warmup(&actor, &input.session_id)
            .await?;
        if canceled {
            let _ = clear_external_import_monitor_apply_targets(&app, &actor).await;
        }

        Ok(canceled)
    }

    async fn finalize_external_import(
        &self,
        ctx: &Context<'_>,
        input: FinalizeExternalImportInput,
    ) -> GqlResult<bool> {
        require_app_permission(ctx, AppPermission::ManageCatalogSettings).await?;
        let actor = actor_from_ctx(ctx)?;
        let app = app_from_ctx(ctx)?;

        let connections = ExternalImportWarmupConnections {
            sonarr: input.sonarr.clone(),
            radarr: input.radarr.clone(),
        };
        let _session_id = ensure_external_import_monitor_warmup_completed(
            &app,
            &actor,
            connections,
            input.monitor_warmup_session_id.as_deref(),
        )
        .await?;

        if input.radarr.is_none() || input.selected_movies_paths.is_empty() {
            clear_external_import_monitor_apply_target(&app, &actor, MediaFacet::Movie).await?;
        }

        if input.sonarr.is_none() || input.selected_series_paths.is_empty() {
            clear_external_import_monitor_apply_target(&app, &actor, MediaFacet::Series).await?;
        }

        if input.sonarr.is_none() || input.selected_anime_paths.is_empty() {
            clear_external_import_monitor_apply_target(&app, &actor, MediaFacet::Anime).await?;
        }

        Ok(true)
    }

    /// Re-connect to Sonarr/Radarr, fetch configs, and create selected items in Scryer.
    async fn execute_external_import(
        &self,
        ctx: &Context<'_>,
        input: ExecuteExternalImportInput,
    ) -> GqlResult<ExternalImportResultPayload> {
        let actor = actor_from_ctx(ctx)?;

        let app = app_from_ctx(ctx)?;

        let selected_dc_keys: HashSet<String> = input
            .selected_download_client_dedup_keys
            .into_iter()
            .collect();
        let selected_idx_keys: HashSet<String> =
            input.selected_indexer_dedup_keys.into_iter().collect();
        let dc_api_key_overrides: HashMap<String, String> = input
            .download_client_api_key_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.api_key))
            .collect();
        let dc_password_overrides: HashMap<String, String> = input
            .download_client_password_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.password))
            .collect();
        let idx_api_key_overrides: HashMap<String, String> = input
            .indexer_api_key_overrides
            .into_iter()
            .map(|o| (o.dedup_key, o.api_key))
            .collect();

        let mut result = ExternalImportResultPayload {
            media_paths_saved: false,
            download_clients_created: 0,
            indexers_created: 0,
            plugins_installed: Vec::new(),
            errors: Vec::new(),
        };

        // ── Save media paths ──────────────────────────────────────────────
        match app
            .save_external_import_library_paths(
                &actor,
                ExternalImportLibraryPathsSelection {
                    movie_paths: input.selected_movies_paths.clone(),
                    series_paths: input.selected_series_paths.clone(),
                    anime_paths: input.selected_anime_paths.clone(),
                },
            )
            .await
        {
            Ok(saved) => result.media_paths_saved = saved,
            Err(err) => result
                .errors
                .push(format!("failed to save selected media paths: {err}")),
        }

        // ── Collect download clients + indexers from external apps ─────────
        let mut all_download_clients: Vec<(ArrDownloadClient, String)> = Vec::new();
        let mut all_indexers: Vec<(ArrIndexer, String)> = Vec::new();
        let mut seen_dc_keys: HashSet<String> = HashSet::new();
        let mut seen_idx_keys: HashSet<String> = HashSet::new();
        let mut prowlarr_groups: HashMap<String, ProwlarrImportGroup> = HashMap::new();
        let linked_prowlarr_base_url = input.prowlarr.as_ref().map(|conn| conn.base_url.as_str());

        if let Some(conn) = &input.prowlarr {
            let dedup_key = prowlarr_dedup_key(&conn.base_url);
            if selected_idx_keys.contains(&dedup_key) {
                merge_direct_prowlarr_group(
                    &mut prowlarr_groups,
                    &conn.base_url,
                    &conn.api_key,
                    &[],
                );
            }
        }

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = if source == "sonarr" {
                ExternalArrClient::for_sonarr_v4(conn.base_url.clone(), conn.api_key.clone())
            } else {
                ExternalArrClient::for_radarr_v6(conn.base_url.clone(), conn.api_key.clone())
            };

            if client.test_connection().await.is_err() {
                result.errors.push(format!("failed to connect to {source}"));
                continue;
            }

            if let Ok(clients) = client.list_download_clients().await {
                for dc in clients {
                    let mapped = map_download_client(&dc, source);
                    if mapped.supported
                        && seen_dc_keys.insert(mapped.dedup_key.clone())
                        && selected_dc_keys.contains(&mapped.dedup_key)
                    {
                        all_download_clients.push((dc, source.to_string()));
                    }
                }
            }

            if let Ok(indexers) = client.list_indexers().await {
                for idx in indexers {
                    if external_import::should_skip_imported_indexer(&idx) {
                        continue;
                    }

                    if let Some(detected) =
                        detect_imported_prowlarr_proxy_indexer(&idx, linked_prowlarr_base_url)
                    {
                        let dedup_key = prowlarr_dedup_key(&detected.base_url);
                        if selected_idx_keys.contains(&dedup_key) {
                            merge_prowlarr_group(&mut prowlarr_groups, detected, source);
                        }
                        continue;
                    }

                    let mapped = map_indexer(&idx, source);
                    if mapped.supported
                        && seen_idx_keys.insert(mapped.dedup_key.clone())
                        && selected_idx_keys.contains(&mapped.dedup_key)
                    {
                        all_indexers.push((idx, source.to_string()));
                    }
                }
            }
        }

        // ── Create download clients ───────────────────────────────────────
        for (dc, _source) in &all_download_clients {
            let Some(scryer_type) = external_import::map_download_client_type(&dc.implementation)
            else {
                continue;
            };

            let host = external_import::field_str(&dc.fields, "host").unwrap_or_default();
            let port = external_import::field_str_or_number(&dc.fields, "port").unwrap_or_default();
            let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
            let url_base = external_import::field_str(&dc.fields, "urlBase").unwrap_or_default();

            let mut config_obj = serde_json::Map::new();
            config_obj.insert("host".into(), serde_json::Value::String(host.clone()));
            config_obj.insert("port".into(), serde_json::Value::String(port.clone()));
            config_obj.insert("use_ssl".into(), serde_json::Value::Bool(use_ssl));
            config_obj.insert("url_base".into(), serde_json::Value::String(url_base));
            config_obj.insert(
                "client_type".into(),
                serde_json::Value::String(scryer_type.to_string()),
            );

            if scryer_type == "sabnzbd" || scryer_type == "weaver" {
                // Prefer a user-supplied override (needed when Sonarr/Radarr masked
                // the key), then fall back to the value fetched from the arr API.
                let dedup_key = format!("{}:{}:{}", scryer_type, host, port);
                let api_key = dc_api_key_overrides
                    .get(&dedup_key)
                    .cloned()
                    .or_else(|| external_import::field_str_sensitive(&dc.fields, "apiKey"));
                if let Some(api_key) = api_key {
                    config_obj.insert("api_key".into(), serde_json::Value::String(api_key));
                }
            } else {
                let dedup_key = format!("{}:{}:{}", scryer_type, host, port);
                if let Some(username) = external_import::field_str(&dc.fields, "username") {
                    config_obj.insert("username".into(), serde_json::Value::String(username));
                }
                let password = dc_password_overrides
                    .get(&dedup_key)
                    .cloned()
                    .or_else(|| external_import::field_str_sensitive(&dc.fields, "password"));
                if let Some(password) = password {
                    config_obj.insert("password".into(), serde_json::Value::String(password));
                }
            }

            let config_json = serde_json::Value::Object(config_obj).to_string();

            match app
                .create_download_client_config(
                    &actor,
                    NewDownloadClientConfig {
                        name: dc.name.clone(),
                        client_type: scryer_type.to_string(),
                        config_json,
                        client_priority: 0,
                        is_enabled: true,
                    },
                )
                .await
            {
                Ok(config) => {
                    result.download_clients_created += 1;
                    if scryer_type == "nzbget"
                        || scryer_type == "sabnzbd"
                        || scryer_type == "weaver"
                    {
                        let _ = app
                            .ensure_download_client_routing_entry_for_client(&actor, &config.id)
                            .await;
                    }
                }
                Err(err) => {
                    result.errors.push(format!(
                        "failed to create download client '{}': {err}",
                        dc.name
                    ));
                }
            }
        }

        // ── Create native Prowlarr parents and sync managed children ───────
        for group in prowlarr_groups.values() {
            let dedup_key = group.dedup_key();
            let override_api_key = idx_api_key_overrides
                .get(&dedup_key)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let api_key = override_api_key.or_else(|| {
                if group.api_key_conflict {
                    None
                } else {
                    group.api_key.clone()
                }
            });
            let Some(api_key) = api_key else {
                let help = prowlarr_api_key_help_url(&group.base_url)
                    .map(|url| format!(" ({url})"))
                    .unwrap_or_default();
                let reason = if group.api_key_conflict {
                    "visible API keys conflicted"
                } else {
                    "API key is missing or masked"
                };
                result.errors.push(format!(
                    "failed to import {}: {reason}; enter the Prowlarr API key from Prowlarr -> Settings -> General{help}",
                    prowlarr_display_name(&group.base_url)
                ));
                continue;
            };

            let name = prowlarr_display_name(&group.base_url);
            let config_json = prowlarr_parent_config_json(&group.base_url, &api_key);
            let existing_parents = match app
                .list_indexer_configs(&actor, Some("prowlarr".to_string()))
                .await
            {
                Ok(configs) => configs,
                Err(err) => {
                    result.errors.push(format!(
                        "failed to inspect existing Prowlarr configs for '{name}': {err}"
                    ));
                    continue;
                }
            };

            let existing_parent = existing_parents.into_iter().find(|config| {
                indexer_config_base_url(config.config_json.as_deref()).is_some_and(
                    |existing_base_url| same_base_url(&existing_base_url, &group.base_url),
                )
            });

            if let Some(existing_config) = existing_parent {
                match app
                    .update_indexer_config(
                        &actor,
                        IndexerConfigUpdate {
                            id: existing_config.id.clone(),
                            name: None,
                            provider_type: None,
                            derived_base_url: None,
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            is_enabled: Some(true),
                            enable_interactive_search: None,
                            enable_auto_search: None,
                            managed_parent_config_id: None,
                            managed_child_key: None,
                            managed_metadata_json: None,
                            caps_snapshot_json: None,
                            config_json: Some(config_json.clone()),
                        },
                    )
                    .await
                {
                    Ok(_) => {}
                    Err(err) => {
                        result
                            .errors
                            .push(format!("failed to update Prowlarr config '{name}': {err}"));
                        continue;
                    }
                }
            } else {
                match app
                    .create_indexer_config(
                        &actor,
                        NewIndexerConfig {
                            name: name.clone(),
                            provider_type: "prowlarr".to_string(),
                            rate_limit_seconds: None,
                            rate_limit_burst: None,
                            is_enabled: true,
                            enable_interactive_search: false,
                            enable_auto_search: false,
                            config_json: Some(config_json.clone()),
                        },
                    )
                    .await
                {
                    Ok(_config) => {
                        result.indexers_created += 1;
                    }
                    Err(err) => {
                        result
                            .errors
                            .push(format!("failed to create Prowlarr config '{name}': {err}"));
                        continue;
                    }
                }
            }
        }

        // ── Auto-install non-builtin plugins needed by selected indexers ──
        let available_providers: HashSet<String> = app
            .available_indexer_provider_types()
            .iter()
            .map(|(pt, _, _, _)| pt.clone())
            .collect();

        let mut auto_installed: HashSet<String> = HashSet::new();
        for (idx, _) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };
            if available_providers.contains(scryer_type) || auto_installed.contains(scryer_type) {
                continue;
            }
            // Plugin not loaded — try to install from registry
            let install_result = match app.install_plugin(&actor, scryer_type).await {
                Ok(inst) => Ok(inst),
                Err(_) => {
                    // Catalog might not be cached yet — refresh and retry
                    let _ = app.refresh_plugin_catalog_internal().await;
                    app.install_plugin(&actor, scryer_type).await
                }
            };
            match install_result {
                Ok(inst) => {
                    auto_installed.insert(scryer_type.to_string());
                    result.plugins_installed.push(inst.name);
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to install {} plugin: {err}", scryer_type));
                }
            }
        }

        // ── Create indexers ───────────────────────────────────────────────
        for (idx, _source) in &all_indexers {
            let Some(scryer_type) =
                external_import::map_indexer_provider_type(&idx.implementation, &idx.fields)
            else {
                continue;
            };

            let base_url = external_import::field_str(&idx.fields, "baseUrl").unwrap_or_default();
            let api_path = external_import::field_str(&idx.fields, "apiPath");
            let dedup_key = format!("{}:{}", scryer_type, base_url);
            let api_key = idx_api_key_overrides
                .get(&dedup_key)
                .cloned()
                .or_else(|| external_import::field_str_sensitive(&idx.fields, "apiKey"));
            let fields = match app.indexer_config_fields_for_provider_type(scryer_type) {
                Ok(fields) => fields,
                Err(_) => continue,
            };
            let config_json = imported_indexer_config_json(
                &fields,
                &base_url,
                api_key.as_deref(),
                api_path.as_deref(),
            );

            // If the plugin was just auto-installed, it may have auto-created a
            // default IndexerConfig. Update that config instead of creating a
            // duplicate. Once claimed, further indexers of the same type create
            // new configs normally.
            if auto_installed.remove(scryer_type) {
                let existing = app
                    .list_indexer_configs(&actor, Some(scryer_type.to_string()))
                    .await
                    .unwrap_or_default();
                if let Some(existing_config) = existing.first() {
                    if existing_config.config_json.as_deref() != Some(config_json.as_str()) {
                        let _ = app
                            .update_indexer_config(
                                &actor,
                                IndexerConfigUpdate {
                                    id: existing_config.id.clone(),
                                    name: Some(idx.name.clone()),
                                    provider_type: None,
                                    derived_base_url: None,
                                    rate_limit_seconds: None,
                                    rate_limit_burst: None,
                                    is_enabled: None,
                                    enable_interactive_search: None,
                                    enable_auto_search: None,
                                    managed_parent_config_id: None,
                                    managed_child_key: None,
                                    managed_metadata_json: None,
                                    caps_snapshot_json: None,
                                    config_json: Some(config_json.clone()),
                                },
                            )
                            .await;
                    }
                    result.indexers_created += 1;
                    continue;
                }
            }

            match app
                .create_indexer_config(
                    &actor,
                    NewIndexerConfig {
                        name: idx.name.clone(),
                        provider_type: scryer_type.to_string(),
                        rate_limit_seconds: None,
                        rate_limit_burst: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        config_json: Some(config_json),
                    },
                )
                .await
            {
                Ok(_) => {
                    result.indexers_created += 1;
                }
                Err(err) => {
                    result
                        .errors
                        .push(format!("failed to create indexer '{}': {err}", idx.name));
                }
            }
        }

        Ok(result)
    }
}

fn movie_scan_hint_from_arr(movie: &ArrMovie) -> Option<LibraryScanHint> {
    let path_key = library_scan_file_leaf_key(movie.file_path.as_deref()?)?;
    let mut ids = Vec::new();
    if let Some(tmdb_id) = movie
        .tmdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tmdb, value))
    {
        ids.push(tmdb_id);
    }
    if let Some(imdb_id) = movie
        .imdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Imdb, value))
    {
        ids.push(imdb_id);
    }

    (!ids.is_empty()).then_some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportRadarr,
        facet: LibraryScanHintFacet::Movie,
        path_key,
        ids,
    })
}

fn series_folder_scan_hint_from_arr(series: &ArrSeries) -> Option<LibraryScanHint> {
    let path_key = library_scan_folder_leaf_key(series.path.as_deref()?)?;
    let ids = series
        .tvdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tvdb, value))
        .map(|id| vec![id])?;

    Some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportSonarr,
        facet: LibraryScanHintFacet::Series,
        path_key,
        ids,
    })
}

fn series_episode_scan_hint_from_arr(
    series: &ArrSeries,
    episode: &ArrEpisode,
) -> Option<LibraryScanHint> {
    let path_key = library_scan_file_leaf_key(episode.file_path.as_deref()?)?;
    let ids = series
        .tvdb_id
        .as_deref()
        .and_then(|value| ExternalIdHint::normalized(ExternalIdProvider::Tvdb, value))
        .map(|id| vec![id])?;

    Some(LibraryScanHint {
        source: LibraryScanHintSource::ExternalImportSonarr,
        facet: LibraryScanHintFacet::Series,
        path_key,
        ids,
    })
}

fn movie_monitor_entry_from_arr(movie: &ArrMovie) -> ExternalImportMonitorMovieEntry {
    ExternalImportMonitorMovieEntry {
        tmdb_id: movie.tmdb_id.clone(),
        imdb_id: movie.imdb_id.clone(),
        path: movie.path.clone(),
        monitored: movie.monitored,
    }
}

fn series_monitor_entry_from_arr(
    series: ArrSeries,
    episodes: Vec<ArrEpisode>,
) -> ExternalImportMonitorSeriesEntry {
    let title_monitored = series.monitored;
    let season_defaults = series
        .seasons
        .iter()
        .map(|season| (season.season_number, season.monitored))
        .collect::<HashMap<_, _>>();

    ExternalImportMonitorSeriesEntry {
        tvdb_id: series.tvdb_id,
        path: series.path,
        monitored: title_monitored,
        seasons: series
            .seasons
            .into_iter()
            .filter(|season| season.monitored != title_monitored)
            .map(|season| ExternalImportMonitorSeasonEntry {
                season_number: season.season_number,
                monitored: season.monitored,
            })
            .collect(),
        episodes: episodes
            .into_iter()
            .filter(|episode| {
                let effective_default = season_defaults
                    .get(&episode.season_number)
                    .copied()
                    .unwrap_or(title_monitored);
                episode.monitored != effective_default
            })
            .map(|episode| ExternalImportMonitorEpisodeEntry {
                tvdb_id: episode.tvdb_id,
                season_number: episode.season_number,
                episode_number: episode.episode_number,
                monitored: episode.monitored,
            })
            .collect(),
    }
}

fn external_import_connection_fingerprint(
    sonarr: Option<&ExternalImportConnectionInput>,
    radarr: Option<&ExternalImportConnectionInput>,
) -> String {
    let normalize = |connection: &ExternalImportConnectionInput| {
        format!(
            "{}|{}",
            connection
                .base_url
                .trim()
                .trim_end_matches('/')
                .to_ascii_lowercase(),
            connection.api_key.trim(),
        )
    };

    [
        sonarr.map(|connection| format!("sonarr={}", normalize(connection))),
        radarr.map(|connection| format!("radarr={}", normalize(connection))),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(";")
}

fn should_publish_progress(count: i32) -> bool {
    count <= 10 || count % 25 == 0
}

fn recompute_warmup_overall_progress(snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot) {
    let components = [
        (
            snapshot.movies_total_known,
            snapshot.movies_progress.clone(),
        ),
        (
            snapshot.series_total_known,
            snapshot.series_progress.clone(),
        ),
        (
            snapshot.episode_fetch_total_known,
            snapshot.episode_fetch_progress.clone(),
        ),
        (
            snapshot.snapshot_build_total_known,
            snapshot.snapshot_build_progress.clone(),
        ),
    ];

    snapshot.overall_total_known = components.iter().all(|(known, _)| *known);
    snapshot.overall_progress.total = components.iter().map(|(_, progress)| progress.total).sum();
    snapshot.overall_progress.completed = components
        .iter()
        .map(|(_, progress)| progress.completed)
        .sum();
    snapshot.overall_progress.failed = components.iter().map(|(_, progress)| progress.failed).sum();
}

async fn publish_warmup_progress(
    app: &scryer_application::AppUseCase,
    session_id: &str,
    snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot,
) {
    recompute_warmup_overall_progress(snapshot);
    app.update_external_import_monitor_warmup_progress(session_id, snapshot.clone())
        .await;
}

async fn wait_for_external_import_monitor_warmup(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
) -> scryer_application::AppResult<ExternalImportMonitorWarmupProgressSnapshot> {
    let mut receiver = app
        .subscribe_external_import_monitor_warmup_progress(actor, session_id)
        .await?;

    loop {
        let snapshot = receiver.borrow().clone();
        if snapshot.status.is_terminal() {
            return Ok(snapshot);
        }

        receiver.changed().await.map_err(|err| {
            AppError::Repository(format!("warmup progress subscription closed: {err}"))
        })?;
    }
}

async fn capture_external_import_monitor_warmup(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
    connections: &ExternalImportWarmupConnections,
    cancel_token: &CancellationToken,
    snapshot: &mut ExternalImportMonitorWarmupProgressSnapshot,
) -> scryer_application::AppResult<()> {
    clear_external_import_monitor_apply_targets(app, actor).await?;

    let mut scan_hints = LibraryScanHintSet::new();
    let mut movie_writer = SnapshotChunkWriter::new(
        app.clone(),
        actor.clone(),
        MediaFacet::Movie,
        ExternalImportMonitorSnapshotEntryKind::Movie,
    );
    let mut series_writer = SnapshotChunkWriter::new(
        app.clone(),
        actor.clone(),
        MediaFacet::Series,
        ExternalImportMonitorSnapshotEntryKind::Series,
    );
    let mut anime_writer = SnapshotChunkWriter::new(
        app.clone(),
        actor.clone(),
        MediaFacet::Anime,
        ExternalImportMonitorSnapshotEntryKind::Series,
    );

    snapshot.status = ExternalImportMonitorWarmupStatus::Running;
    snapshot.movies_total_known = connections.radarr.is_none();
    snapshot.phase = if connections.radarr.is_some() {
        ExternalImportMonitorWarmupPhase::LoadingMovies
    } else if connections.sonarr.is_some() {
        ExternalImportMonitorWarmupPhase::LoadingSeries
    } else {
        ExternalImportMonitorWarmupPhase::BuildingSnapshot
    };
    publish_warmup_progress(app, session_id, snapshot).await;

    if let Some(radarr) = connections.radarr.as_ref() {
        let client =
            ExternalArrClient::for_radarr_v6(radarr.base_url.clone(), radarr.api_key.clone());
        let movies = client.list_movies().await?;
        let movie_total = i32::try_from(movies.len()).unwrap_or(i32::MAX);
        snapshot.movies_total_known = true;
        snapshot.movies_progress.total = movie_total;
        snapshot.snapshot_build_total_known = true;
        snapshot.snapshot_build_progress.total = movie_total;
        snapshot.matched_movie_count = movie_total;
        publish_warmup_progress(app, session_id, snapshot).await;

        for movie in movies {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            if let Some(hint) = movie_scan_hint_from_arr(&movie) {
                scan_hints.push(hint);
            }
            movie_writer
                .push(&movie_monitor_entry_from_arr(&movie))
                .await?;
            snapshot.movies_progress.completed =
                snapshot.movies_progress.completed.saturating_add(1);
            snapshot.snapshot_build_progress.completed =
                snapshot.snapshot_build_progress.completed.saturating_add(1);

            if should_publish_progress(snapshot.movies_progress.completed) {
                publish_warmup_progress(app, session_id, snapshot).await;
            }
        }
    } else {
        snapshot.movies_total_known = true;
    }

    if connections.sonarr.is_some() {
        snapshot.phase = ExternalImportMonitorWarmupPhase::LoadingSeries;
        publish_warmup_progress(app, session_id, snapshot).await;
    }

    if let Some(sonarr) = connections.sonarr.as_ref() {
        let client =
            ExternalArrClient::for_sonarr_v4(sonarr.base_url.clone(), sonarr.api_key.clone());
        let all_series = client.list_series().await?;
        let series_total = i32::try_from(all_series.len()).unwrap_or(i32::MAX);
        snapshot.series_total_known = true;
        snapshot.series_progress.total = series_total;
        snapshot.series_progress.completed = series_total;
        snapshot.matched_series_count = series_total;
        snapshot.snapshot_build_total_known = true;
        snapshot.snapshot_build_progress.total = snapshot
            .snapshot_build_progress
            .total
            .saturating_add(series_total);

        let all_totals_known = all_series
            .iter()
            .all(|series| series.statistics.total_episode_count.is_some());
        let expected_episode_total = all_series
            .iter()
            .filter_map(|series| series.statistics.total_episode_count)
            .fold(0_i32, |acc, value| acc.saturating_add(value));
        let expected_monitored_total = all_series
            .iter()
            .filter_map(|series| series.statistics.monitored_episode_count)
            .fold(0_i32, |acc, value| acc.saturating_add(value));

        snapshot.episode_fetch_total_known = all_totals_known;
        snapshot.episode_fetch_expected_total =
            (expected_episode_total > 0).then_some(expected_episode_total);
        snapshot.episode_fetch_expected_monitored_total =
            (expected_monitored_total > 0).then_some(expected_monitored_total);
        snapshot.episode_fetch_progress.total = expected_episode_total;
        publish_warmup_progress(app, session_id, snapshot).await;

        snapshot.phase = ExternalImportMonitorWarmupPhase::LoadingEpisodes;
        publish_warmup_progress(app, session_id, snapshot).await;

        let mut pending_series = all_series.into_iter();
        let mut join_set = JoinSet::new();

        let spawn_episode_fetch = |join_set: &mut JoinSet<(
            ArrSeries,
            scryer_application::AppResult<Vec<ArrEpisode>>,
        )>,
                                   client: &ExternalArrClient,
                                   series: ArrSeries| {
            let client = client.clone();
            join_set.spawn(async move {
                let series_path = series.path.clone();
                let result = client
                    .list_episodes_for_series(series.id, series_path.as_deref())
                    .await;
                (series, result)
            });
        };

        for _ in 0..SONARR_EPISODE_FETCH_CONCURRENCY {
            let Some(series) = pending_series.next() else {
                break;
            };
            spawn_episode_fetch(&mut join_set, &client, series);
        }

        while let Some(join_result) = join_set.join_next().await {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            let (series, episodes_result) = join_result.map_err(|err| {
                AppError::Repository(format!("failed to join Sonarr episode fetch task: {err}"))
            })?;
            let episodes = episodes_result?;
            let episode_count = i32::try_from(episodes.len()).unwrap_or(i32::MAX);
            if let Some(hint) = series_folder_scan_hint_from_arr(&series) {
                scan_hints.push(hint);
            }
            for episode in &episodes {
                if let Some(hint) = series_episode_scan_hint_from_arr(&series, episode) {
                    scan_hints.push(hint);
                }
            }
            let entry = series_monitor_entry_from_arr(series, episodes);
            series_writer.push(&entry).await?;
            anime_writer.push(&entry).await?;

            snapshot.episode_fetch_progress.completed = snapshot
                .episode_fetch_progress
                .completed
                .saturating_add(episode_count);
            snapshot.snapshot_build_progress.completed =
                snapshot.snapshot_build_progress.completed.saturating_add(1);

            if should_publish_progress(snapshot.snapshot_build_progress.completed)
                || should_publish_progress(snapshot.episode_fetch_progress.completed)
            {
                publish_warmup_progress(app, session_id, snapshot).await;
            }

            if let Some(next_series) = pending_series.next() {
                spawn_episode_fetch(&mut join_set, &client, next_series);
            }
        }
    } else {
        snapshot.series_total_known = true;
        snapshot.episode_fetch_total_known = true;
    }

    snapshot.phase = ExternalImportMonitorWarmupPhase::BuildingSnapshot;
    publish_warmup_progress(app, session_id, snapshot).await;

    movie_writer.finish().await?;
    series_writer.finish().await?;
    anime_writer.finish().await?;
    app.set_external_import_monitor_warmup_scan_hints(session_id, scan_hints)
        .await;
    snapshot.snapshot_build_progress.completed = snapshot.snapshot_build_progress.total;

    Ok(())
}

async fn run_external_import_monitor_warmup_job(
    app: scryer_application::AppUseCase,
    actor: scryer_domain::User,
    session_id: String,
    connections: ExternalImportWarmupConnections,
    cancel_token: CancellationToken,
    mut snapshot: ExternalImportMonitorWarmupProgressSnapshot,
) {
    let outcome = capture_external_import_monitor_warmup(
        &app,
        &actor,
        &session_id,
        &connections,
        &cancel_token,
        &mut snapshot,
    )
    .await;

    if cancel_token.is_cancelled() {
        let _ = clear_external_import_monitor_apply_targets(&app, &actor).await;
        snapshot.status = ExternalImportMonitorWarmupStatus::Canceled;
        snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
        snapshot.error_message = None;
        publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        return;
    }

    match outcome {
        Ok(()) => {
            snapshot.status = ExternalImportMonitorWarmupStatus::Completed;
            snapshot.phase = ExternalImportMonitorWarmupPhase::Ready;
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        }
        Err(err) => {
            let _ = clear_external_import_monitor_apply_targets(&app, &actor).await;
            snapshot.status = ExternalImportMonitorWarmupStatus::Failed;
            snapshot.error_message = Some(err.to_string());
            publish_warmup_progress(&app, &session_id, &mut snapshot).await;
        }
    }
}

async fn ensure_external_import_monitor_warmup_completed(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    connections: ExternalImportWarmupConnections,
    preferred_session_id: Option<&str>,
) -> scryer_application::AppResult<String> {
    let fingerprint = external_import_connection_fingerprint(
        connections.sonarr.as_ref(),
        connections.radarr.as_ref(),
    );

    if let Some(session_id) = preferred_session_id.filter(|session_id| !session_id.is_empty()) {
        let matches_fingerprint = match app
            .external_import_monitor_warmup_connection_fingerprint(actor, session_id)
            .await
        {
            Ok(existing_fingerprint) => existing_fingerprint == fingerprint,
            Err(AppError::NotFound(_)) => false,
            Err(err) => return Err(err),
        };

        if matches_fingerprint
            && let Some(completed_session_id) =
                try_complete_external_import_monitor_warmup_session(app, actor, session_id).await?
        {
            return Ok(completed_session_id);
        }
    }

    for _ in 0..2 {
        let begin = app
            .begin_external_import_monitor_warmup(actor, &fingerprint)
            .await?;
        let session_id = begin.snapshot.session_id.clone();

        if begin.created {
            run_external_import_monitor_warmup_job(
                app.clone(),
                actor.clone(),
                session_id.clone(),
                connections.clone(),
                begin.cancel_token,
                begin.snapshot.clone(),
            )
            .await;
        }

        if let Some(completed_session_id) =
            try_complete_external_import_monitor_warmup_session(app, actor, &session_id).await?
        {
            return Ok(completed_session_id);
        }
    }

    Err(AppError::Repository(
        "external import monitor warmup did not complete successfully".into(),
    ))
}

async fn try_complete_external_import_monitor_warmup_session(
    app: &scryer_application::AppUseCase,
    actor: &scryer_domain::User,
    session_id: &str,
) -> scryer_application::AppResult<Option<String>> {
    let claimed_snapshot = match app
        .claim_external_import_monitor_warmup(actor, session_id)
        .await
    {
        Ok(snapshot) => snapshot,
        Err(AppError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(err),
    };

    let completed_snapshot = if claimed_snapshot.status.is_terminal() {
        claimed_snapshot
    } else {
        wait_for_external_import_monitor_warmup(app, actor, session_id).await?
    };

    Ok(
        (completed_snapshot.status == ExternalImportMonitorWarmupStatus::Completed)
            .then(|| session_id.to_string()),
    )
}

fn map_download_client(
    dc: &ArrDownloadClient,
    source: &str,
) -> ExternalImportDownloadClientPayload {
    let scryer_type = external_import::map_download_client_type(&dc.implementation);
    let host = external_import::field_str(&dc.fields, "host");
    let port = external_import::field_str_or_number(&dc.fields, "port");
    let use_ssl = external_import::field_bool(&dc.fields, "useSsl").unwrap_or(false);
    let url_base = external_import::field_str(&dc.fields, "urlBase");
    let username = external_import::field_str(&dc.fields, "username");
    // Use field_str_sensitive so that Sonarr/Radarr's "********" mask becomes
    // None — callers can then detect that the key must be entered manually.
    let api_key = external_import::field_str_sensitive(&dc.fields, "apiKey");
    let password = external_import::field_str_sensitive(&dc.fields, "password");

    let dedup_key = format!(
        "{}:{}:{}",
        scryer_type.unwrap_or("unsupported"),
        host.as_deref().unwrap_or(""),
        port.as_deref().unwrap_or("")
    );

    ExternalImportDownloadClientPayload {
        sources: vec![source.to_string()],
        name: dc.name.clone(),
        implementation: dc.implementation.clone(),
        scryer_client_type: scryer_type.map(str::to_string),
        host,
        port,
        use_ssl,
        url_base,
        username,
        api_key,
        dedup_key,
        supported: scryer_type.is_some(),
        requires_password_override: password.is_none()
            && scryer_type.is_some_and(|client_type| client_type == "nzbget"),
    }
}

fn map_indexer(idx: &ArrIndexer, source: &str) -> ExternalImportIndexerPayload {
    let scryer_type = external_import::map_indexer_provider_type(&idx.implementation, &idx.fields);
    let base_url = external_import::field_str(&idx.fields, "baseUrl");
    let api_key = external_import::field_str_sensitive(&idx.fields, "apiKey");

    let dedup_key = format!(
        "{}:{}",
        scryer_type.unwrap_or("unsupported"),
        base_url.as_deref().unwrap_or("")
    );

    ExternalImportIndexerPayload {
        sources: vec![source.to_string()],
        name: idx.name.clone(),
        implementation: idx.implementation.clone(),
        scryer_provider_type: scryer_type.map(str::to_string),
        base_url,
        api_key,
        dedup_key,
        supported: scryer_type.is_some(),
        child_count: 0,
        child_names: Vec::new(),
        requires_api_key_override: false,
        api_key_help_url: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use scryer_application::external_import::{
        ArrDownloadClient, ArrEpisode, ArrIndexer, ArrMovie, ArrSeries, ArrSeriesStatistics,
    };
    use scryer_application::{
        ExternalIdProvider, LibraryScanHintFacet, LibraryScanHintSource,
        library_scan_file_leaf_key, library_scan_folder_leaf_key,
    };
    use scryer_domain::{ConfigFieldDef, ConfigFieldRole, ConfigFieldType, ConfigFieldValueSource};
    use serde_json::Value;

    use super::{
        detect_imported_prowlarr_proxy_indexer, imported_indexer_config_json, map_download_client,
        map_indexer, merge_direct_prowlarr_group, merge_prowlarr_group, movie_scan_hint_from_arr,
        prowlarr_dedup_key, series_episode_scan_hint_from_arr, series_folder_scan_hint_from_arr,
    };

    #[test]
    fn radarr_warmup_builds_movie_hint_with_tmdb_and_imdb() {
        let path = "/Movies/The Bourne Supremacy (2004)";
        let file_path = "/Movies/The Bourne Supremacy (2004)/The Bourne Supremacy.mkv";
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some(path.into()),
            file_path: Some(file_path.into()),
            tmdb_id: Some("2502".into()),
            imdb_id: Some("tt0372183".into()),
            monitored: true,
        })
        .expect("movie hint");

        assert_eq!(hint.source, LibraryScanHintSource::ExternalImportRadarr);
        assert_eq!(hint.facet, LibraryScanHintFacet::Movie);
        assert_eq!(
            hint.path_key,
            library_scan_file_leaf_key(file_path).unwrap()
        );
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "2502" })
        );
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Imdb && id.value == "tt0372183" })
        );
    }

    #[test]
    fn radarr_warmup_omits_numeric_only_imdb_hint() {
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some("/Movies/Children of Men (2006)".into()),
            file_path: Some("/Movies/Children of Men (2006)/Children of Men.mkv".into()),
            tmdb_id: Some("9693".into()),
            imdb_id: Some("9693".into()),
            monitored: true,
        })
        .expect("movie hint");

        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "9693" })
        );
        assert!(
            !hint
                .ids
                .iter()
                .any(|id| id.provider == ExternalIdProvider::Imdb)
        );
    }

    #[test]
    fn radarr_warmup_omits_malformed_imdb_hint() {
        let hint = movie_scan_hint_from_arr(&ArrMovie {
            id: 1,
            root_folder_path: "/Movies".into(),
            path: Some("/Movies/Children of Men (2006)".into()),
            file_path: Some("/Movies/Children of Men (2006)/Children of Men.mkv".into()),
            tmdb_id: Some("9693".into()),
            imdb_id: Some("tt0206634-extra".into()),
            monitored: true,
        })
        .expect("movie hint");

        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tmdb && id.value == "9693" })
        );
        assert!(
            !hint
                .ids
                .iter()
                .any(|id| id.provider == ExternalIdProvider::Imdb)
        );
    }

    #[test]
    fn sonarr_warmup_builds_series_hint_with_tvdb() {
        let path = "/Series/Foundation (2021)";
        let series = ArrSeries {
            id: 1,
            root_folder_path: "/Series".into(),
            path: Some(path.into()),
            tvdb_id: Some("366972".into()),
            monitored: true,
            seasons: Vec::new(),
            statistics: ArrSeriesStatistics {
                total_episode_count: None,
                monitored_episode_count: None,
            },
        };
        let hint = series_folder_scan_hint_from_arr(&series).expect("series hint");

        assert_eq!(hint.source, LibraryScanHintSource::ExternalImportSonarr);
        assert_eq!(hint.facet, LibraryScanHintFacet::Series);
        assert_eq!(hint.path_key, library_scan_folder_leaf_key(path).unwrap());
        assert!(
            hint.ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tvdb && id.value == "366972" })
        );

        let episode_path = "/Series/Foundation (2021)/Season 01/Foundation.S01E01.mkv";
        let episode_hint = series_episode_scan_hint_from_arr(
            &series,
            &ArrEpisode {
                id: 1,
                series_id: 1,
                tvdb_id: Some("777001".into()),
                season_number: 1,
                episode_number: 1,
                file_path: Some(episode_path.into()),
                monitored: true,
            },
        )
        .expect("episode hint");
        assert_eq!(
            episode_hint.path_key,
            library_scan_file_leaf_key(episode_path).unwrap()
        );
        assert!(
            episode_hint
                .ids
                .iter()
                .any(|id| { id.provider == ExternalIdProvider::Tvdb && id.value == "366972" })
        );
    }

    #[test]
    fn map_download_client_marks_qbittorrent_as_supported() {
        let payload = map_download_client(
            &ArrDownloadClient {
                id: 1,
                name: "qBittorrent".into(),
                implementation: "qBittorrent".into(),
                fields: HashMap::from([
                    ("host".into(), Value::String("qb.local".into())),
                    ("port".into(), Value::String("8080".into())),
                ]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_client_type.as_deref(), Some("qbittorrent"));
        assert_eq!(payload.dedup_key, "qbittorrent:qb.local:8080");
    }

    #[test]
    fn map_indexer_marks_sonarr_torznab_as_supported() {
        let payload = map_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Indexer".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://torznab.example".into()),
                )]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_provider_type.as_deref(), Some("torznab"));
        assert_eq!(payload.dedup_key, "torznab:https://torznab.example");
    }

    #[test]
    fn map_indexer_marks_sonarr_newznab_preset_as_generic_newznab() {
        let payload = map_indexer(
            &ArrIndexer {
                id: 1,
                name: "NZBGeek".into(),
                implementation: "Newznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://api.nzbgeek.info".into()),
                )]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_provider_type.as_deref(), Some("newznab"));
        assert_eq!(payload.dedup_key, "newznab:https://api.nzbgeek.info");
    }

    #[test]
    fn imported_indexer_config_keeps_base_url_and_api_path_separate() {
        let fields = vec![
            ConfigFieldDef {
                key: "base_url".into(),
                label: "Base URL".into(),
                field_type: ConfigFieldType::String,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: Some(ConfigFieldRole::ConnectionUrl),
                host_binding: None,
                options: vec![],
                help_text: None,
            },
            ConfigFieldDef {
                key: "api_key".into(),
                label: "API Key".into(),
                field_type: ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            },
            ConfigFieldDef {
                key: "api_path".into(),
                label: "API Path".into(),
                field_type: ConfigFieldType::String,
                required: false,
                default_value: Some("/api".into()),
                value_source: ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            },
        ];

        let config_json = imported_indexer_config_json(
            &fields,
            "https://indexer.example",
            Some("secret"),
            Some("/api/v1"),
        );
        let parsed: serde_json::Value =
            serde_json::from_str(&config_json).expect("config json should parse");

        assert_eq!(parsed["base_url"], "https://indexer.example");
        assert_eq!(parsed["api_key"], "secret");
        assert_eq!(parsed["api_path"], "/api/v1");
    }

    #[test]
    fn direct_prowlarr_merge_overrides_arr_key_conflicts_and_keeps_children() {
        let mut groups = HashMap::new();
        merge_prowlarr_group(
            &mut groups,
            scryer_application::external_import::DetectedProwlarrIndexer {
                base_url: "http://prowlarr.local".into(),
                api_key: Some("arr-key-a".into()),
                child_name: "Indexer A".into(),
            },
            "sonarr",
        );
        merge_prowlarr_group(
            &mut groups,
            scryer_application::external_import::DetectedProwlarrIndexer {
                base_url: "http://prowlarr.local".into(),
                api_key: Some("arr-key-b".into()),
                child_name: "Indexer B".into(),
            },
            "radarr",
        );

        merge_direct_prowlarr_group(
            &mut groups,
            "http://prowlarr.local",
            "direct-key",
            &["Indexer B".into(), "Indexer C".into()],
        );

        let group = groups
            .get(&prowlarr_dedup_key("http://prowlarr.local"))
            .expect("merged prowlarr group");
        assert_eq!(group.api_key.as_deref(), Some("direct-key"));
        assert!(!group.api_key_conflict);
        assert_eq!(group.sources, vec!["sonarr", "radarr", "prowlarr"]);
        assert_eq!(
            group.child_names,
            vec![
                "Indexer A".to_string(),
                "Indexer B".to_string(),
                "Indexer C".to_string()
            ]
        );
    }

    #[test]
    fn linked_prowlarr_proxy_detection_accepts_torznab_without_api_path() {
        let detected = detect_imported_prowlarr_proxy_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Child".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("http://prowlarr.local/12345".into()),
                )]),
            },
            Some("http://prowlarr.local"),
        )
        .expect("linked prowlarr proxy");

        assert_eq!(detected.base_url, "http://prowlarr.local");
        assert_eq!(detected.child_name, "Torrent Child");
    }

    #[test]
    fn direct_linked_prowlarr_detection_does_not_match_other_parents() {
        let detected = detect_imported_prowlarr_proxy_indexer(
            &ArrIndexer {
                id: 1,
                name: "Torrent Child".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([
                    (
                        "baseUrl".into(),
                        Value::String("http://other-prowlarr.local/12345".into()),
                    ),
                    ("apiPath".into(), Value::String("/api".into())),
                ]),
            },
            Some("http://prowlarr.local"),
        );

        assert!(detected.is_none());
    }
}
