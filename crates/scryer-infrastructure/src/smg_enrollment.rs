use std::time::Duration;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use ml_dsa::{KeyGen, MlDsa65};
use ring::{hmac, rand::SecureRandom};
use serde::Deserialize;
use tracing::{debug, info, warn};

const SETTINGS_SCOPE_SYSTEM: &str = "system";
const RENEWAL_THRESHOLD_DAYS: i64 = 30;
const PQ_CLIENT_FAMILY: &str = "scryer-stable";

/// Returned when SMG rejects registration due to version incompatibility.
#[derive(Debug, Clone)]
pub struct VersionIncompatible {
    pub minimum_version: String,
    pub your_version: String,
    pub message: String,
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
                "version incompatible: minimum={}, yours={}, message={}",
                v.minimum_version, v.your_version, v.message
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
    pub client_key_pem: String,
    pub client_cert_pem: String,
    pub ca_cert_pem: String,
    pub expires_at: DateTime<Utc>,
    pub pq_seed_b64: Option<String>,
    pub pq_public_key_b64: Option<String>,
    pub pq_key_id: Option<String>,
}

#[derive(Deserialize)]
struct RegisterResponse {
    certificate: String,
    expires_at: String,
    ca_certificate: String,
    #[serde(default)]
    opensubtitles_api_key: Option<String>,
}

#[derive(Deserialize)]
struct PqChallengeResponse {
    challenge_id: String,
    nonce: String,
}

#[derive(Deserialize)]
struct PqRegisterResponse {
    key_id: String,
    #[serde(default)]
    opensubtitles_api_key: Option<String>,
}

/// Load or generate the instance ID (UUIDv4) for this Scryer instance.
pub async fn ensure_instance_id(db: &crate::SqliteServices) -> Result<String, String> {
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
pub async fn clear_enrollment_cache(db: &crate::SqliteServices) -> Result<(), String> {
    for key in &[
        "smg.client_key",
        "smg.client_cert",
        "smg.cert_expires_at",
        "smg.ca_cert",
        "smg.pq_seed",
        "smg.pq_public_key",
        "smg.pq_key_id",
    ] {
        persist_setting(db, key, "").await?;
    }
    Ok(())
}

/// Load existing enrollment from DB, or enroll with SMG if missing/expired.
pub async fn ensure_enrolled(
    db: &crate::SqliteServices,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
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

    if let (Some(pq_seed), Some(pq_public_key), Some(pq_key_id)) =
        (pq_seed, pq_public_key, pq_key_id)
        && !pq_seed.is_empty()
        && !pq_public_key.is_empty()
        && !pq_key_id.is_empty()
    {
        debug!(%instance_id, pq_key_id, "using cached SMG PQ enrollment");
        return Ok(EnrollmentState {
            instance_id,
            client_key_pem: String::new(),
            client_cert_pem: String::new(),
            ca_cert_pem: String::new(),
            expires_at: DateTime::<Utc>::MAX_UTC,
            pq_seed_b64: Some(pq_seed),
            pq_public_key_b64: Some(pq_public_key),
            pq_key_id: Some(pq_key_id),
        });
    }

    match enroll_pq_with_smg(
        db,
        &instance_id,
        registration_url,
        registration_secret,
        ca_cert_override,
    )
    .await
    {
        Ok(state) => return Ok(state),
        Err(
            error @ (EnrollmentError::VersionIncompatible(_) | EnrollmentError::RateLimited(_)),
        ) => {
            return Err(error);
        }
        Err(error) => {
            warn!(error = %error, "SMG PQ enrollment failed; falling back to legacy certificate enrollment");
        }
    }

    let key = load_setting(db, "smg.client_key")
        .await
        .map_err(EnrollmentError::Other)?;
    let cert = load_setting(db, "smg.client_cert")
        .await
        .map_err(EnrollmentError::Other)?;
    let expires_str = load_setting(db, "smg.cert_expires_at")
        .await
        .map_err(EnrollmentError::Other)?;
    let ca_cert = load_setting(db, "smg.ca_cert")
        .await
        .map_err(EnrollmentError::Other)?;

    if let (Some(key), Some(cert), Some(expires_str), Some(ca_cert)) =
        (key, cert, expires_str, ca_cert)
        && let Ok(expires_at) = expires_str.parse::<DateTime<Utc>>()
    {
        let days_remaining = (expires_at - Utc::now()).num_days();
        if days_remaining > RENEWAL_THRESHOLD_DAYS {
            let instance_id = ensure_instance_id(db)
                .await
                .map_err(EnrollmentError::Other)?;
            let ca_cn = extract_pem_cn(&ca_cert).unwrap_or_default();
            let cert_cn = extract_pem_cn(&cert).unwrap_or_default();
            info!(
                %instance_id,
                days_remaining,
                %expires_at,
                cert_cn,
                ca_cn,
                "using cached SMG enrollment (skipping /api/register)"
            );
            return Ok(EnrollmentState {
                instance_id,
                client_key_pem: key,
                client_cert_pem: cert,
                ca_cert_pem: ca_cert,
                expires_at,
                pq_seed_b64: None,
                pq_public_key_b64: None,
                pq_key_id: None,
            });
        }
        info!(days_remaining, "SMG cert expiring soon, re-enrolling");
    }

    enroll_with_smg(
        db,
        &instance_id,
        registration_url,
        registration_secret,
        ca_cert_override,
    )
    .await
}

async fn enroll_pq_with_smg(
    db: &crate::SqliteServices,
    instance_id: &str,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
) -> Result<EnrollmentState, EnrollmentError> {
    let challenge_url = derive_registration_endpoint(registration_url, "/api/register-challenge")?;
    let register_key_url = derive_registration_endpoint(registration_url, "/api/register-key")?;
    let http = enrollment_http_client(ca_cert_override)?;

    let challenge_response = http
        .post(challenge_url)
        .json(&serde_json::json!({
            "client_family": PQ_CLIENT_FAMILY,
            "instance_id": instance_id,
            "secret_id": PQ_CLIENT_FAMILY,
            "version": env!("CARGO_PKG_VERSION"),
        }))
        .send()
        .await
        .map_err(|e| EnrollmentError::Other(format!("SMG PQ challenge request failed: {e}")))?;

    if !challenge_response.status().is_success() {
        return Err(registration_response_error(challenge_response, "SMG PQ challenge").await);
    }

    let challenge: PqChallengeResponse = challenge_response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG PQ challenge response: {e}"))
    })?;
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(challenge.nonce.as_bytes())
        .map_err(|e| EnrollmentError::Other(format!("invalid PQ challenge nonce: {e}")))?;
    let pq_key = generate_pq_keypair()?;
    let proof_message = pq_registration_proof_message(
        "bootstrap",
        &challenge.challenge_id,
        &nonce,
        PQ_CLIENT_FAMILY,
        instance_id,
        &pq_key.key_id,
        &pq_key.public_key_b64,
    );
    let proof_signature =
        sign_pq_seed(&pq_key.seed_b64, &proof_message).map_err(EnrollmentError::Other)?;
    let bootstrap_mac = sign_bootstrap_mac(registration_secret, &proof_message);

    let response = http
        .post(register_key_url)
        .json(&serde_json::json!({
            "challenge_id": challenge.challenge_id,
            "client_family": PQ_CLIENT_FAMILY,
            "instance_id": instance_id,
            "secret_id": PQ_CLIENT_FAMILY,
            "version": env!("CARGO_PKG_VERSION"),
            "public_key": pq_key.public_key_b64,
            "key_id": pq_key.key_id,
            "proof_signature": proof_signature,
            "bootstrap_mac": bootstrap_mac,
        }))
        .send()
        .await
        .map_err(|e| EnrollmentError::Other(format!("SMG PQ registration request failed: {e}")))?;

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

    persist_setting(db, "smg.pq_seed", &pq_key.seed_b64)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.pq_public_key", &pq_key.public_key_b64)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.pq_key_id", &pq_key.key_id)
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
        client_key_pem: String::new(),
        client_cert_pem: String::new(),
        ca_cert_pem: String::new(),
        expires_at: DateTime::<Utc>::MAX_UTC,
        pq_seed_b64: Some(pq_key.seed_b64),
        pq_public_key_b64: Some(pq_key.public_key_b64),
        pq_key_id: Some(pq_key.key_id),
    })
}

async fn enroll_with_smg(
    db: &crate::SqliteServices,
    instance_id: &str,
    registration_url: &str,
    registration_secret: &str,
    ca_cert_override: Option<&str>,
) -> Result<EnrollmentState, EnrollmentError> {
    // Generate EC P-256 keypair
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| EnrollmentError::Other(format!("failed to generate EC P-256 keypair: {e}")))?;
    let private_key_pem = key_pair.serialize_pem();

    // Create CSR with CN=instance_id, O="scryer"
    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, instance_id);
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "scryer");

    let csr = params
        .serialize_request(&key_pair)
        .map_err(|e| EnrollmentError::Other(format!("failed to create CSR: {e}")))?;
    let csr_pem = csr
        .pem()
        .map_err(|e| EnrollmentError::Other(format!("failed to serialize CSR to PEM: {e}")))?;

    // POST to SMG registration endpoint
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(ca_pem) = ca_cert_override {
        let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).map_err(|e| {
            EnrollmentError::Other(format!("failed to parse SCRYER_SMG_CA_CERT: {e}"))
        })?;
        builder = builder.add_root_certificate(cert);
    }
    let http = builder.build().map_err(|e| {
        EnrollmentError::Other(format!("failed to build HTTP client for enrollment: {e}"))
    })?;

    let response = http
        .post(registration_url)
        .json(&serde_json::json!({
            "csr": csr_pem,
            "version": env!("CARGO_PKG_VERSION"),
            "registration_secret": registration_secret,
        }))
        .send()
        .await
        .map_err(|e| EnrollmentError::Other(format!("SMG registration request failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let retry_after =
            parse_retry_after_header(response.headers().get(reqwest::header::RETRY_AFTER));
        let body = response.text().await.unwrap_or_default();

        // Check for structured version incompatibility response (HTTP 422)
        if status.as_u16() == 422
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
            && parsed.get("error").and_then(|v| v.as_str()) == Some("version_incompatible")
        {
            return Err(EnrollmentError::VersionIncompatible(VersionIncompatible {
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
            }));
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(EnrollmentError::RateLimited(RateLimited {
                retry_after,
                message: format!("SMG registration failed (HTTP {status}): {body}"),
            }));
        }

        return Err(EnrollmentError::Other(format!(
            "SMG registration failed (HTTP {status}): {body}"
        )));
    }

    let reg: RegisterResponse = response.json().await.map_err(|e| {
        EnrollmentError::Other(format!("failed to parse SMG registration response: {e}"))
    })?;

    let expires_at = reg.expires_at.parse::<DateTime<Utc>>().map_err(|e| {
        EnrollmentError::Other(format!("invalid expires_at in registration response: {e}"))
    })?;

    validate_certificate(&reg.certificate, instance_id).map_err(EnrollmentError::Other)?;

    // Persist all enrollment data (smg.client_key is sensitive → auto-encrypted by DB layer)
    persist_setting(db, "smg.client_key", &private_key_pem)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.client_cert", &reg.certificate)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.cert_expires_at", &reg.expires_at)
        .await
        .map_err(EnrollmentError::Other)?;
    persist_setting(db, "smg.ca_cert", &reg.ca_certificate)
        .await
        .map_err(EnrollmentError::Other)?;

    // Persist OpenSubtitles API key if provided by SMG
    if let Some(os_key) = &reg.opensubtitles_api_key
        && !os_key.is_empty()
    {
        persist_setting(db, "subtitles.opensubtitles_api_key", os_key)
            .await
            .map_err(EnrollmentError::Other)?;
        info!("OpenSubtitles API key received from SMG");
    }

    let ca_cn = extract_pem_cn(&reg.ca_certificate).unwrap_or_default();
    let cert_issuer = extract_pem_issuer_cn(&reg.certificate).unwrap_or_default();
    info!(
        instance_id,
        expires_at = %expires_at,
        ca_cn,
        cert_issuer,
        "enrolled with SMG (fresh registration)"
    );

    Ok(EnrollmentState {
        instance_id: instance_id.to_string(),
        client_key_pem: private_key_pem,
        client_cert_pem: reg.certificate,
        ca_cert_pem: reg.ca_certificate,
        expires_at,
        pq_seed_b64: None,
        pq_public_key_b64: None,
        pq_key_id: None,
    })
}

struct PqKeypair {
    seed_b64: String,
    public_key_b64: String,
    key_id: String,
}

fn generate_pq_keypair() -> Result<PqKeypair, EnrollmentError> {
    use ml_dsa::signature::Keypair;

    let rng = ring::rand::SystemRandom::new();
    let mut seed_bytes = [0u8; 32];
    rng.fill(&mut seed_bytes)
        .map_err(|_| EnrollmentError::Other("failed to generate ML-DSA seed".to_string()))?;

    let mut seed = ml_dsa::Seed::default();
    seed.copy_from_slice(&seed_bytes);
    let keypair = MlDsa65::from_seed(&seed);
    let public_key = keypair.verifying_key().encode();
    let public_key_bytes = public_key.as_slice();

    Ok(PqKeypair {
        seed_b64: base64::engine::general_purpose::STANDARD.encode(seed_bytes),
        public_key_b64: base64::engine::general_purpose::STANDARD.encode(public_key_bytes),
        key_id: sha256_hex_bytes(public_key_bytes),
    })
}

fn derive_registration_endpoint(
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

fn enrollment_http_client(
    ca_cert_override: Option<&str>,
) -> Result<reqwest::Client, EnrollmentError> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(30));
    if let Some(ca_pem) = ca_cert_override {
        let cert = reqwest::Certificate::from_pem(ca_pem.as_bytes()).map_err(|e| {
            EnrollmentError::Other(format!("failed to parse SCRYER_SMG_CA_CERT: {e}"))
        })?;
        builder = builder.add_root_certificate(cert);
    }
    builder.build().map_err(|e| {
        EnrollmentError::Other(format!("failed to build HTTP client for enrollment: {e}"))
    })
}

async fn registration_response_error(
    response: reqwest::Response,
    operation: &str,
) -> EnrollmentError {
    let status = response.status();
    let retry_after =
        parse_retry_after_header(response.headers().get(reqwest::header::RETRY_AFTER));
    let body = response.text().await.unwrap_or_default();

    if status.as_u16() == 422
        && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
        && parsed.get("error").and_then(|v| v.as_str()) == Some("version_incompatible")
    {
        return EnrollmentError::VersionIncompatible(VersionIncompatible {
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
        });
    }

    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return EnrollmentError::RateLimited(RateLimited {
            retry_after,
            message: format!("{operation} failed (HTTP {status}): {body}"),
        });
    }

    EnrollmentError::Other(format!("{operation} failed (HTTP {status}): {body}"))
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

/// Validate the signed certificate CN matches our instance ID.
fn validate_certificate(cert_pem: &str, expected_cn: &str) -> Result<(), String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse certificate PEM: {e}"))?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| format!("failed to parse certificate DER: {e}"))?;

    let cn = cert
        .subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .unwrap_or("");
    if cn != expected_cn {
        return Err(format!(
            "certificate CN mismatch: expected '{expected_cn}', got '{cn}'"
        ));
    }

    Ok(())
}

/// Build a `reqwest::Identity` from the enrollment state (key + cert PEM bundle).
pub fn build_mtls_identity(state: &EnrollmentState) -> Result<reqwest::Identity, String> {
    let combined = format!("{}\n{}", state.client_key_pem, state.client_cert_pem);
    reqwest::Identity::from_pem(combined.as_bytes())
        .map_err(|e| format!("failed to build mTLS identity: {e}"))
}

/// Parse the CA certificate PEM into a `reqwest::Certificate` for TLS root store.
pub fn build_ca_certificate(state: &EnrollmentState) -> Result<reqwest::Certificate, String> {
    reqwest::Certificate::from_pem(state.ca_cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse CA certificate: {e}"))
}

fn parse_retry_after_header(value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    value
        .and_then(|header| header.to_str().ok())
        .and_then(parse_retry_after_value)
}

fn parse_retry_after_value(value: &str) -> Option<Duration> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(seconds) = trimmed.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }

    let retry_at = chrono::DateTime::parse_from_rfc2822(trimmed).ok()?;
    let now = Utc::now();
    let retry_at_utc = retry_at.with_timezone(&Utc);
    (retry_at_utc > now)
        .then(|| (retry_at_utc - now).to_std().ok())
        .flatten()
}

/// Sign a request for application-layer instance authentication.
///
/// Constructs message `"{timestamp}:{body_hash}"` and signs with ECDSA P-256 SHA-256.
/// Returns a base64-encoded ASN.1 DER signature.
///
/// The verifier (SMG) computes SHA-256 of the same message and calls
/// `ecdsa.VerifyASN1(pubKey, sha256(message), signature)`. The p256 `Signer`
/// trait internally hashes with SHA-256 before signing, so both sides agree on
/// the digest: `SHA-256("{timestamp}:{body_hash}")`.
pub fn sign_request(
    private_key_pem: &str,
    timestamp: i64,
    body_hash: &str,
) -> Result<String, String> {
    use base64::Engine as _;
    use p256::ecdsa::{DerSignature, SigningKey, signature::Signer};
    use p256::pkcs8::DecodePrivateKey;

    let signing_key = SigningKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| format!("failed to parse private key for signing: {e}"))?;

    let message = format!("{timestamp}:{body_hash}");
    let signature: DerSignature = signing_key.sign(message.as_bytes());

    Ok(base64::engine::general_purpose::STANDARD.encode(signature.as_ref()))
}

pub fn sign_pq_request(
    seed_b64: &str,
    method: &str,
    host: &str,
    path_and_query: &str,
    timestamp: i64,
    body_hash: &str,
) -> Result<String, String> {
    let message = canonical_pq_request_message(method, host, path_and_query, timestamp, body_hash);
    sign_pq_seed(seed_b64, &message)
}

fn sign_pq_seed(seed_b64: &str, message: &[u8]) -> Result<String, String> {
    use ml_dsa::signature::Signer;

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
    let keypair = MlDsa65::from_seed(&seed);
    let signature = keypair.signing_key().sign(message);
    let encoded = signature.encode();
    Ok(base64::engine::general_purpose::STANDARD.encode(encoded.as_slice()))
}

fn canonical_pq_request_message(
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

/// Convert a PEM-encoded certificate to base64-encoded DER for the `X-Scryer-Cert` header.
pub fn cert_pem_to_base64_der(cert_pem: &str) -> Result<String, String> {
    use base64::Engine as _;

    let (_, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| format!("failed to parse certificate PEM: {e}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&pem.contents))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn load_setting(db: &crate::SqliteServices, key: &str) -> Result<Option<String>, String> {
    let settings_store = crate::SqliteSettingsStore::new(db);
    let record = settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, key, None)
        .await
        .map_err(|e| format!("failed to read {key}: {e}"))?;
    Ok(record
        .as_ref()
        .and_then(|r| r.value_json.as_deref())
        .and_then(parse_string_json))
}

async fn persist_setting(db: &crate::SqliteServices, key: &str, value: &str) -> Result<(), String> {
    crate::SqliteSettingsStore::new(db)
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            key,
            None,
            serde_json::to_string(value).unwrap(),
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
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
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

/// Extract the Subject CN from a PEM-encoded certificate for logging.
fn extract_pem_cn(pem_str: &str) -> Option<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).ok()?;

    cert.subject()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .map(|s| s.to_string())
}

/// Extract the Issuer CN from a PEM-encoded certificate for logging.
fn extract_pem_issuer_cn(pem_str: &str) -> Option<String> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()).ok()?;
    let (_, cert) = x509_parser::parse_x509_certificate(&pem.contents).ok()?;

    cert.issuer()
        .iter_common_name()
        .next()
        .and_then(|attr| attr.as_str().ok())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        EnrollmentError, canonical_pq_request_message, parse_retry_after_value,
        pq_registration_proof_message, sign_bootstrap_mac,
    };
    use std::time::Duration;

    #[test]
    fn parse_retry_after_value_reads_seconds() {
        assert_eq!(parse_retry_after_value("17"), Some(Duration::from_secs(17)));
    }

    #[test]
    fn parse_retry_after_value_reads_http_date() {
        let future = (chrono::Utc::now() + chrono::Duration::seconds(12))
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();

        let delay = parse_retry_after_value(&future).expect("expected parsed retry-after delay");
        assert!(delay >= Duration::from_secs(10));
        assert!(delay <= Duration::from_secs(12));
    }

    #[test]
    fn parse_retry_after_value_ignores_invalid_values() {
        assert_eq!(parse_retry_after_value("nonsense"), None);
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
    fn canonical_pq_request_message_uses_newline_separated_fields() {
        let message =
            canonical_pq_request_message("post", "smg.example", "/graphql?x=1", 123, "abc");

        assert_eq!(
            String::from_utf8(message).unwrap(),
            "POST\nsmg.example\n/graphql?x=1\n123\nabc"
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
}
