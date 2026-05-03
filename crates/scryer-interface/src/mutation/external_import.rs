use std::collections::{HashMap, HashSet};

use async_graphql::{Context, Object, Result as GqlResult};
use scryer_application::{
    AppError, ExternalImportLibraryPathsSelection, ExternalImportMonitorEpisodeEntry,
    ExternalImportMonitorMovieEntry, ExternalImportMonitorSeasonEntry,
    ExternalImportMonitorSeriesEntry, ExternalImportMonitorSnapshotPayload, IndexerConfigUpdate,
};
use scryer_domain::{Entitlement, MediaFacet, NewDownloadClientConfig, NewIndexerConfig};
use scryer_infrastructure::external_import::{
    self, ArrDownloadClient, ArrEpisode, ArrIndexer, ArrMovie, ArrSeries, ExternalArrClient,
};
use tokio::task::JoinSet;

use crate::context::{actor_from_ctx, app_from_ctx};
use crate::types::*;

#[derive(Default)]
pub(crate) struct ExternalImportMutations;

const SONARR_EPISODE_FETCH_CONCURRENCY: usize = 8;

#[Object]
impl ExternalImportMutations {
    /// Connect to Sonarr and/or Radarr, fetch their configs, return a preview.
    async fn preview_external_import(
        &self,
        ctx: &Context<'_>,
        input: PreviewExternalImportInput,
    ) -> GqlResult<ExternalImportPreviewPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

        if input.sonarr.is_none() && input.radarr.is_none() {
            return Err(async_graphql::Error::new(
                "at least one of sonarr or radarr must be provided",
            ));
        }

        let mut payload = ExternalImportPreviewPayload {
            sonarr_connected: false,
            radarr_connected: false,
            sonarr_version: None,
            radarr_version: None,
            root_folders: Vec::new(),
            download_clients: Vec::new(),
            indexers: Vec::new(),
        };

        // Map from dedup_key → index in payload vecs, so duplicates merge sources.
        let mut dc_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut idx_key_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());
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
                            let mapped = map_indexer(&idx, source);
                            if let Some(&existing) = idx_key_idx.get(&mapped.dedup_key) {
                                payload.indexers[existing].sources.push(source.to_string());
                            } else {
                                idx_key_idx
                                    .insert(mapped.dedup_key.clone(), payload.indexers.len());
                                payload.indexers.push(mapped);
                            }
                        }
                    }
                }
                Err(_) => {
                    if source == "sonarr" {
                        payload.sonarr_connected = false;
                    } else {
                        payload.radarr_connected = false;
                    }
                }
            }
        }

        Ok(payload)
    }

    /// Re-connect to Sonarr/Radarr, fetch configs, and create selected items in Scryer.
    async fn execute_external_import(
        &self,
        ctx: &Context<'_>,
        input: ExecuteExternalImportInput,
    ) -> GqlResult<ExternalImportResultPayload> {
        let actor = actor_from_ctx(ctx)?;
        if !actor.has_entitlement(&Entitlement::ManageConfig) {
            return Err(async_graphql::Error::new("insufficient entitlements"));
        }

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

        for (conn_opt, source) in [(&input.sonarr, "sonarr"), (&input.radarr, "radarr")] {
            let Some(conn) = conn_opt else { continue };
            let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());

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
                if let Some(username) = external_import::field_str(&dc.fields, "username") {
                    config_obj.insert("username".into(), serde_json::Value::String(username));
                }
                if let Some(password) = external_import::field_str(&dc.fields, "password") {
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

            let mut base_url =
                external_import::field_str(&idx.fields, "baseUrl").unwrap_or_default();
            let api_path = external_import::field_str(&idx.fields, "apiPath");
            if let Some(path) = &api_path
                && !path.is_empty()
                && !base_url.is_empty()
            {
                base_url = format!(
                    "{}/{}",
                    base_url.trim_end_matches('/'),
                    path.trim_start_matches('/')
                );
            }

            let api_key = external_import::field_str_sensitive(&idx.fields, "apiKey");

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
                    if api_key.is_some() || existing_config.base_url != base_url {
                        let _ = app
                            .update_indexer_config(
                                &actor,
                                IndexerConfigUpdate {
                                    id: existing_config.id.clone(),
                                    name: Some(idx.name.clone()),
                                    provider_type: None,
                                    base_url: Some(base_url),
                                    api_key_encrypted: api_key,
                                    rate_limit_seconds: None,
                                    rate_limit_burst: None,
                                    is_enabled: None,
                                    enable_interactive_search: None,
                                    enable_auto_search: None,
                                    config_json: None,
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
                        base_url,
                        api_key_encrypted: api_key,
                        rate_limit_seconds: None,
                        rate_limit_burst: None,
                        is_enabled: true,
                        enable_interactive_search: true,
                        enable_auto_search: true,
                        config_json: None,
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

        match &input.radarr {
            Some(conn) if !input.selected_movies_paths.is_empty() => {
                let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());
                match capture_movie_monitor_snapshot(&client, &input.selected_movies_paths).await {
                    Ok(payload) => {
                        if let Err(err) = app
                            .save_external_import_monitor_snapshot(
                                &actor,
                                MediaFacet::Movie,
                                payload,
                            )
                            .await
                        {
                            result
                                .errors
                                .push(format!("failed to save movie monitoring snapshot: {err}"));
                        }
                    }
                    Err(err) => result.errors.push(format!(
                        "failed to capture movie monitoring snapshot: {err}"
                    )),
                }
            }
            _ => {
                if let Err(err) = app
                    .clear_external_import_monitor_snapshot(&actor, MediaFacet::Movie)
                    .await
                {
                    result
                        .errors
                        .push(format!("failed to clear movie monitoring snapshot: {err}"));
                }
            }
        }

        match &input.sonarr {
            Some(conn)
                if !input.selected_series_paths.is_empty()
                    || !input.selected_anime_paths.is_empty() =>
            {
                let client = ExternalArrClient::new(conn.base_url.clone(), conn.api_key.clone());
                match capture_sonarr_monitor_snapshots(
                    &client,
                    &input.selected_series_paths,
                    &input.selected_anime_paths,
                )
                .await
                {
                    Ok((series_payload, anime_payload)) => {
                        if !input.selected_series_paths.is_empty() {
                            if let Err(err) = app
                                .save_external_import_monitor_snapshot(
                                    &actor,
                                    MediaFacet::Series,
                                    series_payload,
                                )
                                .await
                            {
                                result.errors.push(format!(
                                    "failed to save series monitoring snapshot: {err}"
                                ));
                            }
                        } else if let Err(err) = app
                            .clear_external_import_monitor_snapshot(&actor, MediaFacet::Series)
                            .await
                        {
                            result
                                .errors
                                .push(format!("failed to clear series monitoring snapshot: {err}"));
                        }

                        if !input.selected_anime_paths.is_empty() {
                            if let Err(err) = app
                                .save_external_import_monitor_snapshot(
                                    &actor,
                                    MediaFacet::Anime,
                                    anime_payload,
                                )
                                .await
                            {
                                result.errors.push(format!(
                                    "failed to save anime monitoring snapshot: {err}"
                                ));
                            }
                        } else if let Err(err) = app
                            .clear_external_import_monitor_snapshot(&actor, MediaFacet::Anime)
                            .await
                        {
                            result
                                .errors
                                .push(format!("failed to clear anime monitoring snapshot: {err}"));
                        }
                    }
                    Err(err) => {
                        if !input.selected_series_paths.is_empty() {
                            result.errors.push(format!(
                                "failed to capture series monitoring snapshot: {err}"
                            ));
                        }
                        if !input.selected_anime_paths.is_empty() {
                            result.errors.push(format!(
                                "failed to capture anime monitoring snapshot: {err}"
                            ));
                        }
                    }
                }
            }
            _ => {
                if let Err(err) = app
                    .clear_external_import_monitor_snapshot(&actor, MediaFacet::Series)
                    .await
                {
                    result
                        .errors
                        .push(format!("failed to clear series monitoring snapshot: {err}"));
                }
                if let Err(err) = app
                    .clear_external_import_monitor_snapshot(&actor, MediaFacet::Anime)
                    .await
                {
                    result
                        .errors
                        .push(format!("failed to clear anime monitoring snapshot: {err}"));
                }
            }
        }

        Ok(result)
    }
}

fn normalize_import_root_path(path: &str) -> String {
    path.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn selected_root_paths(paths: &[String]) -> HashSet<String> {
    paths
        .iter()
        .map(|path| normalize_import_root_path(path))
        .filter(|path| !path.is_empty())
        .collect()
}

fn movie_monitor_entry_from_arr(movie: ArrMovie) -> ExternalImportMonitorMovieEntry {
    ExternalImportMonitorMovieEntry {
        root_path: movie.root_folder_path,
        tmdb_id: movie.tmdb_id,
        imdb_id: movie.imdb_id,
        monitored: movie.monitored,
    }
}

fn series_monitor_entry_from_arr(
    series: ArrSeries,
    episodes: Vec<scryer_infrastructure::external_import::ArrEpisode>,
) -> ExternalImportMonitorSeriesEntry {
    ExternalImportMonitorSeriesEntry {
        root_path: series.root_folder_path,
        tvdb_id: series.tvdb_id,
        monitored: series.monitored,
        seasons: series
            .seasons
            .into_iter()
            .map(|season| ExternalImportMonitorSeasonEntry {
                season_number: season.season_number,
                monitored: season.monitored,
            })
            .collect(),
        episodes: episodes
            .into_iter()
            .map(|episode| ExternalImportMonitorEpisodeEntry {
                tvdb_id: episode.tvdb_id,
                season_number: episode.season_number,
                episode_number: episode.episode_number,
                monitored: episode.monitored,
            })
            .collect(),
    }
}

async fn capture_movie_monitor_snapshot(
    client: &ExternalArrClient,
    selected_paths: &[String],
) -> scryer_application::AppResult<ExternalImportMonitorSnapshotPayload> {
    let selected_paths = selected_root_paths(selected_paths);
    if selected_paths.is_empty() {
        return Ok(ExternalImportMonitorSnapshotPayload::Movie { entries: vec![] });
    }

    let entries = client
        .list_movies()
        .await?
        .into_iter()
        .filter(|movie| {
            selected_paths.contains(&normalize_import_root_path(&movie.root_folder_path))
        })
        .map(movie_monitor_entry_from_arr)
        .collect();

    Ok(ExternalImportMonitorSnapshotPayload::Movie { entries })
}

fn capture_series_monitor_snapshot(
    series_entries: Vec<ExternalImportMonitorSeriesEntry>,
) -> scryer_application::AppResult<ExternalImportMonitorSnapshotPayload> {
    Ok(ExternalImportMonitorSnapshotPayload::Series {
        entries: series_entries,
    })
}

fn filter_sonarr_series_for_selected_paths(
    all_series: &[ArrSeries],
    selected_paths: &HashSet<String>,
) -> Vec<ArrSeries> {
    all_series
        .iter()
        .filter(|series| {
            selected_paths.contains(&normalize_import_root_path(&series.root_folder_path))
        })
        .cloned()
        .collect()
}

async fn fetch_sonarr_episodes_for_series(
    client: &ExternalArrClient,
    series: &[ArrSeries],
) -> scryer_application::AppResult<HashMap<i64, Vec<ArrEpisode>>> {
    let mut episodes_by_series = HashMap::new();
    let mut pending_series = series.iter().cloned();
    let mut join_set = JoinSet::new();

    let spawn_episode_fetch =
        |join_set: &mut JoinSet<(i64, scryer_application::AppResult<Vec<ArrEpisode>>)>,
         client: &ExternalArrClient,
         series: ArrSeries| {
            let client = client.clone();
            join_set.spawn(async move {
                let series_id = series.id;
                let episodes = client.list_episodes_for_series(series_id).await;
                (series_id, episodes)
            });
        };

    for _ in 0..SONARR_EPISODE_FETCH_CONCURRENCY {
        let Some(series) = pending_series.next() else {
            break;
        };
        spawn_episode_fetch(&mut join_set, client, series);
    }

    while let Some(join_result) = join_set.join_next().await {
        let (series_id, episodes_result) = join_result.map_err(|err| {
            AppError::Repository(format!("failed to join Sonarr episode fetch task: {err}"))
        })?;
        episodes_by_series.insert(series_id, episodes_result?);

        if let Some(series) = pending_series.next() {
            spawn_episode_fetch(&mut join_set, client, series);
        }
    }

    Ok(episodes_by_series)
}

fn series_entries_from_snapshot_data(
    series: Vec<ArrSeries>,
    episodes_by_series: &HashMap<i64, Vec<ArrEpisode>>,
) -> Vec<ExternalImportMonitorSeriesEntry> {
    series
        .into_iter()
        .map(|series| {
            let episodes = episodes_by_series
                .get(&series.id)
                .cloned()
                .unwrap_or_default();
            series_monitor_entry_from_arr(series, episodes)
        })
        .collect()
}

async fn capture_sonarr_monitor_snapshots(
    client: &ExternalArrClient,
    selected_series_paths: &[String],
    selected_anime_paths: &[String],
) -> scryer_application::AppResult<(
    ExternalImportMonitorSnapshotPayload,
    ExternalImportMonitorSnapshotPayload,
)> {
    let selected_series_paths = selected_root_paths(selected_series_paths);
    let selected_anime_paths = selected_root_paths(selected_anime_paths);

    if selected_series_paths.is_empty() && selected_anime_paths.is_empty() {
        return Ok((
            ExternalImportMonitorSnapshotPayload::Series { entries: vec![] },
            ExternalImportMonitorSnapshotPayload::Series { entries: vec![] },
        ));
    }

    let all_series = client.list_series().await?;
    let selected_series =
        filter_sonarr_series_for_selected_paths(&all_series, &selected_series_paths);
    let selected_anime =
        filter_sonarr_series_for_selected_paths(&all_series, &selected_anime_paths);

    let mut unique_matched_series = HashMap::new();
    for series in selected_series.iter().chain(selected_anime.iter()) {
        unique_matched_series
            .entry(series.id)
            .or_insert_with(|| series.clone());
    }

    let matched_series = unique_matched_series.into_values().collect::<Vec<_>>();
    let episodes_by_series = fetch_sonarr_episodes_for_series(client, &matched_series).await?;

    let series_payload = capture_series_monitor_snapshot(series_entries_from_snapshot_data(
        selected_series,
        &episodes_by_series,
    ))?;
    let anime_payload = capture_series_monitor_snapshot(series_entries_from_snapshot_data(
        selected_anime,
        &episodes_by_series,
    ))?;

    Ok((series_payload, anime_payload))
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
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use scryer_infrastructure::external_import::{ArrDownloadClient, ArrIndexer};
    use serde_json::Value;

    use super::{map_download_client, map_indexer};

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
    fn map_indexer_marks_sonarr_animetosho_as_supported() {
        let payload = map_indexer(
            &ArrIndexer {
                id: 1,
                name: "AnimeTosho".into(),
                implementation: "Torznab".into(),
                fields: HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://feed.animetosho.org".into()),
                )]),
            },
            "sonarr",
        );

        assert!(payload.supported);
        assert_eq!(payload.scryer_provider_type.as_deref(), Some("animetosho"));
        assert_eq!(payload.dedup_key, "animetosho:https://feed.animetosho.org");
    }
}
