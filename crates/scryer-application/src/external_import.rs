use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::{AppError, AppResult};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy,
    external_arr_reqwest_client,
};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;

/// Root folder discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrRootFolder {
    pub id: i64,
    pub path: String,
}

/// Download client discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrDownloadClient {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

/// Indexer discovered from a Sonarr/Radarr instance.
#[derive(Debug, Clone)]
pub struct ArrIndexer {
    pub id: i64,
    pub name: String,
    pub implementation: String,
    pub fields: HashMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedProwlarrIndexer {
    pub base_url: String,
    pub api_key: Option<String>,
    pub child_name: String,
}

#[derive(Debug, Clone)]
pub struct ArrMovie {
    pub id: i64,
    pub root_folder_path: String,
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub monitored: bool,
}

#[derive(Debug, Clone)]
pub struct ArrSeriesSeason {
    pub season_number: i32,
    pub monitored: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ArrSeriesStatistics {
    pub total_episode_count: Option<i32>,
    pub monitored_episode_count: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct ArrSeries {
    pub id: i64,
    pub root_folder_path: String,
    pub tvdb_id: Option<String>,
    pub monitored: bool,
    pub seasons: Vec<ArrSeriesSeason>,
    pub statistics: ArrSeriesStatistics,
}

#[derive(Debug, Clone)]
pub struct ArrEpisode {
    pub id: i64,
    pub series_id: i64,
    pub tvdb_id: Option<String>,
    pub season_number: i32,
    pub episode_number: i32,
    pub monitored: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalArrApiBucket {
    SonarrV4,
    RadarrV6,
}

impl ExternalArrApiBucket {
    const fn expected_app_name(self) -> &'static str {
        match self {
            Self::SonarrV4 => "Sonarr",
            Self::RadarrV6 => "Radarr",
        }
    }

    const fn minimum_supported_major(self) -> u64 {
        match self {
            Self::SonarrV4 => 4,
            Self::RadarrV6 => 6,
        }
    }

    const fn supported_api_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::SonarrV4 => &["v3", "v4", "v5"],
            Self::RadarrV6 => &["v3"],
        }
    }

    const fn request_namespace(self) -> &'static str {
        match self {
            Self::SonarrV4 => "sonarr_v4",
            Self::RadarrV6 => "radarr_v6",
        }
    }

    fn validate_status(self, app_name: &str, version: &str) -> AppResult<()> {
        let app_name = app_name.trim();
        if !app_name.eq_ignore_ascii_case(self.expected_app_name()) {
            let message = if app_name.is_empty() {
                format!(
                    "base_url responded but did not identify itself as {}",
                    self.expected_app_name()
                )
            } else {
                format!(
                    "base_url responded as '{}', not {}",
                    app_name,
                    self.expected_app_name()
                )
            };
            return Err(AppError::Validation(message));
        }

        let version = version.trim();
        let Some(version_major) = external_arr_version_major(version) else {
            return Err(AppError::Validation(format!(
                "could not determine {} version from '{}'",
                self.expected_app_name(),
                version
            )));
        };

        if version_major < self.minimum_supported_major() {
            return Err(AppError::Validation(format!(
                "unsupported {} version '{}'; expected major {} or newer",
                self.expected_app_name(),
                version,
                self.minimum_supported_major()
            )));
        }

        Ok(())
    }

    fn validate_api_prefix(self, api_prefix: &str) -> AppResult<()> {
        let api_prefix = api_prefix.trim().trim_matches('/').to_ascii_lowercase();
        if api_prefix.is_empty() {
            return Err(AppError::Validation(format!(
                "{} did not report an API version",
                self.expected_app_name()
            )));
        }

        if self
            .supported_api_prefixes()
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(&api_prefix))
        {
            return Ok(());
        }

        Err(AppError::Validation(format!(
            "unsupported {} API version '{}'; expected one of {}",
            self.expected_app_name(),
            api_prefix,
            self.supported_api_prefixes().join(", ")
        )))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ExternalArrApiInfo {
    current: String,
}

#[derive(Debug, Clone)]
struct ExternalArrSystemStatus {
    app_name: String,
    version: String,
}

fn external_arr_version_major(version: &str) -> Option<u64> {
    version.trim().split('.').next()?.parse().ok()
}

/// HTTP client for Sonarr/Radarr APIs pinned to supported product-major buckets.
#[derive(Clone)]
pub struct ExternalArrClient {
    base_url: String,
    api_key: String,
    outbound_http: OutboundHttpClient,
    api_bucket: ExternalArrApiBucket,
    api_prefix: Arc<RwLock<Option<String>>>,
    system_status: Arc<RwLock<Option<ExternalArrSystemStatus>>>,
}

impl ExternalArrClient {
    pub fn for_sonarr_v4(base_url: String, api_key: String) -> Self {
        Self::new(base_url, api_key, ExternalArrApiBucket::SonarrV4)
    }

    pub fn for_radarr_v6(base_url: String, api_key: String) -> Self {
        Self::new(base_url, api_key, ExternalArrApiBucket::RadarrV6)
    }

    fn new(base_url: String, api_key: String, api_bucket: ExternalArrApiBucket) -> Self {
        let http_client = external_arr_reqwest_client();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            api_bucket,
            api_prefix: Arc::new(RwLock::new(None)),
            system_status: Arc::new(RwLock::new(None)),
        }
    }

    /// Test connectivity and return (app_name, version).
    pub async fn test_connection(&self) -> AppResult<(String, String)> {
        let status = self.ensure_supported_system_status().await?;
        Ok((status.app_name, status.version))
    }

    /// Fetch root folders (media library paths).
    pub async fn list_root_folders(&self) -> AppResult<Vec<ArrRootFolder>> {
        let json = self.api_get("rootfolder").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("rootfolder response was not an array".into()))?;
        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_i64()?;
                let path = item.get("path")?.as_str()?.to_string();
                if path.is_empty() {
                    return None;
                }
                Some(ArrRootFolder { id, path })
            })
            .collect())
    }

    /// Fetch download client configurations.
    ///
    /// Fetches the list first, then re-fetches each client individually because
    /// Sonarr v4+ / Radarr v5+ mask sensitive field values (e.g. `apiKey`) in the
    /// list endpoint response.
    pub async fn list_download_clients(&self) -> AppResult<Vec<ArrDownloadClient>> {
        let json = self.api_get("downloadclient").await?;
        let arr = json.as_array().ok_or_else(|| {
            AppError::Repository("downloadclient response was not an array".into())
        })?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            // Re-fetch individually to get unmasked sensitive fields.
            let fields = match self.api_get(&format!("downloadclient/{id}")).await {
                Ok(detail) => detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
                Err(_) => item
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
            };

            results.push(ArrDownloadClient {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    /// Fetch indexer configurations.
    ///
    /// Like `list_download_clients`, re-fetches each indexer individually to get
    /// unmasked sensitive fields (e.g. `apiKey`).
    pub async fn list_indexers(&self) -> AppResult<Vec<ArrIndexer>> {
        let json = self.api_get("indexer").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("indexer response was not an array".into()))?;

        let mut results = Vec::new();
        for item in arr {
            let id = match item.get("id").and_then(Value::as_i64) {
                Some(id) => id,
                None => continue,
            };
            let name = match item.get("name").and_then(Value::as_str) {
                Some(n) => n.to_string(),
                None => continue,
            };
            let implementation = match item.get("implementation").and_then(Value::as_str) {
                Some(i) => i.to_string(),
                None => continue,
            };

            // Re-fetch individually to get unmasked sensitive fields.
            let fields = match self.api_get(&format!("indexer/{id}")).await {
                Ok(detail) => detail
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
                Err(_) => item
                    .get("fields")
                    .and_then(Value::as_array)
                    .map(|f| flatten_arr_fields(f))
                    .unwrap_or_default(),
            };

            results.push(ArrIndexer {
                id,
                name,
                implementation,
                fields,
            });
        }
        Ok(results)
    }

    pub async fn list_movies(&self) -> AppResult<Vec<ArrMovie>> {
        let json = self.api_get("movie").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("movie response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let root_folder_path = item
                    .get("rootFolderPath")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                if root_folder_path.is_empty() {
                    return None;
                }

                Some(ArrMovie {
                    id,
                    root_folder_path,
                    tmdb_id: value_str_or_number(item.get("tmdbId")),
                    imdb_id: item
                        .get("imdbId")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect())
    }

    pub async fn list_series(&self) -> AppResult<Vec<ArrSeries>> {
        let json = self.api_get("series").await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("series response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let root_folder_path = item
                    .get("rootFolderPath")
                    .and_then(Value::as_str)?
                    .trim()
                    .to_string();
                if root_folder_path.is_empty() {
                    return None;
                }

                let seasons = item
                    .get("seasons")
                    .and_then(Value::as_array)
                    .map(|seasons| {
                        seasons
                            .iter()
                            .filter_map(|season| {
                                let season_number = season
                                    .get("seasonNumber")
                                    .and_then(Value::as_i64)?
                                    .try_into()
                                    .ok()?;
                                Some(ArrSeriesSeason {
                                    season_number,
                                    monitored: season
                                        .get("monitored")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                Some(ArrSeries {
                    id,
                    root_folder_path,
                    tvdb_id: value_str_or_number(item.get("tvdbId")),
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    seasons,
                    statistics: item
                        .get("statistics")
                        .and_then(Value::as_object)
                        .map(|statistics| ArrSeriesStatistics {
                            total_episode_count: value_i32(statistics.get("totalEpisodeCount"))
                                .or_else(|| value_i32(statistics.get("episodeCount"))),
                            monitored_episode_count: value_i32(
                                statistics.get("monitoredEpisodeCount"),
                            ),
                        })
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn list_episodes_for_series(&self, series_id: i64) -> AppResult<Vec<ArrEpisode>> {
        let json = self
            .api_get(&format!("episode?seriesId={series_id}"))
            .await?;
        let arr = json
            .as_array()
            .ok_or_else(|| AppError::Repository("episode response was not an array".into()))?;

        Ok(arr
            .iter()
            .filter_map(|item| {
                let id = item.get("id").and_then(Value::as_i64)?;
                let season_number = item
                    .get("seasonNumber")
                    .and_then(Value::as_i64)?
                    .try_into()
                    .ok()?;
                let episode_number = item
                    .get("episodeNumber")
                    .and_then(Value::as_i64)?
                    .try_into()
                    .ok()?;

                Some(ArrEpisode {
                    id,
                    series_id,
                    tvdb_id: value_str_or_number(item.get("tvdbId")),
                    season_number,
                    episode_number,
                    monitored: item
                        .get("monitored")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect())
    }

    async fn api_get(&self, path: &str) -> AppResult<Value> {
        self.ensure_supported_system_status().await?;
        let api_prefix = self.ensure_supported_api_prefix().await?;
        self.api_get_with_prefix(&api_prefix, path).await
    }

    async fn ensure_supported_system_status(&self) -> AppResult<ExternalArrSystemStatus> {
        if let Some(status) = self.system_status.read().await.clone() {
            return Ok(status);
        }

        let mut guard = self.system_status.write().await;
        if let Some(status) = guard.clone() {
            return Ok(status);
        }

        let api_prefix = self.ensure_supported_api_prefix().await?;
        let json = self
            .api_get_with_prefix(&api_prefix, "system/status")
            .await?;
        let status = ExternalArrSystemStatus {
            app_name: json
                .get("appName")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            version: json
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        self.api_bucket
            .validate_status(&status.app_name, &status.version)?;
        *guard = Some(status.clone());
        Ok(status)
    }

    async fn ensure_supported_api_prefix(&self) -> AppResult<String> {
        if let Some(api_prefix) = self.api_prefix.read().await.clone() {
            return Ok(api_prefix);
        }

        let mut guard = self.api_prefix.write().await;
        if let Some(api_prefix) = guard.clone() {
            return Ok(api_prefix);
        }

        let api_info: ExternalArrApiInfo = self.api_get_unversioned("api").await?;
        let api_prefix = api_info
            .current
            .trim()
            .trim_matches('/')
            .split('/')
            .next_back()
            .unwrap_or_default()
            .to_ascii_lowercase();
        self.api_bucket.validate_api_prefix(&api_prefix)?;
        *guard = Some(api_prefix.clone());
        Ok(api_prefix)
    }

    async fn api_get_with_prefix(&self, api_prefix: &str, path: &str) -> AppResult<Value> {
        let url = format!("{}/api/{}/{}", self.base_url, api_prefix, path);
        self.request_json(&url, path).await
    }

    async fn api_get_unversioned<T>(&self, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!("{}/{}", self.base_url, path.trim_start_matches('/'));
        self.request_json(&url, path).await
    }

    async fn request_json<T>(&self, url: &str, path: &str) -> AppResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let response = self
            .outbound_http
            .send(self.request_policy(path), || {
                self.outbound_http
                    .client()
                    .get(url)
                    .header("X-Api-Key", &self.api_key)
            })
            .await
            .map_err(|error| match error {
                OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
                    match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                        Some(delay) => format!(
                            "external api call to {path} was rate limited; retry after {}s",
                            delay.as_secs()
                        ),
                        None => format!("external api call to {path} was rate limited"),
                    },
                ),
                OutboundHttpError::Transport { source, .. } => {
                    AppError::Repository(format!("external api call to {path} failed: {source}"))
                }
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("external api response read failed: {err}"))
        })?;

        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Repository("invalid API key".into()));
        }
        if !status.is_success() {
            let preview = body.chars().take(400).collect::<String>();
            return Err(AppError::Repository(format!(
                "external api returned status {status}: {preview}"
            )));
        }

        serde_json::from_str(&body)
            .map_err(|err| AppError::Repository(format!("external api returned non-json: {err}")))
    }

    fn request_policy(&self, path: &str) -> RequestPolicy {
        RequestPolicy::safe_read(
            format!(
                "external_arr:{}:{}",
                self.api_bucket.request_namespace(),
                self.base_url
            ),
            format!(
                "external_arr:{}:{path}",
                self.api_bucket.request_namespace()
            ),
        )
        .with_max_retries(2)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(15))
    }
}

/// Map the Sonarr/Radarr implementation name to a Scryer download client type.
pub fn map_download_client_type(implementation: &str) -> Option<&'static str> {
    match implementation.trim().to_ascii_lowercase().as_str() {
        "nzbget" => Some("nzbget"),
        "sabnzbd" => Some("sabnzbd"),
        "qbittorrent" => Some("qbittorrent"),
        _ => None,
    }
}

/// Map the Sonarr/Radarr indexer implementation to a Scryer provider type.
///
/// For Newznab indexers, checks the base URL to identify known services that have
/// native Scryer plugins (e.g. NZBGeek, AnimeTosho) rather than falling back to
/// the generic newznab plugin.
pub fn map_indexer_provider_type(
    implementation: &str,
    fields: &HashMap<String, Value>,
) -> Option<&'static str> {
    let native_provider = known_native_indexer_provider_type(
        field_str(fields, "baseUrl"),
        field_str(fields, "apiPath"),
    );

    match implementation.trim().to_ascii_lowercase().as_str() {
        "newznab" => native_provider.or(Some("newznab")),
        "torznab" => None,
        _ => None,
    }
}

pub fn detect_prowlarr_proxy_indexer(indexer: &ArrIndexer) -> Option<DetectedProwlarrIndexer> {
    let implementation = indexer.implementation.trim().to_ascii_lowercase();
    if implementation != "newznab" && implementation != "torznab" {
        return None;
    }

    let api_path = field_str(&indexer.fields, "apiPath")?;
    if api_path.trim().trim_end_matches('/') != "/api" {
        return None;
    }

    let base_url = prowlarr_parent_base_url(&field_str(&indexer.fields, "baseUrl")?)?;
    Some(DetectedProwlarrIndexer {
        base_url,
        api_key: field_str_sensitive(&indexer.fields, "apiKey"),
        child_name: indexer.name.clone(),
    })
}

pub fn should_skip_imported_indexer(indexer: &ArrIndexer) -> bool {
    let implementation = indexer.implementation.trim().to_ascii_lowercase();
    if implementation != "newznab" && implementation != "torznab" {
        return false;
    }

    let Some(base_url) = field_str(&indexer.fields, "baseUrl") else {
        return false;
    };

    let normalized = base_url.trim().trim_end_matches('/').to_ascii_lowercase();
    normalized.contains("animetosho.org") || normalized.contains("feed.animetosho.org")
}

fn prowlarr_parent_base_url(base_url: &str) -> Option<String> {
    let mut url = url::Url::parse(base_url.trim()).ok()?;
    url.set_query(None);
    url.set_fragment(None);

    let path = url.path().trim_end_matches('/').to_string();
    let indexer_id = path.rsplit('/').next()?;
    if indexer_id.is_empty() || !indexer_id.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let parent_len = path.len().saturating_sub(indexer_id.len());
    let parent_path = path[..parent_len].trim_end_matches('/');
    url.set_path(parent_path);

    Some(url.as_str().trim_end_matches('/').to_string())
}

fn known_native_indexer_provider_type(
    base_url: Option<String>,
    api_path: Option<String>,
) -> Option<&'static str> {
    let endpoint = format!(
        "{} {}",
        base_url.unwrap_or_default().to_lowercase(),
        api_path.unwrap_or_default().to_lowercase()
    );

    if endpoint.contains("nzbgeek.info") {
        Some("nzbgeek")
    } else {
        None
    }
}

/// Supported Sonarr v4+ / Radarr v6+ builds replace sensitive field values with this placeholder
/// in all API responses (both list and individual GET endpoints).
const ARR_MASKED_VALUE: &str = "********";

/// Extract a string value from the flattened fields map.
pub fn field_str(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })
}

/// Like `field_str`, but returns `None` for values masked by Sonarr/Radarr
/// (`"********"`). Use this for sensitive fields such as `apiKey` and
/// `password` so that callers can detect when a real value was not returned.
pub fn field_str_sensitive(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() && s != ARR_MASKED_VALUE => Some(s.clone()),
        _ => None,
    })
}

/// Extract a string from the fields map, falling back to empty string for numeric values.
pub fn field_str_or_number(fields: &HashMap<String, Value>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// Extract a boolean from the fields map.
pub fn field_bool(fields: &HashMap<String, Value>, key: &str) -> Option<bool> {
    fields.get(key).and_then(|v| match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    })
}

fn value_str_or_number(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn value_i32(value: Option<&Value>) -> Option<i32> {
    value
        .and_then(Value::as_i64)
        .and_then(|value| value.try_into().ok())
}

fn flatten_arr_fields(fields: &[Value]) -> HashMap<String, Value> {
    fields
        .iter()
        .filter_map(|f| {
            let name = f.get("name")?.as_str()?.to_string();
            let value = f.get("value")?.clone();
            Some((name, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use super::{
        ArrIndexer, ExternalArrApiBucket, detect_prowlarr_proxy_indexer, map_download_client_type,
        map_indexer_provider_type, should_skip_imported_indexer,
    };

    #[test]
    fn sonarr_v4_bucket_accepts_sonarr_v4_status() {
        ExternalArrApiBucket::SonarrV4
            .validate_status("Sonarr", "4.0.17.2952")
            .expect("supported version");
    }

    #[test]
    fn sonarr_v4_bucket_accepts_supported_api_prefixes() {
        ExternalArrApiBucket::SonarrV4
            .validate_api_prefix("v3")
            .expect("legacy Sonarr v4 api prefix should be supported");
        ExternalArrApiBucket::SonarrV4
            .validate_api_prefix("v5")
            .expect("newer Sonarr api prefix should be supported");
    }

    #[test]
    fn radarr_v6_bucket_accepts_matching_major_version() {
        ExternalArrApiBucket::RadarrV6
            .validate_status("Radarr", "6.0.1.0")
            .expect("supported version");
    }

    #[test]
    fn map_download_client_type_recognizes_qbittorrent_variants() {
        assert_eq!(map_download_client_type("qBittorrent"), Some("qbittorrent"));
        assert_eq!(map_download_client_type("QBittorrent"), Some("qbittorrent"));
        assert_eq!(map_download_client_type("qbittorrent"), Some("qbittorrent"));
    }

    #[test]
    fn map_indexer_provider_type_does_not_map_animetosho_for_newznab() {
        assert_eq!(
            map_indexer_provider_type(
                "Newznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://feed.animetosho.org".into()),
                )]),
            ),
            Some("newznab")
        );
    }

    #[test]
    fn map_indexer_provider_type_does_not_map_animetosho_for_torznab() {
        assert_eq!(
            map_indexer_provider_type(
                "Torznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://feed.animetosho.org".into()),
                )]),
            ),
            None
        );
    }

    #[test]
    fn map_indexer_provider_type_keeps_generic_torznab_unsupported() {
        assert_eq!(
            map_indexer_provider_type(
                "Torznab",
                &HashMap::from([(
                    "baseUrl".into(),
                    Value::String("https://torznab.example.com".into()),
                )]),
            ),
            None
        );
    }

    #[test]
    fn should_skip_imported_indexer_skips_animetosho() {
        assert!(should_skip_imported_indexer(&ArrIndexer {
            id: 1,
            name: "AnimeTosho".into(),
            implementation: "Torznab".into(),
            fields: HashMap::from([(
                "baseUrl".into(),
                Value::String("https://feed.animetosho.org".into()),
            )]),
        }));
    }

    #[test]
    fn should_skip_imported_indexer_keeps_other_indexers() {
        assert!(!should_skip_imported_indexer(&ArrIndexer {
            id: 1,
            name: "NZBGeek".into(),
            implementation: "Newznab".into(),
            fields: HashMap::from([(
                "baseUrl".into(),
                Value::String("https://api.nzbgeek.info".into()),
            )]),
        }));
    }

    #[test]
    fn detects_prowlarr_proxy_indexer_and_preserves_reverse_proxy_path() {
        let detected = detect_prowlarr_proxy_indexer(&ArrIndexer {
            id: 1,
            name: "NZBGeek".into(),
            implementation: "Newznab".into(),
            fields: HashMap::from([
                (
                    "baseUrl".into(),
                    Value::String("https://media.example.test/prowlarr/12345/".into()),
                ),
                ("apiPath".into(), Value::String("/api".into())),
                ("apiKey".into(), Value::String("secret".into())),
            ]),
        })
        .expect("prowlarr proxy indexer");

        assert_eq!(detected.base_url, "https://media.example.test/prowlarr");
        assert_eq!(detected.api_key.as_deref(), Some("secret"));
        assert_eq!(detected.child_name, "NZBGeek");
    }

    #[test]
    fn detects_prowlarr_proxy_indexer_with_unbounded_numeric_id() {
        let detected = detect_prowlarr_proxy_indexer(&ArrIndexer {
            id: 1,
            name: "Large ID".into(),
            implementation: "Torznab".into(),
            fields: HashMap::from([
                (
                    "baseUrl".into(),
                    Value::String("http://prowlarr.local/123456".into()),
                ),
                ("apiPath".into(), Value::String("/api/".into())),
            ]),
        })
        .expect("prowlarr proxy indexer");

        assert_eq!(detected.base_url, "http://prowlarr.local");
        assert_eq!(detected.api_key, None);
    }

    #[test]
    fn ignores_non_prowlarr_shaped_arr_indexers() {
        assert!(
            detect_prowlarr_proxy_indexer(&ArrIndexer {
                id: 1,
                name: "Direct Newznab".into(),
                implementation: "Newznab".into(),
                fields: HashMap::from([
                    (
                        "baseUrl".into(),
                        Value::String("https://indexer.example/api".into()),
                    ),
                    ("apiPath".into(), Value::String("/api".into())),
                ]),
            })
            .is_none()
        );
    }
}
