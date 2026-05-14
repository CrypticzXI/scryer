use std::path::PathBuf;

use async_compression::Level;
use async_compression::tokio::{bufread::ZstdDecoder, write::GzipEncoder};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::multipart;
use scryer_application::{
    AppError, AppResult, DownloadClient, DownloadClientAddRequest, DownloadClientStatus,
    DownloadGrabResult, NullStagedNzbStore, StagedNzbRef, StagedNzbStore,
};
use scryer_domain::{CompletedDownload, DownloadQueueItem, DownloadQueueState};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, RateLimitRegistry, RequestPolicy, default_reqwest_client,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::Semaphore;
use tokio_util::io::ReaderStream;
use tracing::{debug, warn};

use super::{
    extract_f64_value, extract_i64_value, parse_duration_seconds, resolve_staged_nzb_for_request,
};

#[derive(Clone)]
pub struct SabnzbdDownloadClient {
    base_url: String,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
    outbound_http: OutboundHttpClient,
    staged_nzb_store: Arc<dyn StagedNzbStore>,
    staged_nzb_pipeline_limit: Arc<Semaphore>,
}

#[derive(Debug, Deserialize)]
struct SabnzbdConfigEnvelope {
    config: SabnzbdConfig,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdConfig {
    #[serde(default)]
    misc: SabnzbdConfigMisc,
    #[serde(default)]
    categories: Vec<SabnzbdCategory>,
    #[serde(default)]
    sorters: Vec<SabnzbdSorter>,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdConfigMisc {
    #[serde(default)]
    complete_dir: String,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    tv_categories: Vec<String>,
    #[serde(default)]
    enable_tv_sorting: bool,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    movie_categories: Vec<String>,
    #[serde(default)]
    enable_movie_sorting: bool,
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    date_categories: Vec<String>,
    #[serde(default)]
    enable_date_sorting: bool,
    #[serde(default)]
    history_retention: String,
    #[serde(default)]
    history_retention_option: String,
    #[serde(default)]
    history_retention_number: i64,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdCategory {
    #[serde(default, alias = "Name")]
    _name: String,
    #[serde(default, alias = "Dir")]
    dir: String,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdSorter {
    #[serde(default, deserialize_with = "deserialize_sab_string_list")]
    sort_cats: Vec<String>,
    #[serde(default)]
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct SabnzbdFullStatusEnvelope {
    status: SabnzbdFullStatus,
}

#[derive(Debug, Default, Deserialize)]
struct SabnzbdFullStatus {
    #[serde(default, rename = "completedir")]
    complete_dir: String,
}

impl SabnzbdDownloadClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            Some(api_key),
            None,
            None,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_auth(
        base_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
    ) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            api_key,
            username,
            password,
            Arc::new(NullStagedNzbStore),
            Arc::new(Semaphore::new(4)),
        )
    }

    pub fn with_staged_nzb_store(
        base_url: String,
        api_key: String,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        Self::with_auth_and_staged_nzb_store(
            base_url,
            Some(api_key),
            None,
            None,
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        )
    }

    pub fn with_auth_and_staged_nzb_store(
        base_url: String,
        api_key: Option<String>,
        username: Option<String>,
        password: Option<String>,
        staged_nzb_store: Arc<dyn StagedNzbStore>,
        staged_nzb_pipeline_limit: Arc<Semaphore>,
    ) -> Self {
        let http_client = default_reqwest_client();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: normalize_optional_auth_value(api_key),
            username: normalize_optional_auth_value(username),
            password: normalize_optional_auth_value(password),
            outbound_http: OutboundHttpClient::new(http_client.clone(), RateLimitRegistry::new()),
            staged_nzb_store,
            staged_nzb_pipeline_limit,
        }
    }

    fn sab_gzip_path(staged_nzb: &StagedNzbRef) -> PathBuf {
        staged_nzb
            .compressed_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!("{}.sab.gz.part", staged_nzb.id))
    }

    async fn build_transient_gzip_artifact(
        &self,
        staged_nzb: &StagedNzbRef,
    ) -> AppResult<(PathBuf, u64)> {
        let gzip_path = Self::sab_gzip_path(staged_nzb);
        let input = File::open(&staged_nzb.compressed_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to open staged nzb {}: {error}",
                    staged_nzb.compressed_path.display()
                ))
            })?;
        let output = File::create(&gzip_path).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create sabnzbd gzip file {}: {error}",
                gzip_path.display()
            ))
        })?;

        let mut decoder = ZstdDecoder::new(BufReader::new(input));
        let mut encoder = GzipEncoder::with_quality(BufWriter::new(output), Level::Fastest);
        tokio::io::copy(&mut decoder, &mut encoder)
            .await
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd nzb gzip compression failed: {error}"))
            })?;
        encoder.shutdown().await.map_err(|error| {
            AppError::Repository(format!("sabnzbd nzb gzip finalization failed: {error}"))
        })?;
        let mut writer = encoder.into_inner();
        writer.flush().await.map_err(|error| {
            AppError::Repository(format!("sabnzbd nzb gzip flush failed: {error}"))
        })?;

        let gzip_len = tokio::fs::metadata(&gzip_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to stat sabnzbd gzip file {}: {error}",
                    gzip_path.display()
                ))
            })?
            .len();

        Ok((gzip_path, gzip_len))
    }

    fn api_url(&self) -> String {
        format!("{}/api", self.base_url)
    }

    async fn api_get(&self, params: &[(&str, &str)]) -> AppResult<Value> {
        self.api_get_with_policy(params, self.read_policy("sabnzbd_api"))
            .await
    }

    async fn api_get_mutation(
        &self,
        params: &[(&str, &str)],
        request_label: &'static str,
    ) -> AppResult<Value> {
        self.api_get_with_policy(params, self.mutation_policy(request_label))
            .await
    }

    async fn api_get_with_policy(
        &self,
        params: &[(&str, &str)],
        policy: RequestPolicy,
    ) -> AppResult<Value> {
        let url = self.api_url();
        let mut query = self.auth_query_params()?;
        query.push(("output".to_string(), "json".to_string()));
        query.extend(
            params
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );

        let response = self
            .outbound_http
            .send(policy, || {
                self.outbound_http.client().get(&url).query(&query)
            })
            .await
            .map_err(|error| map_sabnzbd_outbound_error("sabnzbd api call", error))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|err| AppError::Repository(format!("sabnzbd response read failed: {err}")))?;

        if !status.is_success() {
            let preview = body.chars().take(600).collect::<String>();
            return Err(AppError::Repository(format!(
                "sabnzbd api returned status {status}: {preview}"
            )));
        }

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!("sabnzbd returned non-json response: {err}"))
        })?;

        if let Some(false) = json.get("status").and_then(Value::as_bool) {
            let error_msg = json
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(AppError::Repository(format!(
                "sabnzbd api error: {error_msg}"
            )));
        }

        Ok(json)
    }

    async fn history_slots_page(&self, start: usize, limit: usize) -> AppResult<Vec<Value>> {
        let start_param = start.to_string();
        let limit_param = limit.to_string();
        let json = self
            .api_get(&[
                ("mode", "history"),
                ("start", start_param.as_str()),
                ("limit", limit_param.as_str()),
            ])
            .await?;

        Ok(json
            .get("history")
            .and_then(|history| history.get("slots"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_config(&self) -> AppResult<SabnzbdConfig> {
        let json = self.api_get(&[("mode", "get_config")]).await?;
        serde_json::from_value::<SabnzbdConfigEnvelope>(json)
            .map(|response| response.config)
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd config response parse failed: {error}"))
            })
    }

    async fn get_full_status(&self) -> AppResult<SabnzbdFullStatus> {
        let json = self
            .api_get(&[("mode", "fullstatus"), ("skip_dashboard", "1")])
            .await?;
        serde_json::from_value::<SabnzbdFullStatusEnvelope>(json)
            .map(|response| response.status)
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd fullstatus response parse failed: {error}"))
            })
    }

    fn auth_query_params(&self) -> AppResult<Vec<(String, String)>> {
        if let Some(api_key) = self.api_key.as_ref() {
            return Ok(vec![("apikey".to_string(), api_key.clone())]);
        }

        match (self.username.as_ref(), self.password.as_ref()) {
            (Some(username), Some(password)) => Ok(vec![
                ("ma_username".to_string(), username.clone()),
                ("ma_password".to_string(), password.clone()),
            ]),
            _ => Err(AppError::Validation(
                "sabnzbd requires an API key or username/password".to_string(),
            )),
        }
    }

    fn derive_sorting_mode(config: &SabnzbdConfig) -> Option<String> {
        if config
            .sorters
            .iter()
            .any(|sorter| sorter.is_active && !sorter.sort_cats.is_empty())
            || (config.misc.enable_tv_sorting && !config.misc.tv_categories.is_empty())
        {
            return Some("TV".to_string());
        }

        if config.misc.enable_movie_sorting && !config.misc.movie_categories.is_empty() {
            return Some("Movie".to_string());
        }

        if config.misc.enable_date_sorting && !config.misc.date_categories.is_empty() {
            return Some("Date".to_string());
        }

        None
    }

    fn removes_completed_downloads(config: &SabnzbdConfig) -> bool {
        match config.misc.history_retention_option.as_str() {
            "all" => false,
            "number-archive" | "number-delete" | "all-archive" | "all-delete" => true,
            "days-archive" | "days-delete" => config.misc.history_retention_number < 14,
            _ => {
                let retention = config.misc.history_retention.trim();
                if retention.is_empty() {
                    return false;
                }

                if let Some(days) = retention.strip_suffix('d') {
                    return days.parse::<i64>().unwrap_or(i64::MAX) < 14;
                }

                retention != "0"
            }
        }
    }

    fn output_roots_from_config(
        &self,
        config: &SabnzbdConfig,
        full_status: Option<&SabnzbdFullStatus>,
    ) -> Vec<String> {
        let complete_dir = resolved_complete_dir(&config.misc.complete_dir, full_status);
        let mut roots = Vec::new();

        if !complete_dir.is_empty() {
            roots.push(complete_dir.clone());
        }

        for category in &config.categories {
            let path = category_output_root(&complete_dir, &category.dir);
            if !path.is_empty() {
                roots.push(path);
            }
        }

        dedupe_strings(roots)
    }

    pub async fn test_connection(&self) -> AppResult<String> {
        // First check connectivity with unauthenticated version call
        let url = self.api_url();
        let response = self
            .outbound_http
            .send(self.read_policy("sabnzbd_test_connection"), || {
                self.outbound_http
                    .client()
                    .get(&url)
                    .query(&[("mode", "version"), ("output", "json")])
            })
            .await
            .map_err(|error| map_sabnzbd_outbound_error("sabnzbd test call", error))?;

        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "sabnzbd test call returned status {status}"
            )));
        }

        let body = response.text().await.map_err(|err| {
            AppError::Repository(format!("sabnzbd test response read failed: {err}"))
        })?;

        let json: Value = serde_json::from_str(&body).map_err(|err| {
            AppError::Repository(format!(
                "sabnzbd test call returned non-json response: {err}"
            ))
        })?;

        let version = json
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("sabnzbd")
            .to_string();

        // Check version >= 3.0.0
        let mut warnings = Vec::new();
        let version_parts: Vec<u32> = version.split('.').filter_map(|p| p.parse().ok()).collect();
        if version_parts.len() >= 2 && version_parts[0] < 3 {
            warnings.push(format!(
                "SABnzbd {version} is outdated; version 3.0.0+ is recommended"
            ));
        }

        // Validate the API key by making an authenticated request
        self.api_get(&[("mode", "queue"), ("limit", "0")])
            .await
            .map_err(|err| {
                AppError::Repository(format!("sabnzbd authentication validation failed: {err}"))
            })?;

        if warnings.is_empty() {
            Ok(version)
        } else {
            Ok(format!("{version} ({})", warnings.join("; ")))
        }
    }
}

#[async_trait]
impl DownloadClient for SabnzbdDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let title = &request.title;
        let nzb_name = request
            .source_title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(title.name.as_str());

        let staged = resolve_staged_nzb_for_request(
            &self.outbound_http,
            &self.staged_nzb_store,
            &self.staged_nzb_pipeline_limit,
            request,
        )
        .await?;
        let mut transient_gzip_path: Option<PathBuf> = None;

        let result: AppResult<DownloadGrabResult> = async {
            let (gzip_path, gzip_len) = self
                .build_transient_gzip_artifact(&staged.staged_nzb)
                .await?;
            transient_gzip_path = Some(gzip_path.clone());
            self.staged_nzb_store.mark_artifact_active(&gzip_path)?;

            let url = self.api_url();
            let cat = request
                .category
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let password = request
                .source_password
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty() && *value != "0")
                .map(str::to_string);
            let nzb_name_owned = nzb_name.to_string();
            let queue_priority =
                sabnzbd_queue_priority(request.queue_priority.as_deref()).to_string();
            let gzip_path_for_request = gzip_path.clone();
            let nzb_filename_for_request = if nzb_name.to_ascii_lowercase().ends_with(".nzb") {
                format!("{nzb_name}.gz")
            } else {
                format!("{nzb_name}.nzb.gz")
            };
            let response = self
                .outbound_http
                .send_async(self.mutation_policy("sabnzbd_addfile"), || {
                    let url = url.clone();
                    let nzb_name_owned = nzb_name_owned.clone();
                    let queue_priority = queue_priority.clone();
                    let gzip_path_for_request = gzip_path_for_request.clone();
                    let nzb_filename_for_request = nzb_filename_for_request.clone();
                    let cat = cat.clone();
                    let password = password.clone();
                    let auth_api_key = self.api_key.clone();
                    let auth_username = self.username.clone();
                    let auth_password = self.password.clone();
                    async move {
                        let gzip_file =
                            File::open(&gzip_path_for_request).await.map_err(|error| {
                                AppError::Repository(format!(
                                    "failed to reopen sabnzbd gzip file {}: {error}",
                                    gzip_path_for_request.display()
                                ))
                            })?;
                        let nzb_part = multipart::Part::stream_with_length(
                            reqwest::Body::wrap_stream(ReaderStream::new(gzip_file)),
                            gzip_len,
                        )
                        .file_name(nzb_filename_for_request)
                        .mime_str("application/gzip")
                        .map_err(|err| {
                            AppError::Repository(format!("sabnzbd multipart build failed: {err}"))
                        })?;

                        let mut form = multipart::Form::new()
                            .text("output", "json")
                            .text("mode", "addfile")
                            .text("nzbname", nzb_name_owned)
                            .text("priority", queue_priority)
                            .part("nzbfile", nzb_part);
                        form = if let Some(api_key) = auth_api_key {
                            form.text("apikey", api_key)
                        } else if let (Some(username), Some(password)) =
                            (auth_username, auth_password)
                        {
                            form.text("ma_username", username)
                                .text("ma_password", password)
                        } else {
                            return Err(AppError::Validation(
                                "sabnzbd requires an API key or username/password".to_string(),
                            ));
                        };

                        if let Some(cat) = cat {
                            form = form.text("cat", cat);
                        }
                        if let Some(password) = password {
                            form = form.text("password", password);
                        }

                        Ok::<_, AppError>(self.outbound_http.client().post(&url).multipart(form))
                    }
                })
                .await
                .map_err(|error| match error {
                    scryer_outbound_http::OutboundRequestError::Build(error) => error,
                    scryer_outbound_http::OutboundRequestError::Http(error) => {
                        map_sabnzbd_outbound_error("sabnzbd addfile call", error)
                    }
                })?;

            let status = response.status();
            let body = response.text().await.map_err(|err| {
                AppError::Repository(format!("sabnzbd addfile response read failed: {err}"))
            })?;

            if !status.is_success() {
                let preview = body.chars().take(600).collect::<String>();
                return Err(AppError::Repository(format!(
                    "sabnzbd addfile returned status {status}: {preview}"
                )));
            }

            let json: Value = serde_json::from_str(&body).map_err(|err| {
                AppError::Repository(format!("sabnzbd addfile returned non-json response: {err}"))
            })?;

            if let Some(false) = json.get("status").and_then(Value::as_bool) {
                let error_msg = json
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(AppError::Repository(format!(
                    "sabnzbd addfile error: {error_msg}"
                )));
            }

            let nzo_id = json
                .get("nzo_ids")
                .and_then(Value::as_array)
                .and_then(|ids| ids.first())
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    AppError::Repository("sabnzbd addfile did not return an nzo_id".into())
                })?;

            debug!(
                nzo_id = nzo_id.as_str(),
                title = title.name.as_str(),
                nzb_name = nzb_name,
                "sabnzbd addfile succeeded"
            );

            Ok(DownloadGrabResult {
                job_id: nzo_id,
                client_id: None,
                client_type: "sabnzbd".to_string(),
            })
        }
        .await;

        if let Some(gzip_path) = transient_gzip_path {
            if let Err(error) = self.staged_nzb_store.mark_artifact_inactive(&gzip_path) {
                warn!(
                    path = %gzip_path.display(),
                    error = %error,
                    "failed to mark transient sabnzbd gzip artifact inactive"
                );
            }
            if let Err(error) = tokio::fs::remove_file(&gzip_path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %gzip_path.display(),
                    error = %error,
                    "failed to delete transient sabnzbd gzip artifact"
                );
            }
        }

        if staged.self_staged
            && let Err(error) = self
                .staged_nzb_store
                .delete_staged_nzb(&staged.staged_nzb)
                .await
        {
            warn!(
                staged_nzb_id = staged.staged_nzb.id.as_str(),
                error = %error,
                "failed to delete self-staged sabnzbd nzb artifact"
            );
        }

        result
    }

    async fn test_connection(&self) -> AppResult<String> {
        SabnzbdDownloadClient::test_connection(self).await
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let json = self.api_get(&[("mode", "queue")]).await?;

        let slots = json
            .get("queue")
            .and_then(|q| q.get("slots"))
            .and_then(Value::as_array);

        let slots = match slots {
            Some(s) => s,
            None => return Ok(Vec::new()),
        };

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let raw_filename = slot
                    .get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download");
                let (title_name, is_encrypted) =
                    if let Some(stripped) = raw_filename.strip_prefix("ENCRYPTED / ") {
                        (stripped.to_string(), true)
                    } else {
                        (raw_filename.to_string(), false)
                    };

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let state = sabnzbd_queue_state(status);

                let percentage = slot
                    .get("percentage")
                    .and_then(|v| v.as_str().or_else(|| v.as_u64().map(|_| "")))
                    .and_then(|s| {
                        if s.is_empty() {
                            slot.get("percentage")
                                .and_then(Value::as_u64)
                                .map(|v| v as u8)
                        } else {
                            s.parse::<u8>().ok()
                        }
                    })
                    .unwrap_or(0);

                let size_bytes = extract_f64_value(slot.get("mb")).map(|mb| {
                    if !mb.is_finite() || mb <= 0.0 {
                        0
                    } else {
                        (mb * 1_048_576f64).round() as i64
                    }
                });

                let remaining_seconds = slot
                    .get("timeleft")
                    .and_then(Value::as_str)
                    .and_then(parse_duration_seconds);

                let pp_status = if state == DownloadQueueState::Downloading {
                    sabnzbd_postprocessing_stage(status)
                } else {
                    None
                };

                let attention_required = is_encrypted;
                let attention_reason = if is_encrypted {
                    Some("ENCRYPTED".to_string())
                } else {
                    pp_status
                };

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: percentage,
                    size_bytes,
                    remaining_seconds,
                    queued_at: None,
                    last_updated_at: None,
                    attention_required,
                    attention_reason,
                    download_client_item_id: nzo_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                })
            })
            .collect())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        let slots = self.history_slots_page(0, 50).await?;
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let title_name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let (state, mut attention_reason) = sabnzbd_history_state(status);

                // SABnzbd provides a dedicated fail_message field with the actual
                // failure detail (e.g. "54 articles were missing"). Use it when the
                // status line alone didn't produce a reason.
                if state == DownloadQueueState::Failed && attention_reason.is_none() {
                    attention_reason = slot
                        .get("fail_message")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    size_bytes: extract_i64_value(slot.get("bytes")),
                    remaining_seconds: None,
                    queued_at: extract_i64_value(slot.get("time_added")).map(|v| v.to_string()),
                    last_updated_at: completed_ts.map(|v| v.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzo_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                })
            })
            .collect())
    }

    async fn list_history_page(
        &self,
        offset: usize,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        let slots = self.history_slots_page(offset, limit).await?;
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let title_name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                let (state, mut attention_reason) = sabnzbd_history_state(status);

                if state == DownloadQueueState::Failed && attention_reason.is_none() {
                    attention_reason = slot
                        .get("fail_message")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                }

                Some(DownloadQueueItem {
                    id: nzo_id.clone(),
                    title_id: None,
                    episode_id: None,
                    title_name,
                    facet: None,
                    client_id: String::new(),
                    client_name: String::new(),
                    client_type: "sabnzbd".to_string(),
                    state,
                    progress_percent: if state == DownloadQueueState::Completed {
                        100
                    } else {
                        0
                    },
                    size_bytes: extract_i64_value(slot.get("bytes")),
                    remaining_seconds: None,
                    queued_at: extract_i64_value(slot.get("time_added"))
                        .map(|value| value.to_string()),
                    last_updated_at: completed_ts.map(|value| value.to_string()),
                    attention_required: matches!(state, DownloadQueueState::Failed),
                    attention_reason,
                    download_client_item_id: nzo_id,
                    import_status: None,
                    import_error_code: None,
                    import_error_message: None,
                    imported_at: None,
                    delete_status: None,
                    delete_error_message: None,
                    is_scryer_origin: false,
                    tracked_state: None,
                    tracked_status: None,
                    tracked_status_messages: Vec::new(),
                    tracked_match_type: None,
                })
            })
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        let slots = self.history_slots_page(0, 50).await?;
        let cutoff_ts = Utc::now().timestamp() - (7 * 24 * 60 * 60);

        Ok(slots
            .iter()
            .filter_map(|slot| {
                let slot = slot.as_object()?;

                let status = slot.get("status").and_then(Value::as_str).unwrap_or("");
                if !status.eq_ignore_ascii_case("Completed") {
                    return None;
                }

                let nzo_id = slot.get("nzo_id").and_then(Value::as_str)?.to_string();

                let completed_ts = extract_i64_value(slot.get("completed"));
                if let Some(ts) = completed_ts
                    && ts < cutoff_ts
                {
                    return None;
                }

                let dest_dir = slot
                    .get("storage")
                    .or_else(|| slot.get("path"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();

                if dest_dir.is_empty() {
                    return None;
                }

                let name = slot
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed download")
                    .to_string();

                let category = slot
                    .get("category")
                    .and_then(Value::as_str)
                    .filter(|c| !c.is_empty() && *c != "*")
                    .map(str::to_string);

                let size_bytes = extract_i64_value(slot.get("bytes"));

                let completed_at =
                    completed_ts.map(|ts| DateTime::from_timestamp(ts, 0).unwrap_or_else(Utc::now));

                Some(CompletedDownload {
                    client_type: "sabnzbd".to_string(),
                    client_id: String::new(),
                    download_client_item_id: nzo_id,
                    name,
                    dest_dir,
                    category,
                    size_bytes,
                    completed_at,
                    parameters: Vec::new(),
                })
            })
            .collect())
    }

    async fn get_client_status(&self) -> AppResult<DownloadClientStatus> {
        let config = self.get_config().await?;
        let full_status = self.get_full_status().await.ok();

        Ok(DownloadClientStatus {
            version: None,
            is_localhost: is_localhost_base_url(&self.base_url),
            remote_output_roots: self.output_roots_from_config(&config, full_status.as_ref()),
            removes_completed_downloads: Some(Self::removes_completed_downloads(&config)),
            sorting_mode: Self::derive_sorting_mode(&config),
            warnings: Vec::new(),
        })
    }

    async fn pause_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get_mutation(
            &[("mode", "queue"), ("name", "pause"), ("value", id)],
            "sabnzbd_pause_queue_item",
        )
        .await?;
        Ok(())
    }

    async fn resume_queue_item(&self, id: &str) -> AppResult<()> {
        self.api_get_mutation(
            &[("mode", "queue"), ("name", "resume"), ("value", id)],
            "sabnzbd_resume_queue_item",
        )
        .await?;
        Ok(())
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if is_history {
            self.api_get_mutation(
                &[("mode", "history"), ("name", "delete"), ("value", id)],
                "sabnzbd_delete_history_item",
            )
            .await?;
        } else {
            self.api_get_mutation(
                &[
                    ("mode", "queue"),
                    ("name", "delete"),
                    ("value", id),
                    ("del_files", "1"),
                ],
                "sabnzbd_delete_queue_item",
            )
            .await?;
        }
        Ok(())
    }
}

impl SabnzbdDownloadClient {
    fn read_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::safe_read(format!("sabnzbd:{}", self.base_url), request_label)
            .with_max_retries(2)
            .with_backoff(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(15),
            )
    }

    fn mutation_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::no_retry(format!("sabnzbd:{}", self.base_url), request_label).with_backoff(
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(15),
        )
    }
}

fn map_sabnzbd_outbound_error(operation: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => AppError::Repository(
            match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                Some(delay) => {
                    format!(
                        "{operation} was rate limited; retry after {}s",
                        delay.as_secs()
                    )
                }
                None => format!("{operation} was rate limited"),
            },
        ),
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("{operation} failed: {source}"))
        }
    }
}

fn sabnzbd_queue_priority(raw_priority: Option<&str>) -> i32 {
    match raw_priority
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("force") => 2,
        Some("very high") | Some("high") => 1,
        Some("normal") => 0,
        Some("low") | Some("very low") => -1,
        _ => -1,
    }
}

fn sabnzbd_queue_state(status: &str) -> DownloadQueueState {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "DOWNLOADING" => DownloadQueueState::Downloading,
        "QUEUED" | "FETCHING" | "PROPAGATING" | "GRABBING" => DownloadQueueState::Queued,
        "PAUSED" => DownloadQueueState::Paused,
        // Post-processing stages reported in queue (SABnzbd 4.x can show these)
        "VERIFYING" | "QUICKCHECK" => DownloadQueueState::Verifying,
        "REPAIRING" => DownloadQueueState::Repairing,
        "EXTRACTING" => DownloadQueueState::Extracting,
        "MOVING" | "RUNNING" => DownloadQueueState::Downloading,
        _ => DownloadQueueState::Queued,
    }
}

fn sabnzbd_postprocessing_stage(status: &str) -> Option<String> {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "VERIFYING" | "QUICKCHECK" => Some("VERIFYING".to_string()),
        "REPAIRING" => Some("REPAIRING".to_string()),
        "EXTRACTING" => Some("UNPACKING".to_string()),
        "MOVING" => Some("MOVING".to_string()),
        "RUNNING" => Some("EXECUTING_SCRIPT".to_string()),
        _ => None,
    }
}

fn sabnzbd_history_state(status: &str) -> (DownloadQueueState, Option<String>) {
    let normalized = status.to_ascii_uppercase();
    match normalized.as_str() {
        "COMPLETED" => (DownloadQueueState::Completed, None),
        "FAILED" => (DownloadQueueState::Failed, None),
        "QUEUED" => (DownloadQueueState::Queued, None),
        // Active post-processing stages in history
        "VERIFYING" | "QUICKCHECK" => (DownloadQueueState::Verifying, None),
        "REPAIRING" => (DownloadQueueState::Repairing, None),
        "EXTRACTING" => (DownloadQueueState::Extracting, None),
        "MOVING" | "RUNNING" => (DownloadQueueState::Downloading, None),
        _ => {
            if normalized.starts_with("FAILED") {
                let reason = status
                    .split_once(" - ")
                    .map(|(_, detail)| detail.trim().to_string())
                    .filter(|d| !d.is_empty());
                (DownloadQueueState::Failed, reason)
            } else {
                (DownloadQueueState::Completed, None)
            }
        }
    }
}

fn deserialize_sab_string_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(match value {
        Value::Null => Vec::new(),
        Value::String(value) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Value::Array(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    })
}

fn normalize_optional_auth_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if deduped.iter().any(|existing| existing == trimmed) {
            continue;
        }
        deduped.push(trimmed.to_string());
    }
    deduped
}

fn resolved_complete_dir(
    configured_complete_dir: &str,
    full_status: Option<&SabnzbdFullStatus>,
) -> String {
    let complete_dir = configured_complete_dir.trim();
    if complete_dir.is_empty() {
        return full_status
            .map(|status| status.complete_dir.trim().to_string())
            .unwrap_or_default();
    }

    let configured_path = std::path::Path::new(complete_dir);
    if configured_path.is_absolute() {
        return complete_dir.to_string();
    }

    full_status
        .map(|status| status.complete_dir.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| complete_dir.to_string())
}

fn category_output_root(complete_dir: &str, category_dir: &str) -> String {
    let trimmed_dir = category_dir.trim().trim_end_matches('*');
    if trimmed_dir.is_empty() {
        return complete_dir.trim().to_string();
    }
    if complete_dir.trim().is_empty() {
        return trimmed_dir.to_string();
    }

    join_output_root(complete_dir, trimmed_dir)
}

fn join_output_root(base: &str, suffix: &str) -> String {
    let base = base.trim().trim_end_matches(['/', '\\']);
    let suffix = suffix.trim().trim_start_matches(['/', '\\']);
    if base.is_empty() {
        return suffix.to_string();
    }
    if suffix.is_empty() {
        return base.to_string();
    }
    format!("{base}/{suffix}")
}

fn is_localhost_base_url(base_url: &str) -> Option<bool> {
    let parsed = reqwest::Url::parse(base_url).ok()?;
    let host = parsed.host_str()?;
    Some(matches!(
        host,
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "host.docker.internal"
    ))
}
