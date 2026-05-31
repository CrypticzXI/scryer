use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use reqwest::StatusCode;
use scryer_application::{
    AppError, AppResult, ExternalIdentityVerifier, JellyfinServerUser, PlexServerDiscovery,
    VerifiedExternalIdentity,
};
use scryer_domain::ExternalAccountProvider;
use scryer_outbound_http::generic_reqwest_client;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

const PLEX_BASE_URL: &str = "https://plex.tv";
const SCRYER_PRODUCT: &str = "Scryer";
const SCRYER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct HttpExternalIdentityVerifier {
    client: reqwest::Client,
    plex_base_url: Url,
}

impl HttpExternalIdentityVerifier {
    pub fn new() -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_base_url: Url::parse(PLEX_BASE_URL).expect("valid Plex base URL"),
        }
    }

    #[cfg(test)]
    fn with_plex_base_url(plex_base_url: Url) -> Self {
        Self {
            client: generic_reqwest_client(),
            plex_base_url,
        }
    }

    fn plex_url(&self, path: &str) -> AppResult<Url> {
        self.plex_base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| AppError::Repository(format!("invalid Plex endpoint URL: {error}")))
    }

    async fn find_jellyfin_api_key(
        &self,
        keys_url: &Url,
        admin_token: &str,
        app_name: &str,
    ) -> AppResult<Option<String>> {
        let response = self
            .client
            .get(keys_url.clone())
            .header("Accept", "application/json")
            .header("X-Emby-Token", admin_token)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to list Jellyfin API keys: {error}"))
            })?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Jellyfin admin token cannot list API keys".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin API key listing failed with status {status}"
                )));
            }
        }

        let keys = response
            .json::<JellyfinApiKeyQueryResult>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin API key list response: {error}"))
            })?;
        Ok(keys
            .items
            .into_iter()
            .filter(|key| {
                key.app_name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case(app_name))
            })
            .filter_map(|key| {
                let token = key.access_token?.trim().to_string();
                (!token.is_empty()).then_some((key.date_created.unwrap_or_default(), token))
            })
            .max_by(|left, right| left.0.cmp(&right.0))
            .map(|(_, token)| token))
    }
}

#[async_trait]
impl ExternalIdentityVerifier for HttpExternalIdentityVerifier {
    async fn verify_plex(
        &self,
        connection_id: &str,
        machine_id: Option<&str>,
        plex_auth_token: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let connection_id = connection_id.trim();
        let token = plex_auth_token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("Plex auth token is required".into()));
        }

        let account_response = self
            .client
            .get(self.plex_url("users/account.json")?)
            .header("Accept", "application/json")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| AppError::Repository(format!("failed to reach Plex: {error}")))?;

        match account_response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized("invalid Plex auth token".into()));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Plex account validation failed with status {status}"
                )));
            }
        }

        let account_json = account_response.json::<Value>().await.map_err(|error| {
            AppError::Repository(format!("invalid Plex account response: {error}"))
        })?;
        let user = account_json
            .get("user")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                AppError::Repository("Plex account response did not include a user".into())
            })?;
        let external_user_id = json_value_string(user.get("id"))
            .or_else(|| json_value_string(user.get("uuid")))
            .ok_or_else(|| {
                AppError::Repository("Plex account response did not include a user id".into())
            })?;
        let username = json_value_string(user.get("username"))
            .or_else(|| json_value_string(user.get("title")))
            .or_else(|| json_value_string(user.get("email")))
            .unwrap_or_else(|| external_user_id.clone());
        let display_name = json_value_string(user.get("title")).or_else(|| Some(username.clone()));
        let avatar_url = json_value_string(user.get("thumb"));

        if let Some(machine_id) = machine_id.map(str::trim).filter(|value| !value.is_empty()) {
            let resources_response = self
                .client
                .get(self.plex_url("api/resources?includeHttps=1")?)
                .header("Accept", "application/xml")
                .header("X-Plex-Token", token)
                .send()
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to reach Plex resources: {error}"))
                })?;
            match resources_response.status() {
                StatusCode::OK => {}
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                    return Err(AppError::Unauthorized("invalid Plex auth token".into()));
                }
                status => {
                    return Err(AppError::Repository(format!(
                        "Plex resources validation failed with status {status}"
                    )));
                }
            }
            let resources_xml = resources_response.text().await.map_err(|error| {
                AppError::Repository(format!("invalid Plex resources response: {error}"))
            })?;
            if !plex_resources_include_machine(&resources_xml, machine_id)? {
                return Err(AppError::Unauthorized(
                    "Plex account does not have access to the configured server".into(),
                ));
            }
        }

        Ok(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Plex,
            connection_id: connection_id.to_string(),
            external_user_id,
            username,
            display_name,
            avatar_url,
        })
    }

    async fn discover_plex_servers(
        &self,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>> {
        let token = plex_auth_token.trim();
        if token.is_empty() {
            return Err(AppError::Unauthorized("Plex auth token is required".into()));
        }
        let resources_response = self
            .client
            .get(self.plex_url("api/resources?includeHttps=1")?)
            .header("Accept", "application/xml")
            .header("X-Plex-Token", token)
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Plex resources: {error}"))
            })?;
        match resources_response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized("invalid Plex auth token".into()));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Plex resources discovery failed with status {status}"
                )));
            }
        }
        let resources_xml = resources_response.text().await.map_err(|error| {
            AppError::Repository(format!("invalid Plex resources response: {error}"))
        })?;
        plex_server_discoveries(&resources_xml)
    }

    async fn verify_jellyfin(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let base_url = jellyfin_base_url(base_url)?;
        let auth_url = base_url.join("Users/AuthenticateByName").map_err(|error| {
            AppError::Validation(format!("Jellyfin authentication URL is invalid: {error}"))
        })?;

        let response = self
            .client
            .post(auth_url)
            .header(
                "Authorization",
                jellyfin_authorization_header(connection_id),
            )
            .header("Accept", "application/json")
            .json(&JellyfinAuthRequest { username, password })
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "invalid Jellyfin credentials".into(),
                ));
            }
            StatusCode::BAD_REQUEST => {
                return Err(AppError::Validation(
                    "Jellyfin rejected the supplied credentials".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin authentication failed with status {status}"
                )));
            }
        }

        let auth = response
            .json::<JellyfinAuthResponse>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin authentication response: {error}"))
            })?;
        let remote_username = auth
            .user
            .name
            .unwrap_or_else(|| username.trim().to_string());
        let avatar_url = jellyfin_user_avatar_url(
            &base_url,
            &auth.user.id,
            auth.user.primary_image_tag.as_deref(),
        );

        Ok(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: connection_id.trim().to_string(),
            external_user_id: auth.user.id,
            username: remote_username.clone(),
            display_name: Some(remote_username),
            avatar_url,
        })
    }

    async fn test_jellyfin_connection(&self, base_url: &str) -> AppResult<()> {
        let base_url = jellyfin_base_url(base_url)?;
        let info_url = base_url.join("System/Info/Public").map_err(|error| {
            AppError::Validation(format!("Jellyfin system info URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(info_url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin connection test failed with status {}",
                response.status()
            )));
        }

        let info = response
            .json::<JellyfinPublicInfo>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin system info response: {error}"))
            })?;
        let product_name = info
            .product_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                AppError::Validation(
                    "the supplied URL did not identify itself as a Jellyfin server".into(),
                )
            })?;
        if !product_name.to_ascii_lowercase().contains("jellyfin") {
            return Err(AppError::Validation(
                "the supplied URL did not identify itself as a Jellyfin server".into(),
            ));
        }
        Ok(())
    }

    async fn test_jellyfin_api_key(&self, base_url: &str, api_key: &str) -> AppResult<()> {
        let base_url = jellyfin_base_url(base_url)?;
        let users_url = base_url.join("Users").map_err(|error| {
            AppError::Validation(format!("Jellyfin users URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(users_url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key.trim())
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to validate Jellyfin API key: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(AppError::Unauthorized(
                "Jellyfin API key is invalid or does not have user-list access".into(),
            )),
            status => Err(AppError::Repository(format!(
                "Jellyfin API key validation failed with status {status}"
            ))),
        }
    }

    async fn exchange_jellyfin_admin_api_key(
        &self,
        connection_id: &str,
        base_url: &str,
        username: &str,
        password: &str,
    ) -> AppResult<String> {
        let base_url = jellyfin_base_url(base_url)?;
        let auth_url = base_url.join("Users/AuthenticateByName").map_err(|error| {
            AppError::Validation(format!("Jellyfin authentication URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .post(auth_url)
            .header(
                "Authorization",
                jellyfin_authorization_header(connection_id),
            )
            .header("Accept", "application/json")
            .json(&JellyfinAuthRequest { username, password })
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to reach Jellyfin connection: {error}"))
            })?;

        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "invalid Jellyfin admin credentials".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin admin authentication failed with status {status}"
                )));
            }
        }

        let auth = response
            .json::<JellyfinAuthResponse>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin authentication response: {error}"))
            })?;
        if !auth
            .user
            .policy
            .as_ref()
            .is_some_and(|policy| policy.is_administrator)
        {
            return Err(AppError::Unauthorized(
                "Jellyfin account must be an administrator to create a Scryer API key".into(),
            ));
        }
        let Some(token) = auth
            .access_token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err(AppError::Repository(
                "Jellyfin did not return an admin access token; paste an API key manually".into(),
            ));
        };

        let keys_url = base_url.join("Auth/Keys").map_err(|error| {
            AppError::Validation(format!("Jellyfin API key URL is invalid: {error}"))
        })?;
        if let Some(existing) = self
            .find_jellyfin_api_key(&keys_url, token, SCRYER_PRODUCT)
            .await?
        {
            return Ok(existing);
        }

        let response = self
            .client
            .post(keys_url.clone())
            .header("Accept", "application/json")
            .header("X-Emby-Token", token)
            .query(&[("app", SCRYER_PRODUCT)])
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to create Jellyfin API key: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Repository(format!(
                "Jellyfin did not create a usable API key (status {}); paste an API key manually",
                response.status()
            )));
        }

        self.find_jellyfin_api_key(&keys_url, token, SCRYER_PRODUCT)
            .await?
            .ok_or_else(|| {
                AppError::Repository(
                    "Jellyfin created an API key but did not expose it through Auth/Keys; paste an API key manually".into(),
                )
            })
    }

    async fn list_jellyfin_users(
        &self,
        base_url: &str,
        api_key: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        let base_url = jellyfin_base_url(base_url)?;
        let users_url = base_url.join("Users").map_err(|error| {
            AppError::Validation(format!("Jellyfin users URL is invalid: {error}"))
        })?;
        let response = self
            .client
            .get(users_url)
            .header("Accept", "application/json")
            .header("X-Emby-Token", api_key.trim())
            .send()
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to list Jellyfin users: {error}"))
            })?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(AppError::Unauthorized(
                    "Jellyfin API key is invalid or cannot list users".into(),
                ));
            }
            status => {
                return Err(AppError::Repository(format!(
                    "Jellyfin user listing failed with status {status}"
                )));
            }
        }

        let mut users = response
            .json::<Vec<JellyfinUser>>()
            .await
            .map_err(|error| {
                AppError::Repository(format!("invalid Jellyfin user list response: {error}"))
            })?
            .into_iter()
            .map(|user| {
                let avatar_url = jellyfin_user_avatar_url(
                    &base_url,
                    &user.id,
                    user.primary_image_tag.as_deref(),
                );
                let username = user.name.unwrap_or_else(|| user.id.clone());
                JellyfinServerUser {
                    id: user.id,
                    username: username.clone(),
                    display_name: Some(username),
                    avatar_url,
                }
            })
            .collect::<Vec<_>>();
        if let Some(search) = search.map(str::trim).filter(|value| !value.is_empty()) {
            let search = search.to_ascii_lowercase();
            users.retain(|user| {
                user.username.to_ascii_lowercase().contains(&search)
                    || user.id.to_ascii_lowercase().contains(&search)
            });
        }
        Ok(users)
    }
}

#[derive(serde::Serialize)]
struct JellyfinAuthRequest<'a> {
    #[serde(rename = "Username")]
    username: &'a str,
    #[serde(rename = "Pw")]
    password: &'a str,
}

#[derive(Deserialize)]
struct JellyfinPublicInfo {
    #[serde(rename = "ProductName")]
    product_name: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinAuthResponse {
    #[serde(rename = "User")]
    user: JellyfinUser,
    #[serde(rename = "AccessToken")]
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinApiKeyQueryResult {
    #[serde(rename = "Items")]
    items: Vec<JellyfinApiKeyInfo>,
}

#[derive(Deserialize)]
struct JellyfinApiKeyInfo {
    #[serde(rename = "AccessToken")]
    access_token: Option<String>,
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    #[serde(rename = "DateCreated")]
    date_created: Option<String>,
}

#[derive(Deserialize)]
struct JellyfinUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "PrimaryImageTag")]
    primary_image_tag: Option<String>,
    #[serde(rename = "Policy")]
    policy: Option<JellyfinUserPolicy>,
}

#[derive(Deserialize)]
struct JellyfinUserPolicy {
    #[serde(rename = "IsAdministrator")]
    is_administrator: bool,
}

fn jellyfin_base_url(base_url: &str) -> AppResult<Url> {
    let mut base_url = Url::parse(base_url)
        .map_err(|error| AppError::Validation(format!("Jellyfin base URL is invalid: {error}")))?;
    if base_url.query().is_some() || base_url.fragment().is_some() {
        return Err(AppError::Validation(
            "Jellyfin connection base URL must not include a query or fragment".into(),
        ));
    }
    if !base_url.path().ends_with('/') {
        base_url.set_path(&format!("{}/", base_url.path()));
    }
    Ok(base_url)
}

fn jellyfin_user_avatar_url(
    base_url: &Url,
    user_id: &str,
    primary_image_tag: Option<&str>,
) -> Option<String> {
    let tag = primary_image_tag
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut image_url = base_url
        .join(&format!("Users/{user_id}/Images/Primary"))
        .ok()?;
    image_url.query_pairs_mut().append_pair("tag", tag);
    Some(image_url.to_string())
}

fn jellyfin_authorization_header(connection_id: &str) -> String {
    let device_id = format!("SCRYER_{}", connection_id.replace('"', ""));
    format!(
        "MediaBrowser Client=\"{SCRYER_PRODUCT}\", Device=\"{SCRYER_PRODUCT}\", DeviceId=\"{device_id}\", Version=\"{SCRYER_VERSION}\""
    )
}

fn json_value_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn plex_server_discoveries(resources_xml: &str) -> AppResult<Vec<PlexServerDiscovery>> {
    let mut servers = Vec::new();
    let mut reader = Reader::from_str(resources_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                if element.name().as_ref() != b"Device" {
                    continue;
                }
                let mut machine_id = None;
                let mut name = None;
                let mut product = None;
                let mut provides = None;
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|error| {
                        AppError::Repository(format!("invalid Plex resources XML: {error}"))
                    })?;
                    let value = attribute
                        .normalized_value(XmlVersion::Implicit1_0)
                        .map_err(|error| {
                            AppError::Repository(format!("invalid Plex resources XML: {error}"))
                        })?
                        .trim()
                        .to_string();
                    if value.is_empty() {
                        continue;
                    }
                    match attribute.key.as_ref() {
                        b"machineIdentifier" => machine_id = Some(value),
                        b"name" => name = Some(value),
                        b"product" => product = Some(value),
                        b"provides" => provides = Some(value),
                        _ => {}
                    }
                }
                if provides
                    .as_deref()
                    .is_some_and(|value| !value.split(',').any(|part| part.trim() == "server"))
                {
                    continue;
                }
                if let Some(machine_id) = machine_id {
                    let name = name.or(product).unwrap_or_else(|| machine_id.clone());
                    servers.push(PlexServerDiscovery {
                        id: machine_id,
                        name,
                    });
                }
            }
            Ok(Event::Eof) => {
                servers.sort_by(|left, right| {
                    left.name
                        .to_ascii_lowercase()
                        .cmp(&right.name.to_ascii_lowercase())
                        .then_with(|| left.id.cmp(&right.id))
                });
                servers.dedup_by(|left, right| left.id == right.id);
                return Ok(servers);
            }
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex resources XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

fn plex_resources_include_machine(resources_xml: &str, machine_id: &str) -> AppResult<bool> {
    let mut reader = Reader::from_str(resources_xml);
    reader.config_mut().trim_text(true);
    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) | Ok(Event::Empty(element)) => {
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|error| {
                        AppError::Repository(format!("invalid Plex resources XML: {error}"))
                    })?;
                    if attribute.key.as_ref() == b"machineIdentifier" {
                        let value = attribute
                            .normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|error| {
                                AppError::Repository(format!("invalid Plex resources XML: {error}"))
                            })?;
                        if value == machine_id {
                            return Ok(true);
                        }
                    }
                }
            }
            Ok(Event::Eof) => return Ok(false),
            Err(error) => {
                return Err(AppError::Repository(format!(
                    "invalid Plex resources XML: {error}"
                )));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn verifier_with_plex(plex_base_url: Url) -> HttpExternalIdentityVerifier {
        HttpExternalIdentityVerifier::with_plex_base_url(plex_base_url)
    }

    #[tokio::test]
    async fn jellyfin_verification_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "User": {
                    "Id": "jf-user",
                    "Name": "Jelly User",
                    "PrimaryImageTag": "tag"
                }
            })))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let verified = verifier
            .verify_jellyfin("jellyfin-main", &server.uri(), "jelly", "secret")
            .await
            .expect("verify jellyfin");

        assert_eq!(verified.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(verified.external_user_id, "jf-user");
        assert_eq!(verified.username, "Jelly User");
        let expected_avatar_url = format!("{}/Users/jf-user/Images/Primary?tag=tag", server.uri());
        assert_eq!(
            verified.avatar_url.as_deref(),
            Some(expected_avatar_url.as_str())
        );
    }

    #[tokio::test]
    async fn jellyfin_user_listing_returns_avatar_urls() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/Users"))
            .and(header("x-emby-token", "api-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "Id": "jf-user",
                "Name": "Jelly User",
                "PrimaryImageTag": "avatar-tag"
            }])))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let users = verifier
            .list_jellyfin_users(&server.uri(), "api-key", None)
            .await
            .expect("list jellyfin users");

        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "Jelly User");
        let expected_avatar_url = format!(
            "{}/Users/jf-user/Images/Primary?tag=avatar-tag",
            server.uri()
        );
        assert_eq!(
            users[0].avatar_url.as_deref(),
            Some(expected_avatar_url.as_str())
        );
    }

    #[tokio::test]
    async fn jellyfin_connection_test_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ProductName": "Jellyfin Server"
            })))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        verifier
            .test_jellyfin_connection(&server.uri())
            .await
            .expect("test jellyfin connection");
    }

    #[tokio::test]
    async fn jellyfin_connection_test_reports_failure_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/System/Info/Public"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let result = verifier.test_jellyfin_connection(&server.uri()).await;

        assert!(matches!(result, Err(AppError::Repository(_))));
    }

    #[tokio::test]
    async fn jellyfin_invalid_credentials_are_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let result = verifier
            .verify_jellyfin("jellyfin-main", &server.uri(), "jelly", "bad")
            .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn jellyfin_admin_exchange_creates_key_then_reads_keys() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "AccessToken": "admin-token",
                "User": {
                    "Id": "admin-user",
                    "Name": "Admin User",
                    "Policy": { "IsAdministrator": true }
                }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let key_list_calls = Arc::new(AtomicUsize::new(0));
        let key_list_calls_for_mock = Arc::clone(&key_list_calls);
        Mock::given(method("GET"))
            .and(path("/Auth/Keys"))
            .and(header("x-emby-token", "admin-token"))
            .respond_with(move |_request: &wiremock::Request| {
                if key_list_calls_for_mock.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "Items": [],
                        "TotalRecordCount": 0
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(json!({
                        "Items": [{
                            "AppName": "Scryer",
                            "AccessToken": "generated-token",
                            "DateCreated": "2026-05-30T00:00:00.0000000Z"
                        }],
                        "TotalRecordCount": 1
                    }))
                }
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/Auth/Keys"))
            .and(header("x-emby-token", "admin-token"))
            .and(query_param("app", SCRYER_PRODUCT))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let verifier = HttpExternalIdentityVerifier::new();

        let api_key = verifier
            .exchange_jellyfin_admin_api_key("jellyfin-main", &server.uri(), "admin", "secret")
            .await
            .expect("exchange jellyfin admin credentials for api key");

        assert_eq!(api_key, "generated-token");
        assert_eq!(key_list_calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn plex_token_verification_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .and(header("x-plex-token", "token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": {
                    "id": 123,
                    "username": "plexuser",
                    "title": "Plex User",
                    "thumb": "https://plex.tv/avatar.jpg"
                }
            })))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let verified = verifier
            .verify_plex("plex-main", None, "token")
            .await
            .expect("verify plex");

        assert_eq!(verified.provider, ExternalAccountProvider::Plex);
        assert_eq!(verified.external_user_id, "123");
        assert_eq!(verified.username, "plexuser");
        assert_eq!(verified.display_name.as_deref(), Some("Plex User"));
        assert_eq!(
            verified.avatar_url.as_deref(),
            Some("https://plex.tv/avatar.jpg")
        );
    }

    #[tokio::test]
    async fn plex_invalid_token_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let result = verifier.verify_plex("plex-main", None, "bad-token").await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn plex_machine_match_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": "plex-user", "username": "plexuser" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(query_param("includeHttps", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Device machineIdentifier="machine-1" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        verifier
            .verify_plex("plex-main", Some("machine-1"), "token")
            .await
            .expect("machine should match");
    }

    #[tokio::test]
    async fn plex_machine_mismatch_is_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/users/account.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "user": { "id": "plex-user", "username": "plexuser" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/resources"))
            .and(query_param("includeHttps", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<MediaContainer><Device machineIdentifier="other-machine" /></MediaContainer>"#,
            ))
            .mount(&server)
            .await;
        let verifier = verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"));

        let result = verifier
            .verify_plex("plex-main", Some("machine-1"), "token")
            .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }
}
