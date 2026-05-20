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
    descriptor: &PluginDescriptor,
    indexer_name: &str,
    config: &IndexerConfig,
) -> Option<std::collections::HashMap<String, String>> {
    match config.config_json.as_deref() {
        Some(json_str) => match parse_config_json_entries(json_str) {
            Ok(map) => Some(normalize_indexer_config_entries(descriptor, config, map)),
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

fn normalize_indexer_config_entries(
    descriptor: &PluginDescriptor,
    config: &IndexerConfig,
    mut entries: std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    let mut extracted_api_path: Option<String> = None;
    let mut extracted_additional_params: Option<String> = None;
    let normalize_as_direct_nab = config.is_direct_nab();

    if let Some(connection_url_key) = descriptor
        .config_fields()
        .iter()
        .find(|field| field.role == Some(ConfigFieldRole::ConnectionUrl))
        .map(|field| field.key.as_str())
    {
        let normalized_connection_url = if normalize_as_direct_nab {
            entries
                .get(connection_url_key)
                .and_then(|value| normalize_direct_nab_connection_url(value))
                .map(|parts| {
                    extracted_api_path = parts.api_path;
                    extracted_additional_params = parts.additional_params;
                    parts.base_url
                })
        } else {
            entries
                .get(connection_url_key)
                .and_then(|value| normalize_connection_url(value))
        };

        match normalized_connection_url {
            Some(value) => {
                entries.insert(connection_url_key.to_string(), value);
            }
            None => {
                entries.remove(connection_url_key);
            }
        }
    }

    let normalized_api_path = extracted_api_path.or_else(|| {
        entries
            .get("api_path")
            .and_then(|value| normalize_api_path(value))
    });
    match normalized_api_path {
        Some(value) => {
            entries.insert("api_path".to_string(), value);
        }
        None => {
            entries.remove("api_path");
        }
    }

    let normalized_additional_params = merge_additional_params(
        extracted_additional_params.as_deref(),
        entries.get("additional_params").map(String::as_str),
    );

    match normalized_additional_params {
        Some(value) => {
            entries.insert("additional_params".to_string(), value);
        }
        None => {
            entries.remove("additional_params");
        }
    }

    entries
}

fn normalize_connection_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    (!trimmed.is_empty()).then_some(trimmed.to_string())
}

#[derive(Debug, PartialEq, Eq)]
struct NormalizedDirectNabConnection {
    base_url: String,
    api_path: Option<String>,
    additional_params: Option<String>,
}

fn normalize_direct_nab_connection_url(raw: &str) -> Option<NormalizedDirectNabConnection> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let Ok(url) = url::Url::parse(trimmed) else {
        return normalize_connection_url(raw).map(|base_url| NormalizedDirectNabConnection {
            base_url,
            api_path: None,
            additional_params: None,
        });
    };

    let mut normalized = url.clone();
    normalized.set_query(None);
    normalized.set_fragment(None);
    normalized.set_path("");

    let origin = normalized.to_string().trim_end_matches('/').to_string();
    if origin.is_empty() {
        return None;
    }

    let raw_path = url.path().trim();
    let api_path = normalize_api_path(raw_path);

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url.query_pairs() {
        let key = key.trim();
        if key.is_empty() || is_direct_nab_control_query_key(key) {
            continue;
        }
        serializer.append_pair(key, value.trim());
    }
    let serialized_params = serializer.finish();
    let additional_params = (!serialized_params.is_empty()).then_some(serialized_params);

    Some(NormalizedDirectNabConnection {
        base_url: origin,
        api_path,
        additional_params,
    })
}

fn normalize_api_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_matches('/');
    (!trimmed.is_empty()).then(|| format!("/{trimmed}"))
}

fn normalize_additional_params(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
    if trimmed.is_empty() {
        return None;
    }

    let pairs = url::form_urlencoded::parse(trimmed.as_bytes()).collect::<Vec<_>>();
    if pairs.is_empty() {
        return None;
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(&key, &value);
    }

    let normalized = serializer.finish();
    (!normalized.is_empty()).then_some(normalized)
}

fn merge_additional_params(extracted: Option<&str>, existing: Option<&str>) -> Option<String> {
    if extracted.is_none() {
        return existing.and_then(normalize_additional_params);
    }
    if existing.is_none() {
        return extracted.and_then(normalize_additional_params);
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    let mut any = false;

    for raw in [extracted, existing].into_iter().flatten() {
        let trimmed = raw.trim().trim_start_matches(['?', '&']).trim();
        if trimmed.is_empty() {
            continue;
        }

        for (key, value) in url::form_urlencoded::parse(trimmed.as_bytes()) {
            let key = key.trim();
            if key.is_empty() {
                continue;
            }
            serializer.append_pair(key, value.trim());
            any = true;
        }
    }

    any.then(|| serializer.finish())
}

fn is_direct_nab_control_query_key(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "apikey"
            | "api_key"
            | "key"
            | "token"
            | "t"
            | "q"
            | "cat"
            | "o"
            | "extended"
            | "limit"
            | "offset"
            | "imdbid"
            | "tvdbid"
            | "tmdbid"
            | "season"
            | "ep"
            | "rid"
            | "tvmazeid"
            | "traktid"
            | "doubanid"
            | "imdbtitle"
            | "imdbyear"
            | "genre"
            | "year"
            | "group"
    )
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
        .and_then(|entries| entries.get(&field.key).map(String::as_str))
        .or(field.default_value.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
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

    #[test]
    fn normalizes_additional_params_for_safe_query_appending() {
        assert_eq!(
            normalize_additional_params(" ?foo=bar baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(
            normalize_additional_params(" &foo=bar%20baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(
            normalize_additional_params(" foo=bar%20baz&zap=1 "),
            Some("foo=bar+baz&zap=1".to_string())
        );
        assert_eq!(normalize_additional_params(" ? "), None);
    }

    #[test]
    fn normalizes_connection_url_and_api_path_for_sloppy_input() {
        assert_eq!(
            normalize_connection_url(" https://indexer.example.com/// "),
            Some("https://indexer.example.com".to_string())
        );
        assert_eq!(normalize_connection_url("   "), None);
        assert_eq!(
            normalize_api_path(" /api/v1/api// "),
            Some("/api/v1/api".to_string())
        );
        assert_eq!(normalize_api_path(" /// "), None);
    }

    #[test]
    fn normalizes_direct_nab_connection_urls_with_embedded_query_state() {
        assert_eq!(
            normalize_direct_nab_connection_url(
                " https://api.nzbgeek.info/api?t=search&q=legacy&cat=2000,2040&attrs=poster&apikey=secret "
            ),
            Some(NormalizedDirectNabConnection {
                base_url: "https://api.nzbgeek.info".to_string(),
                api_path: Some("/api".to_string()),
                additional_params: Some("attrs=poster".to_string()),
            })
        );
        assert_eq!(
            normalize_direct_nab_connection_url(" https://api.nzbgeek.info/nzbapi/ "),
            Some(NormalizedDirectNabConnection {
                base_url: "https://api.nzbgeek.info".to_string(),
                api_path: Some("/nzbapi".to_string()),
                additional_params: None,
            })
        );
    }

    #[test]
    fn merges_extracted_and_existing_additional_params() {
        assert_eq!(
            merge_additional_params(Some("attrs=poster&dl=1"), Some(" ?foo=bar baz&zap=1 "),),
            Some("attrs=poster&dl=1&foo=bar+baz&zap=1".to_string())
        );
    }

    fn descriptor_with_base_url_role(provider_type: &str) -> PluginDescriptor {
        PluginDescriptor {
            id: format!("{provider_type}_test"),
            name: "Test".to_string(),
            version: "0.0.0".to_string(),
            sdk_version: "0.0.0".to_string(),
            sdk_constraint: ">=0.0.0".to_string(),
            socket_permissions: vec![],
            provider: crate::types::ProviderDescriptor::Indexer(crate::types::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: vec![],
                source_kind: crate::types::IndexerSourceKind::Usenet,
                capabilities: crate::types::IndexerCapabilities::default(),
                scoring_policies: vec![],
                config_fields: vec![crate::types::ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: crate::types::ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: Default::default(),
                    role: Some(ConfigFieldRole::ConnectionUrl),
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                }],
                allowed_hosts: vec![],
                rate_limit_seconds: None,
            }),
        }
    }

    fn sample_indexer_config(
        provider_type: &str,
        managed_parent_config_id: Option<&str>,
    ) -> IndexerConfig {
        IndexerConfig {
            id: "cfg".to_string(),
            name: "Test".to_string(),
            provider_type: provider_type.to_string(),
            base_url: String::new(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            managed_parent_config_id: managed_parent_config_id.map(ToString::to_string),
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn preserves_managed_prowlarr_child_proxy_path_for_newznab_provider() {
        let descriptor = descriptor_with_base_url_role("newznab");
        let config = sample_indexer_config("newznab", Some("parent"));
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            "base_url".to_string(),
            "http://localhost:9696/1".to_string(),
        );
        entries.insert("api_path".to_string(), "/api".to_string());

        let normalized = normalize_indexer_config_entries(&descriptor, &config, entries);

        assert_eq!(
            normalized.get("base_url").map(String::as_str),
            Some("http://localhost:9696/1")
        );
        assert_eq!(normalized.get("api_path").map(String::as_str), Some("/api"));
        assert!(!normalized.contains_key("additional_params"));
    }
}
