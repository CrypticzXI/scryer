use std::collections::BTreeMap;
use std::io::Cursor;
use std::time::Duration;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use scryer_application::{AppError, AppResult, IndexerCapsSnapshotRefresher};
use scryer_domain::{
    IndexerCapsSearchNode, IndexerCapsSnapshot, IndexerCategoryDescriptor, IndexerCategoryModel,
    IndexerCategoryValueKind, IndexerConfig,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    external_arr_reqwest_client,
};
use serde_json::Value;

const DIRECT_NAB_CAPS_USER_AGENT: &str = "scryer-indexer-caps/0.1";

pub(crate) fn is_direct_nab_provider(provider_type: &str) -> bool {
    matches!(
        provider_type.trim().to_ascii_lowercase().as_str(),
        "newznab" | "nzbgeek" | "torznab"
    )
}

#[derive(Debug, Clone)]
struct DirectNabConfig {
    base_url: String,
    api_key: Option<String>,
    api_path: String,
}

impl DirectNabConfig {
    fn from_indexer_config(config: &IndexerConfig) -> AppResult<Self> {
        let value = config
            .config_json
            .as_deref()
            .map(serde_json::from_str::<Value>)
            .transpose()
            .map_err(|error| {
                AppError::Validation(format!("indexer config_json is invalid: {error}"))
            })?
            .unwrap_or(Value::Null);

        let base_url = value
            .get("base_url")
            .and_then(Value::as_str)
            .or_else(|| (!config.base_url.trim().is_empty()).then_some(config.base_url.as_str()))
            .unwrap_or_default()
            .trim()
            .trim_end_matches('/')
            .to_string();
        if base_url.is_empty() {
            return Err(AppError::Validation(
                "indexer caps refresh requires a base_url".into(),
            ));
        }

        let api_key = value
            .get("api_key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let api_path = value
            .get("api_path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("/api")
            .to_string();

        Ok(Self {
            base_url,
            api_key,
            api_path,
        })
    }

    fn caps_url(&self) -> AppResult<String> {
        let normalized_path = if self.api_path.trim().is_empty() {
            "/api".to_string()
        } else if self.api_path.starts_with('/') {
            self.api_path.trim().to_string()
        } else {
            format!("/{}", self.api_path.trim())
        };
        let endpoint = format!("{}{}", self.base_url.trim_end_matches('/'), normalized_path);
        let mut url = reqwest::Url::parse(&endpoint).map_err(|error| {
            AppError::Validation(format!("indexer caps base_url is invalid: {error}"))
        })?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("t", "caps");
            if let Some(api_key) = self.api_key.as_deref() {
                pairs.append_pair("apikey", api_key);
            }
        }
        Ok(url.to_string())
    }
}

#[derive(Clone)]
pub struct DirectNabCapsSnapshotRefresher {
    outbound_http: OutboundHttpClient,
}

impl DirectNabCapsSnapshotRefresher {
    pub fn new() -> Self {
        Self {
            outbound_http: OutboundHttpClient::new(
                external_arr_reqwest_client(),
                RateLimitRegistry::new(),
            ),
        }
    }
}

impl Default for DirectNabCapsSnapshotRefresher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IndexerCapsSnapshotRefresher for DirectNabCapsSnapshotRefresher {
    async fn fetch_for_config(
        &self,
        config: &IndexerConfig,
    ) -> AppResult<Option<IndexerCapsSnapshot>> {
        if !is_direct_nab_provider(&config.provider_type) {
            return Ok(None);
        }

        let direct_config = DirectNabConfig::from_indexer_config(config)?;
        let url = direct_config.caps_url()?;
        let response = self
            .outbound_http
            .send(
                RequestPolicy::safe_read(
                    format!("direct_nab_caps:{}", direct_config.base_url),
                    format!(
                        "direct_nab_caps:{}",
                        config.provider_type.trim().to_ascii_lowercase()
                    ),
                )
                .with_max_retries(2)
                .with_backoff(Duration::from_secs(1), Duration::from_secs(15)),
                || {
                    self.outbound_http
                        .client()
                        .get(url.clone())
                        .header("Accept", "application/xml, text/xml, application/rss+xml")
                        .header("User-Agent", DIRECT_NAB_CAPS_USER_AGENT)
                },
            )
            .await
            .map_err(|error| match error {
                OutboundHttpError::RateLimited(rate_limited) => {
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => AppError::Repository(format!(
                            "indexer caps refresh was rate limited (retry after {}s)",
                            delay.as_secs()
                        )),
                        None => AppError::Repository(
                            "indexer caps refresh was rate limited".to_string(),
                        ),
                    }
                }
                OutboundHttpError::Transport { source, .. } => {
                    AppError::Repository(format!("indexer caps request failed: {source}"))
                }
            })?;

        let status = response.status();
        let body = response.bytes().await.map_err(|error| {
            AppError::Repository(format!("indexer caps response read failed: {error}"))
        })?;
        if !status.is_success() {
            let body_snippet = String::from_utf8_lossy(&body);
            return Err(AppError::Repository(format!(
                "indexer caps request failed with status {}: {}",
                status,
                body_snippet.trim()
            )));
        }

        parse_caps_snapshot_xml(&body).map(Some)
    }
}

pub(crate) fn parse_caps_snapshot_xml(body: &[u8]) -> AppResult<IndexerCapsSnapshot> {
    let mut reader = Reader::from_reader(Cursor::new(body));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut snapshot = IndexerCapsSnapshot::default();
    let mut categories = BTreeMap::<String, IndexerCategoryDescriptor>::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                match element.name().as_ref() {
                    b"server" => {
                        snapshot.server_title = attr_value(&element, b"title")?;
                    }
                    b"limits" => {
                        snapshot.limits_default = attr_i64(&element, b"default")?;
                        snapshot.limits_max = attr_i64(&element, b"max")?;
                    }
                    b"search" => {
                        snapshot.search = Some(parse_caps_node(&element)?);
                    }
                    b"tv-search" => {
                        snapshot.tv_search = Some(parse_caps_node(&element)?);
                    }
                    b"movie-search" => {
                        snapshot.movie_search = Some(parse_caps_node(&element)?);
                    }
                    b"music-search" => {
                        snapshot.music_search = Some(parse_caps_node(&element)?);
                    }
                    b"audio-search" => {
                        snapshot.audio_search = Some(parse_caps_node(&element)?);
                    }
                    b"book-search" => {
                        snapshot.book_search = Some(parse_caps_node(&element)?);
                    }
                    b"category" | b"subcat" => {
                        if let Some(id) = attr_value(&element, b"id")?
                            .map(|value| value.trim().to_string())
                            .filter(|value| !value.is_empty())
                        {
                            let label = attr_value(&element, b"name")?
                                .map(|value| value.trim().to_string())
                                .filter(|value| !value.is_empty());
                            categories
                                .entry(id.clone())
                                .and_modify(|existing| {
                                    if existing.label.is_none() && label.is_some() {
                                        existing.label = label.clone();
                                    }
                                })
                                .or_insert_with(|| IndexerCategoryDescriptor {
                                    value: id,
                                    label,
                                    value_kind: IndexerCategoryValueKind::Numeric,
                                    facets: Vec::new(),
                                });
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "indexer returned invalid caps XML: {error}"
                )));
            }
            _ => {}
        }
        buf.clear();
    }

    if !categories.is_empty() {
        snapshot.categories = IndexerCategoryModel {
            value_kinds: vec![IndexerCategoryValueKind::Numeric],
            separate_anime_categories: false,
            provider_category_metadata: true,
            categories: categories.into_values().collect(),
        };
    }

    Ok(snapshot)
}

fn parse_caps_node(element: &BytesStart<'_>) -> AppResult<IndexerCapsSearchNode> {
    Ok(IndexerCapsSearchNode {
        available: attr_value(element, b"available")?
            .is_some_and(|value| value.eq_ignore_ascii_case("yes")),
        supported_params: attr_value(element, b"supportedParams")?
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase())
            .collect(),
        search_engine: attr_value(element, b"searchEngine")?
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    })
}

fn attr_value(element: &BytesStart<'_>, key: &[u8]) -> AppResult<Option<String>> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| {
            AppError::Repository(format!(
                "indexer returned invalid caps XML attributes: {error}"
            ))
        })?;
        if attribute.key.as_ref() == key {
            let value = std::str::from_utf8(attribute.value.as_ref()).map_err(|error| {
                AppError::Repository(format!(
                    "indexer returned non-UTF8 caps attribute values: {error}"
                ))
            })?;
            return Ok(Some(value.to_string()));
        }
    }

    Ok(None)
}

fn attr_i64(element: &BytesStart<'_>, key: &[u8]) -> AppResult<Option<i64>> {
    attr_value(element, key)?.map_or(Ok(None), |value| {
        value.trim().parse::<i64>().map(Some).map_err(|error| {
            AppError::Repository(format!(
                "indexer returned invalid numeric caps values: {error}"
            ))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_caps_snapshot_xml_parses_search_nodes_limits_and_categories() {
        let xml = br#"
            <caps>
              <server title="Synthetic Indexer" />
              <limits default="100" max="250" />
              <searching>
                <search available="yes" supportedParams="q" />
                <tv-search available="yes" supportedParams="q,season,ep,tvdbid,rid,tvmazeid" />
                <movie-search available="yes" supportedParams="q,imdbid,genre" searchEngine="raw" />
                <music-search available="no" supportedParams="q" />
                <audio-search available="yes" supportedParams="q" />
                <book-search available="no" supportedParams="q" />
              </searching>
              <categories>
                <category id="2000" name="Movies">
                  <subcat id="2010" name="Movies HD" />
                </category>
              </categories>
            </caps>
        "#;

        let snapshot = parse_caps_snapshot_xml(xml).expect("caps xml should parse");

        assert_eq!(snapshot.server_title.as_deref(), Some("Synthetic Indexer"));
        assert_eq!(snapshot.limits_default, Some(100));
        assert_eq!(snapshot.limits_max, Some(250));
        assert_eq!(
            snapshot
                .movie_search
                .as_ref()
                .expect("movie search node")
                .supported_params,
            vec!["q", "imdbid", "genre"]
        );
        assert_eq!(
            snapshot
                .tv_search
                .as_ref()
                .expect("tv search node")
                .supported_params,
            vec!["q", "season", "ep", "tvdbid", "rid", "tvmazeid"]
        );
        assert_eq!(
            snapshot
                .movie_search
                .as_ref()
                .expect("movie search node")
                .search_engine
                .as_deref(),
            Some("raw")
        );
        assert_eq!(
            snapshot
                .categories
                .categories
                .iter()
                .map(|category| (category.value.clone(), category.label.clone()))
                .collect::<Vec<_>>(),
            vec![
                ("2000".to_string(), Some("Movies".to_string())),
                ("2010".to_string(), Some("Movies HD".to_string()))
            ]
        );
    }

    #[test]
    fn parse_caps_snapshot_xml_lowercases_supported_params_and_respects_availability() {
        let xml = br#"
            <caps>
              <searching>
                <movie-search available="no" supportedParams="Q,TMDBID,IMDbId" />
              </searching>
            </caps>
        "#;

        let snapshot = parse_caps_snapshot_xml(xml).expect("caps xml should parse");
        let movie = snapshot.movie_search.expect("movie search node");

        assert!(!movie.available);
        assert_eq!(movie.supported_params, vec!["q", "tmdbid", "imdbid"]);
    }
}
