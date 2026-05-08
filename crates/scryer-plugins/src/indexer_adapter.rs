use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, DownloadSourceKind, IndexerClient, IndexerRoutingPlan,
    IndexerSearchResponse, IndexerSearchResult, SearchMode,
};
use scryer_domain::{IndexerConfig, TaggedAlias};
use std::sync::mpsc;
use tracing::{info, warn};

use crate::loader::{apply_allowed_hosts, build_plugin, parse_config_json_entries};
use crate::types::{
    ConfigFieldRole, EXPORT_INDEXER_SEARCH, IndexerProtocol, IndexerSourceKind, PluginDescriptor,
    PluginSearchContext, PluginSearchOrigin, PluginSearchQueryKind, PluginSearchRequest,
    PluginSearchRequestKind, PluginSearchResponse, PluginSearchSubjectKind, decode_plugin_result,
    normalize_external_ids, normalize_indexer_info_hash, tagged_alias_to_sdk,
};

pub struct WasmIndexerClient {
    descriptor: PluginDescriptor,
    indexer_name: String,
    worker: IndexerPluginWorker,
}

struct IndexerPluginWorker {
    tx: mpsc::Sender<IndexerPluginCommand>,
}

struct IndexerPluginCommand {
    input: String,
    response: tokio::sync::oneshot::Sender<AppResult<String>>,
}

impl IndexerPluginWorker {
    fn start(
        manifest: extism::Manifest,
        descriptor: &PluginDescriptor,
        indexer_name: &str,
    ) -> AppResult<Self> {
        let (tx, rx) = mpsc::channel::<IndexerPluginCommand>();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let plugin_name = descriptor.name.clone();
        let indexer_label = indexer_name.to_string();
        let thread_name = format!("scryer-wasm-indexer-{indexer_name}");

        std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let mut plugin = match build_plugin(manifest) {
                    Ok(plugin) => {
                        let _ = ready_tx.send(Ok(()));
                        plugin
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.to_string()));
                        return;
                    }
                };

                while let Ok(command) = rx.recv() {
                    let start = std::time::Instant::now();
                    let result = plugin
                        .call::<&str, String>(EXPORT_INDEXER_SEARCH, &command.input)
                        .map_err(|e| {
                            AppError::Repository(format!(
                                "plugin {EXPORT_INDEXER_SEARCH}() failed: {e}"
                            ))
                        });
                    let elapsed = start.elapsed();

                    tracing::debug!(
                        plugin = plugin_name.as_str(),
                        indexer = indexer_label.as_str(),
                        elapsed_ms = elapsed.as_millis() as u64,
                        "WASM plugin search call completed"
                    );

                    let _ = command.response.send(result);
                }
            })
            .map_err(|e| AppError::Repository(format!("failed to start plugin worker: {e}")))?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(error)) => Err(AppError::Repository(format!(
                "failed to compile WASM plugin for {indexer_name}: {error}"
            ))),
            Err(error) => Err(AppError::Repository(format!(
                "plugin worker stopped during startup: {error}"
            ))),
        }
    }

    async fn call_search(&self, input: String) -> AppResult<String> {
        let (response, result) = tokio::sync::oneshot::channel();
        self.tx
            .send(IndexerPluginCommand { input, response })
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?;
        result
            .await
            .map_err(|_| AppError::Repository("plugin worker stopped".into()))?
    }
}

impl WasmIndexerClient {
    pub fn new(
        wasm_bytes: Vec<u8>,
        descriptor: PluginDescriptor,
        indexer_name: String,
        config: IndexerConfig,
    ) -> Result<Self, AppError> {
        let manifest = build_manifest(wasm_bytes, &descriptor, &indexer_name, &config);
        let worker = IndexerPluginWorker::start(manifest, &descriptor, &indexer_name)?;

        info!(
            indexer = indexer_name.as_str(),
            plugin = descriptor.name.as_str(),
            "WASM plugin compiled and cached"
        );

        Ok(Self {
            descriptor,
            indexer_name,
            worker,
        })
    }
}

fn build_manifest(
    wasm_bytes: Vec<u8>,
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
) -> extism::Manifest {
    let mut manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes)]);
    let config_entries = build_config_entries(descriptor, indexer_name, config);
    let connection_url = resolve_connection_url(descriptor, config_entries.as_ref());
    manifest = apply_allowed_hosts(
        manifest,
        descriptor,
        connection_url.as_deref(),
        config.config_json.as_deref(),
    );
    manifest = manifest.with_timeout(std::time::Duration::from_secs(30));

    if let Some(map) = &config_entries {
        for (key, value) in map {
            manifest = manifest.with_config_key(key, value);
        }
    }

    manifest
}

fn build_config_entries(
    _descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
) -> Option<std::collections::HashMap<String, String>> {
    match config.config_json.as_deref() {
        Some(json_str) => match parse_config_json_entries(json_str) {
            Ok(map) => Some(map),
            Err(error) => {
                warn!(
                    indexer = indexer_name,
                    error = %error,
                    "failed to parse config_json; config keys will not be injected"
                );
                None
            }
        },
        None => None,
    }
}

fn resolve_connection_url(
    descriptor: &PluginDescriptor,
    config_entries: Option<&std::collections::HashMap<String, String>>,
) -> Option<String> {
    let field = descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))?;
    config_entries
        .and_then(|entries| entries.get(&field.key))
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .or_else(|| field.default_value.clone())
}

fn build_search_context(
    query: &str,
    ids: &std::collections::HashMap<String, String>,
    facet: Option<&str>,
    mode: SearchMode,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> PluginSearchContext {
    let is_recent_request = matches!(mode, SearchMode::Auto)
        && query.trim().is_empty()
        && ids.is_empty()
        && season.is_none()
        && episode.is_none()
        && absolute_episode.is_none();

    let normalized_facet = facet
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    let subject_kind = match normalized_facet.as_deref() {
        Some("movie") => PluginSearchSubjectKind::Movie,
        Some("anime") if episode.is_some() || absolute_episode.is_some() => {
            PluginSearchSubjectKind::AnimeEpisode
        }
        Some("special") => PluginSearchSubjectKind::Special,
        _ if episode.is_some() || absolute_episode.is_some() => PluginSearchSubjectKind::Episode,
        _ if season.is_some() => PluginSearchSubjectKind::Season,
        Some("collection") => PluginSearchSubjectKind::Collection,
        Some("series") | Some("anime") | Some("title") => PluginSearchSubjectKind::Title,
        _ => PluginSearchSubjectKind::Unknown,
    };

    let query_kind = if !ids.is_empty() {
        if ids.len() > 1 {
            PluginSearchQueryKind::AggregateId
        } else {
            PluginSearchQueryKind::Id
        }
    } else if query.trim().is_empty() {
        PluginSearchQueryKind::Fallback
    } else if normalized_facet.is_some() {
        PluginSearchQueryKind::Title
    } else {
        PluginSearchQueryKind::Text
    };

    PluginSearchContext {
        request_kind: if is_recent_request {
            PluginSearchRequestKind::Recent
        } else {
            PluginSearchRequestKind::Search
        },
        search_origin: if is_recent_request {
            PluginSearchOrigin::Rss
        } else {
            match mode {
                SearchMode::Interactive => PluginSearchOrigin::Interactive,
                SearchMode::Auto => PluginSearchOrigin::Automatic,
            }
        },
        subject_kind,
        query_kind,
        ..PluginSearchContext::default()
    }
}

fn merge_result_extra(
    result: &scryer_plugin_sdk::PluginSearchResult,
) -> std::collections::HashMap<String, serde_json::Value> {
    let provider_extra = result.provider_extra.clone();
    let mut extra = std::collections::HashMap::new();

    insert_value(&mut extra, "source_kind", result.source_kind);
    insert_value(&mut extra, "protocol", result.protocol);

    let normalized_external_ids = normalize_external_ids(
        result
            .external_ids
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    );
    if !normalized_external_ids.is_empty() {
        insert_json(&mut extra, "external_ids", &normalized_external_ids);
        if let Some(imdb_id) = normalized_external_ids.get("imdb_id") {
            insert_json(&mut extra, "response_imdbid", imdb_id);
        }
        if let Some(tvdb_id) = normalized_external_ids.get("tvdb_id") {
            insert_json(&mut extra, "response_tvdbid", tvdb_id);
        }
        if let Some(anidb_id) = normalized_external_ids.get("anidb_id") {
            insert_json(&mut extra, "response_anidbid", anidb_id);
        }
    }

    if !result.categories.is_empty() {
        insert_json(&mut extra, "categories", &result.categories);
    }
    if !result.provider_categories.is_empty() {
        insert_json(
            &mut extra,
            "provider_categories",
            &result.provider_categories,
        );
    }

    if let Some(magnet_url) = result.magnet_url.as_deref() {
        insert_json(&mut extra, "magnet_url", magnet_url);
        insert_json(&mut extra, "magnet_uri", magnet_url);
    }

    let info_hash_v1 = normalize_indexer_info_hash(result.info_hash_v1.as_deref())
        .filter(|value| value.len() == 40);
    let info_hash_v2 = normalize_indexer_info_hash(result.info_hash_v2.as_deref())
        .filter(|value| value.len() == 64);
    if let Some(info_hash_v1) = info_hash_v1.as_deref() {
        insert_json(&mut extra, "info_hash_v1", info_hash_v1);
        insert_json(&mut extra, "info_hash", info_hash_v1);
    }
    if let Some(info_hash_v2) = info_hash_v2.as_deref() {
        insert_json(&mut extra, "info_hash_v2", info_hash_v2);
    }

    insert_value(&mut extra, "seeders", result.seeders);
    insert_value(&mut extra, "peers", result.peers);
    insert_value(&mut extra, "leechers", result.leechers);
    insert_value(
        &mut extra,
        "download_volume_factor",
        result.download_volume_factor,
    );
    insert_value(
        &mut extra,
        "upload_volume_factor",
        result.upload_volume_factor,
    );
    insert_value(
        &mut extra,
        "downloadvolumefactor",
        result.download_volume_factor,
    );
    insert_value(
        &mut extra,
        "uploadvolumefactor",
        result.upload_volume_factor,
    );
    insert_value(&mut extra, "origin", result.origin.as_deref());
    insert_value(&mut extra, "source", result.source.as_deref());
    insert_value(&mut extra, "container", result.container.as_deref());
    insert_value(&mut extra, "codec", result.codec.as_deref());
    insert_value(&mut extra, "resolution", result.resolution.as_deref());

    if !result.indexer_flags.is_empty() {
        insert_json(&mut extra, "indexer_flags", &result.indexer_flags);
    }

    insert_value(&mut extra, "comment_url", result.comment_url.as_deref());
    insert_value(&mut extra, "minimum_seed_ratio", result.minimum_seed_ratio);
    insert_value(
        &mut extra,
        "minimum_seed_time_minutes",
        result.minimum_seed_time_minutes,
    );
    insert_value(
        &mut extra,
        "season_pack_seed_ratio",
        result.season_pack_seed_ratio,
    );
    insert_value(
        &mut extra,
        "season_pack_seed_time_minutes",
        result.season_pack_seed_time_minutes,
    );

    for (key, value) in provider_extra {
        extra.entry(key).or_insert(value);
    }

    extra
}

fn explicit_source_kind(
    result: &scryer_plugin_sdk::PluginSearchResult,
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<DownloadSourceKind> {
    match result.source_kind {
        Some(IndexerSourceKind::Usenet) => Some(DownloadSourceKind::NzbUrl),
        Some(IndexerSourceKind::Torrent) => {
            if result.magnet_url.is_some() || extra.contains_key("magnet_uri") {
                Some(DownloadSourceKind::MagnetUri)
            } else {
                Some(DownloadSourceKind::TorrentFile)
            }
        }
        Some(IndexerSourceKind::Generic) | None => match result.protocol {
            Some(IndexerProtocol::Usenet) => Some(DownloadSourceKind::NzbUrl),
            Some(IndexerProtocol::Torrent) => {
                if result.magnet_url.is_some() || extra.contains_key("magnet_uri") {
                    Some(DownloadSourceKind::MagnetUri)
                } else {
                    Some(DownloadSourceKind::TorrentFile)
                }
            }
            _ => None,
        },
    }
}

fn insert_json<T: serde::Serialize>(
    extra: &mut std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    value: T,
) {
    if !extra.contains_key(key)
        && let Ok(value) = serde_json::to_value(value)
    {
        if value.is_null() {
            return;
        }
        extra.insert(key.to_string(), value);
    }
}

fn insert_value<T: serde::Serialize>(
    extra: &mut std::collections::HashMap<String, serde_json::Value>,
    key: &str,
    value: T,
) {
    insert_json(extra, key, value);
}

#[async_trait]
impl IndexerClient for WasmIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        let context = build_search_context(
            &query,
            &ids,
            facet.as_deref(),
            mode,
            season,
            episode,
            absolute_episode,
        );
        let request = PluginSearchRequest {
            query,
            ids,
            facet,
            category,
            categories: newznab_categories.unwrap_or_default(),
            limit: 1000,
            season,
            episode,
            absolute_episode,
            tagged_aliases: tagged_aliases
                .into_iter()
                .map(tagged_alias_to_sdk)
                .collect(),
            context: Some(context),
        };

        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize plugin request: {e}"))
        })?;

        tracing::debug!(plugin = %self.descriptor.name, %input, "plugin search request");

        let output = self.worker.call_search(input).await?;

        let response: PluginSearchResponse = decode_plugin_result(&output, EXPORT_INDEXER_SEARCH)?;

        let source = format!(
            "{} ({})",
            self.indexer_name,
            self.descriptor.provider_type()
        );
        let results = response
            .results
            .into_iter()
            .map(|r| {
                let extra = merge_result_extra(&r);
                let source_kind = explicit_source_kind(&r, &extra).or_else(|| {
                    DownloadSourceKind::infer_from_indexer_result(
                        Some(self.descriptor.plugin_type()),
                        r.download_url.as_deref(),
                        r.link.as_deref(),
                        &extra,
                    )
                });

                IndexerSearchResult {
                    source: source.clone(),
                    title: r.title,
                    link: r.link,
                    download_url: r.download_url,
                    source_kind,
                    size_bytes: r.size_bytes,
                    published_at: r.published_at,
                    thumbs_up: r.thumbs_up,
                    thumbs_down: r.thumbs_down,
                    indexer_languages: if r.languages.is_empty() {
                        None
                    } else {
                        Some(r.languages)
                    },
                    indexer_subtitles: if r.subtitles.is_empty() {
                        None
                    } else {
                        Some(r.subtitles)
                    },
                    indexer_grabs: r.grabs,
                    password_hint: r.password_hint,
                    candidate_token: None,
                    parsed_release_metadata: None,
                    quality_profile_decision: None,
                    extra,
                    guid: r.guid,
                    info_url: r.info_url,
                    provenance: None,
                    queue_scope: None,
                    auto_eligible: None,
                    auto_decision_code: None,
                    auto_decision_summary: None,
                }
            })
            .collect();

        Ok(IndexerSearchResponse {
            results,
            api_current: response.api_current,
            api_max: response.api_max,
            grab_current: response.grab_current,
            grab_max: response.grab_max,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_episode_id_context_for_auto_search() {
        let context = build_search_context(
            "Example Show S01E02",
            &std::collections::HashMap::from([("tvdb_id".to_string(), "123".to_string())]),
            Some("series"),
            SearchMode::Auto,
            Some(1),
            Some(2),
            None,
        );

        assert_eq!(context.request_kind, PluginSearchRequestKind::Search);
        assert_eq!(context.search_origin, PluginSearchOrigin::Automatic);
        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Episode);
        assert_eq!(context.query_kind, PluginSearchQueryKind::Id);
    }

    #[test]
    fn builds_recent_context_for_category_only_auto_request() {
        let context = build_search_context(
            "",
            &std::collections::HashMap::new(),
            Some("series"),
            SearchMode::Auto,
            None,
            None,
            None,
        );

        assert_eq!(context.request_kind, PluginSearchRequestKind::Recent);
        assert_eq!(context.search_origin, PluginSearchOrigin::Rss);
        assert_eq!(context.subject_kind, PluginSearchSubjectKind::Title);
        assert_eq!(context.query_kind, PluginSearchQueryKind::Fallback);
    }

    #[test]
    fn merges_v13_result_fields_into_extra_with_top_level_precedence() {
        let result = scryer_plugin_sdk::PluginSearchResult {
            title: "Example".to_string(),
            source_kind: Some(IndexerSourceKind::Torrent),
            protocol: Some(IndexerProtocol::Torrent),
            external_ids: std::collections::HashMap::from([
                ("imdb".to_string(), "tt1234567".to_string()),
                ("tvdb".to_string(), "987".to_string()),
            ]),
            categories: vec!["TV".to_string()],
            magnet_url: Some("magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01".into()),
            info_hash_v1: Some("abcdef0123456789abcdef0123456789abcdef01".into()),
            seeders: Some(42),
            provider_extra: std::collections::HashMap::from([
                (
                    "magnet_uri".to_string(),
                    serde_json::Value::from("magnet:?existing"),
                ),
                (
                    "response_imdbid".to_string(),
                    serde_json::Value::from("tt0000000"),
                ),
                (
                    "provider_specific".to_string(),
                    serde_json::Value::from("kept"),
                ),
            ]),
            ..scryer_plugin_sdk::PluginSearchResult::default()
        };

        let extra = merge_result_extra(&result);
        assert_eq!(
            extra.get("response_imdbid"),
            Some(&serde_json::Value::from("tt1234567"))
        );
        assert_eq!(
            extra.get("response_tvdbid"),
            Some(&serde_json::Value::from("987"))
        );
        assert_eq!(extra.get("seeders"), Some(&serde_json::Value::from(42)));
        assert_eq!(
            extra.get("info_hash"),
            Some(&serde_json::Value::from(
                "abcdef0123456789abcdef0123456789abcdef01"
            ))
        );
        assert_eq!(
            extra.get("magnet_uri"),
            Some(&serde_json::Value::from(
                "magnet:?xt=urn:btih:abcdef0123456789abcdef0123456789abcdef01"
            ))
        );
        assert_eq!(
            extra.get("provider_specific"),
            Some(&serde_json::Value::from("kept"))
        );
    }
}
