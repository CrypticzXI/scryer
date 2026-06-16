use async_graphql::http::ALL_WEBSOCKET_PROTOCOLS;
use async_graphql::{Data, ErrorExtensionValues, Response as GraphQLResponse, ServerError};
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use scryer_application::{AppError, AppUseCase, AuthenticatedTokenClaims, JwtSessionScope};
use scryer_domain::AppPermission;
use scryer_interface::context::{AuthRuntimeStateHandle, ConnectionAuthEpoch, MfaVerification};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::admin_routes::ErrorResponse;
use crate::base_path::BasePath;
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

#[derive(Clone, Debug)]
pub(crate) struct CorsConfig {
    pub(crate) allow_all: bool,
    pub(crate) allowed_origins: Vec<String>,
}

impl CorsConfig {
    pub(crate) fn from_env() -> Self {
        let raw = std::env::var("SCRYER_CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| default_cors_allowed_origins().join(","));

        let origins = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();

        let allow_all = origins
            .iter()
            .any(|origin| matches!(origin.as_str(), "*" | "https://*" | "http://*"));

        Self {
            allow_all,
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

#[derive(Clone, Debug, Default)]
pub(crate) struct WebSocketOriginPolicy {
    allowed_origins: Vec<String>,
}

impl WebSocketOriginPolicy {
    pub(crate) fn from_env(cors: &CorsConfig) -> Self {
        let origins = match std::env::var("SCRYER_WS_ALLOWED_ORIGINS") {
            Ok(raw) => parse_websocket_allowed_origins(&raw),
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

fn default_cors_allowed_origins() -> Vec<String> {
    let mut origins = if cfg!(debug_assertions) {
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

    if cfg!(debug_assertions)
        && let Ok(web_ui_url) = std::env::var("SCRYER_WEB_UI_URL")
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
        return Some(trimmed.to_string());
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
    let initial_data = graphql_ws_connection_data(connection_epoch, initial_actor.clone());

    ws.protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| async move {
            let app_for_init = app.clone();
            let initial_actor = initial_actor.clone();
            GraphQLWebSocket::new(stream, schema, protocol)
                .with_data(initial_data)
                .on_connection_init(move |value: serde_json::Value| async move {
                    let auth_value = value.get("Authorization").and_then(|v| v.as_str());
                    let actor = resolve_ws_connection_init_actor(
                        &app_for_init,
                        auth_enabled,
                        local_bypass_active,
                        initial_actor.clone(),
                        auth_value,
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
    let batch = if let Some(actor) = actor {
        match batch {
            async_graphql::BatchRequest::Single(req) => async_graphql::BatchRequest::Single(
                req.data(actor.mfa_verification()).data(actor.user),
            ),
            async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
                reqs.into_iter()
                    .map(|req| req.data(actor.mfa_verification()).data(actor.user.clone()))
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
}

impl ResolvedActor {
    fn mfa_verification(&self) -> MfaVerification {
        MfaVerification {
            verified_until: self.token_claims.mfa_verified_until,
            step_up_verified_until: self.token_claims.mfa_step_up_verified_until,
            session_scope: self.token_claims.session_scope,
        }
    }
}

fn graphql_ws_connection_data(connection_epoch: u64, actor: Option<ResolvedActor>) -> Data {
    let mut data = Data::default();
    data.insert(ConnectionAuthEpoch(connection_epoch));
    if let Some(actor) = actor {
        data.insert(actor.mfa_verification());
        data.insert(actor.user);
    }
    data
}

async fn resolve_ws_connection_init_actor(
    app: &AppUseCase,
    auth_enabled: bool,
    local_bypass_active: bool,
    initial_actor: Option<ResolvedActor>,
    auth_value: Option<&str>,
) -> Result<Option<ResolvedActor>, async_graphql::Error> {
    if !auth_enabled {
        return Ok(initial_actor);
    }

    let Some(raw) = auth_value else {
        return Ok(initial_actor);
    };

    match parse_bearer_token(raw) {
        Some(token) => match app.authenticate_token_with_claims(token).await {
            Ok((user, token_claims)) => app
                .attach_user_authorization(user)
                .await
                .map(|user| Some(ResolvedActor { user, token_claims }))
                .map_err(|e| async_graphql::Error::new(format!("authentication failed: {e}"))),
            Err(_) if local_bypass_active => Ok(initial_actor),
            Err(e) => Err(async_graphql::Error::new(format!(
                "authentication failed: {e}"
            ))),
        },
        None if local_bypass_active => Ok(initial_actor),
        None => Err(async_graphql::Error::new("invalid authorization header")),
    }
}

async fn resolve_actor(
    state: &AuthState,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> Option<ResolvedActor> {
    let snapshot = state.auth_runtime.snapshot();
    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    let actor = if !snapshot.effective_form_login_enabled {
        resolve_default_user(&state.app)
            .await
            .map(|user| (user, AuthenticatedTokenClaims::default()))
    } else {
        match authorization_token_from_headers(headers) {
            Ok(Some(token)) => match state.app.authenticate_token_with_claims(token).await {
                Ok((user, token_claims)) => Some((user, token_claims)),
                Err(_) if local_bypass => resolve_default_user(&state.app)
                    .await
                    .map(|user| (user, mfa_bypass_token_claims())),
                Err(_) => None,
            },
            Ok(None) | Err(_) if local_bypass => resolve_default_user(&state.app)
                .await
                .map(|user| (user, mfa_bypass_token_claims())),
            Ok(None) | Err(_) => None,
        }
    };

    match actor {
        Some((user, token_claims)) => state
            .app
            .attach_user_authorization(user)
            .await
            .ok()
            .map(|user| ResolvedActor { user, token_claims }),
        None => None,
    }
}

async fn resolve_default_user(app_use_case: &AppUseCase) -> Option<scryer_domain::User> {
    match app_use_case.find_default_user().await {
        Ok(Some(user)) => Some(user),
        Ok(None) => app_use_case.find_or_create_default_user().await.ok(),
        Err(_) => None,
    }
}

fn mfa_bypass_token_claims() -> AuthenticatedTokenClaims {
    AuthenticatedTokenClaims {
        mfa_verified_until: Some(i64::MAX),
        mfa_step_up_verified_until: Some(i64::MAX),
        ..AuthenticatedTokenClaims::default()
    }
}

fn ensure_full_session_claims(claims: &AuthenticatedTokenClaims) -> Result<(), AppError> {
    if claims.session_scope == JwtSessionScope::MfaEnrollment {
        return Err(AppError::MfaEnrollmentRequired(
            "MFA enrollment must be completed before accessing Scryer".into(),
        ));
    }

    Ok(())
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

pub(crate) async fn resolve_actor_with_app_permission(
    app_use_case: &AppUseCase,
    auth_runtime: &AuthRuntimeStateHandle,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
    required_permission: AppPermission,
) -> Result<String, AppError> {
    let snapshot = auth_runtime.snapshot();
    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    let actor = if !snapshot.effective_form_login_enabled {
        resolve_default_user_required(app_use_case).await?
    } else {
        match authorization_token_from_headers(headers) {
            Ok(Some(token)) => match app_use_case.authenticate_token_with_claims(token).await {
                Ok((actor, claims)) => {
                    ensure_full_session_claims(&claims)?;
                    actor
                }
                Err(_) if local_bypass => resolve_default_user_required(app_use_case).await?,
                Err(error) => return Err(error),
            },
            Ok(None) if local_bypass => resolve_default_user_required(app_use_case).await?,
            Ok(None) => return Err(AppError::Unauthorized("authorization required".into())),
            Err(_) if local_bypass => resolve_default_user_required(app_use_case).await?,
            Err(error) => return Err(error),
        }
    };

    let actor = app_use_case.attach_user_authorization(actor).await?;
    app_use_case
        .require_app_permission(&actor, required_permission)
        .await?;

    Ok(actor.id)
}

async fn resolve_default_user_required(
    app_use_case: &AppUseCase,
) -> Result<scryer_domain::User, AppError> {
    match app_use_case.find_default_user().await? {
        Some(user) => Ok(user),
        None => app_use_case.find_or_create_default_user().await,
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
            && forwarded_client_ip(headers).is_some_and(is_local_network_ip);
    }

    is_local_network_ip(peer_ip)
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

fn has_proxy_forwarding_headers(headers: &HeaderMap) -> bool {
    headers.contains_key("x-forwarded-for")
        || headers.contains_key("x-real-ip")
        || headers.contains_key(header::FORWARDED)
        || headers.contains_key("x-forwarded-host")
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
                Json(ErrorResponse {
                    error: decision.message,
                }),
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
    path == "/admin" || path.starts_with("/admin/") || path == "/api" || path.starts_with("/api/")
}

pub(crate) fn map_app_error(error: AppError) -> Response {
    match error {
        AppError::Unauthorized(message) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::Validation(message) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::PluginInstallInProgress(message) => {
            (StatusCode::CONFLICT, Json(ErrorResponse { error: message })).into_response()
        }
        AppError::NotFound(message) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::DownloadFeedbackTimeout(message) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::DownloadSubmitAmbiguous(message) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::DownloadSubmitUnavailable(message) => (
            StatusCode::BAD_GATEWAY,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::MfaStepUpRequired(message)
        | AppError::TotpEnrollmentRequired(message)
        | AppError::MfaEnrollmentRequired(message)
        | AppError::TotpInvalidCode(message)
        | AppError::TotpRecoveryCodeUsed(message) => (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
        AppError::Repository(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { error: message }),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    #[test]
    fn default_cors_origins_match_runtime_mode() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::remove_var("SCRYER_WEB_UI_URL");
        }

        let origins = default_cors_allowed_origins();

        if cfg!(debug_assertions) {
            assert!(
                origins
                    .iter()
                    .any(|origin| origin == "http://localhost:3000")
            );
        } else {
            assert!(origins.is_empty());
        }
    }

    #[test]
    fn web_ui_origin_only_extends_dev_mode_defaults() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::set_var("SCRYER_WEB_UI_URL", "http://127.0.0.1:4545/app");
        }

        let origins = default_cors_allowed_origins();

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

        // SAFETY: tests serialize access to process env via ENV_LOCK.
        unsafe {
            std::env::remove_var("SCRYER_WEB_UI_URL");
        }
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
    fn enrollment_scoped_claims_are_not_full_admin_sessions() {
        let claims = AuthenticatedTokenClaims {
            session_scope: JwtSessionScope::MfaEnrollment,
            ..AuthenticatedTokenClaims::default()
        };

        let error = ensure_full_session_claims(&claims).expect_err("enrollment scope rejected");

        assert!(matches!(error, AppError::MfaEnrollmentRequired(_)));
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
    fn spa_fallback_routes_do_not_consume_http_api_quota() {
        assert!(skip_http_rate_limit(&Method::GET, "/activity"));
        assert!(skip_http_rate_limit(&Method::GET, "/settings/profile"));
    }

    #[test]
    fn admin_routes_still_consume_http_api_quota() {
        assert!(!skip_http_rate_limit(&Method::GET, "/admin/settings"));
        assert!(!skip_http_rate_limit(&Method::GET, "/api/system/jobs"));
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
