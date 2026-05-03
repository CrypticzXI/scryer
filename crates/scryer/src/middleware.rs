use async_graphql::Data;
use async_graphql::http::{ALL_WEBSOCKET_PROTOCOLS, GraphiQLSource};
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::{HeaderMap, Method, Request, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use scryer_application::{AppError, AppUseCase};
use scryer_domain::Entitlement;
use scryer_interface::context::{AuthRuntimeStateHandle, ConnectionAuthEpoch};
use std::net::{IpAddr, SocketAddr};

use crate::admin_routes::ErrorResponse;
use crate::base_path::BasePath;

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

fn default_cors_allowed_origins() -> Vec<String> {
    let mut origins = vec![
        "http://localhost:3000".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "http://0.0.0.0:3000".to_string(),
        "http://host.docker.internal:3000".to_string(),
        "http://nodejs:3000".to_string(),
    ];

    if let Ok(web_ui_url) = std::env::var("SCRYER_WEB_UI_URL")
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

pub(crate) async fn graphiql_handler() -> impl IntoResponse {
    let base_path = BasePath::from_env();
    let endpoint = base_path.join("/graphql");
    axum::response::Html(GraphiQLSource::build().endpoint(&endpoint).finish())
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

    let mut initial_data = Data::default();
    initial_data.insert(ConnectionAuthEpoch(connection_epoch));
    if (!auth_enabled || local_bypass_active)
        && let Ok(user) = app.find_or_create_default_user().await
    {
        initial_data.insert(user);
    }

    ws.protocols(ALL_WEBSOCKET_PROTOCOLS)
        .on_upgrade(move |stream| async move {
            let app_for_init = app.clone();
            GraphQLWebSocket::new(stream, schema, protocol)
                .with_data(initial_data)
                .on_connection_init(move |value: serde_json::Value| async move {
                    let mut data = Data::default();
                    data.insert(ConnectionAuthEpoch(connection_epoch));
                    if !auth_enabled {
                        return Ok(data);
                    }
                    let auth_value = value.get("Authorization").and_then(|v| v.as_str());
                    if let Some(raw) = auth_value {
                        match parse_bearer_token(raw) {
                            Some(token) => match app_for_init.authenticate_token(token).await {
                                Ok(user) => {
                                    data.insert(user);
                                }
                                Err(_) if local_bypass_active => {
                                    return Ok(data);
                                }
                                Err(e) => {
                                    return Err(async_graphql::Error::new(format!(
                                        "authentication failed: {e}"
                                    )));
                                }
                            },
                            None if local_bypass_active => {
                                return Ok(data);
                            }
                            None => {
                                return Err(async_graphql::Error::new(
                                    "invalid authorization header",
                                ));
                            }
                        }
                    } else if local_bypass_active {
                        return Ok(data);
                    }
                    Ok(data)
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
    let mut batch = body.into_inner();
    let response_status = graphql_response_status(&mut batch);
    let batch = if let Some(user) = actor {
        match batch {
            async_graphql::BatchRequest::Single(req) => {
                async_graphql::BatchRequest::Single(req.data(user))
            }
            async_graphql::BatchRequest::Batch(reqs) => async_graphql::BatchRequest::Batch(
                reqs.into_iter().map(|req| req.data(user.clone())).collect(),
            ),
        }
    } else {
        batch
    };

    // Wrap execution in a single-item stream so the future is dropped (cancelled)
    // when hyper detects the client has disconnected.
    let schema = state.schema.clone();
    let body_stream = futures_util::stream::once(async move {
        let batch_response = schema.execute_batch(batch).await;
        Ok::<_, std::io::Error>(
            serde_json::to_vec(&batch_response).unwrap_or_else(|_| b"{}".to_vec()),
        )
    });

    Response::builder()
        .status(response_status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(body_stream))
        .unwrap()
}

fn graphql_response_status(batch: &mut async_graphql::BatchRequest) -> StatusCode {
    let _ = batch;
    StatusCode::OK
}

async fn resolve_actor(
    state: &AuthState,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> Option<scryer_domain::User> {
    let snapshot = state.auth_runtime.snapshot();
    if !snapshot.effective_form_login_enabled {
        return state.app.find_or_create_default_user().await.ok();
    }

    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    match authorization_token_from_headers(headers) {
        Ok(Some(token)) => match state.app.authenticate_token(token).await {
            Ok(user) => Some(user),
            Err(_) if local_bypass => state.app.find_or_create_default_user().await.ok(),
            Err(_) => None,
        },
        Ok(None) | Err(_) if local_bypass => state.app.find_or_create_default_user().await.ok(),
        Ok(None) | Err(_) => None,
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

pub(crate) async fn resolve_actor_with_entitlement(
    app_use_case: &AppUseCase,
    auth_runtime: &AuthRuntimeStateHandle,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
    required_entitlement: Entitlement,
) -> Result<String, AppError> {
    let snapshot = auth_runtime.snapshot();
    if !snapshot.effective_form_login_enabled {
        let actor = app_use_case.find_or_create_default_user().await?;
        return Ok(actor.id);
    }

    let local_bypass = local_ip_bypass_active(&snapshot, headers, remote_addr);
    let actor = match authorization_token_from_headers(headers) {
        Ok(Some(token)) => match app_use_case.authenticate_token(token).await {
            Ok(actor) => actor,
            Err(_) if local_bypass => app_use_case.find_or_create_default_user().await?,
            Err(error) => return Err(error),
        },
        Ok(None) if local_bypass => app_use_case.find_or_create_default_user().await?,
        Ok(None) => return Err(AppError::Unauthorized("authorization required".into())),
        Err(_) if local_bypass => app_use_case.find_or_create_default_user().await?,
        Err(error) => return Err(error),
    };

    if !actor.has_entitlement(&required_entitlement) {
        return Err(AppError::Unauthorized(
            "authenticated user does not have required entitlement".into(),
        ));
    }

    Ok(actor.id)
}

fn local_ip_bypass_active(
    snapshot: &scryer_interface::context::AuthRuntimeStateSnapshot,
    headers: &HeaderMap,
    remote_addr: Option<SocketAddr>,
) -> bool {
    if !snapshot.effective_form_login_enabled || !snapshot.skip_login_for_local_ips {
        return false;
    }

    request_client_ip(headers, remote_addr).is_some_and(is_local_network_ip)
        || remote_addr
            .map(|addr| addr.ip())
            .is_some_and(is_trusted_proxy_ip)
            && request_target_is_local(headers)
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

fn request_target_is_local(headers: &HeaderMap) -> bool {
    forwarded_host_header(headers)
        .or_else(|| host_header(headers))
        .is_some_and(is_local_host_value)
}

fn forwarded_host_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-host")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
}

fn host_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
}

fn is_local_host_value(raw: &str) -> bool {
    let trimmed = raw.trim().trim_matches('"');
    if trimmed.is_empty() {
        return false;
    }

    let Ok(authority) = trimmed.parse::<http::uri::Authority>() else {
        return trimmed.eq_ignore_ascii_case("localhost")
            || parse_forwarded_ip_token(trimmed).is_some_and(is_local_network_ip);
    };

    let host = authority.host().trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || parse_forwarded_ip_token(host).is_some_and(is_local_network_ip)
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

    #[test]
    fn scan_library_mutation_returns_ok_status() {
        let mut batch = async_graphql::BatchRequest::Single(async_graphql::Request::new(
            "mutation StartScan { scanLibrary(facet: movie) { sessionId } }",
        ));

        assert_eq!(graphql_response_status(&mut batch), StatusCode::OK);
    }

    #[test]
    fn rehydrate_all_metadata_mutation_returns_ok_status() {
        let mut batch = async_graphql::BatchRequest::Single(async_graphql::Request::new(
            "mutation RehydrateAllMetadata { rehydrateAllMetadata(language: \"jpn\") }",
        ));

        assert_eq!(graphql_response_status(&mut batch), StatusCode::OK);
    }

    #[test]
    fn queue_manual_import_mutation_returns_ok_status() {
        let mut batch = async_graphql::BatchRequest::Single(async_graphql::Request::new(
            r#"mutation QueueManualImport {
                queueManualImport(input: {
                    titleId: "title-1"
                    clientType: "nzbget"
                    downloadClientItemId: "download-1"
                }) {
                    importId
                }
            }"#,
        ));

        assert_eq!(graphql_response_status(&mut batch), StatusCode::OK);
    }

    #[test]
    fn non_scan_requests_keep_ok_status() {
        let mut batch = async_graphql::BatchRequest::Single(async_graphql::Request::new(
            "query ActiveScans { activeLibraryScans { sessionId } }",
        ));

        assert_eq!(graphql_response_status(&mut batch), StatusCode::OK);
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

    #[test]
    fn local_ip_bypass_accepts_direct_private_and_loopback_clients() {
        let snapshot = scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        };
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
    fn local_bypass_accepts_localhost_host_through_trusted_proxy() {
        let snapshot = scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(header::HOST, HeaderValue::from_static("localhost:3000"));

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_bypass_accepts_private_host_through_trusted_proxy() {
        let snapshot = scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", HeaderValue::from_static("8.8.8.8"));
        headers.insert(
            "x-forwarded-host",
            HeaderValue::from_static("172.16.5.173:3000"),
        );

        assert!(local_ip_bypass_active(
            &snapshot,
            &headers,
            Some(SocketAddr::from((Ipv4Addr::new(172, 18, 0, 2), 3000))),
        ));
    }

    #[test]
    fn local_bypass_rejects_public_host_with_public_forwarded_ip() {
        let snapshot = scryer_interface::context::AuthRuntimeStateSnapshot {
            form_login_enabled: true,
            skip_login_for_local_ips: true,
            effective_form_login_enabled: true,
            env_override_active: false,
            env_override_description: None,
            epoch: 1,
        };
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
}
