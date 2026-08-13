use std::collections::{HashMap, HashSet};

use futures_util::StreamExt;
use reqwest::{Client, Response, StatusCode};
use scryer_application::{
    AppError, AppResult, EmbyApiKeyExchange, EmbyApiKeyExchangeCleanup, EmbyAvatar,
    EmbyConnectAddressStatus, EmbyConnectIdentityVerification, EmbyConnectServer,
    EmbyConnectUserType, EmbyServerIdentity, EmbyServerUser, VerifiedExternalIdentity,
};
use scryer_domain::ExternalAccountProvider;
use serde::{Deserialize, Serialize};
use url::Url;

const SCRYER_PRODUCT: &str = "Scryer";
const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");
const JSON_LIMIT: usize = 1024 * 1024;
const AVATAR_LIMIT: usize = 2 * 1024 * 1024;
const PAGE_LIMIT: usize = 100;
const RECORD_LIMIT: usize = 10_000;

#[derive(Clone)]
struct ConnectSession {
    user_id: String,
    access_token: String,
}

#[derive(Clone)]
struct ConnectServerSecret {
    system_id: String,
    access_key: String,
    name: String,
    user_type: EmbyConnectUserType,
    local_address: Option<String>,
    remote_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PublicSystemInfo {
    #[serde(rename = "Id", alias = "id")]
    id: Option<String>,
    #[serde(rename = "ServerName", alias = "serverName")]
    server_name: Option<String>,
    #[serde(rename = "Version", alias = "version")]
    version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SystemInfo {
    #[serde(rename = "Id", alias = "id")]
    id: Option<String>,
    #[serde(rename = "ServerName", alias = "serverName")]
    server_name: Option<String>,
    #[serde(rename = "Version", alias = "version")]
    version: Option<String>,
}

#[derive(Serialize)]
struct AuthRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Clone, Debug, Deserialize)]
struct UserDto {
    #[serde(rename = "Id", alias = "id")]
    id: String,
    #[serde(rename = "Name", alias = "name")]
    name: Option<String>,
    #[serde(rename = "ConnectUserName", alias = "connectUserName")]
    connect_user_name: Option<String>,
    #[serde(rename = "PrimaryImageTag", alias = "primaryImageTag")]
    primary_image_tag: Option<String>,
    #[serde(rename = "Policy", alias = "policy")]
    policy: Option<UserPolicy>,
    #[serde(rename = "HasPassword", alias = "hasPassword")]
    has_password: Option<bool>,
    #[serde(rename = "HasConfiguredPassword", alias = "hasConfiguredPassword")]
    has_configured_password: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct UserPolicy {
    #[serde(rename = "IsAdministrator", alias = "isAdministrator", default)]
    is_administrator: bool,
    #[serde(rename = "IsDisabled", alias = "isDisabled", default)]
    is_disabled: bool,
}

#[derive(Deserialize)]
struct AuthResponse {
    #[serde(rename = "User", alias = "user")]
    user: UserDto,
    #[serde(rename = "AccessToken", alias = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "ServerId", alias = "serverId")]
    server_id: Option<String>,
}

#[derive(Deserialize)]
struct KeyQueryResult {
    #[serde(rename = "Items", alias = "items", default)]
    items: Vec<KeyInfo>,
    #[serde(rename = "TotalRecordCount", alias = "totalRecordCount", default)]
    total_record_count: usize,
}

#[derive(Clone, Deserialize)]
struct KeyInfo {
    #[serde(rename = "AppName", alias = "appName")]
    app_name: Option<String>,
    #[serde(rename = "AccessToken", alias = "accessToken")]
    access_token: Option<String>,
    #[serde(rename = "DateCreated", alias = "dateCreated")]
    date_created: Option<String>,
}

#[derive(Deserialize)]
struct ConnectAuthResponse {
    #[serde(rename = "AccessToken", alias = "ConnectAccessToken")]
    access_token: Option<String>,
    #[serde(rename = "ConnectUserId")]
    connect_user_id: Option<String>,
    #[serde(rename = "User")]
    user: Option<ConnectUser>,
}

#[derive(Debug, Deserialize)]
struct ConnectUser {
    #[serde(rename = "Id")]
    id: Option<String>,
}

#[derive(Deserialize)]
struct ConnectServerWire {
    #[serde(rename = "SystemId")]
    system_id: Option<String>,
    #[serde(rename = "AccessKey")]
    access_key: Option<String>,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "Url")]
    url: Option<String>,
    #[serde(rename = "LocalAddress")]
    local_address: Option<String>,
    #[serde(rename = "UserType")]
    user_type: Option<String>,
}

#[derive(Deserialize)]
struct ConnectExchangeResponse {
    #[serde(rename = "LocalUserId")]
    local_user_id: Option<String>,
    #[serde(rename = "AccessToken")]
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UserQueryResult {
    #[serde(rename = "Items", alias = "items", default)]
    items: Vec<UserDto>,
    #[serde(rename = "TotalRecordCount", alias = "totalRecordCount", default)]
    total_record_count: usize,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn client_authorization(connection_id: &str, user_id: Option<&str>) -> String {
    let connection_id = connection_id.replace(['"', '\\'], "");
    let user = user_id
        .map(|id| format!(" UserId=\"{}\",", id.replace(['"', '\\'], "")))
        .unwrap_or_default();
    format!(
        "Emby{user} Client=\"{SCRYER_PRODUCT}\", Device=\"{SCRYER_PRODUCT}\", DeviceId=\"scryer-{connection_id}\", Version=\"{SCRYER_VERSION}\""
    )
}

fn normalized_candidate(value: &str) -> AppResult<Url> {
    let value = value.trim().trim_end_matches('/');
    let mut url =
        Url::parse(value).map_err(|_| AppError::Validation("Emby base URL is invalid".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::Validation(
            "Emby base URL must be an absolute HTTP(S) URL without credentials, query, or fragment"
                .into(),
        ));
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

fn endpoint(base: &Url, path: &str) -> AppResult<Url> {
    base.join(path.trim_start_matches('/'))
        .map_err(|_| AppError::Validation("Emby endpoint URL is invalid".into()))
}

async fn limited_bytes(response: Response, limit: usize, operation: &str) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(AppError::Repository(format!(
            "Emby {operation} response was too large"
        )));
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .map(|length| length as usize)
            .unwrap_or_default()
            .min(limit),
    );
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| AppError::Repository(format!("invalid Emby {operation} response")))?;
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(AppError::Repository(format!(
                "Emby {operation} response was too large"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn json_response<T: for<'de> Deserialize<'de>>(
    response: Response,
    operation: &str,
) -> AppResult<T> {
    let bytes = limited_bytes(response, JSON_LIMIT, operation).await?;
    serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Repository(format!("invalid Emby {operation} response")))
}

fn identity_from_public(
    api_base_url: &Url,
    info: PublicSystemInfo,
) -> AppResult<EmbyServerIdentity> {
    let server_id = non_empty(info.id)
        .ok_or_else(|| AppError::Validation("the supplied URL is not an Emby server".into()))?;
    let server_name = non_empty(info.server_name)
        .ok_or_else(|| AppError::Validation("the supplied URL is not an Emby server".into()))?;
    let version = non_empty(info.version)
        .ok_or_else(|| AppError::Validation("the supplied URL is not an Emby server".into()))?;
    Ok(EmbyServerIdentity {
        api_base_url: api_base_url.as_str().trim_end_matches('/').to_string(),
        server_id,
        server_name,
        version,
    })
}

async fn probe_public(client: &Client, base: &Url) -> AppResult<Option<EmbyServerIdentity>> {
    let response = client
        .get(endpoint(base, "System/Info/Public")?)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to reach Emby server".into()))?;
    if response.status() == StatusCode::NOT_FOUND || !response.status().is_success() {
        return Ok(None);
    }
    match json_response::<PublicSystemInfo>(response, "public system info").await {
        Ok(info) => identity_from_public(base, info).map(Some),
        Err(_) => Ok(None),
    }
}

pub(super) async fn resolve_api_base(
    client: &Client,
    _connection_id: &str,
    base_url: &str,
) -> AppResult<EmbyServerIdentity> {
    let base = normalized_candidate(base_url)?;
    if let Some(identity) = probe_public(client, &base).await? {
        return Ok(identity);
    }
    let trimmed_path = base.path().trim_end_matches('/');
    if trimmed_path
        .rsplit('/')
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case("emby"))
    {
        return Err(AppError::Validation(
            "the supplied URL is not an Emby server".into(),
        ));
    }
    let fallback = endpoint(&base, "emby/")?;
    probe_public(client, &fallback)
        .await?
        .ok_or_else(|| AppError::Validation("the supplied URL is not an Emby server".into()))
}

async fn authenticated_system_info(
    client: &Client,
    connection_id: &str,
    base: &Url,
    token: &str,
    user_id: Option<&str>,
) -> AppResult<EmbyServerIdentity> {
    let mut request = client
        .get(endpoint(base, "System/Info")?)
        .header("Accept", "application/json")
        .header("X-Emby-Token", token);
    if let Some(user_id) = user_id {
        request = request.header(
            "X-Emby-Authorization",
            client_authorization(connection_id, Some(user_id)),
        );
    }
    let response = request
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to reach Emby server".into()))?;
    match response.status() {
        status if status.is_success() => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(AppError::Unauthorized("Emby credential is invalid".into()));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby system info failed with status {status}"
            )));
        }
    }
    let info: SystemInfo = json_response(response, "system info").await?;
    let identity = EmbyServerIdentity {
        api_base_url: base.as_str().trim_end_matches('/').to_string(),
        server_id: non_empty(info.id)
            .ok_or_else(|| AppError::Repository("Emby system info omitted Id".into()))?,
        server_name: non_empty(info.server_name)
            .ok_or_else(|| AppError::Repository("Emby system info omitted ServerName".into()))?,
        version: non_empty(info.version)
            .ok_or_else(|| AppError::Repository("Emby system info omitted Version".into()))?,
    };
    Ok(identity)
}

fn require_server_id(identity: &EmbyServerIdentity, expected: Option<&str>) -> AppResult<()> {
    if expected
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some_and(|expected| expected != identity.server_id)
    {
        return Err(AppError::Validation(
            "Emby server identity does not match the saved connection".into(),
        ));
    }
    Ok(())
}

pub(super) async fn test_api_key(
    client: &Client,
    connection_id: &str,
    base_url: &str,
    api_key: &str,
    expected_server_id: Option<&str>,
) -> AppResult<EmbyServerIdentity> {
    let public = resolve_api_base(client, connection_id, base_url).await?;
    require_server_id(&public, expected_server_id)?;
    let base = normalized_candidate(&public.api_base_url)?;
    let authenticated =
        authenticated_system_info(client, connection_id, &base, api_key.trim(), None).await?;
    require_server_id(&authenticated, Some(&public.server_id))?;
    let mut users = endpoint(&base, "Users/Query")?;
    users
        .query_pairs_mut()
        .append_pair("IsDisabled", "false")
        .append_pair("StartIndex", "0")
        .append_pair("Limit", "1");
    let response = client
        .get(users)
        .header("Accept", "application/json")
        .header("X-Emby-Token", api_key.trim())
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to validate Emby user-list access".into()))?;
    match response.status() {
        status if status.is_success() => Ok(authenticated),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::Unauthorized(
            "Emby API key is invalid or cannot list users".into(),
        )),
        status => Err(AppError::Repository(format!(
            "Emby user-list validation failed with status {status}"
        ))),
    }
}

async fn authenticate_local(
    client: &Client,
    connection_id: &str,
    base: &Url,
    username: &str,
    password: &str,
) -> AppResult<AuthResponse> {
    let response = client
        .post(endpoint(base, "Users/AuthenticateByName")?)
        .header("Accept", "application/json")
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, None),
        )
        .json(&AuthRequest { username, password })
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to reach Emby server".into()))?;
    match response.status() {
        StatusCode::OK => {}
        StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(AppError::Unauthorized("invalid Emby credentials".into()));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby authentication failed with status {status}"
            )));
        }
    }
    let auth: AuthResponse = json_response(response, "authentication").await?;
    if auth.user.id.trim().is_empty()
        || auth
            .user
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .is_empty()
        || non_empty(auth.access_token.clone()).is_none()
        || non_empty(auth.server_id.clone()).is_none()
    {
        return Err(AppError::Repository(
            "Emby authentication response omitted required fields".into(),
        ));
    }
    if auth
        .user
        .policy
        .as_ref()
        .is_some_and(|policy| policy.is_disabled)
    {
        return Err(AppError::Unauthorized("Emby account is disabled".into()));
    }
    Ok(auth)
}

async fn logout(client: &Client, connection_id: &str, base: &Url, user_id: &str, token: &str) {
    if let Err(error) = client
        .post(endpoint(base, "Sessions/Logout").expect("static Emby logout path"))
        .header("Accept", "application/json")
        .header("X-Emby-Token", token)
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, Some(user_id)),
        )
        .send()
        .await
    {
        let error_class = if error.is_timeout() {
            "timeout"
        } else if error.is_connect() {
            "connect"
        } else {
            "transport"
        };
        tracing::warn!(
            connection_id,
            operation = "emby_logout",
            error_class,
            "Emby logout failed"
        );
    }
}

async fn delete_api_key(
    client: &Client,
    connection_id: &str,
    base: &Url,
    user_id: &str,
    admin_token: &str,
    api_key: &str,
) -> AppResult<()> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| AppError::Validation("Emby base URL cannot contain path segments".into()))?
        .push("Auth")
        .push("Keys")
        .push(api_key);
    let response = client
        .delete(url)
        .header("Accept", "application/json")
        .header("X-Emby-Token", admin_token)
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, Some(user_id)),
        )
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to compensate Emby API key".into()))?;
    if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(AppError::Repository(format!(
            "Emby API key compensation failed with status {}",
            response.status()
        )))
    }
}

pub(super) async fn finish_api_key_exchange(
    client: &Client,
    connection_id: &str,
    cleanup: EmbyApiKeyExchangeCleanup,
    compensate_created_key: bool,
) {
    let Ok(base) = normalized_candidate(cleanup.api_base_url()) else {
        tracing::warn!(
            connection_id,
            operation = "emby_exchange_cleanup",
            error_class = "invalid_internal_url",
            "Emby API-key exchange cleanup failed"
        );
        return;
    };
    if compensate_created_key
        && let Some(api_key) = cleanup.created_api_key()
        && delete_api_key(
            client,
            connection_id,
            &base,
            cleanup.local_user_id(),
            cleanup.session_access_token(),
            api_key,
        )
        .await
        .is_err()
    {
        tracing::warn!(
            connection_id,
            operation = "emby_key_compensation",
            error_class = "upstream_failure",
            "Emby API-key compensation failed"
        );
    }
    logout(
        client,
        connection_id,
        &base,
        cleanup.local_user_id(),
        cleanup.session_access_token(),
    )
    .await;
}

async fn get_user(
    client: &Client,
    connection_id: &str,
    base: &Url,
    user_id: &str,
    token: &str,
) -> AppResult<UserDto> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| AppError::Validation("Emby base URL cannot contain path segments".into()))?
        .push("Users")
        .push(user_id);
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Emby-Token", token)
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, Some(user_id)),
        )
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to verify Emby user".into()))?;
    match response.status() {
        status if status.is_success() => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err(AppError::Unauthorized(
                "Emby user verification failed".into(),
            ));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby user verification failed with status {status}"
            )));
        }
    }
    let user: UserDto = json_response(response, "user").await?;
    if user.id != user_id
        || user
            .policy
            .as_ref()
            .is_some_and(|policy| policy.is_disabled)
    {
        return Err(AppError::Unauthorized(
            "Emby user verification failed".into(),
        ));
    }
    Ok(user)
}

async fn list_keys(
    client: &Client,
    connection_id: &str,
    base: &Url,
    user_id: &str,
    token: &str,
) -> AppResult<Vec<KeyInfo>> {
    let mut start = 0usize;
    let mut keys = Vec::new();
    let mut seen_pages = HashSet::new();
    loop {
        let mut url = endpoint(base, "Auth/Keys")?;
        url.query_pairs_mut()
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", &PAGE_LIMIT.to_string());
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", token)
            .header(
                "X-Emby-Authorization",
                client_authorization(connection_id, Some(user_id)),
            )
            .send()
            .await
            .map_err(|_| AppError::Repository("failed to list Emby API keys".into()))?;
        match response.status() {
            status if status.is_success() => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Emby administrator cannot list API keys".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Emby API key listing failed with status {status}"
                )));
            }
        }
        let page: KeyQueryResult = json_response(response, "API key list").await?;
        let fingerprint = page
            .items
            .iter()
            .filter_map(|key| key.access_token.as_deref())
            .collect::<Vec<_>>()
            .join("\0");
        if !seen_pages.insert(fingerprint) && !page.items.is_empty() {
            return Err(AppError::Repository("Emby repeated an API key page".into()));
        }
        let count = page.items.len();
        keys.extend(page.items);
        start += count;
        if count == 0 || start >= page.total_record_count || start >= RECORD_LIMIT {
            break;
        }
    }
    Ok(keys)
}

async fn exchange_admin_session(
    client: &Client,
    connection_id: &str,
    base: &Url,
    auth: &AuthResponse,
) -> AppResult<EmbyApiKeyExchange> {
    if !auth
        .user
        .policy
        .as_ref()
        .is_some_and(|policy| policy.is_administrator)
    {
        return Err(AppError::Unauthorized(
            "Emby account must be a local administrator".into(),
        ));
    }
    let token = non_empty(auth.access_token.clone())
        .ok_or_else(|| AppError::Repository("Emby did not return an access token".into()))?;
    let server_id = non_empty(auth.server_id.clone())
        .ok_or_else(|| AppError::Repository("Emby did not return a server id".into()))?;
    let identity =
        authenticated_system_info(client, connection_id, base, &token, Some(&auth.user.id)).await?;
    require_server_id(&identity, Some(&server_id))?;
    let mut keys = list_keys(client, connection_id, base, &auth.user.id, &token).await?;
    keys.sort_by(|left, right| right.date_created.cmp(&left.date_created));
    for key in keys.iter().filter(|key| {
        key.app_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(SCRYER_PRODUCT))
    }) {
        if let Some(candidate) = non_empty(key.access_token.clone())
            && test_api_key(
                client,
                connection_id,
                identity.api_base_url.as_str(),
                &candidate,
                Some(&server_id),
            )
            .await
            .is_ok()
        {
            return Ok(EmbyApiKeyExchange {
                api_key: candidate,
                server_identity: identity,
                created_new_key: false,
                cleanup: Some(EmbyApiKeyExchangeCleanup::new(
                    base.as_str().trim_end_matches('/').to_string(),
                    auth.user.id.clone(),
                    token.clone(),
                    None,
                )),
            });
        }
    }
    let before = keys
        .into_iter()
        .filter_map(|key| non_empty(key.access_token))
        .collect::<HashSet<_>>();
    let mut create_url = endpoint(base, "Auth/Keys")?;
    create_url
        .query_pairs_mut()
        .append_pair("App", SCRYER_PRODUCT);
    let response = client
        .post(create_url)
        .header("Accept", "application/json")
        .header("X-Emby-Token", &token)
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, Some(&auth.user.id)),
        )
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to create Emby API key".into()))?;
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "Emby API key creation failed with status {}",
            response.status()
        )));
    }
    let mut keys = list_keys(client, connection_id, base, &auth.user.id, &token).await?;
    keys.sort_by(|left, right| right.date_created.cmp(&left.date_created));
    let api_key = keys
        .into_iter()
        .filter(|key| {
            key.app_name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(SCRYER_PRODUCT))
        })
        .filter_map(|key| non_empty(key.access_token))
        .find(|key| !before.contains(key))
        .ok_or_else(|| {
            AppError::Repository(
                "Emby created an API key but did not expose it through Auth/Keys".into(),
            )
        })?;
    if let Err(error) = test_api_key(
        client,
        connection_id,
        identity.api_base_url.as_str(),
        &api_key,
        Some(&server_id),
    )
    .await
    {
        let cleanup = EmbyApiKeyExchangeCleanup::new(
            base.as_str().trim_end_matches('/').to_string(),
            auth.user.id.clone(),
            token.clone(),
            Some(api_key),
        );
        finish_api_key_exchange(client, connection_id, cleanup, true).await;
        return Err(error);
    }
    Ok(EmbyApiKeyExchange {
        api_key: api_key.clone(),
        server_identity: identity,
        created_new_key: true,
        cleanup: Some(EmbyApiKeyExchangeCleanup::new(
            base.as_str().trim_end_matches('/').to_string(),
            auth.user.id.clone(),
            token,
            Some(api_key),
        )),
    })
}

pub(super) async fn exchange_local_admin_api_key(
    client: &Client,
    connection_id: &str,
    base_url: &str,
    username: &str,
    password: &str,
) -> AppResult<EmbyApiKeyExchange> {
    let identity = resolve_api_base(client, connection_id, base_url).await?;
    let base = normalized_candidate(&identity.api_base_url)?;
    let auth = authenticate_local(client, connection_id, &base, username.trim(), password).await?;
    let result = async {
        require_server_id(&identity, auth.server_id.as_deref())?;
        exchange_admin_session(client, connection_id, &base, &auth).await
    }
    .await;
    if result.is_err()
        && let Some(token) = non_empty(auth.access_token.clone())
    {
        logout(client, connection_id, &base, &auth.user.id, &token).await;
    }
    result
}

async fn authenticate_connect(
    client: &Client,
    connect_base: &Url,
    username_or_email: &str,
    password: &str,
) -> AppResult<ConnectSession> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("nameOrEmail", username_or_email)
        .append_pair("rawpw", password)
        .finish();
    let response = client
        .post(endpoint(connect_base, "user/authenticate")?)
        .header("Accept", "application/json")
        .header(
            "X-Application",
            format!("{SCRYER_PRODUCT}/{SCRYER_VERSION}"),
        )
        .header(
            "Content-Type",
            "application/x-www-form-urlencoded; charset=UTF-8",
        )
        .body(body)
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to reach Emby Connect".into()))?;
    match response.status() {
        status if status.is_success() => {}
        StatusCode::BAD_REQUEST
        | StatusCode::UNAUTHORIZED
        | StatusCode::FORBIDDEN
        | StatusCode::NOT_FOUND => {
            return Err(AppError::Unauthorized(
                "invalid Emby Connect credentials".into(),
            ));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby Connect authentication failed with status {status}"
            )));
        }
    }
    let auth: ConnectAuthResponse = json_response(response, "Connect authentication").await?;
    let user_id = non_empty(auth.connect_user_id)
        .or_else(|| auth.user.and_then(|user| non_empty(user.id)))
        .ok_or_else(|| AppError::Repository("Emby Connect response omitted user id".into()))?;
    let access_token = non_empty(auth.access_token)
        .ok_or_else(|| AppError::Repository("Emby Connect response omitted access token".into()))?;
    Ok(ConnectSession {
        user_id,
        access_token,
    })
}

async fn connect_servers(
    client: &Client,
    connect_base: &Url,
    session: &ConnectSession,
) -> AppResult<Vec<ConnectServerSecret>> {
    let mut url = endpoint(connect_base, "servers")?;
    url.query_pairs_mut()
        .append_pair("userId", &session.user_id);
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header(
            "X-Application",
            format!("{SCRYER_PRODUCT}/{SCRYER_VERSION}"),
        )
        .header("X-Connect-UserToken", &session.access_token)
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to discover Emby Connect servers".into()))?;
    match response.status() {
        status if status.is_success() => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(AppError::Unauthorized(
                "invalid Emby Connect credentials".into(),
            ));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby Connect server discovery failed with status {status}"
            )));
        }
    }
    let entries: Vec<ConnectServerWire> = json_response(response, "Connect server list").await?;
    let mut dedup = HashMap::new();
    for entry in entries {
        let Some(system_id) = non_empty(entry.system_id) else {
            continue;
        };
        let Some(access_key) = non_empty(entry.access_key) else {
            continue;
        };
        let Some(name) = non_empty(entry.name) else {
            continue;
        };
        let user_type = match entry
            .user_type
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "linkeduser" | "linked_user" => EmbyConnectUserType::LinkedUser,
            "guest" => EmbyConnectUserType::Guest,
            _ => EmbyConnectUserType::Unknown,
        };
        dedup
            .entry(system_id.clone())
            .or_insert(ConnectServerSecret {
                system_id,
                access_key,
                name,
                user_type,
                local_address: non_empty(entry.local_address),
                remote_address: non_empty(entry.url),
            });
    }
    Ok(dedup.into_values().collect())
}

async fn probe_connect_address(
    client: &Client,
    connection_id: &str,
    address: Option<&str>,
    expected_id: &str,
) -> (Option<String>, EmbyConnectAddressStatus) {
    let Some(address) = address else {
        return (None, EmbyConnectAddressStatus::InvalidUrl);
    };
    if normalized_candidate(address).is_err() {
        return (None, EmbyConnectAddressStatus::InvalidUrl);
    }
    match resolve_api_base(client, connection_id, address).await {
        Ok(identity) if identity.server_id == expected_id => (
            Some(identity.api_base_url),
            EmbyConnectAddressStatus::Reachable,
        ),
        Ok(_) => (None, EmbyConnectAddressStatus::ServerIdMismatch),
        Err(AppError::Validation(_)) => (None, EmbyConnectAddressStatus::InvalidUrl),
        Err(_) => (None, EmbyConnectAddressStatus::Unreachable),
    }
}

pub(super) async fn discover_connect_servers(
    client: &Client,
    connect_base: &Url,
    username_or_email: &str,
    password: &str,
) -> AppResult<Vec<EmbyConnectServer>> {
    let session =
        authenticate_connect(client, connect_base, username_or_email.trim(), password).await?;
    let mut servers = Vec::new();
    for secret in connect_servers(client, connect_base, &session).await? {
        let (local_api_base_url, local_status) = probe_connect_address(
            client,
            "connect-discovery",
            secret.local_address.as_deref(),
            &secret.system_id,
        )
        .await;
        let (remote_api_base_url, remote_status) = probe_connect_address(
            client,
            "connect-discovery",
            secret.remote_address.as_deref(),
            &secret.system_id,
        )
        .await;
        let suggested_base_url = local_api_base_url
            .clone()
            .or_else(|| remote_api_base_url.clone());
        servers.push(EmbyConnectServer {
            server_id: secret.system_id,
            name: secret.name,
            user_type: secret.user_type,
            local_address: secret.local_address,
            remote_address: secret.remote_address,
            local_api_base_url,
            remote_api_base_url,
            local_status,
            remote_status,
            suggested_base_url,
        });
    }
    servers.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.server_id.cmp(&b.server_id))
    });
    Ok(servers)
}

async fn exchange_connect_session(
    client: &Client,
    connection_id: &str,
    base: &Url,
    connect_user_id: &str,
    access_key: &str,
) -> AppResult<AuthResponse> {
    let mut url = endpoint(base, "Connect/Exchange")?;
    url.query_pairs_mut()
        .append_pair("format", "json")
        .append_pair("ConnectUserId", connect_user_id);
    let response = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Emby-Token", access_key)
        .header(
            "X-Emby-Authorization",
            client_authorization(connection_id, None),
        )
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to exchange Emby Connect session".into()))?;
    match response.status() {
        status if status.is_success() => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
            return Err(AppError::Unauthorized("Emby Connect sign-in failed".into()));
        }
        status => {
            return Err(AppError::Repository(format!(
                "Emby Connect exchange failed with status {status}"
            )));
        }
    }
    let exchange: ConnectExchangeResponse = json_response(response, "Connect exchange").await?;
    let user_id = non_empty(exchange.local_user_id)
        .ok_or_else(|| AppError::Repository("Emby Connect exchange omitted LocalUserId".into()))?;
    let token = non_empty(exchange.access_token)
        .ok_or_else(|| AppError::Repository("Emby Connect exchange omitted AccessToken".into()))?;
    let user = get_user(client, connection_id, base, &user_id, &token).await?;
    let info =
        authenticated_system_info(client, connection_id, base, &token, Some(&user_id)).await?;
    Ok(AuthResponse {
        user,
        access_token: Some(token),
        server_id: Some(info.server_id),
    })
}

async fn connect_auth_for_server(
    client: &Client,
    connect_base: &Url,
    connection_id: &str,
    base_url: &str,
    expected_server_id: &str,
    username_or_email: &str,
    password: &str,
) -> AppResult<(Url, AuthResponse)> {
    let session =
        authenticate_connect(client, connect_base, username_or_email.trim(), password).await?;
    let secret = connect_servers(client, connect_base, &session)
        .await?
        .into_iter()
        .find(|server| server.system_id == expected_server_id)
        .ok_or_else(|| {
            AppError::Unauthorized(
                "Emby Connect account cannot access the configured server".into(),
            )
        })?;
    let mut candidates = vec![base_url.to_string()];
    if let Some(local) = secret.local_address.clone() {
        candidates.push(local);
    }
    if let Some(remote) = secret.remote_address.clone() {
        candidates.push(remote);
    }
    let mut last_error = None;
    for candidate in candidates {
        match resolve_api_base(client, connection_id, &candidate).await {
            Ok(identity) if identity.server_id == expected_server_id => {
                let base = normalized_candidate(&identity.api_base_url)?;
                match exchange_connect_session(
                    client,
                    connection_id,
                    &base,
                    &session.user_id,
                    &secret.access_key,
                )
                .await
                {
                    Ok(auth) => {
                        require_server_id(&identity, auth.server_id.as_deref())?;
                        return Ok((base, auth));
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            Ok(_) => {
                last_error = Some(AppError::Validation(
                    "Emby server identity does not match the saved connection".into(),
                ))
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| AppError::Repository("no reachable Emby Connect address".into())))
}

pub(super) async fn exchange_connect_admin_api_key(
    client: &Client,
    connect_base: &Url,
    connection_id: &str,
    base_url: &str,
    server_id: &str,
    username_or_email: &str,
    password: &str,
) -> AppResult<EmbyApiKeyExchange> {
    let (base, auth) = connect_auth_for_server(
        client,
        connect_base,
        connection_id,
        base_url,
        server_id,
        username_or_email,
        password,
    )
    .await?;
    let result = exchange_admin_session(client, connection_id, &base, &auth).await;
    if result.is_err()
        && let Some(token) = non_empty(auth.access_token.clone())
    {
        logout(client, connection_id, &base, &auth.user.id, &token).await;
    }
    result
}

fn avatar_proxy_url(connection_id: &str, user_id: &str, tag: Option<&str>) -> Option<String> {
    let tag = tag.map(str::trim).filter(|value| !value.is_empty())?;
    let encode =
        |value: &str| url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    Some(format!(
        "/api/media-server-avatars/{}/{}/{}",
        encode(connection_id),
        encode(user_id),
        encode(tag)
    ))
}

fn verified_identity(connection_id: &str, user: &UserDto) -> VerifiedExternalIdentity {
    let username = user
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| user.id.clone());
    let remote_password_configured = match (user.has_password, user.has_configured_password) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(true) && b.unwrap_or(true)),
    };
    VerifiedExternalIdentity {
        provider: ExternalAccountProvider::Emby,
        connection_id: connection_id.to_string(),
        external_user_id: user.id.clone(),
        username: username.clone(),
        display_name: Some(username),
        avatar_url: avatar_proxy_url(connection_id, &user.id, user.primary_image_tag.as_deref()),
        remote_password_configured,
    }
}

pub(super) async fn verify_local_identity(
    client: &Client,
    connection_id: &str,
    base_url: &str,
    expected_server_id: &str,
    username: &str,
    password: &str,
) -> AppResult<VerifiedExternalIdentity> {
    let identity = resolve_api_base(client, connection_id, base_url).await?;
    require_server_id(&identity, Some(expected_server_id))?;
    let base = normalized_candidate(&identity.api_base_url)?;
    let auth = authenticate_local(client, connection_id, &base, username.trim(), password).await?;
    let token = non_empty(auth.access_token.clone()).ok_or_else(|| {
        AppError::Repository("Emby authentication response omitted AccessToken".into())
    })?;
    let result = async {
        require_server_id(&identity, auth.server_id.as_deref())?;
        let authenticated =
            authenticated_system_info(client, connection_id, &base, &token, Some(&auth.user.id))
                .await?;
        require_server_id(&authenticated, Some(expected_server_id))?;
        let user = get_user(client, connection_id, &base, &auth.user.id, &token).await?;
        Ok(verified_identity(connection_id, &user))
    }
    .await;
    logout(client, connection_id, &base, &auth.user.id, &token).await;
    result
}

pub(super) async fn verify_connect_identity(
    client: &Client,
    connect_base: &Url,
    connection_id: &str,
    base_url: &str,
    expected_server_id: &str,
    username_or_email: &str,
    password: &str,
) -> AppResult<EmbyConnectIdentityVerification> {
    let (base, auth) = connect_auth_for_server(
        client,
        connect_base,
        connection_id,
        base_url,
        expected_server_id,
        username_or_email,
        password,
    )
    .await?;
    let token = non_empty(auth.access_token.clone())
        .ok_or_else(|| AppError::Repository("Emby Connect exchange omitted AccessToken".into()))?;
    let result = Ok(EmbyConnectIdentityVerification {
        identity: verified_identity(connection_id, &auth.user),
        resolved_api_base_url: base.as_str().trim_end_matches('/').to_string(),
    });
    logout(client, connection_id, &base, &auth.user.id, &token).await;
    result
}

pub(super) async fn list_users(
    client: &Client,
    connection_id: &str,
    base_url: &str,
    api_key: &str,
    search: Option<&str>,
) -> AppResult<Vec<EmbyServerUser>> {
    let base = normalized_candidate(base_url)?;
    let mut start = 0usize;
    let mut users = Vec::new();
    let mut seen_pages = HashSet::new();
    loop {
        let mut url = endpoint(&base, "Users/Query")?;
        url.query_pairs_mut()
            .append_pair("IsDisabled", "false")
            .append_pair("StartIndex", &start.to_string())
            .append_pair("Limit", &PAGE_LIMIT.to_string())
            .append_pair("SortOrder", "Ascending");
        let response = client
            .get(url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key.trim())
            .send()
            .await
            .map_err(|_| AppError::Repository("failed to list Emby users".into()))?;
        match response.status() {
            status if status.is_success() => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Emby API key is invalid or cannot list users".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Emby user listing failed with status {status}"
                )));
            }
        }
        let page: UserQueryResult = json_response(response, "user list").await?;
        let fingerprint = page
            .items
            .iter()
            .map(|user| user.id.as_str())
            .collect::<Vec<_>>()
            .join("\0");
        if !seen_pages.insert(fingerprint) && !page.items.is_empty() {
            return Err(AppError::Repository(
                "Emby repeated a user-list page".into(),
            ));
        }
        let count = page.items.len();
        users.extend(page.items.into_iter().filter(|user| {
            !user
                .policy
                .as_ref()
                .is_some_and(|policy| policy.is_disabled)
        }));
        start += count;
        if count == 0 || start >= page.total_record_count || start >= RECORD_LIMIT {
            break;
        }
    }
    let search = search
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let mut users = users
        .into_iter()
        .filter(|user| {
            search.as_ref().is_none_or(|search| {
                [
                    Some(user.id.as_str()),
                    user.name.as_deref(),
                    user.connect_user_name.as_deref(),
                ]
                .into_iter()
                .flatten()
                .any(|value| value.to_ascii_lowercase().contains(search))
            })
        })
        .map(|user| {
            let username = user
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| user.id.clone());
            EmbyServerUser {
                id: user.id.clone(),
                username: username.clone(),
                display_name: Some(username),
                avatar_url: avatar_proxy_url(
                    connection_id,
                    &user.id,
                    user.primary_image_tag.as_deref(),
                ),
            }
        })
        .collect::<Vec<_>>();
    users.sort_by(|a, b| {
        a.username
            .to_ascii_lowercase()
            .cmp(&b.username.to_ascii_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(users)
}

pub(super) async fn fetch_avatar(
    client: &Client,
    base_url: &str,
    api_key: &str,
    user_id: &str,
    image_tag: &str,
) -> AppResult<Option<EmbyAvatar>> {
    if user_id.is_empty() || image_tag.is_empty() || user_id.len() > 256 || image_tag.len() > 256 {
        return Err(AppError::Validation(
            "invalid Emby avatar identifier".into(),
        ));
    }
    let base = normalized_candidate(base_url)?;
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| AppError::Validation("Emby base URL cannot contain path segments".into()))?
        .push("Items")
        .push(user_id)
        .push("Images")
        .push("Primary");
    url.query_pairs_mut()
        .append_pair("Tag", image_tag)
        .append_pair("MaxWidth", "256")
        .append_pair("MaxHeight", "256")
        .append_pair("Quality", "90");
    let response = client
        .get(url)
        .header("X-Emby-Token", api_key.trim())
        .send()
        .await
        .map_err(|_| AppError::Repository("failed to fetch Emby avatar".into()))?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AppError::Repository(format!(
            "Emby avatar request failed with status {}",
            response.status()
        )));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.to_ascii_lowercase().starts_with("image/"))
        .map(str::to_string)
        .ok_or_else(|| AppError::Repository("Emby avatar response was not an image".into()))?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let bytes = limited_bytes(response, AVATAR_LIMIT, "avatar").await?;
    Ok(Some(EmbyAvatar {
        content_type,
        bytes,
        etag,
        last_modified,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_client() -> Client {
        scryer_outbound_http::install_default_rustls_provider();
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test HTTP client")
    }

    async fn mount_public_info(server: &MockServer, server_id: &str) {
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Id": server_id,
                "ServerName": "Test Emby",
                "Version": "4.9.5.0"
            })))
            .mount(server)
            .await;
    }

    #[test]
    fn configured_url_preserves_reverse_proxy_path() {
        let url = normalized_candidate(" https://media.example.test/reverse/emby/ ")
            .expect("valid Emby URL");
        assert_eq!(url.as_str(), "https://media.example.test/reverse/emby/");
        assert_eq!(
            endpoint(&url, "System/Info/Public")
                .expect("system info URL")
                .as_str(),
            "https://media.example.test/reverse/emby/System/Info/Public"
        );
    }

    #[test]
    fn configured_url_rejects_credentials_query_and_fragment() {
        for value in [
            "https://user:pass@media.example.test",
            "https://media.example.test?token=secret",
            "https://media.example.test/#fragment",
            "file:///tmp/emby",
            "media.example.test",
        ] {
            assert!(normalized_candidate(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn authenticated_client_header_has_stable_device_identity() {
        let header = client_authorization("connection 1", Some("local-user"));
        assert!(header.contains("Emby UserId=\"local-user\""));
        assert!(header.contains("Client=\"Scryer\""));
        assert!(header.contains("DeviceId=\"scryer-connection 1\""));
        assert!(header.contains(&format!("Version=\"{SCRYER_VERSION}\"")));
        assert!(!header.contains("Token"));
    }

    #[test]
    fn avatar_proxy_url_does_not_contain_upstream_credentials() {
        let url =
            avatar_proxy_url("connection id", "user/id", Some("tag value")).expect("avatar URL");
        assert_eq!(
            url,
            "/api/media-server-avatars/connection+id/user%2Fid/tag+value"
        );
        assert!(!url.contains("api_key"));
        assert!(!url.contains("token"));
    }

    #[test]
    fn api_key_exchange_debug_redacts_every_secret() {
        let exchange = EmbyApiKeyExchange {
            api_key: "static-secret".into(),
            server_identity: EmbyServerIdentity {
                api_base_url: "https://emby.example.test".into(),
                server_id: "server-id".into(),
                server_name: "Emby".into(),
                version: "4.9.5.0".into(),
            },
            created_new_key: true,
            cleanup: Some(EmbyApiKeyExchangeCleanup::new(
                "https://emby.example.test".into(),
                "local-user-secret".into(),
                "session-secret".into(),
                Some("created-secret".into()),
            )),
        };
        let debug = format!("{exchange:?}");
        for secret in [
            "static-secret",
            "local-user-secret",
            "session-secret",
            "created-secret",
        ] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn local_identity_uses_exact_auth_wire_and_logs_out() {
        let server = MockServer::start().await;
        mount_public_info(&server, "server-1").await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .and(header(
                "X-Emby-Authorization",
                client_authorization("connection-1", None),
            ))
            .and(body_json(serde_json::json!({
                "Username": "alice",
                "Pw": "correct horse"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "User": {"Id": "user-1", "Name": "Alice", "Policy": {"IsDisabled": false}},
                "AccessToken": "temporary-token",
                "ServerId": "server-1"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/System/Info"))
            .and(header("X-Emby-Token", "temporary-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Id": "server-1", "ServerName": "Test Emby", "Version": "4.9.5.0"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/Users/user-1"))
            .and(header("X-Emby-Token", "temporary-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Id": "user-1", "Name": "Alice", "HasPassword": true,
                "Policy": {"IsDisabled": false}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/Sessions/Logout"))
            .and(header("X-Emby-Token", "temporary-token"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let verified = verify_local_identity(
            &test_client(),
            "connection-1",
            &server.uri(),
            "server-1",
            "alice",
            "correct horse",
        )
        .await
        .expect("local identity");

        assert_eq!(verified.external_user_id, "user-1");
        assert_eq!(verified.remote_password_configured, Some(true));
        server.verify().await;
    }

    #[tokio::test]
    async fn connect_compat_shape_deduplicates_servers_and_uses_matching_candidate() {
        let connect = MockServer::start().await;
        let emby = MockServer::start().await;
        mount_public_info(&emby, "server-1").await;
        Mock::given(method("POST"))
            .and(path("/user/authenticate"))
            .and(header("X-Application", format!("Scryer/{SCRYER_VERSION}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ConnectAccessToken": "connect-token",
                "ConnectUserId": "connect-user"
            })))
            .mount(&connect)
            .await;
        Mock::given(method("GET"))
            .and(path("/servers"))
            .and(query_param("userId", "connect-user"))
            .and(header("X-Connect-UserToken", "connect-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"SystemId": "server-1", "AccessKey": "access-key", "Name": "Emby", "LocalAddress": emby.uri(), "UserType": "LinkedUser"},
                {"SystemId": "server-1", "AccessKey": "other-key", "Name": "Duplicate", "Url": "https://unused.invalid", "UserType": "Guest"}
            ])))
            .mount(&connect)
            .await;
        Mock::given(method("GET"))
            .and(path("/Connect/Exchange"))
            .and(query_param("ConnectUserId", "connect-user"))
            .and(header("X-Emby-Token", "access-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "LocalUserId": "local-user", "AccessToken": "local-token"
            })))
            .expect(1)
            .mount(&emby)
            .await;
        Mock::given(method("GET"))
            .and(path("/Users/local-user"))
            .and(header("X-Emby-Token", "local-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Id": "local-user", "Name": "Alice", "Policy": {"IsDisabled": false}
            })))
            .mount(&emby)
            .await;
        Mock::given(method("GET"))
            .and(path("/System/Info"))
            .and(header("X-Emby-Token", "local-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Id": "server-1", "ServerName": "Test Emby", "Version": "4.9.5.0"
            })))
            .mount(&emby)
            .await;
        Mock::given(method("POST"))
            .and(path("/Sessions/Logout"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&emby)
            .await;

        let verification = verify_connect_identity(
            &test_client(),
            &normalized_candidate(&connect.uri()).expect("Connect base"),
            "connection-1",
            "http://127.0.0.1:1",
            "server-1",
            "alice@example.test",
            "password",
        )
        .await
        .expect("Connect identity");

        assert_eq!(verification.identity.external_user_id, "local-user");
        assert_eq!(
            verification.resolved_api_base_url,
            emby.uri().trim_end_matches('/')
        );
        emby.verify().await;
    }

    #[tokio::test]
    async fn avatar_rejects_oversized_content_length_without_buffering_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Items/user/Images/Primary"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/png")
                    .insert_header("Content-Length", (AVATAR_LIMIT + 1).to_string()),
            )
            .mount(&server)
            .await;

        let error = fetch_avatar(&test_client(), &server.uri(), "api-key", "user", "tag")
            .await
            .expect_err("oversized avatar must fail");

        assert!(error.to_string().contains("too large"));
    }
}
