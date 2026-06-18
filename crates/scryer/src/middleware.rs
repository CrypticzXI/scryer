use async_graphql::http::ALL_WEBSOCKET_PROTOCOLS;
use async_graphql::{Data, ErrorExtensionValues, Response as GraphQLResponse, ServerError};
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use aws_lc_rs::hmac;
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use scryer_application::{AppError, AppResult, AppUseCase, AuthenticatedTokenClaims};
use scryer_domain::{ActorCapabilityMask, AppPermissionMask, Id};
use scryer_interface::context::{
    AuthRuntimeStateHandle, ConnectionAuthEpoch, MfaVerification, OAuthActorSession,
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

use crate::base_path::BasePath;
use crate::http_error::ErrorResponse;
use crate::rate_limit::{
    RateLimitKey, ScryerRateLimiter, classify_graphql, rate_limited_graphql_response,
    should_precheck_graphql_login,
};

const X_FORWARDED_PROTO: &str = "x-forwarded-proto";
const GRAPHQL_POST_EXECUTION_TIMEOUT: Duration = Duration::from_secs(60);
const GRAPHQL_POST_EXECUTION_TIMEOUT_CODE: &str = "GRAPHQL_EXECUTION_TIMEOUT";
const AUTHENTICATION_REQUIRED_CODE: &str = "AUTHENTICATION_REQUIRED";
const MFA_STEP_UP_REQUIRED_CODE: &str = "MFA_STEP_UP_REQUIRED";
const MFA_STEP_UP_REQUIRED_STATUS_CODE: u16 = 460;
const INTERNAL_SERVER_ERROR_MESSAGE: &str = "Internal server error";
const CORS_ALLOWED_ORIGINS_ENV: &str = "SCRYER_CORS_ALLOWED_ORIGINS";
const WS_ALLOWED_ORIGINS_ENV: &str = "SCRYER_WS_ALLOWED_ORIGINS";
#[cfg(test)]
const PRODUCTION_CORS_OPT_IN_ENV: &str = "SCRYER_ENABLE_PRODUCTION_CORS";
const WEB_UI_URL_ENV: &str = "SCRYER_WEB_UI_URL";
const AUTHLESS_WEB_CLIENT_HEADER: &str = "x-scryer-web-client";
const AUTHLESS_WEB_CLIENT_COOKIE: &str = "scryer_authless_client";
const AUTHLESS_WEB_CLIENT_TTL_SECONDS: u64 = 5 * 60;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AuthlessAccessPolicy {
    pub(crate) allow_unauthenticated_public_access: bool,
    pub(crate) recovery_mode: bool,
}

#[derive(Clone)]
pub(crate) struct AuthlessAccessGuardState {
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) policy: AuthlessAccessPolicy,
}

#[derive(Clone)]
pub(crate) struct AuthlessWebClientProofState {
    secret: Arc<Vec<u8>>,
}

#[derive(Clone)]
pub(crate) struct AuthlessWebClientProofRouteState {
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) policy: AuthlessAccessPolicy,
    pub(crate) proof: AuthlessWebClientProofState,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthlessWebClientProofResponse {
    proof: String,
    expires_at: u64,
}

impl AuthlessWebClientProofState {
    pub(crate) fn new() -> Self {
        let rng = SystemRandom::new();
        let mut secret = vec![0_u8; 32];
        if rng.fill(&mut secret).is_err() {
            secret = Id::new().0.into_bytes();
        }
        Self {
            secret: Arc::new(secret),
        }
    }

    fn issue(&self) -> AppResult<(String, String, u64)> {
        let mut nonce_bytes = [0_u8; 16];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| AppError::Repository("failed to create web client proof".into()))?;
        let nonce = hex_encode(&nonce_bytes);
        let expires_at = unix_now() + AUTHLESS_WEB_CLIENT_TTL_SECONDS;
        let signature = self.sign(&nonce, expires_at);
        Ok((
            nonce.clone(),
            format!("{nonce}.{expires_at}.{signature}"),
            expires_at,
        ))
    }

    fn validate_headers(&self, headers: &HeaderMap, proof_override: Option<&str>) -> bool {
        let proof = proof_override.or_else(|| {
            headers
                .get(AUTHLESS_WEB_CLIENT_HEADER)
                .and_then(|value| value.to_str().ok())
        });
        let Some(proof) = proof else {
            return false;
        };
        let Some(cookie_nonce) = authless_cookie_nonce(headers) else {
            return false;
        };
        self.validate(proof, &cookie_nonce)
    }

    fn validate(&self, proof: &str, cookie_nonce: &str) -> bool {
        let mut parts = proof.split('.');
        let Some(nonce) = parts.next() else {
            return false;
        };
        let Some(expires_at) = parts.next().and_then(|value| value.parse::<u64>().ok()) else {
            return false;
        };
        let Some(signature) = parts.next() else {
            return false;
        };
        if parts.next().is_some() || nonce != cookie_nonce || expires_at < unix_now() {
            return false;
        }
        constant_time_eq(
            signature.as_bytes(),
            self.sign(nonce, expires_at).as_bytes(),
        )
    }

    fn sign(&self, nonce: &str, expires_at: u64) -> String {
        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.secret);
        let message = format!("scryer-authless-web-client:v1:{nonce}:{expires_at}");
        hex_encode(hmac::sign(&key, message.as_bytes()).as_ref())
    }
}

pub(crate) async fn authless_web_client_proof_handler(
    State(state): State<AuthlessWebClientProofRouteState>,
    request: Request<Body>,
) -> Response {
    let (parts, _) = request.into_parts();
    let remote_addr = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0);
    let headers = parts.headers;
    let snapshot = state.auth_runtime.snapshot();

    if let AuthlessAccessDecision::Reject(reason) =
        authless_web_client_proof_decision(&snapshot, state.policy, &headers, remote_addr)
    {
        warn!("Rejecting authless web client proof request: {reason}");
        let mut response = (
            StatusCode::FORBIDDEN,
            Json(ErrorResponse::new(
                "Scryer web client proof is not available for this request".to_string(),
            )),
        )
            .into_response();
        apply_authless_web_client_response_headers(&mut response);
        return response;
    }

    match state.proof.issue() {
        Ok((nonce, proof, expires_at)) => {
            let mut response =
                Json(AuthlessWebClientProofResponse { proof, expires_at }).into_response();
            apply_authless_web_client_response_headers(&mut response);
            let cookie = authless_web_client_cookie(&nonce, &headers);
            if let Ok(value) = http::HeaderValue::from_str(&cookie) {
                response.headers_mut().append(header::SET_COOKIE, value);
            }
            response
        }
        Err(err) => {
            let mut response = map_app_error(err);
            apply_authless_web_client_response_headers(&mut response);
            response
        }
    }
}

fn authless_web_client_proof_decision(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    policy: AuthlessAccessPolicy,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AuthlessAccessDecision {
    if request_is_cross_site(headers) {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::CrossSiteRequest);
    }

    if local_ip_bypass_active(snapshot, headers, remote_addr) {
        return AuthlessAccessDecision::Allow;
    }

    if snapshot.effective_form_login_enabled {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::AuthRequired);
    }

    authless_access_decision(snapshot, policy, headers, remote_addr)
}

fn request_is_cross_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("cross-site"))
}

fn authless_web_client_cookie(nonce: &str, headers: &HeaderMap) -> String {
    let mut cookie = format!(
        "{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}; Path=/; Max-Age={AUTHLESS_WEB_CLIENT_TTL_SECONDS}; HttpOnly; SameSite=Strict"
    );
    if request_is_secure(headers) {
        cookie.push_str("; Secure");
    }
    cookie
}

fn apply_authless_web_client_response_headers(response: &mut Response) {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        http::HeaderValue::from_static("no-store, max-age=0"),
    );
    response
        .headers_mut()
        .insert(header::PRAGMA, http::HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(header::EXPIRES, http::HeaderValue::from_static("0"));
}

fn request_is_secure(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .any(|proto| proto.trim().eq_ignore_ascii_case("https"))
        })
        .unwrap_or(false)
        || headers
            .get("forwarded")
            .and_then(|value| value.to_str().ok())
            .map(forwarded_header_has_https_proto)
            .unwrap_or(false)
}

fn forwarded_header_has_https_proto(value: &str) -> bool {
    value.split(',').any(|entry| {
        entry.split(';').any(|part| {
            let Some((name, value)) = part.split_once('=') else {
                return false;
            };
            name.trim().eq_ignore_ascii_case("proto")
                && value.trim_matches('"').trim().eq_ignore_ascii_case("https")
        })
    })
}

fn authless_cookie_nonce(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| {
            (name == AUTHLESS_WEB_CLIENT_COOKIE && !value.is_empty()).then(|| value.to_string())
        })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}

#[derive(Clone, Debug)]
pub(crate) struct CorsConfig {
    pub(crate) allow_all: bool,
    pub(crate) allowed_origins: Vec<String>,
}

impl CorsConfig {
    pub(crate) fn from_env() -> Self {
        Self::from_env_for_mode(cfg!(debug_assertions))
    }

    fn from_env_for_mode(debug_assertions: bool) -> Self {
        let configured_origins = std::env::var(CORS_ALLOWED_ORIGINS_ENV).ok();
        let origins = match configured_origins {
            Some(raw) if debug_assertions => parse_cors_allowed_origins(&raw),
            Some(_) => {
                tracing::warn!(
                    env = CORS_ALLOWED_ORIGINS_ENV,
                    "ignoring CORS origins because CORS is dev-mode only"
                );
                Vec::new()
            }
            None => default_cors_allowed_origins_for_mode(debug_assertions),
        };

        Self {
            allow_all: false,
            allowed_origins: origins,
        }
    }

    fn is_allowed(&self, origin: &str) -> bool {
        if self.allow_all {
            return true;
        }
        self.allowed_origins.iter().any(|allowed| allowed == origin)
    }
}

fn parse_cors_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(cors_allowed_origin)
        .collect()
}

fn cors_allowed_origin(origin: &str) -> Option<String> {
    if matches!(origin.trim(), "*" | "http://*" | "https://*") {
        tracing::warn!(
            origin,
            "ignoring wildcard CORS Origin; configure exact origins instead"
        );
        return None;
    }

    canonical_origin(origin)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WebSocketOriginPolicy {
    allowed_origins: Vec<String>,
}

impl WebSocketOriginPolicy {
    pub(crate) fn from_env(cors: &CorsConfig) -> Self {
        Self::from_env_for_mode(cors, cfg!(debug_assertions))
    }

    fn from_env_for_mode(cors: &CorsConfig, debug_assertions: bool) -> Self {
        let origins = match std::env::var(WS_ALLOWED_ORIGINS_ENV) {
            Ok(raw) if debug_assertions => parse_websocket_allowed_origins(&raw),
            Ok(_) => {
                tracing::warn!(
                    env = WS_ALLOWED_ORIGINS_ENV,
                    "ignoring WebSocket origins because CORS is dev-mode only"
                );
                Vec::new()
            }
            Err(_) => cors
                .allowed_origins
                .iter()
                .filter_map(|origin| websocket_allowed_origin(origin))
                .collect(),
        };

        Self {
            allowed_origins: origins,
        }
    }

    fn check(&self, headers: &HeaderMap) -> Result<(), String> {
        let Some(origin) = headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };

        let Some(origin) = websocket_allowed_origin(origin) else {
            return Err("invalid WebSocket Origin".to_string());
        };

        if request_is_same_origin(headers, &origin)
            || self
                .allowed_origins
                .iter()
                .any(|allowed| allowed == &origin)
        {
            return Ok(());
        }

        Err(format!("WebSocket Origin is not allowed: {origin}"))
    }
}

fn parse_websocket_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter_map(websocket_allowed_origin)
        .collect()
}

fn websocket_allowed_origin(origin: &str) -> Option<String> {
    if matches!(origin.trim(), "*" | "http://*" | "https://*") {
        tracing::warn!(
            origin,
            "ignoring wildcard WebSocket Origin; configure exact origins instead"
        );
        return None;
    }
    canonical_origin(origin)
}

fn request_is_same_origin(headers: &HeaderMap, origin: &str) -> bool {
    let Some((origin_scheme, origin_authority)) = split_origin(origin) else {
        return false;
    };
    let Some(request_authority) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_authority)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    if origin_authority != request_authority {
        return false;
    }

    forwarded_proto(headers)
        .is_none_or(|proto| origin_scheme_matches_forwarded_proto(&origin_scheme, &proto))
}

fn split_origin(origin: &str) -> Option<(String, String)> {
    let (scheme, authority) = origin.split_once("://")?;
    Some((scheme.to_ascii_lowercase(), normalize_authority(authority)))
}

fn normalize_authority(authority: &str) -> String {
    authority.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn forwarded_proto(headers: &HeaderMap) -> Option<String> {
    headers
        .get(X_FORWARDED_PROTO)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn origin_scheme_matches_forwarded_proto(origin_scheme: &str, forwarded_proto: &str) -> bool {
    matches!(
        (origin_scheme, forwarded_proto),
        ("http", "http") | ("http", "ws") | ("https", "https") | ("https", "wss")
    )
}

fn default_cors_allowed_origins_for_mode(debug_assertions: bool) -> Vec<String> {
    let mut origins = if debug_assertions {
        vec![
            "http://localhost:3000".to_string(),
            "http://127.0.0.1:3000".to_string(),
            "http://0.0.0.0:3000".to_string(),
            "http://host.docker.internal:3000".to_string(),
            "http://nodejs:3000".to_string(),
        ]
    } else {
        Vec::new()
    };

    if debug_assertions
        && let Ok(web_ui_url) = std::env::var(WEB_UI_URL_ENV)
        && let Some(web_ui_origin) = canonical_origin(&web_ui_url)
    {
        push_origin_if_missing(&mut origins, web_ui_origin.clone());
        add_docker_loopback_aliases(&web_ui_origin, &mut origins);
    }

    origins
}

fn push_origin_if_missing(origins: &mut Vec<String>, candidate: String) {
    if !origins.iter().any(|origin| origin == &candidate) {
        origins.push(candidate);
    }
}

fn canonical_origin(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if matches!(trimmed, "*" | "http://*" | "https://*") {
        return None;
    }

    let uri = trimmed.parse::<Uri>().ok()?;
    let scheme = uri.scheme_str()?;
    let authority = uri.authority()?;
    Some(format!("{scheme}://{authority}"))
}

fn add_docker_loopback_aliases(origin: &str, origins: &mut Vec<String>) {
    let Ok(uri) = origin.parse::<Uri>() else {
        return;
    };
    let Some(scheme) = uri.scheme_str() else {
        return;
    };
    let Some(authority) = uri.authority() else {
        return;
    };

    let host = authority.host();
    let port = authority.port_u16();
    if !matches!(
        host,
        "localhost" | "127.0.0.1" | "0.0.0.0" | "host.docker.internal" | "nodejs"
    ) {
        return;
    }

    for alias in [
        "localhost",
        "127.0.0.1",
        "0.0.0.0",
        "host.docker.internal",
        "nodejs",
    ] {
        let authority = match port {
            Some(port) => format!("{alias}:{port}"),
            None => alias.to_string(),
        };
        push_origin_if_missing(origins, format!("{scheme}://{authority}"));
    }
}

pub(crate) async fn cors_handler(
    request: Request<Body>,
    next: Next,
    policy: CorsConfig,
) -> Response {
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let requested_headers = request
        .headers()
        .get(header::ACCESS_CONTROL_REQUEST_HEADERS)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if request.method() == Method::OPTIONS && origin.as_deref().is_some() {
        let origin = origin.expect("checked above");
        if !policy.is_allowed(&origin) {
            return StatusCode::FORBIDDEN.into_response();
        }

        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors_headers(
            response.headers_mut(),
            &origin,
            requested_headers.as_deref(),
        );
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = origin
        && policy.is_allowed(&origin)
    {
        apply_cors_headers(
            response.headers_mut(),
            &origin,
            requested_headers.as_deref(),
        );
    }

    response
}

pub(crate) fn apply_cors_headers(
    headers: &mut http::HeaderMap,
    origin: &str,
    requested_headers: Option<&str>,
) {
    use http::HeaderValue;

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_str(origin).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );

    let mut allow_headers = "Content-Type, Authorization, X-Scryer-Language".to_string();
    if let Some(requested_headers) = requested_headers {
        let requested_headers = requested_headers.trim();
        if !requested_headers.is_empty() {
            allow_headers = format!("{}, {}", allow_headers, requested_headers);
        }
    }
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_str(&allow_headers).unwrap_or_else(|_| {
            HeaderValue::from_static("Content-Type, Authorization, X-Scryer-Language")
        }),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        HeaderValue::from_static("true"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
}

pub(crate) async fn index_page() -> impl IntoResponse {
    let web_url =
        std::env::var("SCRYER_WEB_UI_URL").unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());
    let base_path = BasePath::from_env();
    let graphql_url = base_path.join("/graphql");
    Html(format!(
        r#"
<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>scryer</title>
    <style>
      :root {{
        color-scheme: dark;
      }}
      body {{
        margin: 0;
        min-height: 100vh;
        font-family: Inter, system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial, sans-serif;
        background: #0f1224;
        color: #e6edff;
        display: grid;
        place-items: center;
      }}
      main {{
        width: min(780px, 100% - 2rem);
      }}
      a {{
        color: #9fb2ff;
      }}
    </style>
  </head>
  <body>
    <main>
      <h1>scryer web UI</h1>
      <p>The SPA has moved to Next.js.</p>
      <p>
        Start the web app in <code>apps/scryer-web</code> and open
        <a href="{web_url}">{web_url}</a>.
      </p>
      <p>
        Backend endpoint: <code>{graphql_url}</code> is still served by this service.
      </p>
    </main>
  </body>
</html>
    "#,
    ))
}

pub(crate) async fn graphql_ws_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    protocol: GraphQLProtocol,
    ws: WebSocketUpgrade,
) -> Response {
    let schema = state.schema.clone();
    let app = state.app.clone();
    let auth_runtime = state.auth_runtime.clone();
    let auth_snapshot = auth_runtime.snapshot();
    let auth_enabled = auth_snapshot.effective_form_login_enabled;
    let local_bypass_active = local_ip_bypass_active(&auth_snapshot, &headers, Some(remote_addr));
    let connection_epoch = auth_snapshot.epoch;
    if let Err(error) = state.ws_origin_policy.check(&headers) {
        tracing::warn!(
            remote_addr = %remote_addr,
            error = %error,
            "rejecting GraphQL WebSocket connection because browser Origin is not allowed"
        );
        return (StatusCode::FORBIDDEN, error).into_response();
    }

    let initial_actor = resolve_actor(&state, &headers, Some(remote_addr)).await;
    let authless_proof_required = initial_actor
        .as_ref()
        .is_some_and(ResolvedActor::requires_authless_web_client_proof);
    let initial_data = graphql_ws_connection_data(
        connection_epoch,
        if authless_proof_required {
            None
        } else {
            initial_actor.clone()
        },
    );
    let authless_web_client_proof = state.authless_web_client_proof.clone();
    let ws_headers = headers.clone();

    ws.protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| async move {
            let app_for_init = app.clone();
            let initial_actor = initial_actor.clone();
            let proof_state = authless_web_client_proof.clone();
            let headers_for_init = ws_headers.clone();
            GraphQLWebSocket::new(stream, schema, protocol)
                .with_data(initial_data)
                .on_connection_init(move |value: serde_json::Value| async move {
                    let auth_value = value.get("Authorization").and_then(|v| v.as_str());
                    let proof_value = value
                        .get("authlessWebClientProof")
                        .or_else(|| value.get("X-Scryer-Web-Client"))
                        .and_then(|v| v.as_str());
                    let actor = resolve_ws_connection_init_actor(
                        &app_for_init,
                        WsConnectionInitActorRequest {
                            auth_enabled,
                            local_bypass_active,
                            initial_actor: initial_actor.clone(),
                            auth_value,
                            authless_proof_required,
                            proof_state: &proof_state,
                            headers: &headers_for_init,
                            proof_value,
                        },
                    )
                    .await?;
                    Ok(graphql_ws_connection_data(connection_epoch, actor))
                })
                .serve()
                .await;
        })
}

#[derive(Clone)]
pub(crate) struct AuthState {
    pub(crate) app: AppUseCase,
    pub(crate) schema: scryer_interface::ApiSchema,
    pub(crate) auth_runtime: AuthRuntimeStateHandle,
    pub(crate) rate_limiter: ScryerRateLimiter,
    pub(crate) ws_origin_policy: WebSocketOriginPolicy,
    pub(crate) authless_web_client_proof: AuthlessWebClientProofState,
}

/// GraphQL handler that returns a streaming response body.
///
/// When the client disconnects (e.g. via `AbortController.abort()` in the browser),
/// hyper stops polling this body stream, which drops the `execute_batch` future.
/// This cancels the entire resolver chain — including any outbound reqwest call to
/// SMG — so the cancellation propagates all the way through to the database query.
pub(crate) async fn graphql_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    body: async_graphql_axum::GraphQLBatchRequest,
) -> Response {
    let actor = resolve_actor(&state, &headers, Some(remote_addr)).await;
    if actor
        .as_ref()
        .is_some_and(ResolvedActor::requires_authless_web_client_proof)
        && !state
            .authless_web_client_proof
            .validate_headers(&headers, None)
    {
        return authless_web_client_forbidden_response(
            "Scryer web client proof is required for unauthenticated access",
        );
    }
    let batch = body.into_inner();
    let client_ip = request_client_ip(&headers, Some(remote_addr)).unwrap_or(remote_addr.ip());
    let rate_limit_key = RateLimitKey::new(
        client_ip,
        actor.as_ref().map(|actor| actor.user.id.as_str()),
    );
    let rate_limit_class = classify_graphql(&batch);
    let precheck_login = should_precheck_graphql_login(&batch);
    if (rate_limit_class != crate::rate_limit::GraphqlRateLimitClass::Login || precheck_login)
        && let Err(decision) = state
            .rate_limiter
            .check_graphql(rate_limit_class, &rate_limit_key)
    {
        let batch_response = rate_limited_graphql_response(&decision);
        let body = serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec());
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
    }
    touch_oauth_grant_last_used(&state.app, actor.as_ref()).await;
    let batch = if let Some(actor) = actor {
        let oauth_session = actor.oauth_session();
        match batch {
            async_graphql::BatchRequest::Single(req) => {
                let mut req = req.data(actor.mfa_verification()).data(actor.user);
                if let Some(oauth_session) = oauth_session {
                    req = req.data(oauth_session);
                }
                async_graphql::BatchRequest::Single(req)
            }
            async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
                reqs.into_iter()
                    .map(|req| {
                        let mut req = req.data(actor.mfa_verification()).data(actor.user.clone());
                        if let Some(oauth_session) = actor.oauth_session() {
                            req = req.data(oauth_session);
                        }
                        req
                    })
                    .collect(),
            ),
        }
    } else {
        batch
    };

    let schema = state.schema.clone();
    let rate_limiter = state.rate_limiter.clone();
    let mut batch_response =
        match tokio::time::timeout(GRAPHQL_POST_EXECUTION_TIMEOUT, schema.execute_batch(batch))
            .await
        {
            Ok(response) => response,
            Err(_) => {
                tracing::warn!(
                    timeout_seconds = GRAPHQL_POST_EXECUTION_TIMEOUT.as_secs(),
                    "graphql POST execution timed out"
                );
                graphql_execution_timeout_response()
            }
        };
    if rate_limit_class == crate::rate_limit::GraphqlRateLimitClass::Login
        && !precheck_login
        && !batch_response.is_ok()
        && let Err(decision) = rate_limiter.record_failed_login(&rate_limit_key)
    {
        batch_response = rate_limited_graphql_response(&decision);
    }
    let response_status = graphql_response_status(&batch_response);
    let body = serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec());

    Response::builder()
        .status(response_status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap()
}

pub(crate) async fn enforce_authless_access_guard(
    State(state): State<AuthlessAccessGuardState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let decision = authless_access_decision(
        &state.auth_runtime.snapshot(),
        state.policy,
        request.headers(),
        Some(remote_addr),
    );

    match decision {
        AuthlessAccessDecision::Allow => next.run(request).await,
        AuthlessAccessDecision::Reject(reason) => {
            let method = request.method().clone();
            let path = request.uri().path().to_string();
            tracing::warn!(
                remote_addr = %remote_addr,
                method = %method,
                path = %path,
                recovery_mode = state.policy.recovery_mode,
                reason = %reason,
                "rejecting auth-disabled request from non-local client"
            );
            (
                StatusCode::FORBIDDEN,
                Json(ErrorResponse::new(
                    "Scryer authentication is disabled; public unauthenticated access is blocked"
                        .to_string(),
                )),
            )
                .into_response()
        }
    }
}

fn graphql_response_status(response: &async_graphql::BatchResponse) -> StatusCode {
    if graphql_response_has_error_code(response, AUTHENTICATION_REQUIRED_CODE) {
        return StatusCode::UNAUTHORIZED;
    }

    if graphql_response_has_error_code(response, MFA_STEP_UP_REQUIRED_CODE) {
        return StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE)
            .expect("MFA step-up status code is a valid HTTP status code");
    }

    StatusCode::OK
}

fn authless_web_client_forbidden_response(message: &str) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse::new(message.to_string())),
    )
        .into_response()
}

fn graphql_response_has_error_code(response: &async_graphql::BatchResponse, code: &str) -> bool {
    match response {
        async_graphql::BatchResponse::Single(response) => response_has_error_code(response, code),
        async_graphql::BatchResponse::Batch(responses) => responses
            .iter()
            .any(|response| response_has_error_code(response, code)),
    }
}

fn response_has_error_code(response: &GraphQLResponse, code: &str) -> bool {
    response.errors.iter().any(|error| {
        let Some(extensions) = &error.extensions else {
            return false;
        };
        matches!(extensions.get("code"), Some(async_graphql::Value::String(value)) if value == code)
    })
}

fn graphql_execution_timeout_response() -> async_graphql::BatchResponse {
    let mut extensions = ErrorExtensionValues::default();
    extensions.set("code", GRAPHQL_POST_EXECUTION_TIMEOUT_CODE);
    extensions.set("timeoutSeconds", GRAPHQL_POST_EXECUTION_TIMEOUT.as_secs());

    let mut error = ServerError::new(
        format!(
            "GraphQL request timed out after {} seconds",
            GRAPHQL_POST_EXECUTION_TIMEOUT.as_secs()
        ),
        None,
    );
    error.extensions = Some(extensions);
    async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]))
}

#[derive(Clone)]
struct ResolvedActor {
    user: scryer_domain::User,
    token_claims: AuthenticatedTokenClaims,
    source: ResolvedActorSource,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResolvedActorSource {
    AuthenticatedToken,
    AuthlessDefault,
}

impl ResolvedActor {
    fn requires_authless_web_client_proof(&self) -> bool {
        self.source == ResolvedActorSource::AuthlessDefault
    }

    fn mfa_verification(&self) -> MfaVerification {
        MfaVerification {
            verified_until: self.token_claims.mfa_verified_until,
            step_up_verified_until: self.token_claims.mfa_step_up_verified_until,
            session_scope: self.token_claims.session_scope,
        }
    }

    fn oauth_session(&self) -> Option<OAuthActorSession> {
        if !self.token_claims.is_oauth_access_token() {
            return None;
        }
        Some(OAuthActorSession {
            client_id: self.token_claims.oauth_client_id.clone()?,
            grant_id: self.token_claims.oauth_grant_id.clone()?,
        })
    }
}

fn graphql_ws_connection_data(connection_epoch: u64, actor: Option<ResolvedActor>) -> Data {
    let mut data = Data::default();
    data.insert(ConnectionAuthEpoch(connection_epoch));
    if let Some(actor) = actor {
        data.insert(actor.mfa_verification());
        if let Some(oauth_session) = actor.oauth_session() {
            data.insert(oauth_session);
        }
        data.insert(actor.user);
    }
    data
}

async fn touch_oauth_grant_last_used(app: &AppUseCase, actor: Option<&ResolvedActor>) {
    let Some(actor) = actor else {
        return;
    };
    let Some(session) = actor.oauth_session() else {
        return;
    };
    if let Err(error) = app
        .touch_oauth_refresh_grant_last_used(&session.client_id, &session.grant_id)
        .await
    {
        tracing::debug!(
            error = %error,
            client_id = %session.client_id,
            grant_id = %session.grant_id,
            "failed to update OAuth grant last-used timestamp"
        );
    }
}

fn touch_oauth_grant_last_used_background(app: &AppUseCase, actor: &ResolvedActor) {
    let Some(session) = actor.oauth_session() else {
        return;
    };
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = app
            .touch_oauth_refresh_grant_last_used(&session.client_id, &session.grant_id)
            .await
        {
            tracing::debug!(
                error = %error,
                client_id = %session.client_id,
                grant_id = %session.grant_id,
                "failed to update OAuth grant last-used timestamp"
            );
        }
    });
}

struct WsConnectionInitActorRequest<'a> {
    auth_enabled: bool,
    local_bypass_active: bool,
    initial_actor: Option<ResolvedActor>,
    auth_value: Option<&'a str>,
    authless_proof_required: bool,
    proof_state: &'a AuthlessWebClientProofState,
    headers: &'a HeaderMap,
    proof_value: Option<&'a str>,
}

async fn resolve_ws_connection_init_actor(
    app: &AppUseCase,
    request: WsConnectionInitActorRequest<'_>,
) -> Result<Option<ResolvedActor>, async_graphql::Error> {
    let WsConnectionInitActorRequest {
        auth_enabled,
        local_bypass_active,
        initial_actor,
        auth_value,
        authless_proof_required,
        proof_state,
        headers,
        proof_value,
    } = request;

    if let Some(raw) = auth_value {
        return match parse_bearer_token(raw) {
            Some(token) => match app.authenticate_token_with_claims(token).await {
                Ok((user, token_claims)) => attach_resolved_actor(
                    app,
                    user,
                    token_claims,
                    ResolvedActorSource::AuthenticatedToken,
                )
                .await
                .map(|actor| {
                    touch_oauth_grant_last_used_background(app, &actor);
                    Some(actor)
                })
                .map_err(|e| async_graphql::Error::new(format!("authentication failed: {e}"))),
                Err(_) if local_bypass_active && authless_proof_required => {
                    if !proof_state.validate_headers(headers, proof_value) {
                        return Err(async_graphql::Error::new(
                            "Scryer web client proof is required for unauthenticated websocket access",
                        ));
                    }
                    Ok(initial_actor)
                }
                Err(_) if local_bypass_active => Ok(initial_actor),
                Err(e) => Err(async_graphql::Error::new(format!(
                    "authentication failed: {e}"
                ))),
            },
            None if local_bypass_active && authless_proof_required => {
                if !proof_state.validate_headers(headers, proof_value) {
                    return Err(async_graphql::Error::new(
                        "Scryer web client proof is required for unauthenticated websocket access",
                    ));
                }
                Ok(initial_actor)
            }
            None if local_bypass_active => Ok(initial_actor),
            None => Err(async_graphql::Error::new("invalid authorization header")),
        };
    }

    if !authless_proof_required
        && let Some(actor) = initial_actor.as_ref()
        && actor.source == ResolvedActorSource::AuthenticatedToken
    {
        touch_oauth_grant_last_used(app, initial_actor.as_ref()).await;
        return Ok(initial_actor);
    }

    if authless_proof_required && !proof_state.validate_headers(headers, proof_value) {
        return Err(async_graphql::Error::new(
            "Scryer web client proof is required for unauthenticated websocket access",
        ));
    }

    if !auth_enabled || local_bypass_active {
        return Ok(initial_actor);
    }

    Ok(None)
}

async fn resolve_actor(
    state: &AuthState,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> Option<ResolvedActor> {
    let snapshot = state.auth_runtime.snapshot();
    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    let actor = match authorization_token_from_headers(headers) {
        Ok(Some(token)) => match state.app.authenticate_token_with_claims(token).await {
            Ok((user, token_claims)) => {
                Some((user, token_claims, ResolvedActorSource::AuthenticatedToken))
            }
            Err(_) if !snapshot.effective_form_login_enabled => {
                resolve_default_user(&state.app).await.map(|user| {
                    (
                        anonymous_user(user),
                        AuthenticatedTokenClaims::default(),
                        ResolvedActorSource::AuthlessDefault,
                    )
                })
            }
            Err(_) if local_bypass => resolve_default_user(&state.app).await.map(|user| {
                (
                    anonymous_user(user),
                    mfa_bypass_token_claims(),
                    ResolvedActorSource::AuthlessDefault,
                )
            }),
            Err(_) => None,
        },
        Ok(None) | Err(_) if !snapshot.effective_form_login_enabled => {
            resolve_default_user(&state.app).await.map(|user| {
                (
                    anonymous_user(user),
                    AuthenticatedTokenClaims::default(),
                    ResolvedActorSource::AuthlessDefault,
                )
            })
        }
        Ok(None) | Err(_) if local_bypass => resolve_default_user(&state.app).await.map(|user| {
            (
                anonymous_user(user),
                mfa_bypass_token_claims(),
                ResolvedActorSource::AuthlessDefault,
            )
        }),
        Ok(None) | Err(_) => None,
    };

    match actor {
        Some((user, token_claims, source)) => {
            attach_resolved_actor(&state.app, user, token_claims, source)
                .await
                .ok()
        }
        None => None,
    }
}

async fn attach_resolved_actor(
    app: &AppUseCase,
    user: scryer_domain::User,
    token_claims: AuthenticatedTokenClaims,
    source: ResolvedActorSource,
) -> AppResult<ResolvedActor> {
    let mut user = app.attach_user_authorization(user).await?;
    user.authorization.actor_capabilities = match source {
        ResolvedActorSource::AuthenticatedToken => token_claims.actor_capabilities,
        ResolvedActorSource::AuthlessDefault => ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
    };
    if token_claims.is_oauth_access_token() {
        user.authorization.app = AppPermissionMask::NONE;
        user.authorization.actor_capabilities = ActorCapabilityMask::NONE;
    }
    Ok(ResolvedActor {
        user,
        token_claims,
        source,
    })
}

async fn resolve_default_user(app_use_case: &AppUseCase) -> Option<scryer_domain::User> {
    match app_use_case.find_default_user().await {
        Ok(Some(user)) => Some(user),
        Ok(None) => app_use_case.find_or_create_default_user().await.ok(),
        Err(_) => None,
    }
}

fn anonymous_user(mut user: scryer_domain::User) -> scryer_domain::User {
    user.username = "Anonymous".to_string();
    user
}

fn mfa_bypass_token_claims() -> AuthenticatedTokenClaims {
    AuthenticatedTokenClaims {
        mfa_verified_until: Some(i64::MAX),
        mfa_step_up_verified_until: Some(i64::MAX),
        ..AuthenticatedTokenClaims::default()
    }
}

fn authorization_token_from_headers(headers: &HeaderMap) -> Result<Option<&str>, AppError> {
    let Some(auth_header) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };

    let raw = auth_header
        .to_str()
        .map_err(|_| AppError::Unauthorized("invalid authorization header".into()))?;
    let token = parse_bearer_token(raw)
        .ok_or_else(|| AppError::Unauthorized("invalid authorization header".into()))?;

    Ok(Some(token))
}

pub(crate) fn parse_bearer_token(raw: &str) -> Option<&str> {
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if scheme.eq_ignore_ascii_case("bearer") {
        Some(token)
    } else {
        None
    }
}

#[cfg(test)]
mod ws_origin_tests {
    use super::*;
    use axum::http::HeaderValue;

    fn ws_headers(host: &str, origin: Option<&str>, forwarded_proto: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        if let Some(forwarded_proto) = forwarded_proto {
            headers.insert(
                X_FORWARDED_PROTO,
                HeaderValue::from_str(forwarded_proto).unwrap(),
            );
        }
        headers
    }

    #[test]
    fn websocket_origin_policy_allows_no_origin_clients() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", None, None);

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_allows_same_origin_lan_host() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("http://192.168.1.25:8080"), None);

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_allows_configured_origin() {
        let policy = WebSocketOriginPolicy {
            allowed_origins: vec!["https://scryer.example.test".to_string()],
        };
        let headers = ws_headers(
            "127.0.0.1:8080",
            Some("https://scryer.example.test"),
            Some("https"),
        );

        assert!(policy.check(&headers).is_ok());
    }

    #[test]
    fn websocket_origin_policy_rejects_cross_site_browser_origin() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("https://evil.example.test"), None);

        assert!(policy.check(&headers).is_err());
    }

    #[test]
    fn websocket_origin_policy_rejects_malformed_browser_origin() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers("192.168.1.25:8080", Some("not an origin"), None);

        assert!(policy.check(&headers).is_err());
    }

    #[test]
    fn websocket_origin_policy_requires_forwarded_proto_match_when_present() {
        let policy = WebSocketOriginPolicy::default();
        let headers = ws_headers(
            "scryer.example.test",
            Some("http://scryer.example.test"),
            Some("https"),
        );

        assert!(policy.check(&headers).is_err());
    }
}

fn local_ip_bypass_active(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> bool {
    if !snapshot.effective_form_login_enabled || !snapshot.skip_login_for_local_ips {
        return false;
    }

    let Some(peer_ip) = remote_addr.map(|addr| addr.ip()) else {
        return false;
    };

    if has_proxy_forwarding_headers(headers) {
        return is_trusted_proxy_ip(peer_ip)
            && forwarded_client_ip_chain(headers)
                .is_ok_and(|client_ips| client_ips.into_iter().all(is_local_network_ip));
    }

    is_local_network_ip(peer_ip)
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthlessAccessDecision {
    Allow,
    Reject(AuthlessAccessRejectReason),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum AuthlessAccessRejectReason {
    AuthRequired,
    CrossSiteRequest,
    MissingRemoteAddress,
    PublicPeer(IpAddr),
    PublicForwardedClient(IpAddr),
    MalformedForwardedClient,
}

impl std::fmt::Display for AuthlessAccessRejectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AuthRequired => f.write_str("authentication is required"),
            Self::CrossSiteRequest => {
                f.write_str("request fetch metadata identifies a cross-site request")
            }
            Self::MissingRemoteAddress => f.write_str("missing remote address"),
            Self::PublicPeer(ip) => write!(f, "peer address {ip} is not private/local"),
            Self::PublicForwardedClient(ip) => {
                write!(f, "forwarded client address {ip} is not private/local")
            }
            Self::MalformedForwardedClient => {
                f.write_str("forwarding headers are present but no valid client IP was found")
            }
        }
    }
}

fn authless_access_decision(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    policy: AuthlessAccessPolicy,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> AuthlessAccessDecision {
    if snapshot.effective_form_login_enabled {
        return AuthlessAccessDecision::Allow;
    }

    if policy.allow_unauthenticated_public_access && !policy.recovery_mode {
        return AuthlessAccessDecision::Allow;
    }

    let Some(peer_ip) = remote_addr.map(|addr| addr.ip()) else {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MissingRemoteAddress);
    };

    if !is_local_network_ip(peer_ip) {
        return AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(peer_ip));
    }

    if has_proxy_forwarding_headers(headers) {
        return match forwarded_client_ip_chain(headers) {
            Ok(client_ips) => match client_ips
                .into_iter()
                .find(|client_ip| !is_local_network_ip(*client_ip))
            {
                Some(public_ip) => AuthlessAccessDecision::Reject(
                    AuthlessAccessRejectReason::PublicForwardedClient(public_ip),
                ),
                None => AuthlessAccessDecision::Allow,
            },
            Err(()) => {
                AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
            }
        };
    }

    AuthlessAccessDecision::Allow
}

fn request_client_ip(headers: &HeaderMap, remote_addr: Option<SocketAddr>) -> Option<IpAddr> {
    let peer_ip = remote_addr?.ip();
    if is_trusted_proxy_ip(peer_ip)
        && let Some(forwarded_ip) = forwarded_client_ip(headers)
    {
        return Some(forwarded_ip);
    }
    Some(peer_ip)
}

fn forwarded_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    x_forwarded_for_client_ip(headers)
        .or_else(|| x_real_ip_client_ip(headers))
        .or_else(|| forwarded_header_client_ip(headers))
}

fn forwarded_client_ip_chain(headers: &HeaderMap) -> Result<Vec<IpAddr>, ()> {
    let mut ips = Vec::new();
    collect_x_forwarded_for_ips(headers, &mut ips)?;
    collect_x_real_ip_ips(headers, &mut ips)?;
    collect_forwarded_header_ips(headers, &mut ips)?;

    if ips.is_empty() { Err(()) } else { Ok(ips) }
}

fn has_proxy_forwarding_headers(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key(header::FORWARDED)
        || headers.contains_key("x-forwarded-host")
        || headers.contains_key(X_FORWARDED_PROTO)
}

fn x_forwarded_for_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').find_map(parse_forwarded_ip_token))
}

fn x_real_ip_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(parse_forwarded_ip_token)
}

fn forwarded_header_client_ip(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get(header::FORWARDED)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value.split(',').find_map(|entry| {
                entry.split(';').find_map(|part| {
                    let (name, raw_value) = part.split_once('=')?;
                    if !name.trim().eq_ignore_ascii_case("for") {
                        return None;
                    }
                    parse_forwarded_ip_token(raw_value)
                })
            })
        })
}

fn collect_x_forwarded_for_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all("x-forwarded-for") {
        let value = value.to_str().map_err(|_| ())?;
        for token in value.split(',') {
            ips.push(parse_forwarded_ip_token(token).ok_or(())?);
        }
    }
    Ok(())
}

fn collect_x_real_ip_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all("x-real-ip") {
        let value = value.to_str().map_err(|_| ())?;
        ips.push(parse_forwarded_ip_token(value).ok_or(())?);
    }
    Ok(())
}

fn collect_forwarded_header_ips(headers: &HeaderMap, ips: &mut Vec<IpAddr>) -> Result<(), ()> {
    for value in headers.get_all(header::FORWARDED) {
        let value = value.to_str().map_err(|_| ())?;
        for entry in value.split(',') {
            for part in entry.split(';') {
                let Some((name, raw_value)) = part.split_once('=') else {
                    continue;
                };
                if name.trim().eq_ignore_ascii_case("for") {
                    ips.push(parse_forwarded_ip_token(raw_value).ok_or(())?);
                }
            }
        }
    }
    Ok(())
}

fn parse_forwarded_ip_token(raw: &str) -> Option<IpAddr> {
    let token = raw.trim().trim_matches('"');
    if token.is_empty() || token.eq_ignore_ascii_case("unknown") {
        return None;
    }

    token
        .parse::<IpAddr>()
        .ok()
        .or_else(|| token.parse::<SocketAddr>().ok().map(|addr| addr.ip()))
        .or_else(|| {
            let bracketed = token.strip_prefix('[')?;
            let end = bracketed.find(']')?;
            bracketed[..end].parse::<IpAddr>().ok()
        })
}

fn is_trusted_proxy_ip(ip: IpAddr) -> bool {
    ip.is_loopback() || is_local_network_ip(ip)
}

fn is_local_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local(),
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unique_local()
                || ipv6.is_unicast_link_local()
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_local_network_ip(IpAddr::V4(mapped)))
        }
    }
}

pub(crate) async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({"status": "ok"}))
}

pub(crate) async fn rate_limit_http_api(
    State(auth_state): State<AuthState>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if skip_http_rate_limit(request.method(), request.uri().path()) {
        return next.run(request).await;
    }

    let client_ip =
        request_client_ip(request.headers(), Some(remote_addr)).unwrap_or(remote_addr.ip());
    let actor = resolve_actor(&auth_state, request.headers(), Some(remote_addr)).await;
    let key = RateLimitKey::new(
        client_ip,
        actor.as_ref().map(|actor| actor.user.id.as_str()),
    );
    match auth_state.rate_limiter.check_http_api(&key) {
        Ok(()) => next.run(request).await,
        Err(decision) => {
            let mut response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse::new(decision.message)),
            )
                .into_response();
            if let Some(retry_after) = decision.retry_after
                && let Ok(value) = http::HeaderValue::from_str(&retry_after.as_secs().to_string())
            {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            response
        }
    }
}

fn skip_http_rate_limit(_method: &Method, path: &str) -> bool {
    !is_rate_limited_http_api_path(path)
}

fn is_rate_limited_http_api_path(path: &str) -> bool {
    path.starts_with("/backups/")
        || path == "/api"
        || path.starts_with("/api/")
        || path == "/authless-client"
        || path == "/oauth/token"
        || path == "/oauth/authorize/decision"
}

pub(crate) fn map_app_error(error: AppError) -> Response {
    match error {
        AppError::Unauthorized(message) => {
            (StatusCode::UNAUTHORIZED, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::Validation(message) => {
            (StatusCode::BAD_REQUEST, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::PluginInstallInProgress(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::NotFound(message) => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadFeedbackTimeout(message) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse::new(message)),
        )
            .into_response(),
        AppError::DownloadSubmitAmbiguous(message) => {
            (StatusCode::BAD_GATEWAY, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::DownloadSubmitUnavailable(message) => {
            (StatusCode::BAD_GATEWAY, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::MfaStepUpRequired(message)
        | AppError::TotpEnrollmentRequired(message)
        | AppError::MfaEnrollmentRequired(message)
        | AppError::TotpInvalidCode(message)
        | AppError::TotpRecoveryCodeUsed(message) => {
            (StatusCode::UNAUTHORIZED, Json(ErrorResponse::new(message))).into_response()
        }
        AppError::Repository(message) => {
            let error_id = Id::new().0;
            tracing::error!(
                error_id = %error_id,
                error_kind = "Repository",
                error = %message,
                "masked internal repository error"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::with_error_id(
                    INTERNAL_SERVER_ERROR_MESSAGE.to_string(),
                    error_id,
                )),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::to_bytes;
    use axum::http::HeaderValue;
    use axum::routing::get;
    use serde_json::Value;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::{LazyLock, Mutex};
    use tower::ServiceExt;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn clear_cors_env() {
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::remove_var(CORS_ALLOWED_ORIGINS_ENV);
            std::env::remove_var(WS_ALLOWED_ORIGINS_ENV);
            std::env::remove_var(PRODUCTION_CORS_OPT_IN_ENV);
            std::env::remove_var(WEB_UI_URL_ENV);
        }
    }

    #[test]
    fn default_cors_origins_match_runtime_mode() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();

        let origins = default_cors_allowed_origins_for_mode(cfg!(debug_assertions));
        let dev_origins = default_cors_allowed_origins_for_mode(true);
        let release_origins = default_cors_allowed_origins_for_mode(false);

        if cfg!(debug_assertions) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://localhost:3000")
            );
        } else {
            assert!(origins.is_empty());
        }
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://localhost:3000")
        );
        assert!(release_origins.is_empty());
    }

    #[test]
    fn web_ui_origin_only_extends_dev_mode_defaults() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WEB_UI_URL_ENV, "http://127.0.0.1:4545/app");
        }

        let origins = default_cors_allowed_origins_for_mode(cfg!(debug_assertions));
        let dev_origins = default_cors_allowed_origins_for_mode(true);
        let release_origins = default_cors_allowed_origins_for_mode(false);

        if cfg!(debug_assertions) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://127.0.0.1:4545")
            );
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://localhost:4545")
            );
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://host.docker.internal:4545")
            );
        } else {
            assert!(origins.is_empty());
        }

        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://127.0.0.1:4545")
        );
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://localhost:4545")
        );
        assert!(
            dev_origins
                .iter()
                .any(|origin| origin == "http://host.docker.internal:4545")
        );
        assert!(release_origins.is_empty());
        clear_cors_env();
    }

    #[test]
    fn cors_env_rejects_wildcard_origins() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(
                CORS_ALLOWED_ORIGINS_ENV,
                "*, http://*, https://*, http://localhost:3000",
            );
        }

        let config = CorsConfig::from_env_for_mode(true);

        assert!(!config.allow_all);
        assert!(config.is_allowed("http://localhost:3000"));
        assert!(!config.is_allowed("http://evil.example"));
        assert!(
            !config
                .allowed_origins
                .iter()
                .any(|origin| matches!(origin.as_str(), "*" | "http://*" | "https://*"))
        );
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_cors_env_without_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(CORS_ALLOWED_ORIGINS_ENV, "http://localhost:3000");
        }

        let config = CorsConfig::from_env_for_mode(false);

        assert!(!config.allow_all);
        assert!(config.allowed_origins.is_empty());
        assert!(!config.is_allowed("http://localhost:3000"));
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_cors_env_even_with_legacy_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(CORS_ALLOWED_ORIGINS_ENV, "http://localhost:3000/app");
            std::env::set_var(PRODUCTION_CORS_OPT_IN_ENV, "1");
        }

        let config = CorsConfig::from_env_for_mode(false);

        assert!(!config.allow_all);
        assert!(config.allowed_origins.is_empty());
        assert!(!config.is_allowed("http://localhost:3000"));
        assert!(!config.is_allowed("http://127.0.0.1:3000"));
        clear_cors_env();
    }

    #[test]
    fn websocket_origin_parser_rejects_wildcards() {
        let origins =
            parse_websocket_allowed_origins("*, http://*, https://*, http://localhost:3000/app");

        assert_eq!(origins, vec!["http://localhost:3000"]);
    }

    #[test]
    fn release_mode_ignores_websocket_env_without_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WS_ALLOWED_ORIGINS_ENV, "http://localhost:3000");
        }

        let cors = CorsConfig {
            allow_all: false,
            allowed_origins: Vec::new(),
        };
        let policy = WebSocketOriginPolicy::from_env_for_mode(&cors, false);

        assert!(policy.allowed_origins.is_empty());
        clear_cors_env();
    }

    #[test]
    fn release_mode_ignores_websocket_env_even_with_legacy_opt_in() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        clear_cors_env();
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var(WS_ALLOWED_ORIGINS_ENV, "http://localhost:3000/app");
            std::env::set_var(PRODUCTION_CORS_OPT_IN_ENV, "1");
        }

        let cors = CorsConfig {
            allow_all: false,
            allowed_origins: Vec::new(),
        };
        let policy = WebSocketOriginPolicy::from_env_for_mode(&cors, false);

        assert!(policy.allowed_origins.is_empty());
        clear_cors_env();
    }

    fn graphql_error_response_with_code(code: &str) -> async_graphql::BatchResponse {
        let mut extensions = ErrorExtensionValues::default();
        extensions.set("code", code);
        let mut error = ServerError::new("graphQL error", None);
        error.extensions = Some(extensions);
        async_graphql::BatchResponse::Single(GraphQLResponse::from_errors(vec![error]))
    }

    #[test]
    fn graphql_authentication_required_response_uses_unauthorized_status() {
        let response = graphql_error_response_with_code(AUTHENTICATION_REQUIRED_CODE);

        assert_eq!(graphql_response_status(&response), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn graphql_mfa_step_up_response_uses_step_up_status() {
        let response = graphql_error_response_with_code(MFA_STEP_UP_REQUIRED_CODE);

        assert_eq!(
            graphql_response_status(&response),
            StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE).unwrap()
        );
    }

    #[test]
    fn graphql_non_mfa_error_response_keeps_ok_status() {
        let response = graphql_error_response_with_code("VALIDATION_FAILED");

        assert_eq!(graphql_response_status(&response), StatusCode::OK);
    }

    #[test]
    fn graphql_batched_mfa_step_up_response_uses_step_up_status() {
        let mut extensions = ErrorExtensionValues::default();
        extensions.set("code", MFA_STEP_UP_REQUIRED_CODE);
        let mut error = ServerError::new("MFA step-up required", None);
        error.extensions = Some(extensions);
        let response = async_graphql::BatchResponse::Batch(vec![
            GraphQLResponse::new(async_graphql::Value::Null),
            GraphQLResponse::from_errors(vec![error]),
        ]);

        assert_eq!(
            graphql_response_status(&response),
            StatusCode::from_u16(MFA_STEP_UP_REQUIRED_STATUS_CODE).unwrap()
        );
    }

    #[test]
    fn graphql_batched_authentication_required_takes_precedence_over_step_up() {
        let mut auth_extensions = ErrorExtensionValues::default();
        auth_extensions.set("code", AUTHENTICATION_REQUIRED_CODE);
        let mut auth_error = ServerError::new("authentication required", None);
        auth_error.extensions = Some(auth_extensions);

        let mut mfa_extensions = ErrorExtensionValues::default();
        mfa_extensions.set("code", MFA_STEP_UP_REQUIRED_CODE);
        let mut mfa_error = ServerError::new("MFA step-up required", None);
        mfa_error.extensions = Some(mfa_extensions);

        let response = async_graphql::BatchResponse::Batch(vec![
            GraphQLResponse::from_errors(vec![mfa_error]),
            GraphQLResponse::from_errors(vec![auth_error]),
        ]);

        assert_eq!(graphql_response_status(&response), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn graphql_timeout_response_reports_sixty_second_execution_timeout() {
        let response = graphql_execution_timeout_response();
        let body = serde_json::to_value(&response).expect("timeout response serializes");

        assert_eq!(
            body["errors"][0]["message"],
            "GraphQL request timed out after 60 seconds"
        );
        assert_eq!(
            body["errors"][0]["extensions"]["code"],
            GRAPHQL_POST_EXECUTION_TIMEOUT_CODE
        );
        assert_eq!(body["errors"][0]["extensions"]["timeoutSeconds"], 60);
    }

    #[tokio::test]
    async fn repository_error_response_masks_details_and_includes_error_id() {
        let response = map_app_error(AppError::Repository(
            "database password leaked in upstream detail".into(),
        ));

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body_text = String::from_utf8(body.to_vec()).expect("response body is utf8");
        let body: Value = serde_json::from_str(&body_text).expect("response body is json");

        assert_eq!(body["error"], INTERNAL_SERVER_ERROR_MESSAGE);
        assert!(
            body["error_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(!body_text.contains("database password"));
        assert!(!body_text.contains("upstream detail"));
    }

    #[tokio::test]
    async fn validation_error_response_omits_error_id() {
        let response = map_app_error(AppError::Validation("bad request".into()));

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let body: Value = serde_json::from_slice(&body).expect("response body is json");

        assert_eq!(body["error"], "bad request");
        assert!(body.get("error_id").is_none());
    }

    #[test]
    fn local_network_ip_ranges_match_expected_blocks() {
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 16, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 31, 255, 254
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            192, 168, 5, 10
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 15, 0, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            172, 32, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_local_network_ip(IpAddr::V4(Ipv4Addr::new(
            169, 254, 10, 20
        ))));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0xfc00, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_local_network_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_local_network_ip(IpAddr::V6(Ipv6Addr::new(
            0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888
        ))));
    }

    fn auth_disabled_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: false,
            skip_login_for_local_ips: false,
            effective_form_login_enabled: false,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    fn auth_enabled_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: false,
            effective_form_login_enabled: true,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    fn protected_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: false,
            recovery_mode: false,
        }
    }

    fn public_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: true,
            recovery_mode: false,
        }
    }

    fn recovery_public_authless_policy() -> AuthlessAccessPolicy {
        AuthlessAccessPolicy {
            allow_unauthenticated_public_access: true,
            recovery_mode: true,
        }
    }

    #[test]
    fn authless_guard_allows_auth_enabled_requests() {
        let headers = HeaderMap::new();
        let decision = authless_access_decision(
            &auth_enabled_snapshot(),
            protected_authless_policy(),
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(decision, AuthlessAccessDecision::Allow);
    }

    #[test]
    fn authless_guard_allows_private_and_loopback_clients() {
        let headers = HeaderMap::new();
        for addr in [
            SocketAddr::from((Ipv4Addr::LOCALHOST, 3000)),
            SocketAddr::from((Ipv4Addr::new(10, 1, 2, 3), 3000)),
            SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000)),
            SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
            SocketAddr::from((Ipv4Addr::new(169, 254, 10, 20), 3000)),
            SocketAddr::from((Ipv6Addr::LOCALHOST, 3000)),
            SocketAddr::from((Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 1), 3000)),
            SocketAddr::from((Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1), 3000)),
        ] {
            assert_eq!(
                authless_access_decision(
                    &auth_disabled_snapshot(),
                    protected_authless_policy(),
                    &headers,
                    Some(addr),
                ),
                AuthlessAccessDecision::Allow,
                "{addr} should be allowed"
            );
        }
    }

    #[test]
    fn authless_guard_rejects_public_clients() {
        let headers = HeaderMap::new();

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((
                    Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888),
                    3000,
                ))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V6(
                Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0x8888)
            )))
        );
    }

    #[test]
    fn authless_guard_public_override_allows_public_clients() {
        let headers = HeaderMap::new();
        let decision = authless_access_decision(
            &auth_disabled_snapshot(),
            public_authless_policy(),
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(decision, AuthlessAccessDecision::Allow);
    }

    #[test]
    fn authless_guard_public_override_does_not_bypass_recovery_mode() {
        let headers = HeaderMap::new();

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                recovery_public_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
    }

    #[test]
    fn authless_guard_does_not_trust_forwarded_headers_from_public_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicPeer(IpAddr::V4(
                Ipv4Addr::new(8, 8, 8, 8)
            )))
        );
    }

    #[test]
    fn authless_guard_rejects_forwarded_proto_without_client_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    #[test]
    fn authless_guard_rejects_public_forwarded_client_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[test]
    fn authless_guard_rejects_public_ip_anywhere_in_forwarded_for_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[test]
    fn authless_guard_rejects_public_ip_anywhere_in_forwarded_header_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("for=192.168.1.25;proto=https, for=8.8.8.8"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::PublicForwardedClient(
                IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))
            ))
        );
    }

    #[test]
    fn authless_guard_allows_private_forwarded_client_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_allows_private_forwarded_chain_through_private_peer() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_allows_forwarded_proto_with_private_client_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Allow
        );
    }

    #[test]
    fn authless_guard_rejects_malformed_forwarded_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    #[test]
    fn authless_guard_rejects_malformed_ip_anywhere_in_forwarded_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, not-an-ip"),
        );

        assert_eq!(
            authless_access_decision(
                &auth_disabled_snapshot(),
                protected_authless_policy(),
                &headers,
                Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
            ),
            AuthlessAccessDecision::Reject(AuthlessAccessRejectReason::MalformedForwardedClient)
        );
    }

    fn authless_guard_test_app(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
    ) -> Router {
        authless_guard_test_app_with_proof_state(
            snapshot,
            policy,
            AuthlessWebClientProofState::new(),
        )
    }

    fn authless_guard_test_app_with_proof_state(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
        _web_client_proof: AuthlessWebClientProofState,
    ) -> Router {
        let state = AuthlessAccessGuardState {
            auth_runtime: AuthRuntimeStateHandle::new(snapshot),
            policy,
        };
        Router::new()
            .route("/graphql", get(|| async { "graphql ok" }))
            .route("/graphql/ws", get(|| async { "ws ok" }))
            .layer(axum::middleware::from_fn_with_state(
                state,
                enforce_authless_access_guard,
            ))
    }

    fn authless_web_client_test_app(
        snapshot: scryer_interface::context::AuthRuntimeStateSnapshot,
        policy: AuthlessAccessPolicy,
    ) -> Router {
        let state = AuthlessWebClientProofRouteState {
            auth_runtime: AuthRuntimeStateHandle::new(snapshot),
            policy,
            proof: AuthlessWebClientProofState::new(),
        };
        Router::new().route(
            "/authless-client",
            get(authless_web_client_proof_handler).with_state(state),
        )
    }

    fn request_with_peer(uri: &str, peer: SocketAddr) -> Request<Body> {
        let mut request = Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    fn request_with_peer_and_authless_proof(
        uri: &str,
        peer: SocketAddr,
        proof_state: &AuthlessWebClientProofState,
    ) -> Request<Body> {
        let (nonce, proof, _) = proof_state.issue().expect("issue proof");
        let mut request = Request::builder()
            .uri(uri)
            .header(AUTHLESS_WEB_CLIENT_HEADER, proof)
            .header(
                header::COOKIE,
                format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"),
            )
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    }

    async fn read_json_response(response: Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("json response")
    }

    #[tokio::test]
    async fn authless_guard_middleware_allows_private_graphql_request() {
        let proof_state = AuthlessWebClientProofState::new();
        let app = authless_guard_test_app_with_proof_state(
            auth_disabled_snapshot(),
            protected_authless_policy(),
            proof_state.clone(),
        );

        let response = app
            .oneshot(request_with_peer_and_authless_proof(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                &proof_state,
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn authless_guard_middleware_rejects_public_graphql_request() {
        let app = authless_guard_test_app(auth_disabled_snapshot(), protected_authless_policy());

        let response = app
            .oneshot(request_with_peer(
                "/graphql",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authless_guard_middleware_rejects_public_websocket_route_before_handler() {
        let app = authless_guard_test_app(auth_disabled_snapshot(), protected_authless_policy());

        let response = app
            .oneshot(request_with_peer(
                "/graphql/ws",
                SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
            ))
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    fn local_bypass_snapshot() -> scryer_interface::context::AuthRuntimeStateSnapshot {
        scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            webauthn_configured: false,
            passkey_enabled: false,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        }
    }

    #[test]
    fn local_ip_bypass_accepts_direct_private_and_loopback_clients() {
        let snapshot = local_bypass_snapshot();
        let headers = HeaderMap::new();

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 16, 5, 173), 3000))),
        ));
        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::LOCALHOST, 3000))),
        ));
        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_claims_satisfy_step_up_checks() {
        let claims = mfa_bypass_token_claims();

        assert_eq!(claims.mfa_verified_until, Some(i64::MAX));
        assert_eq!(
            claims.session_scope,
            scryer_application::JwtSessionScope::Full
        );
    }

    #[test]
    fn forwarded_headers_from_trusted_proxy_are_used() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        );

        assert_eq!(client_ip, Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 25))));
    }

    #[test]
    fn forwarded_headers_from_untrusted_peer_are_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        );

        assert_eq!(client_ip, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn forwarded_ipv6_headers_from_trusted_proxy_are_used() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("[fc00::25]:8443"));

        let client_ip = request_client_ip(
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        );

        assert_eq!(
            client_ip,
            Some(IpAddr::V6(Ipv6Addr::new(0xfc00, 0, 0, 0, 0, 0, 0, 0x25))),
        );
    }

    #[test]
    fn local_ip_bypass_accepts_local_forwarded_client_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_private_forwarded_chain_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_forwarded_proto_with_private_client_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.5"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_accepts_local_forwarded_ipv6_client_through_trusted_proxy() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("[fc00::25]:8443"));

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv6Addr::LOCALHOST, 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_forwarded_proto_without_client_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn spa_fallback_routes_do_not_consume_http_api_quota() {
        assert!(skip_http_rate_limit(&Method::GET, "/activity"));
        assert!(skip_http_rate_limit(&Method::GET, "/settings/profile"));
    }

    #[test]
    fn ticket_download_and_api_routes_consume_http_api_quota() {
        assert!(!skip_http_rate_limit(
            &Method::GET,
            "/backups/scryer.scryer-backup.enc/download"
        ));
        assert!(!skip_http_rate_limit(&Method::GET, "/api/system/jobs"));
    }

    #[test]
    fn oauth_token_and_decision_routes_consume_http_api_quota() {
        assert!(!skip_http_rate_limit(&Method::POST, "/oauth/token"));
        assert!(!skip_http_rate_limit(
            &Method::POST,
            "/oauth/authorize/decision"
        ));
        assert!(skip_http_rate_limit(&Method::GET, "/oauth/authorize"));
    }

    #[test]
    fn authless_web_client_route_consumes_http_api_quota() {
        assert!(!skip_http_rate_limit(&Method::GET, "/authless-client"));
    }

    #[tokio::test]
    async fn authless_web_client_proof_sets_hardened_cookie_and_cache_headers() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store, max-age=0"))
        );
        assert_eq!(
            response.headers().get(header::PRAGMA),
            Some(&HeaderValue::from_static("no-cache"))
        );
        assert_eq!(
            response.headers().get(header::EXPIRES),
            Some(&HeaderValue::from_static("0"))
        );
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie");
        assert!(cookie.starts_with(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}=")));
        assert!(cookie.contains("Path=/"));
        assert!(cookie.contains("Max-Age=300"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!cookie.contains("Secure"));

        let body = read_json_response(response).await;
        assert!(
            body["proof"]
                .as_str()
                .is_some_and(|proof| proof.matches('.').count() == 2)
        );
        assert!(body["expiresAt"].as_u64().is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_sets_secure_cookie_for_https_forwarded_request() {
        let mut request = request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000)),
        );
        request
            .headers_mut()
            .insert(X_FORWARDED_PROTO, HeaderValue::from_static("https"));
        request.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 172.18.0.2"),
        );

        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("set-cookie");
        assert!(cookie.contains("Secure"));
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_public_clients_when_protected() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store, max-age=0"))
        );
        let body = read_json_response(response).await;
        assert_eq!(
            body["error"],
            "Scryer web client proof is not available for this request"
        );
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_cross_site_browser_requests() {
        let mut request = request_with_peer(
            "/authless-client",
            SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
        );
        request
            .headers_mut()
            .insert("sec-fetch-site", HeaderValue::from_static("cross-site"));

        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), protected_authless_policy())
                .oneshot(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[tokio::test]
    async fn authless_web_client_proof_allows_explicit_public_authless_access() {
        let response =
            authless_web_client_test_app(auth_disabled_snapshot(), public_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_allows_local_ip_bypass_clients() {
        let response =
            authless_web_client_test_app(local_bypass_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::SET_COOKIE).is_some());
    }

    #[tokio::test]
    async fn authless_web_client_proof_rejects_regular_login_mode() {
        let response =
            authless_web_client_test_app(auth_enabled_snapshot(), protected_authless_policy())
                .oneshot(request_with_peer(
                    "/authless-client",
                    SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 3000)),
                ))
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
    }

    #[test]
    fn authless_web_client_proof_requires_matching_cookie_nonce() {
        let proof_state = AuthlessWebClientProofState::new();
        let (nonce, proof, _) = proof_state.issue().expect("issue proof");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHLESS_WEB_CLIENT_HEADER,
            HeaderValue::from_str(&proof).expect("proof header"),
        );
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{AUTHLESS_WEB_CLIENT_COOKIE}={nonce}"))
                .expect("cookie header"),
        );

        assert!(proof_state.validate_headers(&headers, None));

        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("scryer_authless_client=other"),
        );
        assert!(!proof_state.validate_headers(&headers, None));
    }

    #[test]
    fn title_images_and_static_assets_do_not_consume_http_api_quota() {
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/images/titles/title-1/poster/original"
        ));
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/images/titles/title-1/fanart/w1280"
        ));
        assert!(skip_http_rate_limit(
            &Method::GET,
            "/assets/index-B3b5rA.js"
        ));
        assert!(skip_http_rate_limit(&Method::GET, "/manifest.json"));
    }

    #[test]
    fn local_ip_bypass_rejects_public_ip_anywhere_in_forwarded_for_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, 8.8.8.8"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_ip_anywhere_in_forwarded_header_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::FORWARDED,
            HeaderValue::from_static("for=192.168.1.25;proto=https, for=8.8.8.8"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_localhost_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_private_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("172.16.5.173:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_host_with_public_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("example.com:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_private_forwarded_host_without_forwarded_client_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("172.16.5.173:3000"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_malformed_forwarded_ip_with_local_host() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("not-an-ip"));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_malformed_ip_anywhere_in_forwarded_chain() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("192.168.1.25, not-an-ip"),
        );

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_ip_bypass_rejects_public_peer_with_spoofed_local_forwarded_ip() {
        let snapshot = local_bypass_snapshot();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("192.168.1.25"));

        assert!(!local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(8, 8, 8, 8), 3000))),
        ));
    }
}
