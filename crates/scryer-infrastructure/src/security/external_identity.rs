use std::sync::Arc;

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::XmlVersion;
use quick_xml::events::Event;
use reqwest::StatusCode;
use scryer_application::{
    AppError, AppResult, AuthProviderConnection, ExternalIdentityVerifier, SettingsRepository,
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
    settings: Arc<dyn SettingsRepository>,
    client: reqwest::Client,
    plex_base_url: Url,
}

impl HttpExternalIdentityVerifier {
    pub fn new(settings: Arc<dyn SettingsRepository>) -> Self {
        Self {
            settings,
            client: generic_reqwest_client(),
            plex_base_url: Url::parse(PLEX_BASE_URL).expect("valid Plex base URL"),
        }
    }

    #[cfg(test)]
    fn with_plex_base_url(settings: Arc<dyn SettingsRepository>, plex_base_url: Url) -> Self {
        Self {
            settings,
            client: generic_reqwest_client(),
            plex_base_url,
        }
    }

    async fn configured_connections(
        &self,
        connection_key: &str,
        legacy_ids_key: &str,
    ) -> AppResult<Vec<AuthProviderConnection>> {
        let configured = self
            .read_setting_json::<Vec<AuthProviderConnection>>(connection_key)
            .await?
            .unwrap_or_default()
            .into_iter()
            .filter_map(normalize_connection)
            .collect::<Vec<_>>();
        if !configured.is_empty() {
            return Ok(configured);
        }

        Ok(self
            .read_setting_json::<Vec<String>>(legacy_ids_key)
            .await?
            .unwrap_or_default()
            .into_iter()
            .filter_map(|id| {
                let id = id.trim().to_string();
                (!id.is_empty()).then(|| AuthProviderConnection {
                    display_name: id.clone(),
                    id,
                    base_url: None,
                    machine_id: None,
                })
            })
            .collect())
    }

    async fn read_setting_json<T>(&self, key_name: &str) -> AppResult<Option<T>>
    where
        T: for<'de> Deserialize<'de>,
    {
        let Some(raw_value) = self
            .settings
            .get_setting_json(scryer_application::SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await?
        else {
            return Ok(None);
        };
        serde_json::from_str(&raw_value).map(Some).map_err(|error| {
            AppError::Repository(format!("invalid auth provider setting: {error}"))
        })
    }

    async fn jellyfin_connection(&self, connection_id: &str) -> AppResult<AuthProviderConnection> {
        find_connection(
            self.configured_connections(
                scryer_application::AUTH_JELLYFIN_CONNECTIONS_KEY,
                scryer_application::AUTH_ALLOWED_JELLYFIN_CONNECTION_IDS_KEY,
            )
            .await?,
            connection_id,
            "Jellyfin",
        )
    }

    async fn plex_connection(&self, connection_id: &str) -> AppResult<AuthProviderConnection> {
        find_connection(
            self.configured_connections(
                scryer_application::AUTH_PLEX_CONNECTIONS_KEY,
                scryer_application::AUTH_ALLOWED_PLEX_CONNECTION_IDS_KEY,
            )
            .await?,
            connection_id,
            "Plex",
        )
    }

    fn plex_url(&self, path: &str) -> AppResult<Url> {
        self.plex_base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| AppError::Repository(format!("invalid Plex endpoint URL: {error}")))
    }
}

#[async_trait]
impl ExternalIdentityVerifier for HttpExternalIdentityVerifier {
    async fn verify_plex(
        &self,
        connection_id: &str,
        plex_auth_token: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let connection = self.plex_connection(connection_id).await?;
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

        if let Some(machine_id) = connection.machine_id.as_deref() {
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
            connection_id: connection.id,
            external_user_id,
            username,
            display_name,
            avatar_url,
        })
    }

    async fn verify_jellyfin(
        &self,
        connection_id: &str,
        username: &str,
        password: &str,
    ) -> AppResult<VerifiedExternalIdentity> {
        let connection = self.jellyfin_connection(connection_id).await?;
        let base_url = connection.base_url.as_deref().ok_or_else(|| {
            AppError::Validation("Jellyfin connection does not have a base URL configured".into())
        })?;
        let mut base_url = Url::parse(base_url).map_err(|error| {
            AppError::Validation(format!("Jellyfin base URL is invalid: {error}"))
        })?;
        if base_url.query().is_some() || base_url.fragment().is_some() {
            return Err(AppError::Validation(
                "Jellyfin connection base URL must not include a query or fragment".into(),
            ));
        }
        if !base_url.path().ends_with('/') {
            base_url.set_path(&format!("{}/", base_url.path()));
        }
        let auth_url = base_url.join("Users/AuthenticateByName").map_err(|error| {
            AppError::Validation(format!("Jellyfin authentication URL is invalid: {error}"))
        })?;

        let response = self
            .client
            .post(auth_url)
            .header(
                "Authorization",
                jellyfin_authorization_header(&connection.id),
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
        let avatar_url = auth.user.primary_image_tag.as_ref().map(|_| {
            format!(
                "{}/Users/{}/Images/Primary",
                base_url.as_str().trim_end_matches('/'),
                auth.user.id
            )
        });

        Ok(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: connection.id,
            external_user_id: auth.user.id,
            username: remote_username.clone(),
            display_name: Some(remote_username),
            avatar_url,
        })
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
struct JellyfinAuthResponse {
    #[serde(rename = "User")]
    user: JellyfinUser,
}

#[derive(Deserialize)]
struct JellyfinUser {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "PrimaryImageTag")]
    primary_image_tag: Option<String>,
}

fn find_connection(
    connections: Vec<AuthProviderConnection>,
    connection_id: &str,
    provider_name: &str,
) -> AppResult<AuthProviderConnection> {
    let connection_id = connection_id.trim();
    connections
        .into_iter()
        .find(|connection| connection.id == connection_id)
        .ok_or_else(|| {
            AppError::Validation(format!(
                "{provider_name} connection is not configured for external auth"
            ))
        })
}

fn normalize_connection(mut connection: AuthProviderConnection) -> Option<AuthProviderConnection> {
    connection.id = connection.id.trim().to_string();
    if connection.id.is_empty() {
        return None;
    }
    connection.display_name = connection.display_name.trim().to_string();
    if connection.display_name.is_empty() {
        connection.display_name = connection.id.clone();
    }
    connection.base_url = connection
        .base_url
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty());
    connection.machine_id = connection
        .machine_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Some(connection)
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
    use std::collections::HashMap;

    use serde_json::json;
    use tokio::sync::Mutex;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    type TestSettingsKey = (String, String, Option<String>);

    #[derive(Default)]
    struct TestSettingsRepository {
        values: Mutex<HashMap<TestSettingsKey, String>>,
    }

    impl TestSettingsRepository {
        async fn set_json<T: serde::Serialize>(&self, key_name: &str, value: &T) {
            self.values.lock().await.insert(
                (
                    scryer_application::SETTINGS_SCOPE_SYSTEM.to_string(),
                    key_name.to_string(),
                    None,
                ),
                serde_json::to_string(value).expect("serialize setting"),
            );
        }
    }

    #[async_trait]
    impl SettingsRepository for TestSettingsRepository {
        async fn get_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self
                .values
                .lock()
                .await
                .get(&(scope.to_string(), key_name.to_string(), scope_id))
                .cloned())
        }

        async fn upsert_setting_json(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
            value_json: String,
            _source: &str,
            _updated_by_user_id: Option<String>,
        ) -> AppResult<()> {
            self.values.lock().await.insert(
                (scope.to_string(), key_name.to_string(), scope_id),
                value_json,
            );
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<()> {
            self.values
                .lock()
                .await
                .remove(&(scope.to_string(), key_name.to_string(), scope_id));
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
            let mut values = self.values.lock().await;
            let before = values.len();
            values.retain(|(_, _, current_scope_id), _| {
                current_scope_id.as_deref() != Some(scope_id)
            });
            Ok((before - values.len()) as u32)
        }
    }

    async fn verifier_with_jellyfin(
        base_url: String,
    ) -> (HttpExternalIdentityVerifier, Arc<TestSettingsRepository>) {
        let settings = Arc::new(TestSettingsRepository::default());
        settings
            .set_json(
                scryer_application::AUTH_JELLYFIN_CONNECTIONS_KEY,
                &vec![AuthProviderConnection {
                    id: "jellyfin-main".to_string(),
                    display_name: "Main Jellyfin".to_string(),
                    base_url: Some(base_url),
                    machine_id: None,
                }],
            )
            .await;
        (
            HttpExternalIdentityVerifier::new(settings.clone()),
            settings,
        )
    }

    async fn verifier_with_plex(
        plex_base_url: Url,
        machine_id: Option<String>,
    ) -> (HttpExternalIdentityVerifier, Arc<TestSettingsRepository>) {
        let settings = Arc::new(TestSettingsRepository::default());
        settings
            .set_json(
                scryer_application::AUTH_PLEX_CONNECTIONS_KEY,
                &vec![AuthProviderConnection {
                    id: "plex-main".to_string(),
                    display_name: "Main Plex".to_string(),
                    base_url: None,
                    machine_id,
                }],
            )
            .await;
        (
            HttpExternalIdentityVerifier::with_plex_base_url(settings.clone(), plex_base_url),
            settings,
        )
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
        let (verifier, _) = verifier_with_jellyfin(server.uri()).await;

        let verified = verifier
            .verify_jellyfin("jellyfin-main", "jelly", "secret")
            .await
            .expect("verify jellyfin");

        assert_eq!(verified.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(verified.external_user_id, "jf-user");
        assert_eq!(verified.username, "Jelly User");
        let expected_avatar_url = format!("{}/Users/jf-user/Images/Primary", server.uri());
        assert_eq!(
            verified.avatar_url.as_deref(),
            Some(expected_avatar_url.as_str())
        );
    }

    #[tokio::test]
    async fn jellyfin_invalid_credentials_are_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/Users/AuthenticateByName"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let (verifier, _) = verifier_with_jellyfin(server.uri()).await;

        let result = verifier
            .verify_jellyfin("jellyfin-main", "jelly", "bad")
            .await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn jellyfin_missing_connection_is_validation_error() {
        let settings = Arc::new(TestSettingsRepository::default());
        let verifier = HttpExternalIdentityVerifier::new(settings);

        let result = verifier.verify_jellyfin("missing", "jelly", "secret").await;

        assert!(matches!(result, Err(AppError::Validation(_))));
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
        let (verifier, _) =
            verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"), None).await;

        let verified = verifier
            .verify_plex("plex-main", "token")
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
        let (verifier, _) =
            verifier_with_plex(Url::parse(&server.uri()).expect("mock URL"), None).await;

        let result = verifier.verify_plex("plex-main", "bad-token").await;

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
        let (verifier, _) = verifier_with_plex(
            Url::parse(&server.uri()).expect("mock URL"),
            Some("machine-1".to_string()),
        )
        .await;

        verifier
            .verify_plex("plex-main", "token")
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
        let (verifier, _) = verifier_with_plex(
            Url::parse(&server.uri()).expect("mock URL"),
            Some("machine-1".to_string()),
        )
        .await;

        let result = verifier.verify_plex("plex-main", "token").await;

        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }
}
