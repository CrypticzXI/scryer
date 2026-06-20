use std::sync::{LazyLock, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use aws_lc_rs::{digest, hmac, rand::SecureRandom};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ml_dsa::{Keypair, MlDsa65, SigningKey};
use scryer_application::{SettingsRepository, SmgScryerUpdateNotice};
use scryer_outbound_http::{
    OutboundHttpClient, OutboundHttpError, OutboundRequestError, RateLimitRegistry, RequestPolicy,
    parse_retry_after, smg_reqwest_client,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::metadata::response_body::read_response_body_preview;

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const PQ_CLIENT_FAMILY: &str = "scryer-stable";
pub(crate) const PQ_AUTH_VERSION_ENV: &str = "SCRYER_SMG_PQ_AUTH_VERSION";
const PQ_AUTH_VERSION_V1: &str = "pqsig-v1";
const PQ_AUTH_VERSION_V2: &str = "pqsig-v2";
const PQ_AUTH_NONCE_BYTES: usize = 24;
// ML-DSA key generation and signing allocate large fixed-size temporaries on the stack.
// Keep that work off Tokio runtime threads and on a dedicated thread with an explicit stack.
const PQ_CRYPTO_THREAD_STACK_SIZE_BYTES: usize = 16 * 1024 * 1024;
const SMG_VERSION_COMPATIBILITY_NOTICE_KEY: &str = "smg.version_compatibility_notice";
const SMG_SCRYER_UPDATE_NOTICE_KEY: &str = "smg.scryer_update_notice";

static SMG_ENROLLMENT_RATE_LIMITS: LazyLock<RateLimitRegistry> =
    LazyLock::new(RateLimitRegistry::new);
static CONFIGURED_PQ_AUTH_VERSION: LazyLock<PqAuthVersion> = LazyLock::new(|| {
    let raw = std::env::var(PQ_AUTH_VERSION_ENV).ok();
    match parse_pq_auth_version(raw.as_deref()) {
        Some(version) => version,
        None => {
            if let Some(raw) = raw
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                warn!(
                    env_var = PQ_AUTH_VERSION_ENV,
                    configured = raw,
                    fallback = PQ_AUTH_VERSION_V2,
                    "invalid SMG PQ auth version override; using default"
                );
            }
            PqAuthVersion::V2
        }
    }
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PqAuthVersion {
    V1,
    V2,
}

impl PqAuthVersion {
    pub(crate) fn header_value(self) -> &'static str {
        match self {
            Self::V1 => PQ_AUTH_VERSION_V1,
            Self::V2 => PQ_AUTH_VERSION_V2,
        }
    }
}

fn parse_pq_auth_version(raw: Option<&str>) -> Option<PqAuthVersion> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => Some(PqAuthVersion::V2),
        Some(value) if value.eq_ignore_ascii_case("v1") => Some(PqAuthVersion::V1),
        Some(value) if value.eq_ignore_ascii_case(PQ_AUTH_VERSION_V1) => Some(PqAuthVersion::V1),
        Some(value) if value.eq_ignore_ascii_case("v2") => Some(PqAuthVersion::V2),
        Some(value) if value.eq_ignore_ascii_case(PQ_AUTH_VERSION_V2) => Some(PqAuthVersion::V2),
        Some(_) => None,
    }
}

pub(crate) fn configured_pq_auth_version() -> PqAuthVersion {
    *CONFIGURED_PQ_AUTH_VERSION
}

pub(crate) fn generate_pq_auth_nonce() -> Result<String, String> {
    let rng = aws_lc_rs::rand::SystemRandom::new();
    let mut nonce = [0u8; PQ_AUTH_NONCE_BYTES];
    rng.fill(&mut nonce)
        .map_err(|_| "failed to generate SMG PQ auth nonce".to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(nonce))
}

/// Returned when SMG reports a version compatibility issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionIncompatible {
    pub status: String,
    pub minimum_version: String,
    pub your_version: String,
    pub message: String,
    pub upgrade_deadline: Option<String>,
}

/// Returned when SMG rejects registration due to a rate limit.
#[derive(Debug, Clone)]
pub struct RateLimited {
    pub retry_after: Option<Duration>,
    pub message: String,
}

/// Errors that can occur during SMG enrollment.
#[derive(Debug)]
pub enum EnrollmentError {
    VersionIncompatible(VersionIncompatible),
    RateLimited(RateLimited),
    Other(String),
}

impl std::fmt::Display for EnrollmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::VersionIncompatible(v) => write!(
                f,
                "version compatibility issue: status={}, minimum={}, yours={}, deadline={:?}, message={}",
                v.status, v.minimum_version, v.your_version, v.upgrade_deadline, v.message
            ),
            Self::RateLimited(rate_limited) => {
                if let Some(retry_after) = rate_limited.retry_after {
                    write!(
                        f,
                        "rate limited: retry_after={}s, message={}",
                        retry_after.as_secs(),
                        rate_limited.message
                    )
                } else {
                    write!(f, "rate limited: message={}", rate_limited.message)
                }
            }
            Self::Other(s) => f.write_str(s),
        }
    }
}

/// Cached enrollment state for the current Scryer instance.
pub struct EnrollmentState {
    pub instance_id: String,
    pub pq_seed_b64: Option<String>,
    pub pq_public_key_b64: Option<String>,
    pub pq_key_id: Option<String>,
    pub pq_enrollment_generation: Option<i64>,
}

#[derive(Deserialize)]
struct PqChallengeResponse {
    challenge_id: String,
    nonce: String,
}

#[derive(Deserialize)]
struct PqRegisterResponse {
    key_id: String,
    enrollment_generation: i64,
    #[serde(default)]
    opensubtitles_api_key: Option<String>,
}

/// Load or generate the instance ID (UUIDv4) for this Scryer instance.
pub async fn ensure_instance_id(db: &dyn SettingsRepository) -> Result<String, String> {
    let existing = load_setting(db, "smg.instance_id").await?;

    if let Some(id) = existing
        && !id.is_empty()
    {
        return Ok(id);
    }

    let instance_id = uuid::Uuid::new_v4().to_string();
    info!(instance_id = %instance_id, "generated new SMG instance ID");

    persist_setting(db, "smg.instance_id", &instance_id).await?;

    Ok(instance_id)
}

/// Clear cached enrollment data from the database so the next call to
/// `ensure_enrolled` performs a fresh registration.
pub async fn clear_enrollment_cache(db: &dyn SettingsRepository) -> Result<(), String> {
    for key in &[
        "smg.client_key",
        "smg.client_cert",
        "smg.cert_expires_at",
        "smg.ca_cert",
        "smg.pq_seed",
        "smg.pq_public_key",
        "smg.pq_key_id",
        "smg.pq_enrollment_generation",
    ] {
        persist_setting(db, key, "").await?;
    }
    Ok(())
}

/// Load existing PQ enrollment from DB, or enroll with SMG if missing.
pub async fn ensure_enrolled(
    db: &dyn SettingsRepository,
    registration_url: &str,
    registration_secret: &str,
) -> Result<EnrollmentState, EnrollmentError> {
    let instance_id = ensure_instance_id(db)
        .await
        .map_err(EnrollmentError::Other)?;
    let pq_seed = load_setting(db, "smg.pq_seed")
        .await
        .map_err(EnrollmentError::Other)?;
    let pq_public_key = load_setting(db, "smg.pq_public_key")
        .await
        .map_err(EnrollmentError::Other)?;
    let pq_key_id = load_setting(db, "smg.pq_key_id")
        .await
        .map_err(EnrollmentError::Other)?;
    let pq_enrollment_generation = load_pq_enrollment_generation(db)
        .await
        .map_err(EnrollmentError::Other)?;

    if let (Some(pq_seed), Some(pq_public_key), Some(pq_key_id)) =
        (pq_seed, pq_public_key, pq_key_id)
        && !pq_seed.is_empty()
        && !pq_public_key.is_empty()
        && !pq_key_id.is_empty()
    {
        debug!(%instance_id, pq_key_id, "using cached SMG PQ enrollment");
        return Ok(EnrollmentState {
            instance_id,
            pq_seed_b64: Some(pq_seed),
            pq_public_key_b64: Some(pq_public_key),
            pq_key_id: Some(pq_key_id),
            pq_enrollment_generation,
        });
    }

    enroll_pq_with_smg(db, &instance_id, registration_url, registration_secret).await
}

async fn enroll_pq_with_smg(
    db: &dyn SettingsRepository,
    instance_id: &str,
    registration_url: &str,
    registration_secret: &str,
) -> Result<EnrollmentState, EnrollmentError> {
    let challenge_url = derive_registration_endpoint(registration_url, "/api/register-challenge")?;
    let register_key_url = derive_registration_endpoint(registration_url, "/api/register-key")?;
    let http = enrollment_outbound_http_client();
    let challenge_payload = serde_json::json!({
        "client_family": PQ_CLIENT_FAMILY,
        "instance_id": instance_id,
        "secret_id": PQ_CLIENT_FAMILY,
        "version": env!("CARGO_PKG_VERSION"),
    });
    let challenge_response = http
        .send(enrollment_request_policy("smg_pq_challenge"), || {
            http.client()
                .post(challenge_url.clone())
                .json(&challenge_payload)
        })
        .await
        .map_err(|error| map_enrollment_outbound_error("SMG PQ challenge request", error))?;

    if !challenge_response.status().is_success() {
        return Err(registration_response_error(challenge_response, "SMG PQ challenge").await);
    }

    let challenge: PqChallengeResponse = challenge_response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG PQ challenge response: {e}"))
    })?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(challenge.nonce.as_bytes())
        .map_err(|e| EnrollmentError::Other(format!("invalid PQ challenge nonce: {e}")))?;
    let pq_key = generate_pq_keypair().await?;
    let proof_message = pq_registration_proof_message(
        "bootstrap",
        &challenge.challenge_id,
        &nonce,
        PQ_CLIENT_FAMILY,
        instance_id,
        &pq_key.key_id,
        &pq_key.public_key_b64,
    );
    let proof_signature = sign_pq_seed(&pq_key.seed_b64, &proof_message)
        .await
        .map_err(EnrollmentError::Other)?;
    let bootstrap_mac = sign_bootstrap_mac(registration_secret, &proof_message);

    let register_payload = serde_json::json!({
        "challenge_id": challenge.challenge_id,
        "client_family": PQ_CLIENT_FAMILY,
        "instance_id": instance_id,
        "secret_id": PQ_CLIENT_FAMILY,
        "version": env!("CARGO_PKG_VERSION"),
        "public_key": pq_key.public_key_b64,
        "key_id": pq_key.key_id,
        "proof_signature": proof_signature,
        "bootstrap_mac": bootstrap_mac,
    });
    let response = http
        .send(enrollment_request_policy("smg_pq_register"), || {
            http.client()
                .post(register_key_url.clone())
                .json(&register_payload)
        })
        .await
        .map_err(|error| map_enrollment_outbound_error("SMG PQ registration request", error))?;

    if !response.status().is_success() {
        return Err(registration_response_error(response, "SMG PQ registration").await);
    }

    let reg: PqRegisterResponse = response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG PQ registration response: {e}"))
    })?;
    if reg.key_id != pq_key.key_id {
        return Err(EnrollmentError::Other(
            "SMG PQ registration returned mismatched key_id".to_string(),
        ));
    }

    persist_pq_enrollment_state(
        db,
        &pq_key.seed_b64,
        &pq_key.public_key_b64,
        &pq_key.key_id,
        reg.enrollment_generation,
    )
    .await
    .map_err(EnrollmentError::Other)?;

    if let Some(os_key) = &reg.opensubtitles_api_key
        && !os_key.is_empty()
    {
        persist_setting(db, "subtitles.opensubtitles_api_key", os_key)
            .await
            .map_err(EnrollmentError::Other)?;
        info!("OpenSubtitles API key received from SMG");
    }

    info!(
        instance_id,
        key_id = pq_key.key_id,
        "enrolled with SMG using PQ request signing"
    );

    Ok(EnrollmentState {
        instance_id: instance_id.to_string(),
        pq_seed_b64: Some(pq_key.seed_b64),
        pq_public_key_b64: Some(pq_key.public_key_b64),
        pq_key_id: Some(pq_key.key_id),
        pq_enrollment_generation: Some(reg.enrollment_generation),
    })
}

pub async fn rotate_pq_enrollment(
    db: &dyn SettingsRepository,
    instance_id: &str,
    current_seed_b64: &str,
    current_key_id: &str,
    registration_url: &str,
) -> Result<EnrollmentState, EnrollmentError> {
    let challenge_url =
        derive_registration_endpoint(registration_url, "/api/register-rotate-challenge")?;
    let rotate_url = derive_registration_endpoint(registration_url, "/api/register-rotate")?;
    let http = enrollment_outbound_http_client();

    let challenge_response = send_authenticated_pq_registration_request(
        &http,
        &challenge_url,
        current_seed_b64,
        current_key_id,
        &serde_json::json!({
            "client_family": PQ_CLIENT_FAMILY,
            "instance_id": instance_id,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
    .await?;

    if !challenge_response.status().is_success() {
        return Err(
            registration_response_error(challenge_response, "SMG PQ rotation challenge").await,
        );
    }

    let challenge: PqChallengeResponse = challenge_response.json().await.map_err(|e| {
        EnrollmentError::Other(format!(
            "failed to parse SMG PQ rotation challenge response: {e}"
        ))
    })?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(challenge.nonce.as_bytes())
        .map_err(|e| EnrollmentError::Other(format!("invalid PQ rotation challenge nonce: {e}")))?;
    let next_key = generate_pq_keypair().await?;
    let proof_message = pq_registration_proof_message(
        "rotate",
        &challenge.challenge_id,
        &nonce,
        PQ_CLIENT_FAMILY,
        instance_id,
        &next_key.key_id,
        &next_key.public_key_b64,
    );
    let proof_signature = sign_pq_seed(&next_key.seed_b64, &proof_message)
        .await
        .map_err(EnrollmentError::Other)?;

    let response = send_authenticated_pq_registration_request(
        &http,
        &rotate_url,
        current_seed_b64,
        current_key_id,
        &serde_json::json!({
            "challenge_id": challenge.challenge_id,
            "client_family": PQ_CLIENT_FAMILY,
            "instance_id": instance_id,
            "version": env!("CARGO_PKG_VERSION"),
            "public_key": next_key.public_key_b64,
            "key_id": next_key.key_id,
            "proof_signature": proof_signature,
        }),
    )
    .await?;

    if !response.status().is_success() {
        return Err(registration_response_error(response, "SMG PQ rotation").await);
    }

    let reg: PqRegisterResponse = response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG PQ rotation response: {e}"))
    })?;
    if reg.key_id != next_key.key_id {
        return Err(EnrollmentError::Other(
            "SMG PQ rotation returned mismatched key_id".to_string(),
        ));
    }

    persist_pq_enrollment_state(
        db,
        &next_key.seed_b64,
        &next_key.public_key_b64,
        &next_key.key_id,
        reg.enrollment_generation,
    )
    .await
    .map_err(EnrollmentError::Other)?;

    if let Some(os_key) = &reg.opensubtitles_api_key
        && !os_key.is_empty()
    {
        persist_setting(db, "subtitles.opensubtitles_api_key", os_key)
            .await
            .map_err(EnrollmentError::Other)?;
    }

    info!(
        instance_id,
        old_key_id = current_key_id,
        new_key_id = next_key.key_id,
        enrollment_generation = reg.enrollment_generation,
        "rotated SMG PQ request-signing key"
    );

    Ok(EnrollmentState {
        instance_id: instance_id.to_string(),
        pq_seed_b64: Some(next_key.seed_b64),
        pq_public_key_b64: Some(next_key.public_key_b64),
        pq_key_id: Some(next_key.key_id),
        pq_enrollment_generation: Some(reg.enrollment_generation),
    })
}

struct PqKeypair {
    seed_b64: String,
    public_key_b64: String,
    key_id: String,
}

enum PqCryptoJob {
    GenerateKeypair {
        reply: tokio::sync::oneshot::Sender<Result<PqKeypair, String>>,
    },
    SignSeed {
        seed_b64: String,
        message: Vec<u8>,
        reply: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
}

struct PqCryptoExecutor {
    tx: mpsc::Sender<PqCryptoJob>,
}

impl PqCryptoExecutor {
    fn global() -> &'static Result<Self, String> {
        static EXECUTOR: OnceLock<Result<PqCryptoExecutor, String>> = OnceLock::new();
        EXECUTOR.get_or_init(Self::spawn)
    }

    fn spawn() -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("smg-pq-crypto".to_string())
            .stack_size(PQ_CRYPTO_THREAD_STACK_SIZE_BYTES)
            .spawn(move || Self::run(rx))
            .map_err(|error| format!("failed to spawn SMG PQ crypto thread: {error}"))?;
        Ok(Self { tx })
    }

    fn run(rx: mpsc::Receiver<PqCryptoJob>) {
        while let Ok(job) = rx.recv() {
            match job {
                PqCryptoJob::GenerateKeypair { reply } => {
                    let _ = reply.send(generate_pq_keypair_sync().map_err(|error| match error {
                        EnrollmentError::Other(message) => message,
                        other => other.to_string(),
                    }));
                }
                PqCryptoJob::SignSeed {
                    seed_b64,
                    message,
                    reply,
                } => {
                    let _ = reply.send(sign_pq_seed_sync(&seed_b64, &message));
                }
            }
        }
    }

    async fn generate_keypair(&self) -> Result<PqKeypair, String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(PqCryptoJob::GenerateKeypair { reply: reply_tx })
            .map_err(|_| "SMG PQ crypto thread is unavailable".to_string())?;
        reply_rx
            .await
            .map_err(|_| "SMG PQ crypto thread dropped key generation response".to_string())?
    }

    async fn sign_seed(&self, seed_b64: &str, message: &[u8]) -> Result<String, String> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(PqCryptoJob::SignSeed {
                seed_b64: seed_b64.to_string(),
                message: message.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| "SMG PQ crypto thread is unavailable".to_string())?;
        reply_rx
            .await
            .map_err(|_| "SMG PQ crypto thread dropped signing response".to_string())?
    }
}

async fn generate_pq_keypair() -> Result<PqKeypair, EnrollmentError> {
    let executor = match PqCryptoExecutor::global() {
        Ok(executor) => executor,
        Err(error) => return Err(EnrollmentError::Other(error.clone())),
    };
    executor
        .generate_keypair()
        .await
        .map_err(EnrollmentError::Other)
}

fn generate_pq_keypair_sync() -> Result<PqKeypair, EnrollmentError> {
    let rng = aws_lc_rs::rand::SystemRandom::new();
    let mut seed_bytes = [0u8; 32];
    rng.fill(&mut seed_bytes)
        .map_err(|_| EnrollmentError::Other("failed to generate ML-DSA seed".to_string()))?;

    let mut seed = ml_dsa::Seed::default();
    seed.copy_from_slice(&seed_bytes);
    let keypair = SigningKey::<MlDsa65>::from_seed(&seed);
    let public_key = keypair.verifying_key().encode();
    let public_key_bytes = public_key.as_slice();

    Ok(PqKeypair {
        seed_b64: base64::engine::general_purpose::STANDARD.encode(seed_bytes),
        public_key_b64: base64::engine::general_purpose::STANDARD.encode(public_key_bytes),
        key_id: sha256_hex_bytes(public_key_bytes),
    })
}

pub(crate) fn derive_registration_endpoint(
    registration_url: &str,
    endpoint_path: &str,
) -> Result<String, EnrollmentError> {
    let base = registration_url.trim_end_matches('/');
    let root = base
        .strip_suffix("/api/register")
        .unwrap_or(base)
        .trim_end_matches('/');
    Ok(format!("{root}{endpoint_path}"))
}

fn enrollment_outbound_http_client() -> OutboundHttpClient {
    OutboundHttpClient::new(smg_reqwest_client(), SMG_ENROLLMENT_RATE_LIMITS.clone())
}

fn enrollment_request_policy(request_label: &'static str) -> RequestPolicy {
    RequestPolicy::no_retry("smg_enrollment", request_label)
        .with_backoff(Duration::from_secs(1), Duration::from_secs(30))
}

fn map_enrollment_outbound_error(operation: &str, error: OutboundHttpError) -> EnrollmentError {
    match error {
        OutboundHttpError::RateLimited(rate_limited) => EnrollmentError::RateLimited(RateLimited {
            retry_after: rate_limited.retry_after,
            message: match rate_limited.retry_after.filter(|delay| !delay.is_zero()) {
                Some(delay) => format!(
                    "{operation} failed: rate limited, retry after {}s",
                    delay.as_secs()
                ),
                None => format!("{operation} failed: rate limited"),
            },
        }),
        OutboundHttpError::Transport { source, .. } => {
            EnrollmentError::Other(format!("{operation} failed: {source}"))
        }
    }
}

async fn send_authenticated_pq_registration_request(
    http: &OutboundHttpClient,
    url: &str,
    current_seed_b64: &str,
    current_key_id: &str,
    payload: &serde_json::Value,
) -> Result<reqwest::Response, EnrollmentError> {
    let endpoint_url = reqwest::Url::parse(url)
        .map_err(|e| EnrollmentError::Other(format!("invalid registration URL: {e}")))?;
    let body_bytes = serde_json::to_vec(payload)
        .map_err(|e| EnrollmentError::Other(format!("failed to serialize PQ payload: {e}")))?;
    let host = canonical_request_host(&endpoint_url)?;
    let path_and_query = canonical_request_path_and_query(&endpoint_url);
    let client = http.client().clone();
    let url = url.to_string();
    let current_seed_b64 = current_seed_b64.to_string();
    let current_key_id = current_key_id.to_string();
    http.send_async(
        enrollment_request_policy("smg_pq_authenticated_request"),
        || {
            let client = client.clone();
            let url = url.clone();
            let current_seed_b64 = current_seed_b64.clone();
            let current_key_id = current_key_id.clone();
            let body_bytes = body_bytes.clone();
            let host = host.clone();
            let path_and_query = path_and_query.clone();
            async move {
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| {
                        EnrollmentError::Other(format!("system clock before UNIX_EPOCH: {e}"))
                    })?
                    .as_secs() as i64;
                let auth_version = configured_pq_auth_version();
                let nonce = match auth_version {
                    PqAuthVersion::V1 => None,
                    PqAuthVersion::V2 => {
                        Some(generate_pq_auth_nonce().map_err(EnrollmentError::Other)?)
                    }
                };
                let body_hash = sha256_hex_bytes(&body_bytes);
                let signature = sign_pq_request(
                    &current_seed_b64,
                    auth_version,
                    reqwest::Method::POST.as_str(),
                    &host,
                    &path_and_query,
                    timestamp,
                    nonce.as_deref(),
                    &body_hash,
                )
                .await
                .map_err(EnrollmentError::Other)?;

                let mut request = client
                    .post(url.clone())
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .header("X-Scryer-Auth-Version", auth_version.header_value())
                    .header("X-Scryer-Key-Id", current_key_id)
                    .header("X-Scryer-Timestamp", timestamp.to_string())
                    .header("X-Scryer-Signature", signature.clone());
                if let Some(nonce) = &nonce {
                    request = request.header("X-Scryer-Nonce", nonce);
                }
                Ok(request.body(body_bytes))
            }
        },
    )
    .await
    .map_err(|error| match error {
        OutboundRequestError::Build(error) => error,
        OutboundRequestError::Http(error) => {
            map_enrollment_outbound_error("SMG PQ authenticated request", error)
        }
    })
}

pub(crate) async fn registration_response_error(
    response: reqwest::Response,
    operation: &str,
) -> EnrollmentError {
    let status = response.status();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|header| header.to_str().ok())
        .and_then(parse_retry_after)
        .map(|(delay, _source)| delay);
    let preview = match read_response_body_preview(response, operation).await {
        Ok(preview) => preview,
        Err(error) => {
            return EnrollmentError::Other(format!(
                "{operation} failed (HTTP {status}): response body read failed: {error}"
            ));
        }
    };

    if status.as_u16() == 422
        && !preview.truncated
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&preview.text)
        && parsed.get("error").and_then(|v| v.as_str()) == Some("version_incompatible")
    {
        return EnrollmentError::VersionIncompatible(VersionIncompatible {
            status: parsed
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("blocked")
                .to_string(),
            minimum_version: parsed
                .get("minimum_version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            your_version: parsed
                .get("your_version")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            message: parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            upgrade_deadline: parsed
                .get("upgrade_deadline")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .filter(|value| !value.trim().is_empty()),
        });
    }

    warn!(
        operation,
        status = %status,
        body_preview = %preview.escaped_text(),
        body_preview_bytes = preview.preview_bytes,
        content_length = ?preview.content_length,
        content_type = ?preview.content_type,
        body_truncated = preview.truncated,
        "SMG enrollment request failed"
    );

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return EnrollmentError::RateLimited(RateLimited {
            retry_after,
            message: format!("{operation} failed (HTTP {status})"),
        });
    }

    EnrollmentError::Other(format!("{operation} failed (HTTP {status})"))
}

fn pq_registration_proof_message(
    challenge_type: &str,
    challenge_id: &str,
    nonce: &[u8],
    client_family: &str,
    instance_id: &str,
    key_id: &str,
    public_key_b64: &str,
) -> Vec<u8> {
    format!(
        "smg-pq-{challenge_type}-v1\n{challenge_id}\n{}\n{client_family}\n{instance_id}\n{key_id}\n{public_key_b64}",
        hex_bytes(nonce)
    )
    .into_bytes()
}

fn sign_bootstrap_mac(registration_secret: &str, message: &[u8]) -> String {
    let secret_hash = sha256_bytes(registration_secret.trim().as_bytes());
    let key = hmac::Key::new(hmac::HMAC_SHA256, &secret_hash);
    base64::engine::general_purpose::STANDARD.encode(hmac::sign(&key, message).as_ref())
}

#[expect(
    clippy::too_many_arguments,
    reason = "PQ request signing mirrors the canonical request fields"
)]
pub(crate) async fn sign_pq_request(
    seed_b64: &str,
    auth_version: PqAuthVersion,
    method: &str,
    host: &str,
    path_and_query: &str,
    timestamp: i64,
    nonce: Option<&str>,
    body_hash: &str,
) -> Result<String, String> {
    let message = match auth_version {
        PqAuthVersion::V1 => {
            canonical_pq_request_message_v1(method, host, path_and_query, timestamp, body_hash)
        }
        PqAuthVersion::V2 => {
            let nonce = nonce.ok_or_else(|| "pqsig-v2 signing requires nonce".to_string())?;
            canonical_pq_request_message_v2(
                method,
                host,
                path_and_query,
                timestamp,
                nonce,
                body_hash,
            )
        }
    };
    sign_pq_seed(seed_b64, &message).await
}

fn canonical_request_host(url: &reqwest::Url) -> Result<String, EnrollmentError> {
    let host = url
        .host()
        .ok_or_else(|| EnrollmentError::Other("registration URL missing host".to_string()))?;
    let host = match host {
        url::Host::Domain(domain) => domain.to_string(),
        url::Host::Ipv4(addr) => addr.to_string(),
        url::Host::Ipv6(addr) => format!("[{addr}]"),
    };
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn canonical_request_path_and_query(url: &reqwest::Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_string(),
    }
}

async fn sign_pq_seed(seed_b64: &str, message: &[u8]) -> Result<String, String> {
    let executor = match PqCryptoExecutor::global() {
        Ok(executor) => executor,
        Err(error) => return Err(error.clone()),
    };
    executor.sign_seed(seed_b64, message).await
}

fn sign_pq_seed_sync(seed_b64: &str, message: &[u8]) -> Result<String, String> {
    use ml_dsa::Signer;

    let seed_bytes = base64::engine::general_purpose::STANDARD
        .decode(seed_b64.as_bytes())
        .map_err(|e| format!("failed to decode ML-DSA seed: {e}"))?;
    if seed_bytes.len() != 32 {
        return Err(format!(
            "invalid ML-DSA seed length: expected 32, got {}",
            seed_bytes.len()
        ));
    }
    let mut seed = ml_dsa::Seed::default();
    seed.copy_from_slice(&seed_bytes);
    let keypair = SigningKey::<MlDsa65>::from_seed(&seed);
    let signature = keypair.sign(message);
    let encoded = signature.encode();
    Ok(base64::engine::general_purpose::STANDARD.encode(encoded.as_slice()))
}

fn canonical_pq_request_message_v1(
    method: &str,
    host: &str,
    path_and_query: &str,
    timestamp: i64,
    body_hash: &str,
) -> Vec<u8> {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        host,
        path_and_query,
        timestamp,
        body_hash
    )
    .into_bytes()
}

fn canonical_pq_request_message_v2(
    method: &str,
    host: &str,
    path_and_query: &str,
    timestamp: i64,
    nonce: &str,
    body_hash: &str,
) -> Vec<u8> {
    format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        method.to_ascii_uppercase(),
        host,
        path_and_query,
        timestamp,
        nonce,
        body_hash
    )
    .into_bytes()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_setting(db: &dyn SettingsRepository, key: &str) -> Result<Option<String>, String> {
    let raw = db
        .get_setting_json(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|e| format!("failed to read {key}: {e}"))?;
    Ok(raw.as_deref().and_then(parse_string_json))
}

pub async fn load_pq_enrollment_generation(
    db: &dyn SettingsRepository,
) -> Result<Option<i64>, String> {
    let Some(raw) = load_setting(db, "smg.pq_enrollment_generation").await? else {
        return Ok(None);
    };
    raw.parse::<i64>()
        .map(Some)
        .map_err(|e| format!("invalid smg.pq_enrollment_generation value: {e}"))
}

pub async fn persist_pq_enrollment_generation(
    db: &dyn SettingsRepository,
    generation: i64,
) -> Result<(), String> {
    persist_setting(db, "smg.pq_enrollment_generation", &generation.to_string()).await
}

pub async fn persist_version_compatibility_notice(
    db: &dyn SettingsRepository,
    notice: Option<&VersionIncompatible>,
) -> Result<(), String> {
    persist_setting_json(db, SMG_VERSION_COMPATIBILITY_NOTICE_KEY, &notice).await
}

pub async fn persist_scryer_update_notice(
    db: &dyn SettingsRepository,
    notice: Option<&SmgScryerUpdateNotice>,
) -> Result<(), String> {
    persist_setting_json(db, SMG_SCRYER_UPDATE_NOTICE_KEY, &notice).await
}

async fn persist_pq_enrollment_state(
    db: &dyn SettingsRepository,
    seed_b64: &str,
    public_key_b64: &str,
    key_id: &str,
    enrollment_generation: i64,
) -> Result<(), String> {
    persist_setting(db, "smg.client_key", "").await?;
    persist_setting(db, "smg.client_cert", "").await?;
    persist_setting(db, "smg.cert_expires_at", "").await?;
    persist_setting(db, "smg.ca_cert", "").await?;
    persist_setting(db, "smg.pq_seed", seed_b64).await?;
    persist_setting(db, "smg.pq_public_key", public_key_b64).await?;
    persist_setting(db, "smg.pq_key_id", key_id).await?;
    persist_pq_enrollment_generation(db, enrollment_generation).await
}

async fn persist_setting(
    db: &dyn SettingsRepository,
    key: &str,
    value: &str,
) -> Result<(), String> {
    persist_setting_json(db, key, value).await
}

async fn persist_setting_json<T: Serialize + ?Sized>(
    db: &dyn SettingsRepository,
    key: &str,
    value: &T,
) -> Result<(), String> {
    let value_json =
        serde_json::to_string(value).map_err(|e| format!("failed to encode {key}: {e}"))?;
    db.upsert_setting_json(
        SETTINGS_SCOPE_SYSTEM,
        key,
        None,
        value_json,
        "smg-enrollment",
        None,
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("failed to persist {key}: {e}"))
}

fn parse_string_json(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

fn sha256_bytes(data: &[u8]) -> [u8; 32] {
    let digest = digest::digest(&digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

fn sha256_hex_bytes(data: &[u8]) -> String {
    hex_bytes(&sha256_bytes(data))
}

fn hex_bytes(data: &[u8]) -> String {
    data.iter()
        .fold(String::with_capacity(data.len() * 2), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

#[cfg(test)]
mod tests {
    use super::{
        EnrollmentError, PqAuthVersion, canonical_pq_request_message_v1,
        canonical_pq_request_message_v2, canonical_request_host, generate_pq_auth_nonce,
        generate_pq_keypair, parse_pq_auth_version, pq_registration_proof_message,
        sign_bootstrap_mac, sign_pq_request,
    };
    use scryer_outbound_http::parse_retry_after;
    use std::time::Duration;

    #[test]
    fn canonical_retry_after_parser_reads_http_date_first() {
        let future = (chrono::Utc::now() + chrono::Duration::seconds(12))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();

        let (delay, source) =
            parse_retry_after(&future).expect("expected parsed retry-after delay");
        assert_eq!(source, scryer_outbound_http::RetryAfterSource::HttpDate);
        assert!(delay >= Duration::from_secs(10));
        assert!(delay <= Duration::from_secs(12));
    }

    #[test]
    fn canonical_retry_after_parser_falls_back_to_seconds() {
        let (delay, source) = parse_retry_after("17").expect("expected parsed retry-after delay");
        assert_eq!(source, scryer_outbound_http::RetryAfterSource::Seconds);
        assert_eq!(delay, Duration::from_secs(17));
    }

    #[test]
    fn canonical_retry_after_parser_ignores_invalid_values() {
        assert_eq!(parse_retry_after("nonsense"), None);
    }

    #[test]
    fn rate_limited_display_includes_retry_after_when_present() {
        let error = EnrollmentError::RateLimited(super::RateLimited {
            retry_after: Some(Duration::from_secs(42)),
            message: "too many registration requests".to_string(),
        });

        let rendered = error.to_string();
        assert!(rendered.contains("retry_after=42s"));
        assert!(rendered.contains("too many registration requests"));
    }

    #[test]
    fn canonical_request_host_formats_http_host_header_value() {
        let ipv4_url = reqwest::Url::parse("http://127.0.0.1:43210/api/register").unwrap();
        let ipv6_url = reqwest::Url::parse("http://[::1]:43210/api/register").unwrap();

        assert_eq!(
            canonical_request_host(&ipv4_url).unwrap(),
            "127.0.0.1:43210"
        );
        assert_eq!(canonical_request_host(&ipv6_url).unwrap(), "[::1]:43210");
    }

    #[test]
    fn canonical_pq_request_message_v1_uses_newline_separated_fields() {
        let message =
            canonical_pq_request_message_v1("post", "smg.example", "/graphql?x=1", 123, "abc");

        assert_eq!(
            String::from_utf8(message).unwrap(),
            "POST\nsmg.example\n/graphql?x=1\n123\nabc"
        );
    }

    #[test]
    fn canonical_pq_request_message_v2_includes_nonce() {
        let message = canonical_pq_request_message_v2(
            "get",
            "smg.example",
            "/graphql?extensions=%7B%7D",
            123,
            "nonce-1",
            "empty-body-hash",
        );

        assert_eq!(
            String::from_utf8(message).unwrap(),
            "GET\nsmg.example\n/graphql?extensions=%7B%7D\n123\nnonce-1\nempty-body-hash"
        );
    }

    #[test]
    fn pq_auth_version_parser_defaults_to_v2_and_accepts_v1_override() {
        assert_eq!(parse_pq_auth_version(None), Some(PqAuthVersion::V2));
        assert_eq!(parse_pq_auth_version(Some("")), Some(PqAuthVersion::V2));
        assert_eq!(
            parse_pq_auth_version(Some("pqsig-v1")),
            Some(PqAuthVersion::V1)
        );
        assert_eq!(parse_pq_auth_version(Some("v1")), Some(PqAuthVersion::V1));
        assert_eq!(
            parse_pq_auth_version(Some("pqsig-v2")),
            Some(PqAuthVersion::V2)
        );
        assert_eq!(parse_pq_auth_version(Some("nope")), None);
    }

    #[test]
    fn pq_auth_nonce_is_url_safe_and_unique() {
        let first = generate_pq_auth_nonce().expect("first nonce");
        let second = generate_pq_auth_nonce().expect("second nonce");

        assert_ne!(first, second);
        assert!(first.len() >= 22);
        assert!(
            first
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
    }

    #[test]
    fn pq_registration_proof_message_matches_server_shape() {
        let message = pq_registration_proof_message(
            "bootstrap",
            "challenge",
            &[0x0a, 0xff],
            "scryer-stable",
            "instance",
            "key",
            "public",
        );

        assert_eq!(
            String::from_utf8(message).unwrap(),
            "smg-pq-bootstrap-v1\nchallenge\n0aff\nscryer-stable\ninstance\nkey\npublic"
        );
    }

    #[test]
    fn bootstrap_mac_is_deterministic_for_same_secret_and_message() {
        let first = sign_bootstrap_mac("secret", b"message");
        let second = sign_bootstrap_mac("secret", b"message");

        assert_eq!(first, second);
    }

    #[test]
    fn pq_crypto_executor_generates_keys_and_signs_requests() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        runtime.block_on(async {
            let keypair = generate_pq_keypair().await.expect("generated pq keypair");
            let v1_signature = sign_pq_request(
                &keypair.seed_b64,
                PqAuthVersion::V1,
                "post",
                "smg.example",
                "/graphql",
                123,
                None,
                "abc123",
            )
            .await
            .expect("signed v1 pq request");
            let v2_signature = sign_pq_request(
                &keypair.seed_b64,
                PqAuthVersion::V2,
                "get",
                "smg.example",
                "/graphql?extensions=%7B%7D",
                124,
                Some("nonce-1"),
                "abc123",
            )
            .await
            .expect("signed v2 pq request");

            assert!(!v1_signature.is_empty());
            assert!(!v2_signature.is_empty());
            assert_ne!(v1_signature, v2_signature);
        });
    }
}
