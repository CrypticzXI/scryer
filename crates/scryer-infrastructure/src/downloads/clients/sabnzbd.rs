use std::path::PathBuf;

use async_compression::tokio::bufread::ZstdDecoder;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::{StatusCode, Url, multipart};
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

#[derive(Clone)]
enum SabApiAuth {
    ApiKey(String),
    Credentials { username: String, password: String },
}

#[derive(Clone)]
enum SabAddfilePayload {
    File { path: PathBuf, len: u64 },
}

struct SabAddfileRequest<'a> {
    url: &'a str,
    nzb_name: &'a str,
    queue_priority: &'a str,
    upload_payload: SabAddfilePayload,
    upload_filename: &'a str,
    upload_mime: &'a str,
    cat: Option<&'a str>,
    password: Option<&'a str>,
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

    fn sab_nzb_path(staged_nzb: &StagedNzbRef) -> PathBuf {
        staged_nzb
            .compressed_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!("{}.sab.nzb.part", staged_nzb.id))
    }

    async fn build_transient_nzb_artifact(
        &self,
        staged_nzb: &StagedNzbRef,
    ) -> AppResult<(PathBuf, u64)> {
        let nzb_path = Self::sab_nzb_path(staged_nzb);
        let input = File::open(&staged_nzb.compressed_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to open staged nzb {}: {error}",
                    staged_nzb.compressed_path.display()
                ))
            })?;
        let mut output = File::create(&nzb_path).await.map_err(|error| {
            AppError::Repository(format!(
                "failed to create sabnzbd nzb file {}: {error}",
                nzb_path.display()
            ))
        })?;

        let mut decoder = ZstdDecoder::new(BufReader::new(input));
        tokio::io::copy(&mut decoder, &mut output)
            .await
            .map_err(|error| {
                AppError::Repository(format!("sabnzbd nzb decompression failed: {error}"))
            })?;
        output
            .flush()
            .await
            .map_err(|error| AppError::Repository(format!("sabnzbd nzb flush failed: {error}")))?;

        let nzb_len = tokio::fs::metadata(&nzb_path)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to stat sabnzbd nzb file {}: {error}",
                    nzb_path.display()
                ))
            })?
            .len();

        Ok((nzb_path, nzb_len))
    }

    fn api_urls(&self) -> Vec<String> {
        build_sab_api_urls(&self.base_url)
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
        let urls = self.api_urls();
        let request_mode = params
            .iter()
            .find_map(|(key, value)| (*key == "mode").then_some(*value));
        let mut form_or_query = vec![("output".to_string(), "json".to_string())];
        form_or_query.extend(
            params
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string())),
        );

        let auth = self.api_auth()?;
        let mut last_retryable_error = None;
        for (index, url) in urls.iter().enumerate() {
            let response = match &auth {
                SabApiAuth::ApiKey(api_key) => {
                    let mut form_or_query = form_or_query.clone();
                    form_or_query.push(("apikey".to_string(), api_key.clone()));
                    self.outbound_http
                        .send(policy.clone(), || {
                            self.outbound_http.client().get(url).query(&form_or_query)
                        })
                        .await
                }
                SabApiAuth::Credentials { username, password } => {
                    let mut form_or_query = form_or_query.clone();
                    form_or_query.push(("ma_username".to_string(), username.clone()));
                    form_or_query.push(("ma_password".to_string(), password.clone()));
                    let encoded_form = url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(
                            form_or_query
                                .iter()
                                .map(|(key, value)| (key.as_str(), value.as_str())),
                        )
                        .finish();
                    self.outbound_http
                        .send(policy.clone(), || {
                            self.outbound_http
                                .client()
                                .post(url)
                                .header("Content-Type", "application/x-www-form-urlencoded")
                                .body(encoded_form.clone())
                        })
                        .await
                }
            }
            .map_err(|error| map_sabnzbd_outbound_error("sabnzbd api call", error))?;

            let status = response.status();
            let body = response.text().await.map_err(|err| {
                AppError::Repository(format!("sabnzbd response read failed: {err}"))
            })?;

            match evaluate_sab_api_response("sabnzbd api", request_mode, status, &body) {
                SabApiResponseEvaluation::Success(json) => return Ok(json),
                SabApiResponseEvaluation::Retry(error) if index + 1 < urls.len() => {
                    debug!(
                        request_mode,
                        url,
                        error = %error,
                        "retrying sab-compatible endpoint with alternate api path"
                    );
                    last_retryable_error = Some(error);
                }
                SabApiResponseEvaluation::Retry(error)
                | SabApiResponseEvaluation::Failure(error) => {
                    return Err(error);
                }
            }
        }
        Err(last_retryable_error.unwrap_or_else(|| {
            AppError::Repository("sabnzbd api call did not return a usable response".to_string())
        }))
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
            .and_then(slots_from_api_section)
            .or_else(|| json.get("slots").and_then(Value::as_array))
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

    fn api_auth(&self) -> AppResult<SabApiAuth> {
        if let Some(api_key) = self.api_key.as_ref() {
            return Ok(SabApiAuth::ApiKey(api_key.clone()));
        }

        match (self.username.as_ref(), self.password.as_ref()) {
            (Some(username), Some(password)) => Ok(SabApiAuth::Credentials {
                username: username.clone(),
                password: password.clone(),
            }),
            _ => Err(AppError::Validation(
                "sabnzbd requires an API key or username/password".to_string(),
            )),
        }
    }

    fn api_auth_strategy_label(&self) -> &'static str {
        if self.api_key.is_some() {
            "api_key"
        } else if self.username.is_some() && self.password.is_some() {
            "credentials"
        } else {
            "missing"
        }
    }

    async fn post_addfile_request(
        &self,
        request: SabAddfileRequest<'_>,
    ) -> AppResult<(reqwest::StatusCode, String)> {
        let auth = self.api_auth()?;
        let upload_payload = request.upload_payload.clone();
        let upload_filename = request.upload_filename.to_string();
        let url = request.url.to_string();
        let nzb_name = request.nzb_name.to_string();
        let queue_priority = request.queue_priority.to_string();
        let upload_mime = request.upload_mime.to_string();
        let cat = request.cat.map(str::to_string);
        let password = request.password.map(str::to_string);
        let auth_strategy = self.api_auth_strategy_label();

        debug!(
            request_mode = "addfile",
            auth_strategy,
            has_category = cat.is_some(),
            has_password = password.is_some(),
            "building sabnzbd enqueue request"
        );

        let response = self
            .outbound_http
            .send_async(self.mutation_policy("sabnzbd_addfile"), move || {
                let auth = auth.clone();
                let url = url.clone();
                let nzb_name = nzb_name.clone();
                let queue_priority = queue_priority.clone();
                let upload_payload = upload_payload.clone();
                let upload_filename = upload_filename.clone();
                let upload_mime = upload_mime.clone();
                let cat = cat.clone();
                let password = password.clone();
                async move {
                    let nzb_part = match upload_payload {
                        SabAddfilePayload::File { path, len } => {
                            let upload_file = File::open(&path).await.map_err(|error| {
                                AppError::Repository(format!(
                                    "failed to reopen sabnzbd upload file {}: {error}",
                                    path.display()
                                ))
                            })?;
                            multipart::Part::stream_with_length(
                                reqwest::Body::wrap_stream(ReaderStream::new(upload_file)),
                                len,
                            )
                        }
                    }
                    .file_name(upload_filename)
                    .mime_str(&upload_mime)
                    .map_err(|err| {
                        AppError::Repository(format!("sabnzbd multipart build failed: {err}"))
                    })?;

                    let mut form = multipart::Form::new()
                        .text("nzbname", nzb_name)
                        .text("priority", queue_priority)
                        .part("nzbfile", nzb_part);
                    let request_builder = match auth {
                        SabApiAuth::ApiKey(api_key) => {
                            self.outbound_http.client().post(&url).query(&[
                                ("mode", "addfile"),
                                ("output", "json"),
                                ("apikey", api_key.as_str()),
                            ])
                        }
                        SabApiAuth::Credentials { username, password } => {
                            self.outbound_http.client().post(&url).query(&[
                                ("mode", "addfile"),
                                ("output", "json"),
                                ("ma_username", username.as_str()),
                                ("ma_password", password.as_str()),
                            ])
                        }
                    };

                    if let Some(cat) = cat {
                        form = form.text("cat", cat);
                    }
                    if let Some(password) = password {
                        form = form.text("password", password);
                    }

                    Ok::<_, AppError>(request_builder.multipart(form))
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

        Ok((status, body))
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
        let json = self
            .api_get_with_policy(
                &[("mode", "version")],
                self.read_policy("sabnzbd_test_connection"),
            )
            .await?;

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

        // Validate the configured auth mode by making an authenticated request.
        self.api_get(&[("mode", "queue"), ("limit", "0")])
            .await
            .map_err(map_sabnzbd_auth_validation_error)?;

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
        let mut transient_nzb_path: Option<PathBuf> = None;

        let result: AppResult<DownloadGrabResult> = async {
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
            let (nzb_path, nzb_len) = self
                .build_transient_nzb_artifact(&staged.staged_nzb)
                .await?;
            self.staged_nzb_store.mark_artifact_active(&nzb_path)?;
            transient_nzb_path = Some(nzb_path.clone());

            let plain_nzb_filename = if nzb_name.to_ascii_lowercase().ends_with(".nzb") {
                nzb_name.to_string()
            } else {
                format!("{nzb_name}.nzb")
            };

            let urls = self.api_urls();
            let mut addfile_json = None;
            let mut last_retryable_error = None;
            for (index, url) in urls.iter().enumerate() {
                let (status, body) = self
                    .post_addfile_request(SabAddfileRequest {
                        url,
                        nzb_name: &nzb_name_owned,
                        queue_priority: &queue_priority,
                        upload_payload: SabAddfilePayload::File {
                            path: nzb_path.clone(),
                            len: nzb_len,
                        },
                        upload_filename: &plain_nzb_filename,
                        upload_mime: "application/x-nzb",
                        cat: cat.as_deref(),
                        password: password.as_deref(),
                    })
                    .await
                    .map_err(|error| {
                        debug!(
                            request_mode = "addfile",
                            auth_strategy = self.api_auth_strategy_label(),
                            title = title.name.as_str(),
                            error = %error,
                            "sabnzbd enqueue request failed before response"
                        );
                        error
                    })?;

                match evaluate_sab_api_response("sabnzbd addfile", Some("addfile"), status, &body) {
                    SabApiResponseEvaluation::Success(json) => {
                        addfile_json = Some(json);
                        break;
                    }
                    SabApiResponseEvaluation::Retry(error) if index + 1 < urls.len() => {
                        debug!(
                            request_mode = "addfile",
                            auth_strategy = self.api_auth_strategy_label(),
                            title = title.name.as_str(),
                            url,
                            error = %error,
                            "retrying sab-compatible enqueue with alternate api path"
                        );
                        last_retryable_error = Some(error);
                    }
                    SabApiResponseEvaluation::Retry(error)
                    | SabApiResponseEvaluation::Failure(error) => return Err(error),
                }
            }

            let json = addfile_json.ok_or_else(|| {
                last_retryable_error.unwrap_or_else(|| {
                    AppError::Repository(
                        "sabnzbd addfile did not return a usable response".to_string(),
                    )
                })
            })?;

            let nzo_id = sab_addfile_nzo_id(&json)
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

        if let Some(nzb_path) = transient_nzb_path {
            if let Err(error) = self.staged_nzb_store.mark_artifact_inactive(&nzb_path) {
                warn!(
                    path = %nzb_path.display(),
                    error = %error,
                    "failed to mark transient sabnzbd nzb artifact inactive"
                );
            }
            if let Err(error) = tokio::fs::remove_file(&nzb_path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    path = %nzb_path.display(),
                    error = %error,
                    "failed to delete transient sabnzbd nzb artifact"
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
            .and_then(slots_from_api_section)
            .or_else(|| json.get("slots").and_then(Value::as_array));

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

fn map_sabnzbd_auth_validation_error(error: AppError) -> AppError {
    match error {
        AppError::Repository(message) => AppError::Repository(format!(
            "sabnzbd authentication validation failed: {message}"
        )),
        AppError::Validation(message) => AppError::Repository(format!(
            "sabnzbd authentication validation failed: {message}"
        )),
        other => AppError::Repository(format!("sabnzbd authentication validation failed: {other}")),
    }
}

fn map_sabnzbd_response_error(operation: &str, status: StatusCode, body: &str) -> AppError {
    let detail = extract_sab_error_detail(body);
    if status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || is_sab_auth_error_message(&detail)
    {
        return AppError::Repository(format!("sabnzbd authentication failed: {detail}"));
    }

    AppError::Repository(format!("{operation} returned status {status}: {detail}"))
}

fn map_sabnzbd_api_error(operation: &str, status: Option<StatusCode>, detail: &str) -> AppError {
    if status
        .is_some_and(|value| value == StatusCode::UNAUTHORIZED || value == StatusCode::FORBIDDEN)
        || is_sab_auth_error_message(detail)
    {
        return AppError::Repository(format!("sabnzbd authentication failed: {detail}"));
    }

    AppError::Repository(format!("{operation} error: {detail}"))
}

enum SabApiResponseEvaluation {
    Success(Value),
    Retry(AppError),
    Failure(AppError),
}

fn evaluate_sab_api_response(
    operation: &str,
    request_mode: Option<&str>,
    status: StatusCode,
    body: &str,
) -> SabApiResponseEvaluation {
    if !status.is_success() {
        let detail = extract_sab_error_detail(body);
        let error = map_sabnzbd_response_error(operation, status, body);
        return if status == StatusCode::UNAUTHORIZED
            || status == StatusCode::FORBIDDEN
            || is_sab_auth_error_message(&detail)
        {
            SabApiResponseEvaluation::Failure(error)
        } else {
            SabApiResponseEvaluation::Retry(error)
        };
    }

    let json: Value = match serde_json::from_str(body) {
        Ok(json) => json,
        Err(err) => {
            return SabApiResponseEvaluation::Retry(AppError::Repository(format!(
                "{operation} returned non-json response: {err}"
            )));
        }
    };

    if !sab_api_mode_matches_response(request_mode, &json) {
        return SabApiResponseEvaluation::Retry(AppError::Repository(format!(
            "{operation} returned unexpected response shape for mode '{}'",
            request_mode.unwrap_or("unknown")
        )));
    }

    if sab_api_status_is_false(&json) {
        let error_msg = sab_api_error_message(&json).unwrap_or("unknown error");
        return SabApiResponseEvaluation::Failure(map_sabnzbd_api_error(
            operation,
            Some(status),
            error_msg,
        ));
    }

    SabApiResponseEvaluation::Success(json)
}

fn build_sab_api_urls(base_url: &str) -> Vec<String> {
    dedupe_strings(vec![
        build_sab_api_url_with_suffix(base_url, &["api"]),
        build_sab_api_url_with_suffix(base_url, &["sabnzbd", "api"]),
    ])
}

fn build_sab_api_url_with_suffix(base_url: &str, suffix: &[&str]) -> String {
    let fallback = || format!("{}/api", base_url.trim_end_matches('/'));
    let Ok(mut url) = Url::parse(base_url) else {
        return fallback();
    };

    let mut path_segments = url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let suffix_segments = suffix
        .iter()
        .copied()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if path_segments
        .as_slice()
        .ends_with(suffix_segments.as_slice())
    {
        // Already normalized.
    } else {
        path_segments.extend(suffix_segments);
    }
    let normalized_path = if path_segments.is_empty() {
        "/api".to_string()
    } else {
        format!("/{}", path_segments.join("/"))
    };
    url.set_path(&normalized_path);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string().trim_end_matches('/').to_string()
}

fn sab_api_mode_matches_response(request_mode: Option<&str>, json: &Value) -> bool {
    match request_mode {
        Some("version") => json.get("version").is_some() || sab_api_status_is_false(json),
        Some("queue") => {
            json.get("queue").is_some()
                || json.get("slots").is_some()
                || json.get("status").is_some()
                || sab_api_status_is_false(json)
        }
        Some("history") => {
            json.get("history").is_some()
                || json.get("slots").is_some()
                || json.get("status").is_some()
                || sab_api_status_is_false(json)
        }
        Some("get_config") => json.get("config").is_some() || sab_api_status_is_false(json),
        Some("fullstatus") => json.get("status").is_some() || sab_api_status_is_false(json),
        Some("addfile") => {
            json.get("nzo_ids").is_some()
                || json.get("status").is_some()
                || sab_api_status_is_false(json)
        }
        _ => true,
    }
}

fn slots_from_api_section(section: &Value) -> Option<&Vec<Value>> {
    match section {
        Value::Array(slots) => Some(slots),
        Value::Object(_) => section.get("slots").and_then(Value::as_array),
        _ => None,
    }
}

fn sab_api_status_is_false(json: &Value) -> bool {
    match json.get("status") {
        Some(Value::Bool(false)) => true,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("false"),
        _ => false,
    }
}

fn sab_api_error_message(json: &Value) -> Option<&str> {
    json.get("error")
        .and_then(Value::as_str)
        .or_else(|| json.get("message").and_then(Value::as_str))
}

fn sab_addfile_nzo_id(json: &Value) -> Option<&str> {
    json.get("nzo_ids")
        .and_then(Value::as_array)
        .and_then(|ids| ids.first())
        .and_then(Value::as_str)
}

fn extract_sab_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|json| sab_api_error_message(&json).map(str::to_string))
        .filter(|detail| !detail.trim().is_empty())
        .unwrap_or_else(|| body.chars().take(600).collect())
}

fn is_sab_auth_error_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("authentication")
        || normalized.contains("unauthorized")
        || normalized.contains("forbidden")
        || normalized.contains("api key required")
        || normalized.contains("api key incorrect")
        || normalized.contains("apikey required")
        || normalized.contains("apikey incorrect")
        || normalized.contains("login failed")
        || normalized.contains("invalid api key")
        || normalized.contains("invalid credentials")
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

#[cfg(test)]
mod tests {
    use super::{
        SabApiResponseEvaluation, build_sab_api_urls, evaluate_sab_api_response,
        sab_api_mode_matches_response,
    };
    use reqwest::StatusCode;
    use serde_json::json;

    #[test]
    fn build_sab_api_urls_includes_sabnzbd_compatibility_path() {
        assert_eq!(
            build_sab_api_urls("http://altmount:8080"),
            vec![
                "http://altmount:8080/api".to_string(),
                "http://altmount:8080/sabnzbd/api".to_string(),
            ]
        );
    }

    #[test]
    fn build_sab_api_urls_preserves_existing_prefix() {
        assert_eq!(
            build_sab_api_urls("http://example.test/altmount"),
            vec![
                "http://example.test/altmount/api".to_string(),
                "http://example.test/altmount/sabnzbd/api".to_string(),
            ]
        );
    }

    #[test]
    fn sab_api_mode_match_rejects_non_sab_version_shape() {
        let json = json!({"data": {"api_key": "abc123"}});
        assert!(!sab_api_mode_matches_response(Some("version"), &json));
    }

    #[test]
    fn evaluate_sab_api_response_marks_non_sab_shape_retryable() {
        let outcome = evaluate_sab_api_response(
            "sabnzbd api",
            Some("queue"),
            StatusCode::OK,
            r#"{"data":{"api_key":"abc123"}}"#,
        );

        assert!(matches!(outcome, SabApiResponseEvaluation::Retry(_)));
    }
}
