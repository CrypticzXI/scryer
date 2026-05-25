use super::*;
use crate::settings::keys::{
    AUTH_ALLOWED_JELLYFIN_CONNECTION_IDS_KEY, AUTH_ALLOWED_PLEX_CONNECTION_IDS_KEY,
    AUTH_ALLOWED_PROVIDERS_KEY, AUTH_JELLYFIN_CONNECTIONS_KEY, AUTH_PLEX_CONNECTIONS_KEY,
    AUTH_PROVIDER_LINKING_ENABLED_KEY, AUTH_PROVIDER_LOGIN_ENABLED_KEY,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthProviderConnection {
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub machine_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthProviderSettings {
    pub allowed_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub provider_login_enabled: Vec<scryer_domain::ExternalAccountProvider>,
    pub provider_linking_enabled: Vec<scryer_domain::ExternalAccountProvider>,
    pub allowed_jellyfin_connection_ids: Vec<String>,
    pub allowed_plex_connection_ids: Vec<String>,
    pub allowed_jellyfin_connections: Vec<AuthProviderConnection>,
    pub allowed_plex_connections: Vec<AuthProviderConnection>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateAuthProviderSettings {
    pub allowed_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub provider_login_enabled: Vec<scryer_domain::ExternalAccountProvider>,
    pub provider_linking_enabled: Vec<scryer_domain::ExternalAccountProvider>,
    pub allowed_jellyfin_connection_ids: Vec<String>,
    pub allowed_plex_connection_ids: Vec<String>,
    pub allowed_jellyfin_connections: Vec<AuthProviderConnection>,
    pub allowed_plex_connections: Vec<AuthProviderConnection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthProviderUse {
    Login,
    Linking,
    Invite,
}

impl AppUseCase {
    pub async fn get_auth_provider_settings(
        &self,
        actor: &User,
    ) -> AppResult<AuthProviderSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_auth_provider_settings().await
    }

    pub async fn get_auth_provider_runtime_settings(&self) -> AppResult<AuthProviderSettings> {
        self.load_auth_provider_settings().await
    }

    pub async fn update_auth_provider_settings(
        &self,
        actor: &User,
        input: UpdateAuthProviderSettings,
    ) -> AppResult<AuthProviderSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let allowed_jellyfin_connections = normalize_auth_provider_connections(
            input.allowed_jellyfin_connections,
            scryer_domain::ExternalAccountProvider::Jellyfin,
        )?;
        let allowed_jellyfin_connections = if allowed_jellyfin_connections.is_empty() {
            auth_provider_connections_from_ids(input.allowed_jellyfin_connection_ids)
        } else {
            allowed_jellyfin_connections
        };
        let allowed_plex_connections = normalize_auth_provider_connections(
            input.allowed_plex_connections,
            scryer_domain::ExternalAccountProvider::Plex,
        )?;
        let allowed_plex_connections = if allowed_plex_connections.is_empty() {
            auth_provider_connections_from_ids(input.allowed_plex_connection_ids)
        } else {
            allowed_plex_connections
        };

        let settings = AuthProviderSettings {
            allowed_providers: normalize_providers(input.allowed_providers),
            provider_login_enabled: normalize_providers(input.provider_login_enabled),
            provider_linking_enabled: normalize_providers(input.provider_linking_enabled),
            allowed_jellyfin_connection_ids: auth_provider_connection_ids(
                &allowed_jellyfin_connections,
            ),
            allowed_plex_connection_ids: auth_provider_connection_ids(&allowed_plex_connections),
            allowed_jellyfin_connections,
            allowed_plex_connections,
        };

        self.upsert_system_setting_json(
            AUTH_ALLOWED_PROVIDERS_KEY,
            &provider_strings(&settings.allowed_providers),
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_PROVIDER_LOGIN_ENABLED_KEY,
            &provider_strings(&settings.provider_login_enabled),
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_PROVIDER_LINKING_ENABLED_KEY,
            &provider_strings(&settings.provider_linking_enabled),
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_ALLOWED_JELLYFIN_CONNECTION_IDS_KEY,
            &settings.allowed_jellyfin_connection_ids,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_JELLYFIN_CONNECTIONS_KEY,
            &settings.allowed_jellyfin_connections,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_ALLOWED_PLEX_CONNECTION_IDS_KEY,
            &settings.allowed_plex_connection_ids,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTH_PLEX_CONNECTIONS_KEY,
            &settings.allowed_plex_connections,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_settings_saved(
            actor,
            "auth_provider_settings",
            None,
            vec![
                AUTH_ALLOWED_PROVIDERS_KEY.to_string(),
                AUTH_PROVIDER_LOGIN_ENABLED_KEY.to_string(),
                AUTH_PROVIDER_LINKING_ENABLED_KEY.to_string(),
                AUTH_ALLOWED_JELLYFIN_CONNECTION_IDS_KEY.to_string(),
                AUTH_JELLYFIN_CONNECTIONS_KEY.to_string(),
                AUTH_ALLOWED_PLEX_CONNECTION_IDS_KEY.to_string(),
                AUTH_PLEX_CONNECTIONS_KEY.to_string(),
            ],
        )
        .await;

        Ok(settings)
    }

    async fn load_auth_provider_settings(&self) -> AppResult<AuthProviderSettings> {
        let legacy_jellyfin_connection_ids = self
            .read_string_list_setting(AUTH_ALLOWED_JELLYFIN_CONNECTION_IDS_KEY)
            .await?;
        let allowed_jellyfin_connections = self
            .read_connection_list_setting(
                AUTH_JELLYFIN_CONNECTIONS_KEY,
                scryer_domain::ExternalAccountProvider::Jellyfin,
            )
            .await?;
        let allowed_jellyfin_connections = if allowed_jellyfin_connections.is_empty() {
            auth_provider_connections_from_ids(legacy_jellyfin_connection_ids)
        } else {
            allowed_jellyfin_connections
        };
        let legacy_plex_connection_ids = self
            .read_string_list_setting(AUTH_ALLOWED_PLEX_CONNECTION_IDS_KEY)
            .await?;
        let allowed_plex_connections = self
            .read_connection_list_setting(
                AUTH_PLEX_CONNECTIONS_KEY,
                scryer_domain::ExternalAccountProvider::Plex,
            )
            .await?;
        let allowed_plex_connections = if allowed_plex_connections.is_empty() {
            auth_provider_connections_from_ids(legacy_plex_connection_ids)
        } else {
            allowed_plex_connections
        };

        Ok(AuthProviderSettings {
            allowed_providers: self
                .read_provider_list_setting(AUTH_ALLOWED_PROVIDERS_KEY)
                .await?,
            provider_login_enabled: self
                .read_provider_list_setting(AUTH_PROVIDER_LOGIN_ENABLED_KEY)
                .await?,
            provider_linking_enabled: self
                .read_provider_list_setting(AUTH_PROVIDER_LINKING_ENABLED_KEY)
                .await?,
            allowed_jellyfin_connection_ids: auth_provider_connection_ids(
                &allowed_jellyfin_connections,
            ),
            allowed_plex_connection_ids: auth_provider_connection_ids(&allowed_plex_connections),
            allowed_jellyfin_connections,
            allowed_plex_connections,
        })
    }

    async fn read_provider_list_setting(
        &self,
        key_name: &str,
    ) -> AppResult<Vec<scryer_domain::ExternalAccountProvider>> {
        let values = self.read_string_list_setting(key_name).await?;
        Ok(normalize_providers(
            values
                .into_iter()
                .filter_map(|value| scryer_domain::ExternalAccountProvider::parse(&value))
                .collect(),
        ))
    }

    async fn read_string_list_setting(&self, key_name: &str) -> AppResult<Vec<String>> {
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json(crate::settings::keys::SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await?
        else {
            return Ok(Vec::new());
        };
        let parsed = serde_json::from_str::<Vec<String>>(&raw_value).unwrap_or_default();
        Ok(normalize_connection_ids(parsed))
    }

    async fn read_connection_list_setting(
        &self,
        key_name: &str,
        provider: scryer_domain::ExternalAccountProvider,
    ) -> AppResult<Vec<AuthProviderConnection>> {
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json(crate::settings::keys::SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await?
        else {
            return Ok(Vec::new());
        };
        let parsed =
            serde_json::from_str::<Vec<AuthProviderConnection>>(&raw_value).unwrap_or_default();
        normalize_auth_provider_connections(parsed, provider)
    }

    fn ensure_auth_provider_allowed(
        &self,
        settings: &AuthProviderSettings,
        provider: &scryer_domain::ExternalAccountProvider,
        connection_id: &str,
        usage: AuthProviderUse,
    ) -> AppResult<()> {
        if !settings.allowed_providers.contains(provider) {
            return Err(AppError::Validation(format!(
                "{} is not enabled as an external account provider",
                provider.as_str()
            )));
        }
        let enabled = match usage {
            AuthProviderUse::Login => &settings.provider_login_enabled,
            AuthProviderUse::Linking => &settings.provider_linking_enabled,
            AuthProviderUse::Invite => &settings.provider_login_enabled,
        };
        if !enabled.contains(provider) {
            return Err(AppError::Validation(format!(
                "{} is not enabled for {}",
                provider.as_str(),
                match usage {
                    AuthProviderUse::Login => "login",
                    AuthProviderUse::Linking => "linking",
                    AuthProviderUse::Invite => "invites",
                }
            )));
        }
        let allowed_connections = match provider {
            scryer_domain::ExternalAccountProvider::Plex => &settings.allowed_plex_connection_ids,
            scryer_domain::ExternalAccountProvider::Jellyfin => {
                &settings.allowed_jellyfin_connection_ids
            }
        };
        if !allowed_connections
            .iter()
            .any(|allowed| allowed == connection_id.trim())
        {
            return Err(AppError::Validation(format!(
                "{} connection is not allowed for external auth",
                provider.as_str()
            )));
        }
        Ok(())
    }

    fn ensure_verified_identity_matches_request(
        &self,
        settings: &AuthProviderSettings,
        expected_provider: &scryer_domain::ExternalAccountProvider,
        expected_connection_id: &str,
        usage: AuthProviderUse,
        verified: &VerifiedExternalIdentity,
    ) -> AppResult<()> {
        if &verified.provider != expected_provider {
            return Err(AppError::Validation(
                "verified external identity provider did not match the requested provider".into(),
            ));
        }
        if verified.connection_id.trim() != expected_connection_id.trim() {
            return Err(AppError::Validation(
                "verified external identity connection did not match the requested connection"
                    .into(),
            ));
        }
        self.ensure_auth_provider_allowed(
            settings,
            &verified.provider,
            &verified.connection_id,
            usage,
        )
    }

    pub async fn list_linked_accounts(
        &self,
        actor: &User,
        user_id: Option<&str>,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        let target_user_id = user_id.unwrap_or(&actor.id);
        if target_user_id != actor.id {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
                .await?;
        }
        self.services
            .identity
            .external_accounts
            .list_by_user_id(target_user_id)
            .await
    }

    pub async fn create_external_account_invite(
        &self,
        actor: &User,
        user_id: &str,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: String,
        external_user_id: String,
        username: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        let connection_id = normalize_connection_id(connection_id);
        let external_user_id = external_user_id.trim().to_string();
        let username = username.trim().to_string();
        let settings = self.load_auth_provider_settings().await?;
        self.ensure_auth_provider_allowed(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Invite,
        )?;
        self.services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

        let account = scryer_domain::UserExternalAccount::pending_claim(
            user_id.to_string(),
            provider,
            connection_id,
            external_user_id,
            username,
        );
        self.services
            .identity
            .external_accounts
            .create(account)
            .await
    }

    pub async fn link_plex_account(
        &self,
        actor: &User,
        connection_id: String,
        plex_auth_token: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let settings = self.load_auth_provider_settings().await?;
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        self.ensure_auth_provider_allowed(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Linking,
        )?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(&connection_id, &plex_auth_token)
            .await?;
        self.ensure_verified_identity_matches_request(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Linking,
            &verified,
        )?;
        self.link_verified_external_account(actor, verified).await
    }

    pub async fn link_jellyfin_account(
        &self,
        actor: &User,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let settings = self.load_auth_provider_settings().await?;
        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        self.ensure_auth_provider_allowed(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Linking,
        )?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &username, &password)
            .await?;
        self.ensure_verified_identity_matches_request(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Linking,
            &verified,
        )?;
        self.link_verified_external_account(actor, verified).await
    }

    async fn link_verified_external_account(
        &self,
        actor: &User,
        verified: VerifiedExternalIdentity,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        if let Some(mut existing) = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                verified.provider.clone(),
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?
        {
            if existing.user_id != actor.id {
                return Err(AppError::Validation(
                    "external account is already linked to another Scryer user".into(),
                ));
            }
            if matches!(
                existing.status,
                scryer_domain::ExternalAccountStatus::Disabled
            ) {
                return Err(AppError::Validation(
                    "external account is disabled and must be repaired by an administrator".into(),
                ));
            }
            existing.username = verified.username;
            existing.display_name = verified.display_name;
            existing.avatar_url = verified.avatar_url;
            existing.status = scryer_domain::ExternalAccountStatus::Active;
            existing.verified_at = Some(Utc::now());
            existing.updated_at = Utc::now();
            return self
                .services
                .identity
                .external_accounts
                .update(existing)
                .await;
        }

        let now = Utc::now();
        let account = scryer_domain::UserExternalAccount {
            id: scryer_domain::Id::new().0,
            user_id: actor.id.clone(),
            provider: verified.provider,
            connection_id: verified.connection_id,
            external_user_id: verified.external_user_id,
            username: verified.username,
            display_name: verified.display_name,
            avatar_url: verified.avatar_url,
            status: scryer_domain::ExternalAccountStatus::Active,
            verified_at: Some(now),
            created_at: now,
            updated_at: now,
        };
        self.services
            .identity
            .external_accounts
            .create(account)
            .await
    }

    pub async fn federated_login_with_plex(
        &self,
        connection_id: String,
        plex_auth_token: String,
    ) -> AppResult<User> {
        let settings = self.load_auth_provider_settings().await?;
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        self.ensure_auth_provider_allowed(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Login,
        )?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(&connection_id, &plex_auth_token)
            .await?;
        self.ensure_verified_identity_matches_request(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Login,
            &verified,
        )?;
        self.login_verified_external_account(verified).await
    }

    pub async fn federated_login_with_jellyfin(
        &self,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<User> {
        let settings = self.load_auth_provider_settings().await?;
        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        self.ensure_auth_provider_allowed(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Login,
        )?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &username, &password)
            .await?;
        self.ensure_verified_identity_matches_request(
            &settings,
            &provider,
            &connection_id,
            AuthProviderUse::Login,
            &verified,
        )?;
        self.login_verified_external_account(verified).await
    }

    async fn login_verified_external_account(
        &self,
        verified: VerifiedExternalIdentity,
    ) -> AppResult<User> {
        let mut account = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                verified.provider,
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?
            .ok_or_else(|| AppError::Unauthorized("external account is not invited".into()))?;

        match account.status {
            scryer_domain::ExternalAccountStatus::Disabled => {
                return Err(AppError::Unauthorized(
                    "external account is disabled".into(),
                ));
            }
            scryer_domain::ExternalAccountStatus::PendingClaim => {
                account.status = scryer_domain::ExternalAccountStatus::Active;
            }
            scryer_domain::ExternalAccountStatus::Active => {}
        }
        account.username = verified.username;
        account.display_name = verified.display_name;
        account.avatar_url = verified.avatar_url;
        account.verified_at = Some(Utc::now());
        account.updated_at = Utc::now();
        self.services
            .identity
            .external_accounts
            .update(account.clone())
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(&account.user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", account.user_id)))?;
        self.cache_jwt_signing_key(&user).await?;
        Ok(user)
    }

    pub async fn unlink_external_account(
        &self,
        actor: &User,
        linked_account_id: &str,
    ) -> AppResult<()> {
        let account = self
            .services
            .identity
            .external_accounts
            .get_by_id(linked_account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("external account {linked_account_id}")))?;
        if account.user_id != actor.id {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
                .await?;
        }
        if matches!(account.status, scryer_domain::ExternalAccountStatus::Active) {
            let accounts = self
                .services
                .identity
                .external_accounts
                .list_by_user_id(&account.user_id)
                .await?;
            let active_count = accounts
                .iter()
                .filter(|account| {
                    matches!(account.status, scryer_domain::ExternalAccountStatus::Active)
                })
                .count();
            if active_count <= 1 {
                self.require_local_fallback_credential(&account.user_id)
                    .await?;
            }
        }
        self.services
            .identity
            .external_accounts
            .delete(linked_account_id)
            .await
    }

    async fn require_local_fallback_credential(&self, user_id: &str) -> AppResult<()> {
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if user.password_hash.is_some() {
            return Ok(());
        }
        if !self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(user_id)
            .await?
            .is_empty()
        {
            return Ok(());
        }
        Err(AppError::Validation(
            "cannot unlink the last external login without a local password or passkey".into(),
        ))
    }
}

fn normalize_providers(
    providers: Vec<scryer_domain::ExternalAccountProvider>,
) -> Vec<scryer_domain::ExternalAccountProvider> {
    let mut normalized = Vec::new();
    for provider in providers {
        if !normalized.contains(&provider) {
            normalized.push(provider);
        }
    }
    normalized
}

fn normalize_connection_ids(values: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for value in values {
        let value = normalize_connection_id(value);
        if !value.is_empty() && !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    normalized
}

fn auth_provider_connections_from_ids(values: Vec<String>) -> Vec<AuthProviderConnection> {
    normalize_connection_ids(values)
        .into_iter()
        .map(|id| AuthProviderConnection {
            display_name: id.clone(),
            id,
            base_url: None,
            machine_id: None,
        })
        .collect()
}

fn auth_provider_connection_ids(connections: &[AuthProviderConnection]) -> Vec<String> {
    normalize_connection_ids(
        connections
            .iter()
            .map(|connection| connection.id.clone())
            .collect(),
    )
}

fn normalize_auth_provider_connections(
    values: Vec<AuthProviderConnection>,
    provider: scryer_domain::ExternalAccountProvider,
) -> AppResult<Vec<AuthProviderConnection>> {
    let mut normalized = Vec::new();
    for value in values {
        let id = normalize_connection_id(value.id);
        if id.is_empty()
            || normalized
                .iter()
                .any(|connection: &AuthProviderConnection| connection.id == id)
        {
            continue;
        }
        let display_name = value.display_name.trim();
        normalized.push(AuthProviderConnection {
            id: id.clone(),
            display_name: if display_name.is_empty() {
                id
            } else {
                display_name.to_string()
            },
            base_url: match provider {
                scryer_domain::ExternalAccountProvider::Jellyfin => {
                    normalize_optional_base_url(value.base_url)?
                }
                scryer_domain::ExternalAccountProvider::Plex => None,
            },
            machine_id: match provider {
                scryer_domain::ExternalAccountProvider::Plex => {
                    normalize_optional_string(value.machine_id)
                }
                scryer_domain::ExternalAccountProvider::Jellyfin => None,
            },
        });
    }
    Ok(normalized)
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_optional_base_url(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = normalize_optional_string(value) else {
        return Ok(None);
    };
    let parsed = url::Url::parse(&value)
        .map_err(|_| AppError::Validation("Jellyfin connection base URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "Jellyfin connection base URL must be an HTTP or HTTPS URL".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::Validation(
            "Jellyfin connection base URL must not include a query or fragment".into(),
        ));
    }
    Ok(Some(parsed.as_str().trim_end_matches('/').to_string()))
}

fn normalize_connection_id(value: impl AsRef<str>) -> String {
    value.as_ref().trim().to_string()
}

fn provider_strings(providers: &[scryer_domain::ExternalAccountProvider]) -> Vec<String> {
    providers
        .iter()
        .map(|provider| provider.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use crate::{
        AppServices, IndexerConfig, IndexerConfigRepository, IndexerConfigUpdate, JwtAuthConfig,
        SettingsRepository, UserExternalAccountRepository, UserRepository,
    };
    use scryer_domain::{
        AppPermission, AppPermissionMask, ExternalAccountProvider, LibraryPermissionMask,
        UserAuthorization, UserExternalAccount,
    };

    type TestSettingsKey = (String, String, Option<String>);
    type TestSettingsValues = HashMap<TestSettingsKey, String>;

    #[derive(Default)]
    struct TestSettingsRepository {
        values: Mutex<TestSettingsValues>,
    }

    #[derive(Default)]
    struct TestExternalAccountRepository {
        accounts: Mutex<Vec<UserExternalAccount>>,
    }

    #[derive(Default)]
    struct TestUserRepository {
        users: Mutex<Vec<User>>,
    }

    impl TestExternalAccountRepository {
        fn new(accounts: Vec<UserExternalAccount>) -> Self {
            Self {
                accounts: Mutex::new(accounts),
            }
        }
    }

    impl TestUserRepository {
        fn new(users: Vec<User>) -> Self {
            Self {
                users: Mutex::new(users),
            }
        }
    }

    struct TestIndexerConfigRepository;

    #[async_trait::async_trait]
    impl UserExternalAccountRepository for TestExternalAccountRepository {
        async fn create(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
            self.accounts.lock().await.push(account.clone());
            Ok(account)
        }

        async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .filter(|account| account.user_id == user_id)
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| account.id == id)
                .cloned())
        }

        async fn get_by_provider_identity(
            &self,
            provider: ExternalAccountProvider,
            connection_id: &str,
            external_user_id: &str,
        ) -> AppResult<Option<UserExternalAccount>> {
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| {
                    account.provider == provider
                        && account.connection_id == connection_id
                        && account.external_user_id == external_user_id
                })
                .cloned())
        }

        async fn update(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
            let mut accounts = self.accounts.lock().await;
            if let Some(existing) = accounts
                .iter_mut()
                .find(|candidate| candidate.id == account.id)
            {
                *existing = account.clone();
            }
            Ok(account)
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.accounts
                .lock()
                .await
                .retain(|account| account.id != id);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl UserRepository for TestUserRepository {
        async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
            Ok(self
                .users
                .lock()
                .await
                .iter()
                .find(|user| user.username == username)
                .cloned())
        }

        async fn create(&self, user: User) -> AppResult<User> {
            self.users.lock().await.push(user.clone());
            Ok(user)
        }

        async fn list_all(&self) -> AppResult<Vec<User>> {
            Ok(self.users.lock().await.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
            Ok(self
                .users
                .lock()
                .await
                .iter()
                .find(|user| user.id == id)
                .cloned())
        }

        async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
            let mut users = self.users.lock().await;
            let user = users
                .iter_mut()
                .find(|user| user.id == id)
                .ok_or_else(|| AppError::NotFound(format!("user {id}")))?;
            user.password_hash = Some(password_hash);
            Ok(user.clone())
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.users.lock().await.retain(|user| user.id != id);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl IndexerConfigRepository for TestIndexerConfigRepository {
        async fn list(&self, _: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(Vec::new())
        }

        async fn get_by_id(&self, _: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(None)
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn update(&self, _: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::Repository("not configured".into()))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
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

    fn test_app(settings: Arc<dyn SettingsRepository>) -> AppUseCase {
        test_app_with_external_accounts(
            settings,
            Arc::new(crate::null_repositories::NullUserExternalAccountRepository),
        )
    }

    fn test_app_with_external_accounts(
        settings: Arc<dyn SettingsRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
    ) -> AppUseCase {
        test_app_with_identity(settings, Arc::new(NullUserRepository), external_accounts)
    }

    fn test_app_with_identity(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
    ) -> AppUseCase {
        let assembly = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            users,
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            settings,
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_external_account_store(external_accounts)
        .build_partial_for_tests();

        AppUseCase::new(
            assembly,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn admin_user() -> User {
        User {
            id: "admin".to_string(),
            username: "admin".to_string(),
            password_hash: Some("hash".to_string()),
            authorization: UserAuthorization {
                app: AppPermissionMask::from_permissions([
                    AppPermission::ManageSystemSettings,
                    AppPermission::ManageUsers,
                ]),
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        }
    }

    #[tokio::test]
    async fn auth_provider_settings_are_normalized_and_persisted() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let admin = admin_user();

        let saved = app
            .update_auth_provider_settings(
                &admin,
                UpdateAuthProviderSettings {
                    allowed_providers: vec![
                        ExternalAccountProvider::Jellyfin,
                        ExternalAccountProvider::Jellyfin,
                        ExternalAccountProvider::Plex,
                    ],
                    provider_login_enabled: vec![
                        ExternalAccountProvider::Plex,
                        ExternalAccountProvider::Plex,
                    ],
                    provider_linking_enabled: vec![ExternalAccountProvider::Jellyfin],
                    allowed_jellyfin_connection_ids: vec![
                        " jellyfin-main ".to_string(),
                        "jellyfin-main".to_string(),
                        String::new(),
                    ],
                    allowed_plex_connection_ids: vec!["plex-main".to_string()],
                    allowed_jellyfin_connections: vec![
                        AuthProviderConnection {
                            id: " jellyfin-main ".to_string(),
                            display_name: "Main Jellyfin".to_string(),
                            base_url: Some("https://jellyfin.example.test/".to_string()),
                            machine_id: Some("ignored".to_string()),
                        },
                        AuthProviderConnection {
                            id: "jellyfin-main".to_string(),
                            display_name: "duplicate".to_string(),
                            base_url: Some("https://duplicate.example.test".to_string()),
                            machine_id: None,
                        },
                    ],
                    allowed_plex_connections: vec![AuthProviderConnection {
                        id: "plex-main".to_string(),
                        display_name: String::new(),
                        base_url: Some("https://ignored.example.test".to_string()),
                        machine_id: Some(" machine-1 ".to_string()),
                    }],
                },
            )
            .await
            .expect("save settings");

        assert_eq!(
            saved.allowed_providers,
            vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex
            ]
        );
        assert_eq!(
            saved.provider_login_enabled,
            vec![ExternalAccountProvider::Plex]
        );
        assert_eq!(saved.allowed_jellyfin_connection_ids, vec!["jellyfin-main"]);
        assert_eq!(saved.allowed_plex_connection_ids, vec!["plex-main"]);
        assert_eq!(
            saved.allowed_jellyfin_connections,
            vec![AuthProviderConnection {
                id: "jellyfin-main".to_string(),
                display_name: "Main Jellyfin".to_string(),
                base_url: Some("https://jellyfin.example.test".to_string()),
                machine_id: None,
            }]
        );
        assert_eq!(
            saved.allowed_plex_connections,
            vec![AuthProviderConnection {
                id: "plex-main".to_string(),
                display_name: "plex-main".to_string(),
                base_url: None,
                machine_id: Some("machine-1".to_string()),
            }]
        );

        let loaded = app
            .get_auth_provider_settings(&admin)
            .await
            .expect("load settings");
        assert_eq!(loaded, saved);
    }

    #[tokio::test]
    async fn jellyfin_connection_base_url_rejects_query_and_fragment() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let admin = admin_user();

        for base_url in [
            "https://jellyfin.example.test?token=leak",
            "https://jellyfin.example.test/#fragment",
        ] {
            let result = app
                .update_auth_provider_settings(
                    &admin,
                    UpdateAuthProviderSettings {
                        allowed_providers: vec![ExternalAccountProvider::Jellyfin],
                        provider_login_enabled: vec![ExternalAccountProvider::Jellyfin],
                        provider_linking_enabled: vec![ExternalAccountProvider::Jellyfin],
                        allowed_jellyfin_connection_ids: vec![],
                        allowed_plex_connection_ids: vec![],
                        allowed_jellyfin_connections: vec![AuthProviderConnection {
                            id: "jellyfin-main".to_string(),
                            display_name: "Main Jellyfin".to_string(),
                            base_url: Some(base_url.to_string()),
                            machine_id: None,
                        }],
                        allowed_plex_connections: vec![],
                    },
                )
                .await;

            assert!(
                matches!(result, Err(AppError::Validation(message)) if message.contains("query or fragment"))
            );
        }
    }

    #[tokio::test]
    async fn link_rejects_connection_not_on_allowlist_before_verification() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let admin = admin_user();
        app.update_auth_provider_settings(
            &admin,
            UpdateAuthProviderSettings {
                allowed_providers: vec![ExternalAccountProvider::Jellyfin],
                provider_login_enabled: vec![],
                provider_linking_enabled: vec![ExternalAccountProvider::Jellyfin],
                allowed_jellyfin_connection_ids: vec!["jellyfin-main".to_string()],
                allowed_plex_connection_ids: vec![],
                allowed_jellyfin_connections: vec![],
                allowed_plex_connections: vec![],
            },
        )
        .await
        .expect("save settings");

        let result = app
            .link_jellyfin_account(
                &admin,
                "jellyfin-other".to_string(),
                "someone".to_string(),
                "secret".to_string(),
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("not allowed"))
        );
    }

    #[tokio::test]
    async fn link_rejects_disabled_existing_account() {
        let admin = admin_user();
        let now = Utc::now();
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestExternalAccountRepository::new(vec![
                UserExternalAccount {
                    id: "linked-account".to_string(),
                    user_id: admin.id.clone(),
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "remote-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                    status: scryer_domain::ExternalAccountStatus::Disabled,
                    verified_at: None,
                    created_at: now,
                    updated_at: now,
                },
            ])),
        );

        let result = app
            .link_verified_external_account(
                &admin,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "remote-user".to_string(),
                    display_name: Some("Remote User".to_string()),
                    avatar_url: None,
                },
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("disabled"))
        );
    }

    #[tokio::test]
    async fn pending_claim_login_activates_and_refreshes_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        };
        let now = Utc::now();
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![
            UserExternalAccount {
                id: "pending-account".to_string(),
                user_id: user.id.clone(),
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "old-name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::PendingClaim,
                verified_at: None,
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
        );

        let logged_in = app
            .login_verified_external_account(VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "fresh-name".to_string(),
                display_name: Some("Fresh Name".to_string()),
                avatar_url: Some("https://jellyfin.example.test/avatar".to_string()),
            })
            .await
            .expect("login succeeds");

        assert_eq!(logged_in.id, user.id);
        let updated = external_accounts
            .get_by_provider_identity(
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main",
                "remote-user",
            )
            .await
            .expect("load account")
            .expect("account exists");
        assert_eq!(updated.status, scryer_domain::ExternalAccountStatus::Active);
        assert_eq!(updated.username, "fresh-name");
        assert_eq!(updated.display_name.as_deref(), Some("Fresh Name"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://jellyfin.example.test/avatar")
        );
        assert!(updated.verified_at.is_some());
    }

    #[tokio::test]
    async fn active_login_refreshes_external_account_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        };
        let now = Utc::now();
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![
            UserExternalAccount {
                id: "active-account".to_string(),
                user_id: user.id.clone(),
                provider: ExternalAccountProvider::Plex,
                connection_id: "plex-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "old-name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::Active,
                verified_at: Some(now),
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
        );

        app.login_verified_external_account(VerifiedExternalIdentity {
            provider: ExternalAccountProvider::Plex,
            connection_id: "plex-main".to_string(),
            external_user_id: "remote-user".to_string(),
            username: "fresh-plex".to_string(),
            display_name: Some("Fresh Plex".to_string()),
            avatar_url: Some("https://plex.example.test/avatar".to_string()),
        })
        .await
        .expect("login succeeds");

        let updated = external_accounts
            .get_by_provider_identity(ExternalAccountProvider::Plex, "plex-main", "remote-user")
            .await
            .expect("load account")
            .expect("account exists");
        assert_eq!(updated.status, scryer_domain::ExternalAccountStatus::Active);
        assert_eq!(updated.username, "fresh-plex");
        assert_eq!(updated.display_name.as_deref(), Some("Fresh Plex"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://plex.example.test/avatar")
        );
    }

    #[tokio::test]
    async fn verified_identity_must_match_requested_connection() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let settings = AuthProviderSettings {
            allowed_providers: vec![ExternalAccountProvider::Jellyfin],
            provider_login_enabled: vec![ExternalAccountProvider::Jellyfin],
            provider_linking_enabled: vec![ExternalAccountProvider::Jellyfin],
            allowed_jellyfin_connection_ids: vec!["jellyfin-main".to_string()],
            allowed_plex_connection_ids: vec![],
            allowed_jellyfin_connections: auth_provider_connections_from_ids(vec![
                "jellyfin-main".to_string(),
            ]),
            allowed_plex_connections: vec![],
        };

        let result = app.ensure_verified_identity_matches_request(
            &settings,
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
            AuthProviderUse::Login,
            &VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-other".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "remote-user".to_string(),
                display_name: None,
                avatar_url: None,
            },
        );

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("did not match"))
        );
    }

    #[tokio::test]
    async fn verified_identity_must_match_requested_provider() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let settings = AuthProviderSettings {
            allowed_providers: vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex,
            ],
            provider_login_enabled: vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex,
            ],
            provider_linking_enabled: vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex,
            ],
            allowed_jellyfin_connection_ids: vec!["jellyfin-main".to_string()],
            allowed_plex_connection_ids: vec!["plex-main".to_string()],
            allowed_jellyfin_connections: auth_provider_connections_from_ids(vec![
                "jellyfin-main".to_string(),
            ]),
            allowed_plex_connections: auth_provider_connections_from_ids(vec![
                "plex-main".to_string(),
            ]),
        };

        let result = app.ensure_verified_identity_matches_request(
            &settings,
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
            AuthProviderUse::Login,
            &VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Plex,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "remote-user".to_string(),
                display_name: None,
                avatar_url: None,
            },
        );

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("provider"))
        );
    }
}
