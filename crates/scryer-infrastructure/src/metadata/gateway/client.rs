use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::future::Future;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use aws_lc_rs::digest;
use reqwest::Client;
use scryer_application::{
    AnimeEpisodeMapping, AnimeMapping, AnimeMovie, AppError, AppResult, BulkMetadataResult,
    EpisodeMetadata, MetadataGateway, MetadataSearchItem, MetadataSearchQuery, MovieMetadata,
    MultiMetadataSearchResult, RichMetadataSearchItem, SeasonMetadata, SeriesMetadata,
    SettingsRepository,
};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, OutboundRequestError, RateLimitRegistry, RequestPolicy,
    metadata_gateway_reqwest_client,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info, warn};

struct ApqCacheEntry {
    etag: String,
    body: String,
}

struct ApqCache {
    map: HashMap<String, ApqCacheEntry>,
    order: VecDeque<String>,
}

impl ApqCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<&ApqCacheEntry> {
        self.map.get(key)
    }

    #[expect(clippy::map_entry)] // entry API borrows map, conflicting with eviction logic
    fn insert(&mut self, key: String, entry: ApqCacheEntry) {
        if self.map.contains_key(&key) {
            self.map.insert(key, entry);
            return;
        }
        if self.map.len() >= 1000
            && let Some(oldest) = self.order.pop_front()
        {
            self.map.remove(&oldest);
        }
        self.order.push_back(key.clone());
        self.map.insert(key, entry);
    }
}

use crate::{graphql::metadata_gateway as graphql_docs, smg_enrollment};

fn sha256_hex(input: &str) -> String {
    let hash = digest::digest(&digest::SHA256, input.as_bytes());
    hash.as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Precompute the SHA-256 hash for a static query string (APQ registration).
fn apq_hash(query: &str) -> String {
    sha256_hex(query)
}

/// Configuration for SMG enrollment (mTLS client certificates).
pub struct SmgEnrollmentConfig {
    pub registration_secret: Option<String>,
    pub ca_cert: Option<String>,
}

/// Signing materials for application-layer instance authentication.
#[derive(Clone)]
enum InstanceAuth {
    Pq {
        instance_id: Arc<String>,
        seed_b64: Arc<String>,
        key_id: Arc<String>,
        enrollment_generation: Option<i64>,
    },
}

/// Tracks the state of mTLS enrollment to prevent rapid-fire retries on failure.
enum MtlsState {
    /// Enrollment hasn't been attempted yet.
    NotAttempted,
    /// Enrollment succeeded; use this client and auth materials.
    Enrolled { client: Client, auth: InstanceAuth },
    /// Enrollment failed; don't retry until `retry_after`.
    Failed { retry_after: Instant, attempts: u32 },
}

/// SHA-256 hex digest of a byte slice (for request body hashing).
fn sha256_hex_bytes(data: &[u8]) -> String {
    let hash = digest::digest(&digest::SHA256, data);
    hash.as_ref()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

/// Attach PQ instance auth headers by signing the full request target.
async fn apply_instance_auth_headers(
    req: reqwest::RequestBuilder,
    auth: &InstanceAuth,
    method: &str,
    url: &reqwest::Url,
    body_bytes: &[u8],
) -> AppResult<reqwest::RequestBuilder> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| AppError::Repository(format!("system clock before UNIX_EPOCH: {e}")))?
        .as_secs() as i64;
    match auth {
        InstanceAuth::Pq {
            seed_b64, key_id, ..
        } => {
            let body_hash = sha256_hex_bytes(body_bytes);
            let host = canonical_request_host(url)?;
            let path_and_query = canonical_request_path_and_query(url);
            let signature = smg_enrollment::sign_pq_request(
                seed_b64,
                method,
                &host,
                &path_and_query,
                timestamp,
                &body_hash,
            )
            .await
            .map_err(|e| AppError::Repository(format!("failed to sign PQ request: {e}")))?;
            debug!(
                timestamp,
                key_id = %key_id,
                sig_len = signature.len(),
                body_hash,
                "attaching PQ X-Scryer-* instance auth headers"
            );
            Ok(req
                .header("X-Scryer-Auth-Version", "pqsig-v1")
                .header("X-Scryer-Key-Id", &**key_id)
                .header("X-Scryer-Timestamp", timestamp.to_string())
                .header("X-Scryer-Signature", signature))
        }
    }
}

fn canonical_request_host(url: &reqwest::Url) -> AppResult<String> {
    let host = url
        .host_str()
        .ok_or_else(|| AppError::Repository("metadata gateway URL missing host".into()))?;
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    })
}

fn canonical_request_path_and_query(url: &reqwest::Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

/// Minimum interval between cert-rejection re-enrollment attempts.
const REENROLLMENT_COOLDOWN: Duration = Duration::from_secs(60);
const METADATA_GATEWAY_MAX_RETRIES: u32 = 3;
const METADATA_GATEWAY_RATE_LIMIT_BASE_DELAY: Duration = Duration::from_secs(2);
const METADATA_GATEWAY_RATE_LIMIT_MAX_DELAY: Duration = Duration::from_secs(30);
const METADATA_GATEWAY_TRANSIENT_BASE_DELAY: Duration = Duration::from_secs(1);
const METADATA_GATEWAY_TRANSIENT_MAX_DELAY: Duration = Duration::from_secs(5);
const METADATA_GATEWAY_MAX_SEARCH_BATCH: usize = 50;
const METADATA_GATEWAY_MAX_BULK_METADATA_ALIAS_BATCH: usize = 100;
const METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const METADATA_GATEWAY_COMPATIBILITY_STARTUP_GUARD: Duration = Duration::from_secs(30 * 60);
const METADATA_GATEWAY_VERSION_COMPATIBILITY_PATH: &str = "/api/version-compatibility";
const SCRYER_RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");
const SCRYER_SMG_USER_AGENT: &str = concat!("Scryer-", env!("CARGO_PKG_VERSION"));

#[derive(Deserialize)]
struct VersionCompatibilitySuccessResponse {
    compatibility: Option<VersionCompatibilityDecisionPayload>,
}

#[derive(Deserialize)]
struct VersionCompatibilityDecisionPayload {
    status: String,
    #[serde(default)]
    minimum_version: String,
    #[serde(default)]
    your_version: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    upgrade_deadline: Option<String>,
}

impl VersionCompatibilityDecisionPayload {
    fn into_notice(self) -> Option<smg_enrollment::VersionIncompatible> {
        if self.status.eq_ignore_ascii_case("supported") {
            return None;
        }

        Some(smg_enrollment::VersionIncompatible {
            status: self.status,
            minimum_version: self.minimum_version,
            your_version: self.your_version,
            message: self.message,
            upgrade_deadline: self
                .upgrade_deadline
                .filter(|value| !value.trim().is_empty()),
        })
    }
}

fn parse_version_compatibility_success(
    body: &[u8],
) -> AppResult<Option<smg_enrollment::VersionIncompatible>> {
    let parsed: VersionCompatibilitySuccessResponse =
        serde_json::from_slice(body).map_err(|error| {
            AppError::Repository(format!(
                "failed to decode SMG version compatibility response: {error}"
            ))
        })?;
    Ok(parsed
        .compatibility
        .and_then(VersionCompatibilityDecisionPayload::into_notice))
}

fn compatibility_poll_phase(instance_id: &str) -> Duration {
    let digest = digest::digest(&digest::SHA256, instance_id.as_bytes());
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&digest.as_ref()[..8]);
    let offset_secs =
        u64::from_be_bytes(raw) % METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL.as_secs();
    Duration::from_secs(offset_secs)
}

fn next_version_compatibility_poll_delay_at(
    now: SystemTime,
    phase: Duration,
    minimum_delay: Duration,
) -> Duration {
    let now_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let interval_secs = METADATA_GATEWAY_COMPATIBILITY_POLL_INTERVAL.as_secs();
    let phase_secs = phase.as_secs() % interval_secs;

    let mut next_slot = (now_secs / interval_secs) * interval_secs + phase_secs;
    if next_slot <= now_secs {
        next_slot = next_slot.saturating_add(interval_secs);
    }

    let earliest = now_secs.saturating_add(minimum_delay.as_secs());
    if next_slot < earliest {
        let delta = earliest - next_slot;
        let skips = delta.div_ceil(interval_secs);
        next_slot = next_slot.saturating_add(skips * interval_secs);
    }

    Duration::from_secs(next_slot.saturating_sub(now_secs))
}

pub struct MetadataGatewayClient {
    http: Client,
    outbound_http: OutboundHttpClient,
    endpoint: String,
    registration_url: String,
    enrollment_config: SmgEnrollmentConfig,
    enrollment_store: Option<Arc<dyn SettingsRepository>>,
    mtls_state: tokio::sync::RwLock<MtlsState>,
    last_reenrollment: tokio::sync::Mutex<Option<Instant>>,
    pq_rotation: tokio::sync::Mutex<()>,
    compatibility_refresh: tokio::sync::Mutex<()>,
    version_incompatible: tokio::sync::Mutex<Option<smg_enrollment::VersionIncompatible>>,
    search_hash: String,
    search_rich_hash: String,
    search_multi_hash: String,
    movie_hash: String,
    series_hash: String,
    apq_cache: RwLock<ApqCache>,
}

impl MetadataGatewayClient {
    pub fn new(
        endpoint: String,
        accept_invalid_certs: bool,
        db: crate::SqliteServices,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        Self::new_with_enrollment_store(
            endpoint,
            accept_invalid_certs,
            Arc::new(crate::SettingsStore::from_sqlite_services(&db)),
            enrollment_config,
        )
    }

    pub fn new_with_enrollment_store(
        endpoint: String,
        accept_invalid_certs: bool,
        enrollment_store: Arc<dyn SettingsRepository>,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        if accept_invalid_certs {
            warn!("metadata gateway client: TLS certificate verification DISABLED");
        }

        let search_hash = apq_hash(graphql_docs::SEARCH_TVDB_QUERY);
        let search_rich_hash = apq_hash(graphql_docs::SEARCH_TVDB_RICH_QUERY);
        let search_multi_hash = apq_hash(graphql_docs::SEARCH_TVDB_MULTI_QUERY);
        let movie_hash = apq_hash(graphql_docs::GET_MOVIE_QUERY);
        let series_hash = apq_hash(graphql_docs::GET_SERIES_QUERY);

        // Derive registration URL from GraphQL endpoint
        let registration_url = if endpoint.ends_with("/graphql") {
            format!(
                "{}/api/register",
                &endpoint[..endpoint.len() - "/graphql".len()]
            )
        } else {
            format!("{}/api/register", endpoint.trim_end_matches('/'))
        };

        debug!(
            endpoint = %endpoint,
            accept_invalid_certs,
            has_registration_secret = enrollment_config.registration_secret.is_some(),
            %search_hash,
            %search_rich_hash,
            %search_multi_hash,
            %movie_hash,
            %series_hash,
            "metadata gateway client initialized (APQ enabled)"
        );

        let http = metadata_gateway_reqwest_client(accept_invalid_certs, SCRYER_SMG_USER_AGENT)
            .expect("failed to build HTTP client");

        Self {
            outbound_http: OutboundHttpClient::new(http.clone(), RateLimitRegistry::new()),
            http,
            endpoint,
            registration_url,
            enrollment_config,
            last_reenrollment: tokio::sync::Mutex::new(None),
            pq_rotation: tokio::sync::Mutex::new(()),
            compatibility_refresh: tokio::sync::Mutex::new(()),
            version_incompatible: tokio::sync::Mutex::new(None),
            enrollment_store: Some(enrollment_store),
            mtls_state: tokio::sync::RwLock::new(MtlsState::NotAttempted),
            search_hash,
            search_rich_hash,
            search_multi_hash,
            movie_hash,
            series_hash,
            apq_cache: RwLock::new(ApqCache::new()),
        }
    }

    pub fn new_without_enrollment_store(
        endpoint: String,
        accept_invalid_certs: bool,
        enrollment_config: SmgEnrollmentConfig,
    ) -> Self {
        if accept_invalid_certs {
            warn!("metadata gateway client: TLS certificate verification DISABLED");
        }
        if enrollment_config.registration_secret.is_some() {
            warn!(
                "SMG enrollment is not available for this datastore engine in the PostgreSQL blank-install slice"
            );
        }

        let search_hash = apq_hash(graphql_docs::SEARCH_TVDB_QUERY);
        let search_rich_hash = apq_hash(graphql_docs::SEARCH_TVDB_RICH_QUERY);
        let search_multi_hash = apq_hash(graphql_docs::SEARCH_TVDB_MULTI_QUERY);
        let movie_hash = apq_hash(graphql_docs::GET_MOVIE_QUERY);
        let series_hash = apq_hash(graphql_docs::GET_SERIES_QUERY);
        let registration_url = if endpoint.ends_with("/graphql") {
            format!(
                "{}/api/register",
                &endpoint[..endpoint.len() - "/graphql".len()]
            )
        } else {
            format!("{}/api/register", endpoint.trim_end_matches('/'))
        };
        let http = metadata_gateway_reqwest_client(accept_invalid_certs, SCRYER_SMG_USER_AGENT)
            .expect("failed to build HTTP client");

        Self {
            outbound_http: OutboundHttpClient::new(http.clone(), RateLimitRegistry::new()),
            http,
            endpoint,
            registration_url,
            enrollment_config,
            last_reenrollment: tokio::sync::Mutex::new(None),
            pq_rotation: tokio::sync::Mutex::new(()),
            compatibility_refresh: tokio::sync::Mutex::new(()),
            version_incompatible: tokio::sync::Mutex::new(None),
            enrollment_store: None,
            mtls_state: tokio::sync::RwLock::new(MtlsState::NotAttempted),
            search_hash,
            search_rich_hash,
            search_multi_hash,
            movie_hash,
            series_hash,
            apq_cache: RwLock::new(ApqCache::new()),
        }
    }

    /// Get the mTLS HTTP client and optional signing materials, enrolling lazily on first call.
    ///
    /// If no registration secret is configured, returns the plain HTTP client with no auth.
    /// If enrollment fails, returns an error with exponential backoff on retries.
    async fn get_http_client(&self) -> AppResult<(Client, Option<InstanceAuth>)> {
        let secret = match &self.enrollment_config.registration_secret {
            Some(s) => s,
            None => return Ok((self.http.clone(), None)),
        };

        // Fast path: check current state under read lock
        {
            let guard = self.mtls_state.read().await;
            match &*guard {
                MtlsState::Enrolled { client, auth } => {
                    return Ok((client.clone(), Some(auth.clone())));
                }
                MtlsState::Failed { retry_after, .. } if Instant::now() < *retry_after => {
                    return Err(AppError::Repository(
                        "SMG instance auth enrollment pending retry (backoff)".into(),
                    ));
                }
                _ => {}
            }
        }

        // Slow path: need to attempt enrollment
        let mut guard = self.mtls_state.write().await;
        // Double-check after acquiring write lock
        match &*guard {
            MtlsState::Enrolled { client, auth } => {
                return Ok((client.clone(), Some(auth.clone())));
            }
            MtlsState::Failed { retry_after, .. } if Instant::now() < *retry_after => {
                return Err(AppError::Repository(
                    "SMG instance auth enrollment pending retry (backoff)".into(),
                ));
            }
            _ => {}
        }

        let attempts = match &*guard {
            MtlsState::Failed { attempts, .. } => *attempts,
            _ => 0,
        };

        match self.try_build_mtls_client(secret).await {
            Ok((client, auth)) => {
                info!(
                    "SMG instance auth enrollment successful, using instance authentication for metadata requests"
                );
                let result = (client.clone(), Some(auth.clone()));
                *guard = MtlsState::Enrolled { client, auth };
                Ok(result)
            }
            Err(e) => {
                let next_attempts = attempts + 1;
                let retry_after = enrollment_retry_delay(&e, attempts);
                warn!(
                    error = %e,
                    attempt = next_attempts,
                    retry_in_secs = retry_after.as_secs(),
                    "SMG instance auth enrollment failed"
                );
                *guard = MtlsState::Failed {
                    retry_after: Instant::now() + retry_after,
                    attempts: next_attempts,
                };
                Err(AppError::Repository(format!(
                    "SMG instance auth enrollment failed: {e}"
                )))
            }
        }
    }

    async fn try_build_mtls_client(
        &self,
        registration_secret: &str,
    ) -> Result<(Client, InstanceAuth), smg_enrollment::EnrollmentError> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            smg_enrollment::EnrollmentError::Other(
                "SMG enrollment persistence is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        let state = match smg_enrollment::ensure_enrolled(
            &**db,
            &self.registration_url,
            registration_secret,
            self.enrollment_config.ca_cert.as_deref(),
        )
        .await
        {
            Ok(state) => state,
            Err(error) => {
                if let smg_enrollment::EnrollmentError::VersionIncompatible(ref incompatibility) =
                    error
                {
                    self.remember_enrollment_version_incompatible(incompatibility)
                        .await;
                }
                return Err(error);
            }
        };

        if let (Some(seed_b64), Some(key_id)) =
            (state.pq_seed_b64.as_ref(), state.pq_key_id.as_ref())
        {
            return Ok((
                self.http.clone(),
                InstanceAuth::Pq {
                    instance_id: Arc::new(state.instance_id.clone()),
                    seed_b64: Arc::new(seed_b64.clone()),
                    key_id: Arc::new(key_id.clone()),
                    enrollment_generation: state.pq_enrollment_generation,
                },
            ));
        }
        Err(smg_enrollment::EnrollmentError::Other(
            "SMG enrollment completed without PQ request-signing state".to_string(),
        ))
    }

    /// Invalidate cached enrollment after a cert rejection (401) from SMG.
    /// Clears SQLite cache and resets state so the next request triggers fresh enrollment.
    /// Returns `true` if invalidation happened, `false` if still within cooldown.
    async fn invalidate_enrollment(&self) -> bool {
        let mut last = self.last_reenrollment.lock().await;
        if let Some(prev) = *last
            && prev.elapsed() < REENROLLMENT_COOLDOWN
        {
            debug!(
                cooldown_remaining_secs = (REENROLLMENT_COOLDOWN - prev.elapsed()).as_secs(),
                "skipping re-enrollment (cooldown active)"
            );
            return false;
        }
        *last = Some(Instant::now());
        drop(last);

        warn!("SMG rejected instance auth — clearing cached enrollment for re-registration");
        if let Some(db) = self.enrollment_store.as_ref() {
            if let Err(e) = smg_enrollment::clear_enrollment_cache(&**db).await {
                warn!(error = %e, "failed to clear enrollment cache from SQLite");
            }
        } else {
            warn!(
                "SMG enrollment cache clear skipped because this datastore engine has no enrollment store"
            );
        }
        let mut guard = self.mtls_state.write().await;
        *guard = MtlsState::NotAttempted;
        true
    }

    async fn store_version_compatibility_notice(
        &self,
        notice: Option<smg_enrollment::VersionIncompatible>,
    ) -> AppResult<()> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            AppError::Repository(
                "SMG compatibility notice persistence is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        smg_enrollment::persist_version_compatibility_notice(&**db, notice.as_ref())
            .await
            .map_err(AppError::Repository)?;
        *self.version_incompatible.lock().await = notice;
        Ok(())
    }

    async fn remember_enrollment_version_incompatible(
        &self,
        incompatibility: &smg_enrollment::VersionIncompatible,
    ) {
        if let Some(db) = self.enrollment_store.as_ref()
            && let Err(error) =
                smg_enrollment::persist_version_compatibility_notice(&**db, Some(incompatibility))
                    .await
        {
            warn!(
                error = %error,
                "failed to persist SMG version compatibility notice from enrollment"
            );
        }
        if let Ok(mut guard) = self.version_incompatible.try_lock() {
            *guard = Some(incompatibility.clone());
        }
    }

    /// Eagerly trigger enrollment in a background task so the mTLS client is ready before
    /// the first real metadata query arrives. Call this once after construction; it is
    /// safe to call concurrently with any other method.
    pub async fn warm_enrollment(&self) -> Option<smg_enrollment::VersionIncompatible> {
        let _ = self.get_http_client().await;
        if self.compatibility_polling_enabled()
            && let Err(error) = self.refresh_version_compatibility(false).await
        {
            warn!(error = %error, "SMG version compatibility warmup failed");
        }
        self.version_incompatible.lock().await.clone()
    }

    pub fn compatibility_polling_enabled(&self) -> bool {
        self.enrollment_config.registration_secret.is_some()
    }

    pub async fn version_compatibility_poll_phase(&self) -> AppResult<Duration> {
        let db = self.enrollment_store.as_ref().ok_or_else(|| {
            AppError::Repository(
                "SMG compatibility polling is not implemented for this datastore engine"
                    .to_string(),
            )
        })?;
        let instance_id = smg_enrollment::ensure_instance_id(&**db)
            .await
            .map_err(AppError::Repository)?;
        Ok(compatibility_poll_phase(&instance_id))
    }

    pub fn next_version_compatibility_poll_delay(
        phase: Duration,
        minimum_delay: Duration,
    ) -> Duration {
        next_version_compatibility_poll_delay_at(SystemTime::now(), phase, minimum_delay)
    }

    pub fn version_compatibility_startup_guard() -> Duration {
        METADATA_GATEWAY_COMPATIBILITY_STARTUP_GUARD
    }

    pub async fn refresh_version_compatibility(
        &self,
        skip_if_busy: bool,
    ) -> AppResult<Option<smg_enrollment::VersionIncompatible>> {
        let _guard = if skip_if_busy {
            match self.compatibility_refresh.try_lock() {
                Ok(guard) => guard,
                Err(_) => return Ok(None),
            }
        } else {
            self.compatibility_refresh.lock().await
        };

        if !self.compatibility_polling_enabled() {
            return Ok(None);
        }

        let url = smg_enrollment::derive_registration_endpoint(
            &self.registration_url,
            METADATA_GATEWAY_VERSION_COMPATIBILITY_PATH,
        )
        .map_err(|error| AppError::Repository(error.to_string()))?;
        let payload = json!({ "version": SCRYER_RUNTIME_VERSION });
        let body_bytes = serde_json::to_vec(&payload).map_err(|error| {
            AppError::Repository(format!("failed to serialize payload: {error}"))
        })?;
        let endpoint_url = reqwest::Url::parse(&url)
            .map_err(|error| AppError::Repository(format!("invalid compatibility URL: {error}")))?;

        let mut retried_after_reenrollment = false;
        loop {
            let (client, auth) = self.get_http_client().await?;
            let build_req = || {
                let client = client.clone();
                let auth = auth.clone();
                let url = url.clone();
                let body_bytes = body_bytes.clone();
                let endpoint_url = endpoint_url.clone();
                async move {
                    let mut req = client
                        .post(url)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body_bytes.clone());
                    if let Some(ref auth) = auth {
                        req = apply_instance_auth_headers(
                            req,
                            auth,
                            reqwest::Method::POST.as_str(),
                            &endpoint_url,
                            &body_bytes,
                        )
                        .await?;
                    }
                    Ok(req)
                }
            };

            let response = self
                .send_request_with_retry(build_req, "SMG version compatibility check")
                .await?;
            self.reconcile_pq_enrollment_generation(auth.as_ref(), response.headers())
                .await;

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                && !retried_after_reenrollment
                && self.enrollment_config.registration_secret.is_some()
            {
                let body = response.text().await.map_err(|error| {
                    AppError::Repository(format!(
                        "SMG version compatibility response read failed: {error}"
                    ))
                })?;
                retried_after_reenrollment = true;
                if !self.invalidate_enrollment().await {
                    return Err(AppError::Repository(format!(
                        "SMG version compatibility check auth rejected ({status}), re-enrollment on cooldown: {body}"
                    )));
                }
                info!("retrying SMG version compatibility check after re-enrollment");
                continue;
            }

            if status.is_success() {
                let body = response.bytes().await.map_err(|error| {
                    AppError::Repository(format!(
                        "SMG version compatibility response read failed: {error}"
                    ))
                })?;
                let notice = parse_version_compatibility_success(&body)?;
                self.store_version_compatibility_notice(notice.clone())
                    .await?;
                return Ok(notice);
            }

            let error = smg_enrollment::registration_response_error(
                response,
                "SMG version compatibility check",
            )
            .await;
            match error {
                smg_enrollment::EnrollmentError::VersionIncompatible(incompatibility) => {
                    self.store_version_compatibility_notice(Some(incompatibility.clone()))
                        .await?;
                    return Ok(Some(incompatibility));
                }
                smg_enrollment::EnrollmentError::RateLimited(rate_limited) => {
                    return Err(AppError::Repository(format!(
                        "SMG version compatibility check rate limited: {}",
                        rate_limited.message
                    )));
                }
                smg_enrollment::EnrollmentError::Other(message) => {
                    return Err(AppError::Repository(message));
                }
            }
        }
    }

    /// Execute a GraphQL query using APQ (Automatic Persisted Queries).
    ///
    /// 1. Send GET with hash only (no query body) — cache-friendly.
    ///    Sends `If-None-Match` if we have a cached ETag; on 304 returns cached body.
    /// 2. If the server returns `PersistedQueryNotFound`, POST with full query + hash to register.
    /// 3. Subsequent GETs for the same hash will hit Cloudflare edge cache.
    async fn execute_graphql_apq<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        hash: &str,
        variables: serde_json::Value,
    ) -> AppResult<T> {
        let extensions = json!({
            "persistedQuery": {
                "version": 1,
                "sha256Hash": hash
            }
        });

        let variables_str = serde_json::to_string(&variables)
            .map_err(|e| AppError::Repository(format!("failed to serialize variables: {e}")))?;
        let extensions_str = serde_json::to_string(&extensions)
            .map_err(|e| AppError::Repository(format!("failed to serialize extensions: {e}")))?;

        let cache_key = format!("{hash}:{variables_str}");

        // Check for a cached ETag to send If-None-Match
        let cached_etag = self
            .apq_cache
            .read()
            .unwrap()
            .get(&cache_key)
            .map(|e| e.etag.clone());

        debug!(endpoint = %self.endpoint, hash, has_etag = cached_etag.is_some(), "APQ GET request");

        let (client, auth) = self.get_http_client().await?;

        // Build URL with query params so the GET request target is complete before signing.
        let mut url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;
        url.query_pairs_mut()
            .append_pair("extensions", &extensions_str)
            .append_pair("variables", &variables_str);

        let get_result = self
            .send_request_with_retry(
                || {
                    let client = client.clone();
                    let cached_etag = cached_etag.clone();
                    let auth = auth.clone();
                    let url = url.clone();
                    async move {
                        let mut req = client.get(url.clone());
                        if let Some(ref etag) = cached_etag {
                            req = req.header(reqwest::header::IF_NONE_MATCH, etag);
                        }
                        if let Some(ref auth) = auth {
                            req = apply_instance_auth_headers(
                                req,
                                auth,
                                reqwest::Method::GET.as_str(),
                                &url,
                                &[],
                            )
                            .await?;
                        }
                        Ok(req)
                    }
                },
                "metadata gateway APQ GET",
            )
            .await;

        match get_result {
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_MODIFIED => {
                self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
                    .await;
                // 304: serve from our local cache
                let body = self
                    .apq_cache
                    .read()
                    .unwrap()
                    .get(&cache_key)
                    .map(|e| e.body.clone())
                    .ok_or_else(|| AppError::Repository("APQ 304 but no cached body".into()))?;
                debug!(hash, "APQ 304 — serving from ETag cache");
                let parsed: GraphqlResponse<T> = serde_json::from_str(&body)
                    .map_err(|e| AppError::Repository(format!("APQ cache: invalid JSON: {e}")))?;
                parsed
                    .data
                    .ok_or_else(|| AppError::Repository("APQ cache: empty data".into()))
            }
            Ok(resp) if resp.status().is_success() => {
                self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
                    .await;
                let etag = resp
                    .headers()
                    .get(reqwest::header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let raw = resp
                    .text()
                    .await
                    .map_err(|e| AppError::Repository(e.to_string()))?;

                let parsed: GraphqlResponse<T> = serde_json::from_str(&raw)
                    .map_err(|e| AppError::Repository(format!("APQ GET: invalid JSON: {e}")))?;

                // Check for PersistedQueryNotFound before caching
                if let Some(ref errors) = parsed.errors {
                    let is_not_found = errors
                        .iter()
                        .any(|e| e.message.contains("PersistedQueryNotFound"));
                    if is_not_found {
                        debug!(hash, "APQ cache miss, registering via POST");
                        return self
                            .execute_graphql_apq_register(query, &extensions, &variables)
                            .await;
                    }
                    let msg = errors
                        .first()
                        .map(|e| e.message.as_str())
                        .unwrap_or("metadata gateway returned errors");
                    return Err(AppError::Repository(msg.to_string()));
                }

                // Store ETag + body for future conditional requests (evicts oldest beyond 1000)
                if let Some(etag) = etag {
                    self.apq_cache
                        .write()
                        .unwrap()
                        .insert(cache_key, ApqCacheEntry { etag, body: raw });
                }

                parsed
                    .data
                    .ok_or_else(|| AppError::Repository("APQ GET: empty data".into()))
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::UNAUTHORIZED => {
                // Cert rejection — invalidate before falling through to POST retry
                // (execute_graphql will handle the actual re-enrollment + retry)
                self.invalidate_enrollment().await;
                self.execute_graphql_apq_register(query, &extensions, &variables)
                    .await
            }
            Ok(resp) => {
                let status = resp.status();
                let raw = resp
                    .text()
                    .await
                    .map_err(|error| AppError::Repository(error.to_string()))?;
                warn!(status = %status, hash, body = %raw, "APQ GET failed");
                Err(AppError::Repository(format!(
                    "metadata gateway request failed ({status}): {raw}"
                )))
            }
            Err(error) => {
                debug!(error = %error, hash, "APQ GET request failed");
                Err(error)
            }
        }
    }

    /// POST with full query + extensions to register the hash, then return the result.
    async fn execute_graphql_apq_register<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        extensions: &serde_json::Value,
        variables: &serde_json::Value,
    ) -> AppResult<T> {
        let payload = json!({
            "query": query,
            "variables": variables,
            "extensions": extensions,
        });

        self.execute_graphql(payload).await
    }

    async fn execute_graphql<T: serde::de::DeserializeOwned>(
        &self,
        payload: serde_json::Value,
    ) -> AppResult<T> {
        debug!(endpoint = %self.endpoint, "sending metadata gateway request");
        let response = self.send_with_retry(&payload).await?;

        let status = response.status();
        let raw_text = response
            .text()
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        debug!(status = %status, body_len = raw_text.len(), "metadata gateway response");

        // On instance-auth rejection, invalidate enrollment and retry once with fresh creds.
        if status == reqwest::StatusCode::UNAUTHORIZED
            && self.enrollment_config.registration_secret.is_some()
        {
            if !self.invalidate_enrollment().await {
                return Err(AppError::Repository(format!(
                    "metadata gateway instance auth rejected ({status}), re-enrollment on cooldown: {raw_text}"
                )));
            }
            info!("retrying metadata request after re-enrollment");
            let retry_resp = self.send_with_retry(&payload).await?;
            let retry_status = retry_resp.status();
            let retry_text = retry_resp
                .text()
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;
            if !retry_status.is_success() {
                warn!(status = %retry_status, body = %retry_text, "metadata gateway request failed after re-enrollment");
                return Err(AppError::Repository(format!(
                    "metadata gateway request failed ({retry_status}): {retry_text}"
                )));
            }
            return self.parse_graphql_response(&retry_text);
        }

        if !status.is_success() {
            warn!(status = %status, body = %raw_text, "metadata gateway request failed");
            return Err(AppError::Repository(format!(
                "metadata gateway request failed ({status}): {raw_text}"
            )));
        }

        self.parse_graphql_response(&raw_text)
    }

    fn parse_graphql_response<T: serde::de::DeserializeOwned>(
        &self,
        raw_text: &str,
    ) -> AppResult<T> {
        let parsed: GraphqlResponse<T> = serde_json::from_str(raw_text).map_err(|err| {
            warn!(body = %raw_text, error = %err, "metadata gateway returned invalid JSON");
            AppError::Repository(format!("metadata gateway returned invalid JSON: {err}"))
        })?;

        if let Some(errors) = parsed.errors {
            let message = errors
                .first()
                .map(|error| error.message.as_str())
                .unwrap_or("metadata gateway returned errors");
            warn!(error = %message, "metadata gateway returned GraphQL errors");
            return Err(AppError::Repository(message.to_string()));
        }

        if parsed.data.is_none() {
            warn!(body = %raw_text, "metadata gateway returned empty data");
        }

        parsed
            .data
            .ok_or_else(|| AppError::Repository("metadata gateway returned empty data".into()))
    }

    async fn send_request_with_retry<F, Fut>(
        &self,
        build_req: F,
        request_label: &'static str,
    ) -> AppResult<reqwest::Response>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = AppResult<reqwest::RequestBuilder>>,
    {
        for retry_index in 0..=METADATA_GATEWAY_MAX_RETRIES {
            match self
                .outbound_http
                .send_async(self.request_policy(request_label), &build_req)
                .await
            {
                Ok(resp) if resp.status().is_server_error() => {
                    if retry_index == METADATA_GATEWAY_MAX_RETRIES {
                        return Ok(resp);
                    }

                    let retry_after = metadata_gateway_transient_delay(retry_index);
                    warn!(
                        request = request_label,
                        status = %resp.status(),
                        retry_attempt = retry_index + 1,
                        retry_after_ms = retry_after.as_millis(),
                        "metadata gateway returned server error, retrying"
                    );
                    tokio::time::sleep(retry_after).await;
                }
                Ok(resp) => return Ok(resp),
                Err(OutboundRequestError::Build(error)) => return Err(error),
                Err(OutboundRequestError::Http(error)) => {
                    return Err(map_metadata_gateway_outbound_error(request_label, error));
                }
            }
        }

        Err(AppError::Repository(format!(
            "{request_label} exhausted retries"
        )))
    }

    fn request_policy(&self, request_label: &'static str) -> RequestPolicy {
        RequestPolicy::safe_read("metadata_gateway", request_label)
            .with_max_retries(METADATA_GATEWAY_MAX_RETRIES)
            .with_backoff(
                METADATA_GATEWAY_RATE_LIMIT_BASE_DELAY,
                METADATA_GATEWAY_RATE_LIMIT_MAX_DELAY,
            )
    }

    async fn reconcile_pq_enrollment_generation(
        &self,
        auth: Option<&InstanceAuth>,
        headers: &reqwest::header::HeaderMap,
    ) {
        let Some(InstanceAuth::Pq {
            instance_id,
            seed_b64,
            key_id,
            enrollment_generation,
        }) = auth
        else {
            return;
        };

        let Some(server_generation) = headers
            .get("X-SMG-Enrollment-Generation")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<i64>().ok())
        else {
            return;
        };

        let local_generation = enrollment_generation.unwrap_or(0);
        if server_generation <= local_generation {
            return;
        }

        let _rotation_guard = self.pq_rotation.lock().await;
        let Some(db) = self.enrollment_store.as_ref() else {
            warn!(
                "SMG PQ enrollment rotation skipped because this datastore engine has no enrollment store"
            );
            return;
        };

        if enrollment_generation.is_none() && server_generation == 1 {
            if let Err(error) =
                smg_enrollment::persist_pq_enrollment_generation(&**db, server_generation).await
            {
                warn!(
                    error = %error,
                    server_generation,
                    key_id = %key_id,
                    "failed to persist initial SMG PQ enrollment generation"
                );
                return;
            }
            let mut state = self.mtls_state.write().await;
            *state = MtlsState::NotAttempted;
            return;
        }

        match smg_enrollment::rotate_pq_enrollment(
            &**db,
            instance_id,
            seed_b64,
            key_id,
            &self.registration_url,
            self.enrollment_config.ca_cert.as_deref(),
        )
        .await
        {
            Ok(_) => {
                info!(
                    instance_id = %instance_id,
                    key_id = %key_id,
                    old_generation = local_generation,
                    new_generation = server_generation,
                    "rotated SMG PQ enrollment after generation advance"
                );
                let mut state = self.mtls_state.write().await;
                *state = MtlsState::NotAttempted;
            }
            Err(error) => {
                warn!(
                    error = %error,
                    instance_id = %instance_id,
                    key_id = %key_id,
                    old_generation = local_generation,
                    new_generation = server_generation,
                    "failed to rotate SMG PQ enrollment after generation advance"
                );
            }
        }
    }

    async fn send_with_retry(&self, payload: &serde_json::Value) -> AppResult<reqwest::Response> {
        let (client, auth) = self.get_http_client().await?;
        let body_bytes = serde_json::to_vec(payload)
            .map_err(|e| AppError::Repository(format!("failed to serialize payload: {e}")))?;
        let endpoint_url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;

        let build_req = || {
            let client = client.clone();
            let auth = auth.clone();
            let endpoint_url = endpoint_url.clone();
            let body_bytes = body_bytes.clone();
            let endpoint = self.endpoint.clone();
            async move {
                let mut req = client
                    .post(endpoint)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_bytes.clone());
                if let Some(ref auth) = auth {
                    req = apply_instance_auth_headers(
                        req,
                        auth,
                        reqwest::Method::POST.as_str(),
                        &endpoint_url,
                        &body_bytes,
                    )
                    .await?;
                }
                Ok(req)
            }
        };

        let response = self
            .send_request_with_retry(build_req, "metadata gateway request")
            .await?;
        self.reconcile_pq_enrollment_generation(auth.as_ref(), response.headers())
            .await;
        Ok(response)
    }

    /// POST a batched GraphQL query directly and return the `data` field as raw JSON.
    ///
    /// Batched alias-heavy requests intentionally bypass APQ. The variable entropy on
    /// these requests makes persisted-query cache hits unlikely enough that the GET +
    /// register dance is wasted overhead.
    ///
    /// Tolerates partial errors (some aliases may resolve while others fail).
    async fn post_batched_graphql_partial(&self, query: &str) -> AppResult<serde_json::Value> {
        let payload = json!({ "query": query });
        self.post_batched_graphql_partial_payload(&payload, "bulk metadata request")
            .await
    }

    async fn post_batched_graphql_partial_payload(
        &self,
        payload: &serde_json::Value,
        request_label: &'static str,
    ) -> AppResult<serde_json::Value> {
        let (client, auth) = self.get_http_client().await?;
        let body_bytes = serde_json::to_vec(payload)
            .map_err(|e| AppError::Repository(format!("failed to serialize payload: {e}")))?;
        let endpoint_url = reqwest::Url::parse(&self.endpoint)
            .map_err(|e| AppError::Repository(format!("invalid endpoint URL: {e}")))?;
        let build_req = || {
            let client = client.clone();
            let auth = auth.clone();
            let endpoint_url = endpoint_url.clone();
            let body_bytes = body_bytes.clone();
            let endpoint = self.endpoint.clone();
            async move {
                let mut req = client
                    .post(endpoint)
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(body_bytes.clone());
                if let Some(ref auth) = auth {
                    req = apply_instance_auth_headers(
                        req,
                        auth,
                        reqwest::Method::POST.as_str(),
                        &endpoint_url,
                        &body_bytes,
                    )
                    .await?;
                }
                Ok(req)
            }
        };
        let resp = self
            .send_request_with_retry(build_req, request_label)
            .await?;
        self.reconcile_pq_enrollment_generation(auth.as_ref(), resp.headers())
            .await;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Repository(format!("bulk metadata read body: {e}")))?;

        // On instance-auth rejection, invalidate and retry with fresh creds.
        if status == reqwest::StatusCode::UNAUTHORIZED
            && self.enrollment_config.registration_secret.is_some()
        {
            if !self.invalidate_enrollment().await {
                return Err(AppError::Repository(format!(
                    "bulk metadata instance auth rejected ({status}), re-enrollment on cooldown: {body}"
                )));
            }
            info!(
                request = request_label,
                "retrying metadata gateway request after re-enrollment"
            );
            let (client2, auth2) = self.get_http_client().await?;
            let build_retry_req = || {
                let client2 = client2.clone();
                let auth2 = auth2.clone();
                let endpoint_url = endpoint_url.clone();
                let body_bytes = body_bytes.clone();
                let endpoint = self.endpoint.clone();
                async move {
                    let mut req = client2
                        .post(endpoint)
                        .header(reqwest::header::CONTENT_TYPE, "application/json")
                        .body(body_bytes.clone());
                    if let Some(ref auth2) = auth2 {
                        req = apply_instance_auth_headers(
                            req,
                            auth2,
                            reqwest::Method::POST.as_str(),
                            &endpoint_url,
                            &body_bytes,
                        )
                        .await?;
                    }
                    Ok(req)
                }
            };
            let resp2 = self
                .send_request_with_retry(build_retry_req, request_label)
                .await?;
            let status2 = resp2.status();
            let body2 = resp2
                .text()
                .await
                .map_err(|e| AppError::Repository(format!("bulk metadata read body: {e}")))?;
            if !status2.is_success() {
                return Err(AppError::Repository(format!(
                    "bulk metadata request failed after re-enrollment ({status2}): {body2}"
                )));
            }
            return self.parse_partial_response(&body2);
        }

        if !status.is_success() {
            return Err(AppError::Repository(format!(
                "bulk metadata request failed ({status}): {body}"
            )));
        }

        self.parse_partial_response(&body)
    }

    fn parse_partial_response(&self, body: &str) -> AppResult<serde_json::Value> {
        let parsed: serde_json::Value = serde_json::from_str(body)
            .map_err(|e| AppError::Repository(format!("bulk metadata invalid JSON: {e}")))?;

        if let Some(errors) = parsed.get("errors")
            && let Some(arr) = errors.as_array()
        {
            for err in arr {
                let msg = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                debug!("bulk metadata partial error: {msg}");
            }
        }

        parsed
            .get("data")
            .cloned()
            .ok_or_else(|| AppError::Repository("bulk metadata: no data in response".into()))
    }
}

// ---------------------------------------------------------------------------
// Bulk query builders (GraphQL aliases)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum BulkMetadataAliasRequest {
    Movie(i64),
    Series(i64),
}

fn build_search_tvdb_batch_query(queries: &[MetadataSearchQuery]) -> Vec<MetadataSearchQuery> {
    let mut normalized = Vec::with_capacity(queries.len());
    let mut seen = HashSet::with_capacity(queries.len());

    for query in queries {
        let trimmed_query = query.query.trim();
        let trimmed_type = query.type_hint.trim();
        if trimmed_query.is_empty() || trimmed_type.is_empty() {
            continue;
        }

        let normalized_query = MetadataSearchQuery {
            query: trimmed_query.to_string(),
            type_hint: trimmed_type.to_string(),
            year: query.year,
        };

        if seen.insert(normalized_query.clone()) {
            normalized.push(normalized_query);
        }
    }

    normalized
}

#[derive(Serialize)]
struct SearchTvdbBatchRequestInput {
    query: String,
    #[serde(rename = "type")]
    type_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    year: Option<i32>,
    limit: i32,
}

fn build_bulk_metadata_alias_requests(
    movie_ids: &[i64],
    series_ids: &[i64],
) -> Vec<BulkMetadataAliasRequest> {
    movie_ids
        .iter()
        .copied()
        .map(BulkMetadataAliasRequest::Movie)
        .chain(
            series_ids
                .iter()
                .copied()
                .map(BulkMetadataAliasRequest::Series),
        )
        .collect()
}

fn merge_bulk_metadata_partial(
    data: &serde_json::Value,
    movies: &mut HashMap<i64, MovieMetadata>,
    series: &mut HashMap<i64, SeriesMetadata>,
) {
    let Some(obj) = data.as_object() else {
        return;
    };

    for (alias, value) in obj {
        if value.is_null() {
            continue;
        }
        if alias.starts_with('m') {
            if let Ok(movie_result) = serde_json::from_value::<MovieResult>(value.clone()) {
                let m = movie_result.movie;
                movies.insert(
                    m.tvdb_id,
                    MovieMetadata {
                        tvdb_id: m.tvdb_id,
                        name: m.name,
                        slug: m.slug,
                        year: m.year,
                        content_status: m.status,
                        overview: m.overview,
                        poster_url: normalize_artwork_url(&m.poster_url),
                        banner_url: pick_artwork_url(&m.artworks, "banner"),
                        background_url: pick_artwork_url(&m.artworks, "background"),
                        language: m.language,
                        runtime_minutes: m.runtime_minutes,
                        sort_title: m.sort_title,
                        imdb_id: m.imdb_id,
                        anidb_id: m.anidb_id,
                        genres: m.genres,
                        studio: m.studio,
                        tmdb_release_date: m.tmdb_release_date,
                    },
                );
            }
        } else if alias.starts_with('s')
            && let Ok(series_result) = serde_json::from_value::<SeriesResult>(value.clone())
        {
            let s = series_result.series;
            series.insert(
                s.tvdb_id,
                SeriesMetadata {
                    tvdb_id: s.tvdb_id,
                    name: s.name,
                    sort_name: s.sort_name,
                    slug: s.slug,
                    year: s.year,
                    content_status: s.status,
                    first_aired: s.first_aired,
                    overview: s.overview,
                    network: s.network,
                    runtime_minutes: s.runtime_minutes,
                    poster_url: normalize_artwork_url(&s.poster_url),
                    banner_url: pick_artwork_url(&s.artworks, "banner"),
                    background_url: pick_artwork_url(&s.artworks, "background"),
                    country: s.country,
                    genres: s.genres,
                    aliases: s.aliases,
                    tagged_aliases: s
                        .tagged_aliases
                        .into_iter()
                        .map(|ta| scryer_domain::TaggedAlias {
                            name: ta.name,
                            language: ta.language,
                        })
                        .collect(),
                    seasons: s
                        .seasons
                        .into_iter()
                        .map(|season| SeasonMetadata {
                            tvdb_id: season.tvdb_id,
                            number: season.number,
                            label: season.label,
                            episode_type: season.episode_type,
                        })
                        .collect(),
                    episodes: s
                        .episodes
                        .into_iter()
                        .map(|ep| EpisodeMetadata {
                            tvdb_id: ep.tvdb_id,
                            episode_number: ep.episode_number,
                            name: ep.name,
                            aired: ep.aired,
                            runtime_minutes: ep.runtime_minutes,
                            is_filler: ep.is_filler,
                            is_recap: ep.is_recap,
                            overview: ep.overview,
                            absolute_number: ep.absolute_number,
                            season_number: ep.season_number,
                        })
                        .collect(),
                    anime_mappings: s
                        .anime_mappings
                        .into_iter()
                        .map(|m| AnimeMapping {
                            mal_id: m.mal_id,
                            mal_dub_id: m.mal_dub_id,
                            anilist_id: m.anilist_id,
                            anidb_id: m.anidb_id,
                            kitsu_id: m.kitsu_id,
                            simkl_id: m.simkl_id,
                            thetvdb_id: m.thetvdb_id,
                            themoviedb_id: m.themoviedb_id,
                            imdb_id: m.imdb_id,
                            trakt_id: m.trakt_id,
                            alt_tvdb_id: m.alt_tvdb_id,
                            thetvdb_season: m.thetvdb_season,
                            thetvdb_part: m.thetvdb_part,
                            score: m.score,
                            anime_media_type: m.anime_media_type.unwrap_or_default(),
                            global_media_type: m.global_media_type.unwrap_or_default(),
                            status: m.status.unwrap_or_default(),
                            mapping_type: m.mapping_type.unwrap_or_default(),
                            episode_mappings: m
                                .episode_mappings
                                .into_iter()
                                .map(|e| AnimeEpisodeMapping {
                                    tvdb_season: e.tvdb_season,
                                    episode_start: e.episode_start,
                                    episode_end: e.episode_end,
                                })
                                .collect(),
                        })
                        .collect(),
                    anime_movies: s
                        .anime_movies
                        .into_iter()
                        .map(|movie| AnimeMovie {
                            movie_tvdb_id: movie.movie_tvdb_id,
                            movie_tmdb_id: movie.movie_tmdb_id,
                            movie_imdb_id: movie.movie_imdb_id,
                            movie_mal_id: movie.movie_mal_id,
                            movie_anidb_id: movie.movie_anidb_id,
                            name: movie.name,
                            slug: movie.slug,
                            year: movie.year,
                            content_status: movie.content_status,
                            overview: movie.overview,
                            poster_url: movie.poster_url,
                            language: movie.language,
                            runtime_minutes: movie.runtime_minutes,
                            sort_title: movie.sort_title,
                            imdb_id: movie.imdb_id,
                            genres: movie.genres,
                            studio: movie.studio,
                            digital_release_date: movie.digital_release_date,
                            association_confidence: movie.association_confidence,
                            continuity_status: movie.continuity_status,
                            movie_form: movie.movie_form,
                            placement: movie.placement,
                            confidence: movie.confidence,
                            signal_summary: movie.signal_summary,
                        })
                        .collect(),
                },
            );
        }
    }
}

fn build_bulk_mixed_query(movie_ids: &[i64], series_ids: &[i64], language: &str) -> String {
    let mut q = String::from("query {\n");
    for (i, &id) in movie_ids.iter().enumerate() {
        let _ = writeln!(
            q,
            "  m{i}: movie(tvdbId: {id}, language: \"{language}\") {{ movie {{ ...MovieFields }} }}"
        );
    }
    for (i, &id) in series_ids.iter().enumerate() {
        let _ = writeln!(
            q,
            "  s{i}: series(id: \"{id}\", includeEpisodes: true, language: \"{language}\") {{ series {{ ...SeriesFields }} }}"
        );
    }
    q.push_str("}\n");

    if !movie_ids.is_empty() {
        q.push_str(graphql_docs::MOVIE_FIELDS_FRAGMENT);
        q.push('\n');
    }

    if !series_ids.is_empty() {
        q.push_str(graphql_docs::SERIES_FIELDS_FRAGMENT);
        q.push('\n');
    }

    q
}

#[cfg(test)]
mod tests {
    use super::{
        MetadataSearchQuery, build_bulk_mixed_query, build_search_tvdb_batch_query,
        compatibility_poll_phase, enrollment_retry_delay, next_version_compatibility_poll_delay_at,
        normalize_artwork_url, normalize_optional_artwork_url, parse_version_compatibility_success,
    };
    use std::time::{Duration, SystemTime};

    use crate::{
        graphql::metadata_gateway as graphql_docs,
        smg_enrollment::{EnrollmentError, RateLimited},
    };

    #[test]
    fn bulk_series_query_requests_tagged_aliases() {
        let query = build_bulk_mixed_query(&[], &[424536], "eng");

        assert!(query.contains("...SeriesFields"));
        assert!(query.contains("fragment SeriesFields on TvdbSeries"));
        assert!(query.contains("tagged_aliases"));
        assert!(query.contains("language"));
    }

    #[test]
    fn search_tvdb_batch_queries_trim_dedupe_and_preserve_first_seen_order() {
        let queries = vec![
            MetadataSearchQuery {
                query: "  Lantern Tide  ".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2001),
            },
            MetadataSearchQuery {
                query: "Lantern Tide".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2001),
            },
            MetadataSearchQuery {
                query: "   ".to_string(),
                type_hint: "series".to_string(),
                year: None,
            },
            MetadataSearchQuery {
                query: "Velvet Comet".to_string(),
                type_hint: "anime".to_string(),
                year: None,
            },
            MetadataSearchQuery {
                query: "Lantern Tide".to_string(),
                type_hint: "movie".to_string(),
                year: Some(2002),
            },
        ];

        let normalized = build_search_tvdb_batch_query(&queries);

        assert_eq!(normalized.len(), 3);
        assert_eq!(normalized[0].query, "Lantern Tide");
        assert_eq!(normalized[0].type_hint, "movie");
        assert_eq!(normalized[0].year, Some(2001));
        assert_eq!(normalized[1].query, "Velvet Comet");
        assert_eq!(normalized[1].type_hint, "anime");
        assert_eq!(normalized[1].year, None);
        assert_eq!(normalized[2].query, "Lantern Tide");
        assert_eq!(normalized[2].type_hint, "movie");
        assert_eq!(normalized[2].year, Some(2002));
    }

    #[test]
    fn search_tvdb_batch_query_uses_dedicated_field() {
        assert!(graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("searchTvdbBatch"));
        assert!(!graphql_docs::SEARCH_TVDB_BATCH_QUERY.contains("searchTvdb(query:"));
    }

    #[test]
    fn normalize_artwork_url_collapses_duplicate_path_separators() {
        let url = "https://artworks.thetvdb.com/banners/movies/147325/backgrounds//5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg";

        assert_eq!(
            normalize_artwork_url(url),
            "https://artworks.thetvdb.com/banners/movies/147325/backgrounds/5vyMUvxy6W0xU9Unnh5M7WXkh4l.jpg"
        );
    }

    #[test]
    fn normalize_optional_artwork_url_preserves_missing_and_existing_urls() {
        assert_eq!(normalize_optional_artwork_url(None), None);
        assert_eq!(
            normalize_optional_artwork_url(Some(
                "https://artworks.thetvdb.com/banners/posters/example.jpg".to_string()
            )),
            Some("https://artworks.thetvdb.com/banners/posters/example.jpg".to_string())
        );
    }

    #[test]
    fn enrollment_retry_delay_prefers_rate_limit_header_delay() {
        let delay = enrollment_retry_delay(
            &EnrollmentError::RateLimited(RateLimited {
                retry_after: Some(Duration::from_secs(75)),
                message: "cloudflare rate limit".to_string(),
            }),
            0,
        );

        assert_eq!(delay, Duration::from_secs(75));
    }

    #[test]
    fn enrollment_retry_delay_falls_back_when_header_is_missing() {
        let delay = enrollment_retry_delay(
            &EnrollmentError::RateLimited(RateLimited {
                retry_after: None,
                message: "cloudflare rate limit".to_string(),
            }),
            1,
        );

        assert_eq!(delay, Duration::from_secs(60));
    }

    #[test]
    fn compatibility_poll_phase_is_stable_and_bounded() {
        let first = compatibility_poll_phase("instance-a");
        let second = compatibility_poll_phase("instance-a");
        let different = compatibility_poll_phase("instance-b");

        assert_eq!(first, second);
        assert!(first < Duration::from_secs(6 * 60 * 60));
        assert!(different < Duration::from_secs(6 * 60 * 60));
    }

    #[test]
    fn next_version_compatibility_poll_delay_skips_slots_inside_startup_guard() {
        let now = std::time::UNIX_EPOCH + Duration::from_secs(6 * 60 * 60 + 5 * 60);
        let phase = Duration::from_secs(10 * 60);

        let delay =
            next_version_compatibility_poll_delay_at(now, phase, Duration::from_secs(30 * 60));

        assert_eq!(delay, Duration::from_secs(6 * 60 * 60 + 5 * 60));
    }

    #[test]
    fn next_version_compatibility_poll_delay_uses_next_ring_slot_without_guard() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(6 * 60 * 60 + 5 * 60);
        let phase = Duration::from_secs(10 * 60);

        let delay = next_version_compatibility_poll_delay_at(now, phase, Duration::from_secs(0));

        assert_eq!(delay, Duration::from_secs(5 * 60));
    }

    #[test]
    fn parse_version_compatibility_success_returns_none_for_supported() {
        let body = br#"{
            "compatibility": {
                "status": "supported",
                "minimum_version": "",
                "your_version": "0.12.0",
                "message": ""
            }
        }"#;

        let notice = parse_version_compatibility_success(body).expect("parse supported response");

        assert!(notice.is_none());
    }

    #[test]
    fn parse_version_compatibility_success_preserves_deprecated_notice() {
        let body = br#"{
            "compatibility": {
                "status": "deprecated",
                "minimum_version": "0.12.1",
                "your_version": "0.12.0",
                "message": "Upgrade recommended soon.",
                "upgrade_deadline": "2026-05-31"
            }
        }"#;

        let notice = parse_version_compatibility_success(body).expect("parse deprecated response");
        let notice = notice.expect("deprecated notice");

        assert_eq!(notice.status, "deprecated");
        assert_eq!(notice.minimum_version, "0.12.1");
        assert_eq!(notice.your_version, "0.12.0");
        assert_eq!(notice.message, "Upgrade recommended soon.");
        assert_eq!(notice.upgrade_deadline.as_deref(), Some("2026-05-31"));
    }
}

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

// --- Search types ---

#[derive(Deserialize)]
struct SearchTvdbResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbResult,
}

#[derive(Deserialize)]
struct SearchTvdbBatchResponse {
    #[serde(rename = "searchTvdbBatch")]
    search_tvdb_batch: Vec<SearchTvdbBatchResult>,
}

#[derive(Deserialize)]
struct SearchTvdbBatchResult {
    query: String,
    #[serde(rename = "type")]
    type_hint: String,
    year: Option<i32>,
    results: Vec<SearchTvdbItem>,
}

#[derive(Deserialize)]
struct SearchTvdbResult {
    results: Vec<SearchTvdbItem>,
}

#[derive(Deserialize)]
struct SearchTvdbItem {
    #[serde(rename = "tvdb_id")]
    tvdb_id: i64,
    name: String,
    year: Option<i32>,
}

#[derive(Deserialize)]
struct SearchTvdbRichItem {
    tvdb_id: i64,
    name: String,
    imdb_id: Option<String>,
    slug: Option<String>,
    #[serde(rename = "type")]
    type_hint: Option<String>,
    year: Option<i32>,
    status: Option<String>,
    overview: Option<String>,
    popularity: Option<f64>,
    poster_url: Option<String>,
    language: Option<String>,
    runtime_minutes: Option<i32>,
    sort_title: Option<String>,
}

#[derive(Deserialize)]
struct SearchTvdbRichResponse {
    #[serde(rename = "searchTvdb")]
    search_tvdb: SearchTvdbRichResult,
}

#[derive(Deserialize)]
struct SearchTvdbRichResult {
    results: Vec<SearchTvdbRichItem>,
}

// --- Multi-search types ---

#[derive(Deserialize)]
struct SearchTvdbMultiResponse {
    #[serde(rename = "searchTvdbMulti")]
    search_tvdb_multi: SearchTvdbMultiResult,
}

#[derive(Deserialize)]
struct SearchTvdbMultiResult {
    movies: Vec<SearchTvdbRichItem>,
    series: Vec<SearchTvdbRichItem>,
    anime: Vec<SearchTvdbRichItem>,
}

// --- Movie types ---

#[derive(Deserialize)]
struct MovieResponse {
    movie: MovieResult,
}

#[derive(Deserialize)]
struct MovieResult {
    movie: MovieItem,
}

#[derive(Deserialize)]
struct MovieItem {
    tvdb_id: i64,
    name: String,
    slug: String,
    year: Option<i32>,
    status: String,
    overview: String,
    poster_url: String,
    language: String,
    runtime_minutes: i32,
    sort_title: String,
    imdb_id: String,
    #[serde(default)]
    anidb_id: Option<i64>,
    genres: Vec<String>,
    studio: String,
    tmdb_release_date: Option<String>,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
}

// --- Artwork helper ---

#[derive(Deserialize)]
struct ArtworkItem {
    kind: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct TaggedAliasItem {
    name: String,
    language: String,
}

fn pick_artwork_url(artworks: &[ArtworkItem], kind: &str) -> Option<String> {
    artworks
        .iter()
        .find(|a| a.kind == kind)
        .map(|a| normalize_artwork_url(&a.url))
}

fn normalize_optional_artwork_url(url: Option<String>) -> Option<String> {
    url.map(|value| normalize_artwork_url(&value))
}

fn bounded_exponential_backoff(attempt: u32, base: Duration, max: Duration) -> Duration {
    let multiplier = 1u32 << attempt.min(4);
    let delay = base.saturating_mul(multiplier);
    if delay > max { max } else { delay }
}

fn enrollment_retry_delay(error: &smg_enrollment::EnrollmentError, attempt: u32) -> Duration {
    if let smg_enrollment::EnrollmentError::RateLimited(rate_limited) = error
        && let Some(retry_after) = rate_limited.retry_after
        && !retry_after.is_zero()
    {
        return retry_after;
    }

    bounded_exponential_backoff(attempt, Duration::from_secs(30), Duration::from_secs(300))
}

fn metadata_gateway_transient_delay(attempt: u32) -> Duration {
    bounded_exponential_backoff(
        attempt,
        METADATA_GATEWAY_TRANSIENT_BASE_DELAY,
        METADATA_GATEWAY_TRANSIENT_MAX_DELAY,
    )
}

fn map_metadata_gateway_outbound_error(request_label: &str, error: OutboundHttpError) -> AppError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => {
            let retry_after_seconds = rate_limited
                .retry_after
                .filter(|delay| !delay.is_zero())
                .map(|delay| delay.as_secs());
            AppError::Repository(match retry_after_seconds {
                Some(seconds) if seconds > 0 => format!(
                    "{request_label} was rate limited by the metadata gateway; retry after {seconds}s"
                ),
                _ => format!("{request_label} was rate limited by the metadata gateway"),
            })
        }
        OutboundHttpError::Transport { source, .. } => {
            AppError::Repository(format!("{request_label} failed: {source}"))
        }
    }
}

fn normalize_artwork_url(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let Ok(mut parsed) = reqwest::Url::parse(trimmed) else {
        return trimmed.to_string();
    };

    let normalized_path = parsed
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    parsed.set_path(&format!("/{normalized_path}"));

    parsed.to_string()
}

// --- Series types ---

#[derive(Deserialize)]
struct SeriesResponse {
    series: SeriesResult,
}

#[derive(Deserialize)]
struct SeriesResult {
    series: SeriesItem,
}

#[derive(Deserialize)]
struct SeriesItem {
    tvdb_id: i64,
    name: String,
    sort_name: String,
    slug: String,
    status: String,
    year: Option<i32>,
    first_aired: String,
    overview: String,
    network: String,
    runtime_minutes: i32,
    poster_url: String,
    country: String,
    genres: Vec<String>,
    aliases: Vec<String>,
    #[serde(default)]
    tagged_aliases: Vec<TaggedAliasItem>,
    #[serde(default)]
    artworks: Vec<ArtworkItem>,
    seasons: Vec<SeriesSeasonItem>,
    episodes: Vec<SeriesEpisodeItem>,
    #[serde(default)]
    anime_mappings: Vec<AnimeMappingItem>,
    #[serde(default)]
    anime_movies: Vec<AnimeMovieItem>,
}

#[derive(Deserialize)]
struct SeriesSeasonItem {
    tvdb_id: i64,
    number: i32,
    label: String,
    episode_type: String,
}

#[derive(Deserialize)]
struct SeriesEpisodeItem {
    tvdb_id: i64,
    episode_number: i32,
    season_number: i32,
    name: String,
    aired: String,
    runtime_minutes: i32,
    is_filler: bool,
    is_recap: bool,
    overview: String,
    absolute_number: String,
}

#[derive(Deserialize)]
struct AnimeMappingItem {
    mal_id: Option<i64>,
    mal_dub_id: Option<i64>,
    anilist_id: Option<i64>,
    anidb_id: Option<i64>,
    kitsu_id: Option<i64>,
    simkl_id: Option<i64>,
    thetvdb_id: Option<i64>,
    themoviedb_id: Option<i64>,
    imdb_id: Option<i64>,
    trakt_id: Option<i64>,
    alt_tvdb_id: Option<i64>,
    thetvdb_season: Option<i32>,
    thetvdb_part: Option<i32>,
    score: Option<f64>,
    anime_media_type: Option<String>,
    global_media_type: Option<String>,
    status: Option<String>,
    #[serde(default)]
    mapping_type: Option<String>,
    #[serde(default)]
    episode_mappings: Vec<AnimeEpisodeMappingItem>,
}

#[derive(Deserialize)]
struct AnimeEpisodeMappingItem {
    tvdb_season: i32,
    episode_start: i32,
    episode_end: i32,
}

#[derive(Deserialize)]
struct AnimeMovieItem {
    movie_tvdb_id: Option<i64>,
    movie_tmdb_id: Option<i64>,
    movie_imdb_id: Option<String>,
    movie_mal_id: Option<i64>,
    #[serde(default)]
    movie_anidb_id: Option<i64>,
    name: String,
    slug: String,
    year: Option<i32>,
    content_status: String,
    overview: String,
    poster_url: String,
    language: String,
    runtime_minutes: i32,
    sort_title: String,
    imdb_id: String,
    genres: Vec<String>,
    studio: String,
    digital_release_date: Option<String>,
    association_confidence: String,
    continuity_status: String,
    movie_form: String,
    placement: String,
    confidence: String,
    signal_summary: String,
}

#[async_trait]
impl MetadataGateway for MetadataGatewayClient {
    async fn search_tvdb(
        &self,
        query: &str,
        type_hint: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": 10,
            "year": year,
        });

        let data: SearchTvdbResponse = self
            .execute_graphql_apq(
                graphql_docs::SEARCH_TVDB_QUERY,
                &self.search_hash,
                variables,
            )
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| MetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                year: item.year,
            })
            .collect())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        let deduped_queries = build_search_tvdb_batch_query(queries);

        if deduped_queries.is_empty() {
            return Ok(HashMap::new());
        }

        let mut results = HashMap::new();

        for chunk in deduped_queries.chunks(METADATA_GATEWAY_MAX_SEARCH_BATCH) {
            let request_started_at = Instant::now();
            debug!(
                query_count = chunk.len(),
                "metadata gateway batched search request"
            );
            let request_inputs = chunk
                .iter()
                .map(|query| SearchTvdbBatchRequestInput {
                    query: query.query.clone(),
                    type_hint: query.type_hint.clone(),
                    year: query.year,
                    limit: 10,
                })
                .collect::<Vec<_>>();
            let payload = json!({
                "query": graphql_docs::SEARCH_TVDB_BATCH_QUERY,
                "variables": {
                    "requests": request_inputs,
                    "language": language,
                },
            });
            let data: SearchTvdbBatchResponse = self.execute_graphql(payload).await?;
            debug!(
                query_count = chunk.len(),
                elapsed_ms = request_started_at.elapsed().as_millis() as u64,
                "metadata gateway batched search complete"
            );
            for item in data.search_tvdb_batch {
                let query_spec = MetadataSearchQuery {
                    query: item.query,
                    type_hint: item.type_hint,
                    year: item.year,
                };
                let items = item
                    .results
                    .into_iter()
                    .map(|entry| MetadataSearchItem {
                        tvdb_id: entry.tvdb_id.to_string(),
                        name: entry.name,
                        year: entry.year,
                    })
                    .collect::<Vec<_>>();
                results.insert(query_spec, items);
            }

            for query in chunk {
                results.entry(query.clone()).or_default();
            }
        }

        Ok(results)
    }

    async fn search_tvdb_rich(
        &self,
        query: &str,
        type_hint: &str,
        limit: i32,
        language: &str,
        year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        let variables = json!({
            "query": query,
            "type": type_hint,
            "limit": limit,
            "language": language,
            "year": year,
        });

        let data: SearchTvdbRichResponse = self
            .execute_graphql_apq(
                graphql_docs::SEARCH_TVDB_RICH_QUERY,
                &self.search_rich_hash,
                variables,
            )
            .await?;

        Ok(data
            .search_tvdb
            .results
            .into_iter()
            .map(|item| RichMetadataSearchItem {
                tvdb_id: item.tvdb_id.to_string(),
                name: item.name,
                imdb_id: item.imdb_id,
                slug: item.slug,
                type_hint: item.type_hint,
                year: item.year,
                status: item.status,
                overview: item.overview,
                popularity: item.popularity,
                poster_url: normalize_optional_artwork_url(item.poster_url),
                language: item.language,
                runtime_minutes: item.runtime_minutes,
                sort_title: item.sort_title,
            })
            .collect())
    }

    async fn search_tvdb_multi(
        &self,
        query: &str,
        limit: i32,
        language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        let variables = json!({
            "query": query,
            "limit": limit,
            "language": language,
        });

        let data: SearchTvdbMultiResponse = self
            .execute_graphql_apq(
                graphql_docs::SEARCH_TVDB_MULTI_QUERY,
                &self.search_multi_hash,
                variables,
            )
            .await?;

        let convert = |items: Vec<SearchTvdbRichItem>| -> Vec<RichMetadataSearchItem> {
            items
                .into_iter()
                .map(|item| RichMetadataSearchItem {
                    tvdb_id: item.tvdb_id.to_string(),
                    name: item.name,
                    imdb_id: item.imdb_id,
                    slug: item.slug,
                    type_hint: item.type_hint,
                    year: item.year,
                    status: item.status,
                    overview: item.overview,
                    popularity: item.popularity,
                    poster_url: normalize_optional_artwork_url(item.poster_url),
                    language: item.language,
                    runtime_minutes: item.runtime_minutes,
                    sort_title: item.sort_title,
                })
                .collect()
        };

        Ok(MultiMetadataSearchResult {
            movies: convert(data.search_tvdb_multi.movies),
            series: convert(data.search_tvdb_multi.series),
            anime: convert(data.search_tvdb_multi.anime),
        })
    }

    async fn get_movie(&self, tvdb_id: i64, language: &str) -> AppResult<MovieMetadata> {
        let variables = json!({
            "tvdbId": tvdb_id,
            "language": language,
        });

        let data: MovieResponse = self
            .execute_graphql_apq(graphql_docs::GET_MOVIE_QUERY, &self.movie_hash, variables)
            .await?;
        let m = data.movie.movie;

        Ok(MovieMetadata {
            tvdb_id: m.tvdb_id,
            name: m.name,
            slug: m.slug,
            year: m.year,
            content_status: m.status,
            overview: m.overview,
            poster_url: normalize_artwork_url(&m.poster_url),
            banner_url: pick_artwork_url(&m.artworks, "banner"),
            background_url: pick_artwork_url(&m.artworks, "background"),
            language: m.language,
            runtime_minutes: m.runtime_minutes,
            sort_title: m.sort_title,
            imdb_id: m.imdb_id,
            anidb_id: m.anidb_id,
            genres: m.genres,
            studio: m.studio,
            tmdb_release_date: m.tmdb_release_date,
        })
    }

    async fn get_series(&self, tvdb_id: i64, language: &str) -> AppResult<SeriesMetadata> {
        let variables = json!({
            "id": tvdb_id.to_string(),
            "includeEpisodes": true,
            "language": language,
        });

        let data: SeriesResponse = self
            .execute_graphql_apq(graphql_docs::GET_SERIES_QUERY, &self.series_hash, variables)
            .await?;
        let s = data.series.series;

        Ok(SeriesMetadata {
            tvdb_id: s.tvdb_id,
            name: s.name,
            sort_name: s.sort_name,
            slug: s.slug,
            year: s.year,
            content_status: s.status,
            first_aired: s.first_aired,
            overview: s.overview,
            network: s.network,
            runtime_minutes: s.runtime_minutes,
            poster_url: normalize_artwork_url(&s.poster_url),
            banner_url: pick_artwork_url(&s.artworks, "banner"),
            background_url: pick_artwork_url(&s.artworks, "background"),
            country: s.country,
            genres: s.genres,
            aliases: s.aliases,
            tagged_aliases: s
                .tagged_aliases
                .into_iter()
                .map(|ta| scryer_domain::TaggedAlias {
                    name: ta.name,
                    language: ta.language,
                })
                .collect(),
            seasons: s
                .seasons
                .into_iter()
                .map(|season| SeasonMetadata {
                    tvdb_id: season.tvdb_id,
                    number: season.number,
                    label: season.label,
                    episode_type: season.episode_type,
                })
                .collect(),
            episodes: s
                .episodes
                .into_iter()
                .map(|ep| EpisodeMetadata {
                    tvdb_id: ep.tvdb_id,
                    episode_number: ep.episode_number,
                    name: ep.name,
                    aired: ep.aired,
                    runtime_minutes: ep.runtime_minutes,
                    is_filler: ep.is_filler,
                    is_recap: ep.is_recap,
                    overview: ep.overview,
                    absolute_number: ep.absolute_number,
                    season_number: ep.season_number,
                })
                .collect(),
            anime_mappings: s
                .anime_mappings
                .into_iter()
                .map(|m| AnimeMapping {
                    mal_id: m.mal_id,
                    mal_dub_id: m.mal_dub_id,
                    anilist_id: m.anilist_id,
                    anidb_id: m.anidb_id,
                    kitsu_id: m.kitsu_id,
                    simkl_id: m.simkl_id,
                    thetvdb_id: m.thetvdb_id,
                    themoviedb_id: m.themoviedb_id,
                    imdb_id: m.imdb_id,
                    trakt_id: m.trakt_id,
                    alt_tvdb_id: m.alt_tvdb_id,
                    thetvdb_season: m.thetvdb_season,
                    thetvdb_part: m.thetvdb_part,
                    score: m.score,
                    anime_media_type: m.anime_media_type.unwrap_or_default(),
                    global_media_type: m.global_media_type.unwrap_or_default(),
                    status: m.status.unwrap_or_default(),
                    mapping_type: m.mapping_type.unwrap_or_default(),
                    episode_mappings: m
                        .episode_mappings
                        .into_iter()
                        .map(|e| AnimeEpisodeMapping {
                            tvdb_season: e.tvdb_season,
                            episode_start: e.episode_start,
                            episode_end: e.episode_end,
                        })
                        .collect(),
                })
                .collect(),
            anime_movies: s
                .anime_movies
                .into_iter()
                .map(|movie| AnimeMovie {
                    movie_tvdb_id: movie.movie_tvdb_id,
                    movie_tmdb_id: movie.movie_tmdb_id,
                    movie_imdb_id: movie.movie_imdb_id,
                    movie_mal_id: movie.movie_mal_id,
                    movie_anidb_id: movie.movie_anidb_id,
                    name: movie.name,
                    slug: movie.slug,
                    year: movie.year,
                    content_status: movie.content_status,
                    overview: movie.overview,
                    poster_url: movie.poster_url,
                    language: movie.language,
                    runtime_minutes: movie.runtime_minutes,
                    sort_title: movie.sort_title,
                    imdb_id: movie.imdb_id,
                    genres: movie.genres,
                    studio: movie.studio,
                    digital_release_date: movie.digital_release_date,
                    association_confidence: movie.association_confidence,
                    continuity_status: movie.continuity_status,
                    movie_form: movie.movie_form,
                    placement: movie.placement,
                    confidence: movie.confidence,
                    signal_summary: movie.signal_summary,
                })
                .collect(),
        })
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        series_tvdb_ids: &[i64],
        language: &str,
    ) -> AppResult<BulkMetadataResult> {
        if movie_tvdb_ids.is_empty() && series_tvdb_ids.is_empty() {
            return Ok(BulkMetadataResult::default());
        }

        let unique_movies: Vec<i64> = movie_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let unique_series: Vec<i64> = series_tvdb_ids
            .iter()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let request_started_at = Instant::now();

        debug!(
            movies = unique_movies.len(),
            series = unique_series.len(),
            "bulk metadata request"
        );

        let mut movies = HashMap::new();
        let mut series = HashMap::new();

        let bulk_requests = build_bulk_metadata_alias_requests(&unique_movies, &unique_series);
        for chunk in bulk_requests.chunks(METADATA_GATEWAY_MAX_BULK_METADATA_ALIAS_BATCH) {
            let mut chunk_movie_ids = Vec::new();
            let mut chunk_series_ids = Vec::new();
            for request in chunk {
                match request {
                    BulkMetadataAliasRequest::Movie(tvdb_id) => chunk_movie_ids.push(*tvdb_id),
                    BulkMetadataAliasRequest::Series(tvdb_id) => chunk_series_ids.push(*tvdb_id),
                }
            }

            let query = build_bulk_mixed_query(&chunk_movie_ids, &chunk_series_ids, language);
            let data = self.post_batched_graphql_partial(&query).await?;
            merge_bulk_metadata_partial(&data, &mut movies, &mut series);
        }

        debug!(
            movies_resolved = movies.len(),
            series_resolved = series.len(),
            elapsed_ms = request_started_at.elapsed().as_millis() as u64,
            "bulk metadata complete"
        );
        Ok(BulkMetadataResult { movies, series })
    }
}
