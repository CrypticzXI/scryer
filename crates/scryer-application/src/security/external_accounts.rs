use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalAuthRuntimeConnection {
    pub id: String,
    pub provider: scryer_domain::ExternalAccountProvider,
    pub display_name: String,
    pub login_enabled: bool,
    pub linking_enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExternalAuthRuntimeSettings {
    pub login_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub linking_providers: Vec<scryer_domain::ExternalAccountProvider>,
    pub connections: Vec<ExternalAuthRuntimeConnection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalAuthUse {
    Login,
    Linking,
    Invite,
}

fn normalize_provider_username(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

impl AppUseCase {
    pub async fn get_external_auth_runtime_settings(
        &self,
    ) -> AppResult<ExternalAuthRuntimeSettings> {
        let media_connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?;
        let mut connections = Vec::new();
        let mut login_providers = Vec::new();
        let mut linking_providers = Vec::new();

        for connection in media_connections
            .into_iter()
            .filter(|connection| connection.enabled)
        {
            if !connection.login_enabled && !connection.linking_enabled {
                continue;
            }
            let provider = match connection.provider {
                scryer_domain::MediaServerProvider::Jellyfin => {
                    scryer_domain::ExternalAccountProvider::Jellyfin
                }
                scryer_domain::MediaServerProvider::Plex => {
                    if connection.machine_id.is_none() {
                        continue;
                    }
                    scryer_domain::ExternalAccountProvider::Plex
                }
                scryer_domain::MediaServerProvider::Emby => continue,
            };
            if connection.login_enabled && !login_providers.contains(&provider) {
                login_providers.push(provider.clone());
            }
            if connection.linking_enabled && !linking_providers.contains(&provider) {
                linking_providers.push(provider.clone());
            }
            connections.push(ExternalAuthRuntimeConnection {
                id: connection.id,
                provider,
                display_name: connection.display_name,
                login_enabled: connection.login_enabled,
                linking_enabled: connection.linking_enabled,
            });
        }

        Ok(ExternalAuthRuntimeSettings {
            login_providers,
            linking_providers,
            connections,
        })
    }

    async fn auth_connection_for_use(
        &self,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: &str,
        usage: ExternalAuthUse,
    ) -> AppResult<scryer_domain::MediaServerConnection> {
        let connection_id = connection_id.trim();
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id)
            .await?
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "{} connection is not configured",
                    provider.as_str()
                ))
            })?;
        let expected_provider = match provider {
            scryer_domain::ExternalAccountProvider::Jellyfin => {
                scryer_domain::MediaServerProvider::Jellyfin
            }
            scryer_domain::ExternalAccountProvider::Plex => {
                scryer_domain::MediaServerProvider::Plex
            }
        };
        if connection.provider != expected_provider {
            return Err(AppError::Validation(format!(
                "{} connection is not configured for external auth",
                provider.as_str()
            )));
        }
        if !connection.enabled {
            return Err(AppError::Validation(format!(
                "{} connection is disabled",
                provider.as_str()
            )));
        }
        let enabled = match usage {
            ExternalAuthUse::Login | ExternalAuthUse::Invite => connection.login_enabled,
            ExternalAuthUse::Linking => connection.linking_enabled,
        };
        if !enabled {
            return Err(AppError::Validation(format!(
                "{} is not enabled for {}",
                provider.as_str(),
                match usage {
                    ExternalAuthUse::Login => "login",
                    ExternalAuthUse::Linking => "linking",
                    ExternalAuthUse::Invite => "invites",
                }
            )));
        }
        if provider == scryer_domain::ExternalAccountProvider::Plex
            && connection.machine_id.is_none()
        {
            return Err(AppError::Validation(
                "Plex server discovery is required before using Plex for auth".into(),
            ));
        }
        Ok(connection)
    }

    fn ensure_verified_identity_matches_request(
        &self,
        expected_provider: &scryer_domain::ExternalAccountProvider,
        expected_connection_id: &str,
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
        Ok(())
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

    pub async fn list_external_account_invites(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::UserExternalAccount>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        let users = self.services.identity.users.list_all().await?;
        let mut accounts = Vec::new();
        for user in users {
            accounts.extend(
                self.services
                    .identity
                    .external_accounts
                    .list_by_user_id(&user.id)
                    .await?,
            );
        }
        accounts.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.provider.as_str().cmp(right.provider.as_str()))
                .then_with(|| left.username.cmp(&right.username))
        });
        Ok(accounts)
    }

    pub async fn create_external_account_invite(
        &self,
        actor: &User,
        user_id: &str,
        provider: scryer_domain::ExternalAccountProvider,
        connection_id: String,
        provider_user_identifier: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        let connection_id = normalize_connection_id(connection_id);
        let provider_user_identifier = provider_user_identifier.trim().to_string();
        if provider_user_identifier.is_empty() {
            return Err(AppError::Validation(
                "provider user identifier is required".into(),
            ));
        }
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Invite)
            .await?;
        self.services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;

        let (external_user_id, username, display_name, avatar_url) = match provider {
            scryer_domain::ExternalAccountProvider::Jellyfin => {
                let resolved_user = if let Some(api_key) = connection.api_key.as_deref() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .list_jellyfin_users(
                            &connection.base_url,
                            api_key,
                            Some(&provider_user_identifier),
                        )
                        .await
                        .ok()
                        .and_then(|users| {
                            users.into_iter().find(|user| {
                                user.id.eq_ignore_ascii_case(&provider_user_identifier)
                                    || user
                                        .username
                                        .eq_ignore_ascii_case(&provider_user_identifier)
                            })
                        })
                } else {
                    None
                };
                let username = resolved_user
                    .as_ref()
                    .map(|user| user.username.clone())
                    .unwrap_or_else(|| provider_user_identifier.clone());
                if self
                    .services
                    .identity
                    .external_accounts
                    .get_pending_claim_by_provider_username(
                        provider.clone(),
                        &connection_id,
                        &username,
                    )
                    .await?
                    .is_some()
                {
                    return Err(AppError::Validation(
                        "Jellyfin account already has a pending invite".into(),
                    ));
                }
                if let Some(user) = resolved_user {
                    if self
                        .services
                        .identity
                        .external_accounts
                        .get_by_provider_identity(provider.clone(), &connection_id, &user.id)
                        .await?
                        .is_some()
                    {
                        return Err(AppError::Validation(
                            "Jellyfin account is already linked or invited".into(),
                        ));
                    }
                    (Some(user.id), username, user.display_name, user.avatar_url)
                } else {
                    (None, username, None, None)
                }
            }
            scryer_domain::ExternalAccountProvider::Plex => (
                Some(provider_user_identifier.clone()),
                provider_user_identifier,
                None,
                None,
            ),
        };

        let mut account = scryer_domain::UserExternalAccount::pending_claim(
            user_id.to_string(),
            provider,
            connection_id,
            external_user_id,
            username,
        );
        account.display_name = display_name;
        account.avatar_url = avatar_url;
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
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Linking)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(
                &connection_id,
                connection.machine_id.as_deref(),
                &plex_auth_token,
            )
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.link_verified_external_account(actor, verified).await
    }

    pub async fn link_jellyfin_account(
        &self,
        actor: &User,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Linking)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &connection.base_url, &username, &password)
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.link_verified_external_account(actor, verified).await
    }

    async fn link_verified_external_account(
        &self,
        actor: &User,
        verified: VerifiedExternalIdentity,
    ) -> AppResult<scryer_domain::UserExternalAccount> {
        let mut existing = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                verified.provider.clone(),
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?;

        if existing.is_none()
            && verified.provider == scryer_domain::ExternalAccountProvider::Jellyfin
        {
            existing = self
                .services
                .identity
                .external_accounts
                .get_pending_claim_by_provider_username(
                    verified.provider.clone(),
                    &verified.connection_id,
                    &normalize_provider_username(&verified.username),
                )
                .await?;
        }

        if let Some(mut existing) = existing {
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
            existing.external_user_id = Some(verified.external_user_id);
            existing.username = verified.username;
            existing.display_name = verified.display_name;
            existing.avatar_url = verified.avatar_url;
            existing.status = scryer_domain::ExternalAccountStatus::Active;
            let now = Utc::now();
            existing.verified_at = Some(now);
            existing.updated_at = now;
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
            external_user_id: Some(verified.external_user_id),
            username: verified.username,
            display_name: verified.display_name,
            avatar_url: verified.avatar_url,
            status: scryer_domain::ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: None,
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
        let provider = scryer_domain::ExternalAccountProvider::Plex;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Login)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_plex(
                &connection_id,
                connection.machine_id.as_deref(),
                &plex_auth_token,
            )
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.login_verified_external_account(verified, connection)
            .await
    }

    pub async fn federated_login_with_jellyfin(
        &self,
        connection_id: String,
        username: String,
        password: String,
    ) -> AppResult<User> {
        let provider = scryer_domain::ExternalAccountProvider::Jellyfin;
        let connection_id = normalize_connection_id(connection_id);
        let connection = self
            .auth_connection_for_use(provider.clone(), &connection_id, ExternalAuthUse::Login)
            .await?;
        let verified = self
            .services
            .integrations
            .external_identity_verifier
            .verify_jellyfin(&connection_id, &connection.base_url, &username, &password)
            .await?;
        self.ensure_verified_identity_matches_request(&provider, &connection_id, &verified)?;
        self.login_verified_external_account(verified, connection)
            .await
    }

    async fn login_verified_external_account(
        &self,
        verified: VerifiedExternalIdentity,
        connection: scryer_domain::MediaServerConnection,
    ) -> AppResult<User> {
        let provider = verified.provider.clone();
        let mut account = self
            .services
            .identity
            .external_accounts
            .get_by_provider_identity(
                provider.clone(),
                &verified.connection_id,
                &verified.external_user_id,
            )
            .await?;

        if account.is_none() && provider == scryer_domain::ExternalAccountProvider::Jellyfin {
            account = self
                .services
                .identity
                .external_accounts
                .get_pending_claim_by_provider_username(
                    provider,
                    &verified.connection_id,
                    &normalize_provider_username(&verified.username),
                )
                .await?;
        }

        let mut auto_added_user = None;
        let mut account = if let Some(account) = account {
            account
        } else if connection.auto_add_enabled {
            let (user, account) = self
                .create_auto_added_external_account(&verified, &connection)
                .await?;
            auto_added_user = Some(user);
            account
        } else {
            return Err(AppError::Unauthorized(
                "external account is not invited".into(),
            ));
        };

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
        account.external_user_id = Some(verified.external_user_id);
        account.username = verified.username;
        account.display_name = verified.display_name;
        account.avatar_url = verified.avatar_url;
        let now = Utc::now();
        account.verified_at = Some(now);
        account.last_login_at = Some(now);
        account.updated_at = now;
        self.services
            .identity
            .external_accounts
            .update(account.clone())
            .await?;

        let user = if let Some(user) = auto_added_user {
            user
        } else {
            self.services
                .identity
                .users
                .get_by_id(&account.user_id)
                .await?
                .ok_or_else(|| AppError::NotFound(format!("user {}", account.user_id)))?
        };
        self.cache_jwt_signing_key(&user).await?;
        Ok(user)
    }

    async fn create_auto_added_external_account(
        &self,
        verified: &VerifiedExternalIdentity,
        connection: &scryer_domain::MediaServerConnection,
    ) -> AppResult<(User, scryer_domain::UserExternalAccount)> {
        if !connection.auto_add_enabled {
            return Err(AppError::Unauthorized(
                "external account is not invited".into(),
            ));
        }
        let username = self.unique_auto_added_username(&verified.username).await?;
        let user = User {
            id: scryer_domain::Id::new().0,
            username,
            password_hash: None,
            account_kind: scryer_domain::UserAccountKind::ExternalAutoProvisioned,
            authorization: Default::default(),
        };
        let grants = connection
            .default_library_grants
            .iter()
            .map(|grant| scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: grant.library_id.clone(),
                permissions: grant.permissions,
            })
            .collect();

        let now = Utc::now();
        self.services
            .identity
            .external_accounts
            .create_auto_added_user_with_account(
                user.clone(),
                connection.default_app_permissions,
                grants,
                scryer_domain::UserExternalAccount {
                    id: scryer_domain::Id::new().0,
                    user_id: user.id,
                    provider: verified.provider.clone(),
                    connection_id: verified.connection_id.clone(),
                    external_user_id: Some(verified.external_user_id.clone()),
                    username: verified.username.clone(),
                    display_name: verified.display_name.clone(),
                    avatar_url: verified.avatar_url.clone(),
                    status: scryer_domain::ExternalAccountStatus::Active,
                    verified_at: Some(now),
                    last_login_at: Some(now),
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
    }

    async fn unique_auto_added_username(&self, provider_username: &str) -> AppResult<String> {
        let base = provider_username.trim();
        let base = if base.is_empty() { "media-user" } else { base };
        if self
            .services
            .identity
            .users
            .get_by_username(base)
            .await?
            .is_none()
        {
            return Ok(base.to_string());
        }
        for suffix in 2..=9999 {
            let candidate = format!("{base}-{suffix}");
            if self
                .services
                .identity
                .users
                .get_by_username(&candidate)
                .await?
                .is_none()
            {
                return Ok(candidate);
            }
        }
        Err(AppError::Validation(
            "could not allocate a Scryer username for the external account".into(),
        ))
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

fn normalize_connection_id(value: impl AsRef<str>) -> String {
    value.as_ref().trim().to_string()
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
        MediaServerConnectionRepository, SettingsRepository, UserExternalAccountRepository,
        UserRepository,
    };
    use scryer_domain::{
        AppPermission, AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus,
        LibraryPermissionMask, MediaServerConnection, UserAuthorization, UserExternalAccount,
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

    #[derive(Default)]
    struct TestMediaServerConnectionRepository {
        connections: Mutex<Vec<MediaServerConnection>>,
    }

    impl TestExternalAccountRepository {
        fn new(accounts: Vec<UserExternalAccount>) -> Self {
            Self {
                accounts: Mutex::new(accounts),
            }
        }
    }

    impl TestMediaServerConnectionRepository {
        fn new(connections: Vec<MediaServerConnection>) -> Self {
            Self {
                connections: Mutex::new(connections),
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
                        && account.external_user_id.as_deref() == Some(external_user_id)
                })
                .cloned())
        }

        async fn get_pending_claim_by_provider_username(
            &self,
            provider: ExternalAccountProvider,
            connection_id: &str,
            username: &str,
        ) -> AppResult<Option<UserExternalAccount>> {
            let normalized_username = normalize_provider_username(username);
            Ok(self
                .accounts
                .lock()
                .await
                .iter()
                .find(|account| {
                    account.provider == provider
                        && account.connection_id == connection_id
                        && account.external_user_id.is_none()
                        && account.status == ExternalAccountStatus::PendingClaim
                        && normalize_provider_username(&account.username) == normalized_username
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

        async fn create_auto_added_user_with_account(
            &self,
            user: User,
            _app_permissions: AppPermissionMask,
            _library_grants: Vec<scryer_domain::LibraryGrant>,
            account: UserExternalAccount,
        ) -> AppResult<(User, UserExternalAccount)> {
            let account = self.create(account).await?;
            Ok((user, account))
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
    impl MediaServerConnectionRepository for TestMediaServerConnectionRepository {
        async fn list(
            &self,
            provider: Option<scryer_domain::MediaServerProvider>,
        ) -> AppResult<Vec<MediaServerConnection>> {
            Ok(self
                .connections
                .lock()
                .await
                .iter()
                .filter(|connection| {
                    provider
                        .as_ref()
                        .is_none_or(|provider| &connection.provider == provider)
                })
                .cloned()
                .collect())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<MediaServerConnection>> {
            Ok(self
                .connections
                .lock()
                .await
                .iter()
                .find(|connection| connection.id == id)
                .cloned())
        }

        async fn create(
            &self,
            connection: MediaServerConnection,
        ) -> AppResult<MediaServerConnection> {
            self.connections.lock().await.push(connection.clone());
            Ok(connection)
        }

        async fn update(
            &self,
            connection: MediaServerConnection,
        ) -> AppResult<MediaServerConnection> {
            let mut connections = self.connections.lock().await;
            if let Some(existing) = connections
                .iter_mut()
                .find(|candidate| candidate.id == connection.id)
            {
                *existing = connection.clone();
            }
            Ok(connection)
        }

        async fn delete(&self, id: &str) -> AppResult<()> {
            self.connections
                .lock()
                .await
                .retain(|connection| connection.id != id);
            Ok(())
        }

        async fn has_external_accounts(&self, _: &str) -> AppResult<bool> {
            Ok(false)
        }

        async fn has_notification_channels(&self, _: &str) -> AppResult<bool> {
            Ok(false)
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

        async fn auth_session_version(&self, _user_id: &str) -> AppResult<Option<String>> {
            Ok(None)
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
        test_app_with_identity_and_media_servers(
            settings,
            users,
            external_accounts,
            vec![
                test_media_server_connection(
                    scryer_domain::MediaServerProvider::Jellyfin,
                    "jellyfin-main",
                ),
                test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main"),
            ],
        )
    }

    fn test_app_with_identity_and_media_servers(
        settings: Arc<dyn SettingsRepository>,
        users: Arc<dyn UserRepository>,
        external_accounts: Arc<dyn UserExternalAccountRepository>,
        media_server_connections: Vec<MediaServerConnection>,
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
        .with_media_server_connection_store(Arc::new(TestMediaServerConnectionRepository::new(
            media_server_connections,
        )))
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
            account_kind: Default::default(),
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

    fn regular_user(id: &str) -> User {
        User {
            id: id.to_string(),
            username: id.to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        }
    }

    fn active_jellyfin_account(user_id: &str) -> UserExternalAccount {
        let now = Utc::now();
        UserExternalAccount {
            id: format!("{user_id}-jellyfin"),
            user_id: user_id.to_string(),
            provider: ExternalAccountProvider::Jellyfin,
            connection_id: "jellyfin-main".to_string(),
            external_user_id: Some(format!("{user_id}-remote")),
            username: user_id.to_string(),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::Active,
            verified_at: Some(now),
            last_login_at: Some(now),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_media_server_connection(
        provider: scryer_domain::MediaServerProvider,
        id: &str,
    ) -> scryer_domain::MediaServerConnection {
        scryer_domain::MediaServerConnection {
            id: id.to_string(),
            provider: provider.clone(),
            display_name: id.to_string(),
            base_url: match provider {
                scryer_domain::MediaServerProvider::Plex => "https://plex.tv".to_string(),
                scryer_domain::MediaServerProvider::Jellyfin => {
                    "https://jellyfin.example.test".to_string()
                }
                scryer_domain::MediaServerProvider::Emby => "https://emby.example.test".to_string(),
            },
            enabled: true,
            login_enabled: true,
            linking_enabled: true,
            auto_add_enabled: false,
            default_app_permissions: AppPermissionMask::NONE,
            default_library_grants: Vec::new(),
            machine_id: match provider {
                scryer_domain::MediaServerProvider::Plex => Some("machine-1".to_string()),
                scryer_domain::MediaServerProvider::Jellyfin
                | scryer_domain::MediaServerProvider::Emby => None,
            },
            api_key: None,
            path_mappings: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn jellyfin_invite_uses_username_without_external_id() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let external_accounts = Arc::new(TestExternalAccountRepository::default());
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            external_accounts.clone(),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main".to_string(),
                " JellyUser ".to_string(),
            )
            .await
            .expect("create jellyfin invite");

        assert_eq!(account.provider, ExternalAccountProvider::Jellyfin);
        assert_eq!(account.connection_id, "jellyfin-main");
        assert_eq!(account.external_user_id, None);
        assert_eq!(account.username, "JellyUser");
        assert_eq!(account.status, ExternalAccountStatus::PendingClaim);
        assert_eq!(account.last_login_at, None);
    }

    #[tokio::test]
    async fn duplicate_pending_jellyfin_username_for_connection_is_rejected() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let now = Utc::now();
        let external_accounts = Arc::new(TestExternalAccountRepository::new(vec![
            UserExternalAccount {
                id: "pending-account".to_string(),
                user_id: target.id.clone(),
                provider: ExternalAccountProvider::Jellyfin,
                connection_id: "jellyfin-main".to_string(),
                external_user_id: None,
                username: "JellyUser".to_string(),
                display_name: None,
                avatar_url: None,
                status: ExternalAccountStatus::PendingClaim,
                verified_at: None,
                last_login_at: None,
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            external_accounts,
        );
        let result = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Jellyfin,
                "jellyfin-main".to_string(),
                " jellyuser ".to_string(),
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("pending invite"))
        );
    }

    #[tokio::test]
    async fn auto_added_external_user_cannot_be_given_local_password() {
        let admin = admin_user();
        let mut target = regular_user("jellyfin-user");
        target.account_kind = scryer_domain::UserAccountKind::ExternalAutoProvisioned;
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let result = app
            .set_user_password(&admin, &target.id, "local-password".to_string())
            .await;

        assert!(
            matches!(
                result,
                Err(AppError::Validation(ref message))
                    if message == "externally managed users cannot set a Scryer password"
            ),
            "expected externally managed password validation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn passwordless_linked_local_user_can_be_given_initial_password() {
        let admin = admin_user();
        let target = regular_user("linked-passwordless-user");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let updated = app
            .set_user_password(&admin, &target.id, "local-password".to_string())
            .await
            .expect("local linked user can receive an initial password");

        assert_eq!(updated.account_kind, scryer_domain::UserAccountKind::Local);
        assert!(updated.password_hash.is_some());
    }

    #[tokio::test]
    async fn passwordless_local_user_can_set_initial_own_password() {
        let target = regular_user("passwordless-admin");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
        );

        let updated = app
            .set_initial_own_password(&target, "local-password".to_string())
            .await
            .expect("local passwordless user can set an initial password");

        assert_eq!(updated.account_kind, scryer_domain::UserAccountKind::Local);
        assert!(updated.password_hash.is_some());
    }

    #[tokio::test]
    async fn auto_added_external_user_cannot_set_initial_own_password() {
        let mut target = regular_user("auto-added-self");
        target.account_kind = scryer_domain::UserAccountKind::ExternalAutoProvisioned;
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let result = app
            .set_initial_own_password(&target, "local-password".to_string())
            .await;

        assert!(
            matches!(
                result,
                Err(AppError::Validation(ref message))
                    if message == "externally managed users cannot set a Scryer password"
            ),
            "expected externally managed password validation, got {result:?}"
        );
    }

    #[tokio::test]
    async fn password_backed_linked_user_can_rotate_local_password() {
        let admin = admin_user();
        let mut target = regular_user("linked-user");
        target.password_hash = Some("existing-local-password-hash".to_string());
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::new(vec![
                active_jellyfin_account(&target.id),
            ])),
        );

        let updated = app
            .set_user_password(&admin, &target.id, "new-local-password".to_string())
            .await
            .expect("linked local user should be allowed to rotate password");

        assert!(updated.password_hash.is_some());
        assert_ne!(updated.password_hash, target.password_hash);
    }

    #[tokio::test]
    async fn plex_invite_remains_id_based() {
        let admin = admin_user();
        let target = regular_user("user-1");
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![target.clone()])),
            Arc::new(TestExternalAccountRepository::default()),
        );
        let account = app
            .create_external_account_invite(
                &admin,
                &target.id,
                ExternalAccountProvider::Plex,
                "plex-main".to_string(),
                "plex-user-1".to_string(),
            )
            .await
            .expect("create plex invite");

        assert_eq!(account.provider, ExternalAccountProvider::Plex);
        assert_eq!(account.connection_id, "plex-main");
        assert_eq!(account.external_user_id.as_deref(), Some("plex-user-1"));
        assert_eq!(account.username, "plex-user-1");
        assert_eq!(account.status, ExternalAccountStatus::PendingClaim);
        assert_eq!(account.last_login_at, None);
    }

    #[tokio::test]
    async fn admin_can_list_external_account_invites_across_users() {
        let admin = admin_user();
        let first = regular_user("user-1");
        let second = regular_user("user-2");
        let external_accounts = Arc::new(TestExternalAccountRepository::default());
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![first.clone(), second.clone()])),
            external_accounts,
        );
        app.create_external_account_invite(
            &admin,
            &first.id,
            ExternalAccountProvider::Jellyfin,
            "jellyfin-main".to_string(),
            "first-jellyfin".to_string(),
        )
        .await
        .expect("create first invite");
        app.create_external_account_invite(
            &admin,
            &second.id,
            ExternalAccountProvider::Plex,
            "plex-main".to_string(),
            "second-plex".to_string(),
        )
        .await
        .expect("create second invite");

        let invites = app
            .list_external_account_invites(&admin)
            .await
            .expect("list external account invites");
        let user_ids = invites
            .iter()
            .map(|account| account.user_id.as_str())
            .collect::<Vec<_>>();
        assert!(user_ids.contains(&first.id.as_str()));
        assert!(user_ids.contains(&second.id.as_str()));

        let result = app.list_external_account_invites(&first).await;
        assert!(matches!(result, Err(AppError::Unauthorized(_))));
    }

    #[tokio::test]
    async fn external_auth_runtime_settings_are_derived_from_media_servers() {
        let mut jellyfin_login = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-login",
        );
        jellyfin_login.display_name = "Jellyfin Login".to_string();
        jellyfin_login.login_enabled = true;
        jellyfin_login.linking_enabled = false;

        let mut jellyfin_link = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-link",
        );
        jellyfin_link.display_name = "Jellyfin Link".to_string();
        jellyfin_link.login_enabled = false;
        jellyfin_link.linking_enabled = true;

        let mut disabled_jellyfin = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-disabled",
        );
        disabled_jellyfin.enabled = false;

        let mut auth_flags_off = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-off",
        );
        auth_flags_off.login_enabled = false;
        auth_flags_off.linking_enabled = false;

        let mut plex =
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main");
        plex.display_name = "Plex Main".to_string();

        let mut plex_without_machine = test_media_server_connection(
            scryer_domain::MediaServerProvider::Plex,
            "plex-no-machine",
        );
        plex_without_machine.machine_id = None;

        let emby =
            test_media_server_connection(scryer_domain::MediaServerProvider::Emby, "emby-main");

        let app = test_app_with_identity_and_media_servers(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::default()),
            vec![
                jellyfin_login,
                jellyfin_link,
                disabled_jellyfin,
                auth_flags_off,
                plex,
                plex_without_machine,
                emby,
            ],
        );

        let settings = app
            .get_external_auth_runtime_settings()
            .await
            .expect("load runtime settings");

        assert_eq!(
            settings.login_providers,
            vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex
            ]
        );
        assert_eq!(
            settings.linking_providers,
            vec![
                ExternalAccountProvider::Jellyfin,
                ExternalAccountProvider::Plex
            ]
        );
        assert_eq!(
            settings
                .connections
                .iter()
                .map(|connection| connection.id.as_str())
                .collect::<Vec<_>>(),
            vec!["jellyfin-login", "jellyfin-link", "plex-main"]
        );
        assert_eq!(settings.connections[0].display_name, "Jellyfin Login");
        assert!(settings.connections[0].login_enabled);
        assert!(!settings.connections[0].linking_enabled);
        assert!(!settings.connections[1].login_enabled);
        assert!(settings.connections[1].linking_enabled);
        assert_eq!(
            settings.connections[2].provider,
            ExternalAccountProvider::Plex
        );
    }

    #[tokio::test]
    async fn external_auth_runtime_settings_empty_when_no_connection_exposes_auth() {
        let mut disabled = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-disabled",
        );
        disabled.enabled = false;

        let mut flags_off = test_media_server_connection(
            scryer_domain::MediaServerProvider::Jellyfin,
            "jellyfin-off",
        );
        flags_off.login_enabled = false;
        flags_off.linking_enabled = false;

        let mut plex_without_machine = test_media_server_connection(
            scryer_domain::MediaServerProvider::Plex,
            "plex-no-machine",
        );
        plex_without_machine.machine_id = None;

        let app = test_app_with_identity_and_media_servers(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(NullUserRepository),
            Arc::new(TestExternalAccountRepository::default()),
            vec![disabled, flags_off, plex_without_machine],
        );

        let settings = app
            .get_external_auth_runtime_settings()
            .await
            .expect("load runtime settings");

        assert!(settings.login_providers.is_empty());
        assert!(settings.linking_providers.is_empty());
        assert!(settings.connections.is_empty());
    }

    #[tokio::test]
    async fn link_rejects_connection_not_on_allowlist_before_verification() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));
        let admin = admin_user();

        let result = app
            .link_jellyfin_account(
                &admin,
                "jellyfin-other".to_string(),
                "someone".to_string(),
                "secret".to_string(),
            )
            .await;

        assert!(
            matches!(result, Err(AppError::Validation(message)) if message.contains("not configured"))
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
                    external_user_id: Some("remote-user".to_string()),
                    username: "remote-user".to_string(),
                    display_name: None,
                    avatar_url: None,
                    status: scryer_domain::ExternalAccountStatus::Disabled,
                    verified_at: None,
                    last_login_at: None,
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
    async fn link_jellyfin_claims_pending_username_invite_without_external_id() {
        let admin = admin_user();
        let now = Utc::now();
        let app = test_app_with_external_accounts(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestExternalAccountRepository::new(vec![
                UserExternalAccount {
                    id: "pending-account".to_string(),
                    user_id: admin.id.clone(),
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: None,
                    username: "Remote User".to_string(),
                    display_name: None,
                    avatar_url: None,
                    status: scryer_domain::ExternalAccountStatus::PendingClaim,
                    verified_at: None,
                    last_login_at: None,
                    created_at: now,
                    updated_at: now,
                },
            ])),
        );

        let account = app
            .link_verified_external_account(
                &admin,
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "jellyfin-user-id".to_string(),
                    username: "remote user".to_string(),
                    display_name: Some("Remote User".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/avatar.png".to_string()),
                },
            )
            .await
            .expect("link pending Jellyfin invite");

        assert_eq!(account.id, "pending-account");
        assert_eq!(account.user_id, admin.id);
        assert_eq!(
            account.external_user_id.as_deref(),
            Some("jellyfin-user-id")
        );
        assert_eq!(account.status, ExternalAccountStatus::Active);
        assert_eq!(account.display_name.as_deref(), Some("Remote User"));
        assert_eq!(
            account.avatar_url.as_deref(),
            Some("https://jellyfin.example.test/avatar.png")
        );
        assert!(account.verified_at.is_some());
        assert!(account.last_login_at.is_none());
    }

    #[tokio::test]
    async fn pending_claim_login_activates_and_refreshes_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            account_kind: Default::default(),
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
                external_user_id: None,
                username: "Fresh-Name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::PendingClaim,
                verified_at: None,
                last_login_at: None,
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
            .login_verified_external_account(
                VerifiedExternalIdentity {
                    provider: ExternalAccountProvider::Jellyfin,
                    connection_id: "jellyfin-main".to_string(),
                    external_user_id: "remote-user".to_string(),
                    username: "fresh-name".to_string(),
                    display_name: Some("Fresh Name".to_string()),
                    avatar_url: Some("https://jellyfin.example.test/avatar".to_string()),
                },
                test_media_server_connection(
                    scryer_domain::MediaServerProvider::Jellyfin,
                    "jellyfin-main",
                ),
            )
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
        assert_eq!(updated.external_user_id.as_deref(), Some("remote-user"));
        assert_eq!(updated.username, "fresh-name");
        assert_eq!(updated.display_name.as_deref(), Some("Fresh Name"));
        assert_eq!(
            updated.avatar_url.as_deref(),
            Some("https://jellyfin.example.test/avatar")
        );
        assert!(updated.verified_at.is_some());
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn active_login_refreshes_external_account_metadata() {
        let user = User {
            id: "user-1".to_string(),
            username: "local-user".to_string(),
            password_hash: None,
            account_kind: Default::default(),
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
                external_user_id: Some("remote-user".to_string()),
                username: "old-name".to_string(),
                display_name: None,
                avatar_url: None,
                status: scryer_domain::ExternalAccountStatus::Active,
                verified_at: Some(now),
                last_login_at: None,
                created_at: now,
                updated_at: now,
            },
        ]));
        let app = test_app_with_identity(
            Arc::new(TestSettingsRepository::default()),
            Arc::new(TestUserRepository::new(vec![user.clone()])),
            external_accounts.clone(),
        );

        app.login_verified_external_account(
            VerifiedExternalIdentity {
                provider: ExternalAccountProvider::Plex,
                connection_id: "plex-main".to_string(),
                external_user_id: "remote-user".to_string(),
                username: "fresh-plex".to_string(),
                display_name: Some("Fresh Plex".to_string()),
                avatar_url: Some("https://plex.example.test/avatar".to_string()),
            },
            test_media_server_connection(scryer_domain::MediaServerProvider::Plex, "plex-main"),
        )
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
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn verified_identity_must_match_requested_connection() {
        let app = test_app(Arc::new(TestSettingsRepository::default()));

        let result = app.ensure_verified_identity_matches_request(
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
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

        let result = app.ensure_verified_identity_matches_request(
            &ExternalAccountProvider::Jellyfin,
            "jellyfin-main",
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
