use super::*;

#[derive(Default)]
pub(super) struct MockIndexerClient;

#[async_trait]
impl IndexerClient for MockIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        if let Some(tvdb) = ids.get("tvdb_id") {
            tracing::info!(tvdb_id = %tvdb, category = ?category, "mock nzbgeek search");
        }
        if let Some(imdb) = ids.get("imdb_id") {
            tracing::info!(imdb_id = %imdb, category = ?category, "mock nzbgeek search");
        }
        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: format!("match for {query}"),
                link: None,
                download_url: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: Default::default(),
                guid: None,
                info_url: None,
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

pub(super) struct MockIndexerPluginProvider {
    pub(super) client: Arc<dyn IndexerClient>,
}

impl IndexerPluginProvider for MockIndexerPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        Some(Arc::clone(&self.client))
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec!["nzbgeek".to_string(), "torrent_rss".to_string()]
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        vec![]
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let connection_key = match provider_type {
            "torrent_rss" => "feed_url",
            _ => "base_url",
        };
        let mut fields = vec![scryer_domain::ConfigFieldDef {
            key: connection_key.to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_domain::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        }];
        if provider_type != "torrent_rss" {
            fields.push(scryer_domain::ConfigFieldDef {
                key: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: scryer_domain::ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: scryer_domain::ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            });
        }
        fields
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedIndexerSearch {
    pub(super) query: String,
    pub(super) season: Option<u32>,
    pub(super) episode: Option<u32>,
}

#[derive(Default, Clone)]
pub(super) struct TrackingIndexerClient {
    pub(super) searches: Arc<Mutex<Vec<RecordedIndexerSearch>>>,
}

#[async_trait]
impl IndexerClient for TrackingIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.searches.lock().await.push(RecordedIndexerSearch {
            query: query.clone(),
            season,
            episode,
        });

        let release_title = match (season, episode) {
            (Some(season), Some(episode)) => {
                format!("{query}.S{season:02}E{episode:02}.1080p.WEB-DL")
            }
            (Some(season), None) => format!("{query}.S{season:02}.1080p.WEB-DL"),
            (None, _) => format!("{query}.2024.1080p.WEB-DL"),
        };
        let release_slug = release_title.replace([' ', '/'], ".");

        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: release_title.clone(),
                link: Some(format!("https://example.invalid/info/{release_slug}")),
                download_url: Some(format!(
                    "https://example.invalid/download/{release_slug}.nzb"
                )),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some(format!("guid-{release_slug}")),
                info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct FixedReleaseIndexerClient {
    pub(super) release_title: String,
    pub(super) indexer_languages: Option<Vec<String>>,
}

impl FixedReleaseIndexerClient {
    pub(super) fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            indexer_languages: None,
        }
    }
}

#[async_trait]
impl IndexerClient for FixedReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some("https://example.invalid/download.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: self.indexer_languages.clone(),
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some("guid-fixed-release".to_string()),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct SharedUrlMovieIndexerClient {
    pub(super) download_url: String,
}

impl SharedUrlMovieIndexerClient {
    pub(super) fn new(download_url: impl Into<String>) -> Self {
        Self {
            download_url: download_url.into(),
        }
    }
}

#[async_trait]
impl IndexerClient for SharedUrlMovieIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        let query = query.trim();
        let release_title = if query.contains("Deferred Movie") {
            "Deferred.Movie.2024.1080p.WEB-DL-GRP".to_string()
        } else if query.contains("Rejected Movie") {
            "Rejected.Movie.2024.1080p.WEB-DL-GRP".to_string()
        } else {
            let release_stem = query
                .split_whitespace()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(".");
            format!("{release_stem}.2024.1080p.WEB-DL-GRP")
        };

        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some(self.download_url.clone()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some(format!("guid-{release_title}")),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedSearchCall {
    pub(super) query: String,
    pub(super) ids: std::collections::HashMap<String, String>,
    pub(super) category: Option<String>,
    pub(super) facet: Option<String>,
    pub(super) id_search_facet: Option<String>,
    pub(super) newznab_categories: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RecordedStructuredQueryCall {
    pub(super) query: String,
    pub(super) season: Option<u32>,
    pub(super) episode: Option<u32>,
    pub(super) absolute_episode: Option<u32>,
}

#[derive(Clone)]
pub(super) struct RecordingCategoriesIndexerClient {
    pub(super) release_title: String,
    pub(super) calls: Arc<Mutex<Vec<RecordedSearchCall>>>,
}

impl RecordingCategoriesIndexerClient {
    pub(super) fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct RecordingStructuredQueryIndexerClient {
    pub(super) calls: Arc<Mutex<Vec<RecordedStructuredQueryCall>>>,
}

#[async_trait]
impl IndexerClient for RecordingCategoriesIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        facet: Option<String>,
        id_search_facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedSearchCall {
            query,
            ids,
            category,
            facet,
            id_search_facet,
            newznab_categories,
        });

        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some("https://example.invalid/download.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some("guid-recording-release".to_string()),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[async_trait]
impl IndexerClient for RecordingStructuredQueryIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedStructuredQueryCall {
            query,
            season,
            episode,
            absolute_episode,
        });

        Ok(IndexerSearchResponse {
            results: vec![],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
pub(super) struct MultiReleaseIndexerClient {
    pub(super) release_titles: Vec<String>,
}

impl MultiReleaseIndexerClient {
    pub(super) fn new(release_titles: Vec<&str>) -> Self {
        Self {
            release_titles: release_titles.into_iter().map(str::to_string).collect(),
        }
    }
}

#[async_trait]
impl IndexerClient for MultiReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _id_search_facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
        _cancel_token: tokio_util::sync::CancellationToken,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: self
                .release_titles
                .iter()
                .enumerate()
                .map(|(index, release_title)| IndexerSearchResult {
                    source: "nzbgeek".into(),
                    title: release_title.clone(),
                    link: Some(format!("https://example.invalid/info/{index}")),
                    download_url: Some(format!("https://example.invalid/download/{index}.nzb")),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
                    quality_profile_decision: None,
                    extra: Default::default(),
                    guid: Some(format!("guid-multi-release-{index}")),
                    info_url: Some(format!("https://example.invalid/info/{index}")),
                    provenance: None,
                    auto_eligible: None,
                    auto_decision_code: None,
                    auto_decision_summary: None,
                    candidate_token: None,
                    queue_scope: None,
                })
                .collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

pub(super) struct MockMetadataGateway {
    pub(super) movies: HashMap<i64, MovieMetadata>,
}

#[async_trait]
impl MetadataGateway for MockMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.movies
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let movies = movie_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.movies
                    .get(tvdb_id)
                    .cloned()
                    .map(|movie| (*tvdb_id, movie))
            })
            .collect();
        Ok(BulkMetadataResult {
            movies,
            series: HashMap::new(),
        })
    }
}
