use super::*;

#[cfg(not(test))]
const MEDIA_SERVER_USER_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(test)]
const MEDIA_SERVER_USER_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(50);

#[derive(Clone, Debug)]
pub struct MediaServerConnectionDraft {
    pub provider: MediaServerProvider,
    pub display_name: String,
    pub base_url: String,
    pub enabled: bool,
    pub login_enabled: bool,
    pub linking_enabled: bool,
    pub auto_add_enabled: bool,
    pub default_app_permissions: AppPermissionMask,
    pub default_library_grants: Vec<MediaServerDefaultLibraryGrant>,
    pub machine_id: Option<String>,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub path_mappings: Vec<MediaServerPathMapping>,
}

#[derive(Clone, Debug, Default)]
pub struct MediaServerConnectionPatch {
    pub id: String,
    pub provider: Option<MediaServerProvider>,
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    pub enabled: Option<bool>,
    pub login_enabled: Option<bool>,
    pub linking_enabled: Option<bool>,
    pub auto_add_enabled: Option<bool>,
    pub default_app_permissions: Option<AppPermissionMask>,
    pub default_library_grants: Option<Vec<MediaServerDefaultLibraryGrant>>,
    pub machine_id: Option<String>,
    pub clear_machine_id: bool,
    pub plex_auth_token: Option<String>,
    pub plex_server_id: Option<String>,
    pub api_key: Option<String>,
    pub clear_api_key: bool,
    pub admin_username: Option<String>,
    pub admin_password: Option<String>,
    pub path_mappings: Option<Vec<MediaServerPathMapping>>,
}

impl AppUseCase {
    pub async fn list_media_server_connections(
        &self,
        actor: &User,
        provider: Option<MediaServerProvider>,
    ) -> AppResult<Vec<MediaServerConnection>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .media_server_connections
            .list(provider)
            .await
    }

    pub async fn get_media_server_connection(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<MediaServerConnection>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .media_server_connections
            .get_by_id(id.trim())
            .await
    }

    pub async fn create_media_server_connection(
        &self,
        actor: &User,
        draft: MediaServerConnectionDraft,
    ) -> AppResult<MediaServerConnection> {
        self.require_media_server_permission(actor, &draft).await?;
        let now = Utc::now();
        let machine_id = self
            .resolve_plex_machine_id(
                &draft.provider,
                None,
                draft.machine_id.clone(),
                draft.plex_auth_token.as_deref(),
                draft.plex_server_id.as_deref(),
            )
            .await?;
        let api_key_supplied = draft
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let api_key = match draft.provider {
            MediaServerProvider::Plex => draft.api_key.clone().or(draft.plex_auth_token.clone()),
            MediaServerProvider::Jellyfin | MediaServerProvider::Emby => draft.api_key.clone(),
        };
        let mut connection = self
            .normalize_media_server_connection(
                scryer_domain::Id::new().0,
                draft.provider,
                draft.display_name,
                draft.base_url,
                draft.enabled,
                draft.login_enabled,
                draft.linking_enabled,
                draft.auto_add_enabled,
                draft.default_app_permissions,
                draft.default_library_grants,
                machine_id,
                api_key,
                draft.path_mappings,
                now,
                now,
            )
            .await?;
        connection.api_key = self
            .jellyfin_api_key_from_credentials_or_input(
                &connection,
                draft.admin_username.as_deref(),
                draft.admin_password.as_deref(),
                connection.api_key.clone(),
                api_key_supplied,
            )
            .await?;
        self.test_media_server_connection_internal(
            &connection,
            draft.plex_auth_token.as_deref(),
            false,
        )
        .await?;

        let created = self
            .services
            .integrations
            .media_server_connections
            .create(connection)
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "media_server_connection",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(created)
    }

    pub async fn update_media_server_connection(
        &self,
        actor: &User,
        patch: MediaServerConnectionPatch,
    ) -> AppResult<MediaServerConnection> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let id = patch.id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::Validation(
                "media server connection id is required".into(),
            ));
        }
        let existing = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(&id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media server connection {id}")))?;

        let provider = patch
            .provider
            .clone()
            .unwrap_or_else(|| existing.provider.clone());
        if media_server_update_requires_manage_permissions(&existing, &patch, &provider) {
            self.require_app_permission(actor, AppPermission::ManagePermissions)
                .await?;
        }
        if provider != existing.provider
            && self
                .services
                .integrations
                .media_server_connections
                .has_external_accounts(&id)
                .await?
        {
            return Err(AppError::Validation(
                "cannot change provider for a connection with linked accounts".into(),
            ));
        }
        let requested_machine_id = if patch.clear_machine_id {
            None
        } else {
            patch.machine_id.clone().or(existing.machine_id.clone())
        };
        let machine_id = self
            .resolve_plex_machine_id(
                &provider,
                existing.machine_id.as_deref(),
                requested_machine_id,
                patch.plex_auth_token.as_deref(),
                patch.plex_server_id.as_deref(),
            )
            .await?;
        let api_key_supplied = patch
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| !value.is_empty());
        let existing_api_key = if existing.provider == provider {
            existing.api_key.clone()
        } else {
            None
        };
        let api_key = if patch.clear_api_key {
            None
        } else if provider == MediaServerProvider::Plex {
            patch
                .plex_auth_token
                .clone()
                .or(patch.api_key.clone())
                .or(existing_api_key)
        } else {
            patch.api_key.clone().or(existing_api_key)
        };

        let mut connection = self
            .normalize_media_server_connection(
                id.clone(),
                provider,
                patch
                    .display_name
                    .unwrap_or_else(|| existing.display_name.clone()),
                patch.base_url.unwrap_or_else(|| existing.base_url.clone()),
                patch.enabled.unwrap_or(existing.enabled),
                patch.login_enabled.unwrap_or(existing.login_enabled),
                patch.linking_enabled.unwrap_or(existing.linking_enabled),
                patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled),
                patch
                    .default_app_permissions
                    .unwrap_or(existing.default_app_permissions),
                patch
                    .default_library_grants
                    .unwrap_or_else(|| existing.default_library_grants.clone()),
                machine_id,
                api_key,
                patch
                    .path_mappings
                    .unwrap_or_else(|| existing.path_mappings.clone()),
                existing.created_at,
                Utc::now(),
            )
            .await?;

        connection.api_key = self
            .jellyfin_api_key_from_credentials_or_input(
                &connection,
                patch.admin_username.as_deref(),
                patch.admin_password.as_deref(),
                connection.api_key.clone(),
                api_key_supplied,
            )
            .await?;
        self.test_media_server_connection_internal(
            &connection,
            patch.plex_auth_token.as_deref(),
            false,
        )
        .await?;

        let updated = self
            .services
            .integrations
            .media_server_connections
            .update(connection)
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "media_server_connection",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub async fn delete_media_server_connection(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if self
            .services
            .integrations
            .media_server_connections
            .has_external_accounts(id)
            .await?
        {
            return Err(AppError::Validation(
                "media server connection is referenced by linked accounts; disable it instead"
                    .into(),
            ));
        }
        if self
            .services
            .integrations
            .media_server_connections
            .has_notification_channels(id)
            .await?
        {
            return Err(AppError::Validation(
                "media server connection is referenced by notification channels; disable it instead"
                    .into(),
            ));
        }
        self.services
            .integrations
            .media_server_connections
            .delete(id)
            .await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "media_server_connection",
            Some(id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(())
    }

    pub async fn test_media_server_connection(
        &self,
        actor: &User,
        id: &str,
        plex_auth_token: Option<&str>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media server connection {}", id.trim())))?;
        self.test_media_server_connection_internal(&connection, plex_auth_token, true)
            .await
    }

    pub async fn discover_plex_media_servers(
        &self,
        actor: &User,
        plex_auth_token: &str,
    ) -> AppResult<Vec<PlexServerDiscovery>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .external_identity_verifier
            .discover_plex_servers(plex_auth_token)
            .await
    }

    pub async fn list_jellyfin_server_users(
        &self,
        actor: &User,
        connection_id: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<JellyfinServerUser>> {
        self.require_app_permission(actor, AppPermission::ManageUsers)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(connection_id.trim())
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("media server connection {connection_id}"))
            })?;
        if connection.provider != MediaServerProvider::Jellyfin {
            return Err(AppError::Validation(
                "Jellyfin user listing requires a Jellyfin connection".into(),
            ));
        }
        let api_key = connection.api_key.as_deref().ok_or_else(|| {
            AppError::Validation(
                "Jellyfin user listing requires a saved API key; save an API key to load Jellyfin users"
                    .into(),
            )
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .list_jellyfin_users(&connection.base_url, api_key, search)
            .await
    }

    pub async fn list_media_server_users(
        &self,
        actor: &User,
        search: Option<&str>,
    ) -> AppResult<Vec<MediaServerUserGroup>> {
        self.require_app_permission(actor, AppPermission::ManageUsers)
            .await?;
        let search = search.map(str::trim).filter(|value| !value.is_empty());
        let mut connections = self
            .services
            .integrations
            .media_server_connections
            .list(None)
            .await?;
        connections.sort_by(|left, right| {
            left.provider
                .as_str()
                .cmp(right.provider.as_str())
                .then_with(|| {
                    left.display_name
                        .to_ascii_lowercase()
                        .cmp(&right.display_name.to_ascii_lowercase())
                })
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut tasks = tokio::task::JoinSet::new();
        let verifier = Arc::clone(&self.services.integrations.external_identity_verifier);
        let search = search.map(ToString::to_string);
        for connection in connections {
            if !connection.enabled || !connection.login_enabled {
                continue;
            }

            let Some(provider) = external_account_provider_for_media_server(&connection.provider)
            else {
                continue;
            };

            let verifier = Arc::clone(&verifier);
            let search = search.clone();
            tasks.spawn(async move {
                list_media_server_user_group_with_timeout(verifier, connection, provider, search)
                    .await
            });
        }

        let mut groups = Vec::new();
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(group) => groups.push(group),
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        "media server user lookup task failed before returning a group"
                    );
                }
            }
        }

        groups.sort_by(|left, right| {
            left.provider
                .as_str()
                .cmp(right.provider.as_str())
                .then_with(|| {
                    left.connection_name
                        .to_ascii_lowercase()
                        .cmp(&right.connection_name.to_ascii_lowercase())
                })
                .then_with(|| left.connection_id.cmp(&right.connection_id))
        });

        Ok(groups)
    }

    async fn require_media_server_permission(
        &self,
        actor: &User,
        draft: &MediaServerConnectionDraft,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        if !draft.default_app_permissions.is_empty()
            || draft
                .default_library_grants
                .iter()
                .any(|grant| !grant.permissions.is_empty())
        {
            self.require_app_permission(actor, AppPermission::ManagePermissions)
                .await?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "normalization mirrors the API payload"
    )]
    async fn normalize_media_server_connection(
        &self,
        id: String,
        provider: MediaServerProvider,
        display_name: String,
        base_url: String,
        enabled: bool,
        login_enabled: bool,
        linking_enabled: bool,
        auto_add_enabled: bool,
        default_app_permissions: AppPermissionMask,
        default_library_grants: Vec<MediaServerDefaultLibraryGrant>,
        machine_id: Option<String>,
        api_key: Option<String>,
        path_mappings: Vec<MediaServerPathMapping>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> AppResult<MediaServerConnection> {
        let id = id.trim().to_string();
        if id.is_empty() {
            return Err(AppError::Validation(
                "media server connection id is required".into(),
            ));
        }
        let base_url = normalize_media_server_base_url(&provider, base_url)?;
        let display_name = display_name.trim().to_string();
        let display_name = if display_name.is_empty() {
            default_media_server_display_name(&provider).to_string()
        } else {
            display_name
        };
        let machine_id = normalize_optional_string(machine_id);
        let api_key = normalize_optional_string(api_key);
        let path_mappings = normalize_path_mappings(path_mappings)?;
        let default_library_grants = normalize_default_library_grants(default_library_grants);

        let (
            login_enabled,
            linking_enabled,
            auto_add_enabled,
            default_app_permissions,
            default_library_grants,
        ) = if provider.supports_external_auth() {
            (
                login_enabled,
                linking_enabled,
                auto_add_enabled,
                default_app_permissions,
                default_library_grants,
            )
        } else {
            (false, false, false, AppPermissionMask::NONE, Vec::new())
        };
        if provider == MediaServerProvider::Plex
            && (login_enabled || linking_enabled || auto_add_enabled)
            && machine_id.is_none()
        {
            return Err(AppError::Validation(
                "Discover and select a Plex server before enabling login, linking, or auto-add"
                    .into(),
            ));
        }

        Ok(MediaServerConnection {
            id,
            provider: provider.clone(),
            display_name,
            base_url,
            enabled,
            login_enabled,
            linking_enabled,
            auto_add_enabled,
            default_app_permissions,
            default_library_grants,
            machine_id: match provider {
                MediaServerProvider::Plex => machine_id,
                MediaServerProvider::Jellyfin | MediaServerProvider::Emby => None,
            },
            api_key: match provider {
                MediaServerProvider::Jellyfin
                | MediaServerProvider::Emby
                | MediaServerProvider::Plex => api_key,
            },
            path_mappings: match provider {
                MediaServerProvider::Jellyfin
                | MediaServerProvider::Emby
                | MediaServerProvider::Plex => path_mappings,
            },
            created_at,
            updated_at,
        })
    }

    async fn jellyfin_api_key_from_credentials_or_input(
        &self,
        connection: &MediaServerConnection,
        admin_username: Option<&str>,
        admin_password: Option<&str>,
        api_key: Option<String>,
        api_key_supplied: bool,
    ) -> AppResult<Option<String>> {
        if connection.provider != MediaServerProvider::Jellyfin {
            return Ok(api_key);
        }
        let admin_username = admin_username
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let admin_password = admin_password
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if api_key_supplied && (admin_username.is_some() || admin_password.is_some()) {
            return Err(AppError::Validation(
                "choose either a Jellyfin API key or Jellyfin admin login credentials, not both"
                    .into(),
            ));
        }
        match (admin_username, admin_password) {
            (Some(username), Some(password)) => {
                let generated = self
                    .services
                    .integrations
                    .external_identity_verifier
                    .exchange_jellyfin_admin_api_key(
                        &connection.id,
                        &connection.base_url,
                        username,
                        password,
                    )
                    .await?;
                Ok(Some(generated))
            }
            (None, None) => Ok(api_key),
            _ => Err(AppError::Validation(
                "both Jellyfin admin username and password are required".into(),
            )),
        }
    }

    async fn test_media_server_connection_internal(
        &self,
        connection: &MediaServerConnection,
        plex_auth_token: Option<&str>,
        require_plex_token: bool,
    ) -> AppResult<()> {
        match connection.provider {
            MediaServerProvider::Jellyfin => {
                self.services
                    .integrations
                    .external_identity_verifier
                    .test_jellyfin_connection(&connection.base_url)
                    .await?;
                if let Some(api_key) = connection.api_key.as_deref() {
                    self.services
                        .integrations
                        .external_identity_verifier
                        .test_jellyfin_api_key(&connection.base_url, api_key)
                        .await?;
                }
            }
            MediaServerProvider::Plex => {
                let has_auth_capability = connection.login_enabled
                    || connection.linking_enabled
                    || connection.auto_add_enabled;
                if has_auth_capability && connection.machine_id.is_none() {
                    return Err(AppError::Validation(
                        "Discover and select a Plex server before enabling login, linking, or auto-add"
                            .into(),
                    ));
                }
                let token = plex_auth_token
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if require_plex_token && token.is_none() {
                    return Err(AppError::Validation(
                        "Sign in with Plex to test this connection".into(),
                    ));
                }
                if let Some(token) = token {
                    let servers = self
                        .services
                        .integrations
                        .external_identity_verifier
                        .discover_plex_servers(token)
                        .await?;
                    if let Some(machine_id) = connection.machine_id.as_deref()
                        && !servers.iter().any(|server| server.id == machine_id)
                    {
                        return Err(AppError::Unauthorized(
                            "Plex account does not have access to the selected server".into(),
                        ));
                    }
                }
            }
            MediaServerProvider::Emby => {}
        }
        Ok(())
    }

    async fn resolve_plex_machine_id(
        &self,
        provider: &MediaServerProvider,
        existing_machine_id: Option<&str>,
        requested_machine_id: Option<String>,
        plex_auth_token: Option<&str>,
        plex_server_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        if *provider != MediaServerProvider::Plex {
            return Ok(None);
        }

        let token = plex_auth_token
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let selected_server_id = plex_server_id
            .map(str::trim)
            .filter(|value| !value.is_empty());

        if let Some(token) = token {
            let servers = self
                .services
                .integrations
                .external_identity_verifier
                .discover_plex_servers(token)
                .await?;
            if servers.is_empty() {
                return Err(AppError::Validation(
                    "Plex did not return any accessible servers for that account".into(),
                ));
            }
            let selected = if let Some(selected_server_id) = selected_server_id {
                servers
                    .iter()
                    .find(|server| server.id == selected_server_id)
                    .ok_or_else(|| {
                        AppError::Validation(
                            "The selected Plex server was not returned by Plex discovery".into(),
                        )
                    })?
            } else if servers.len() == 1 {
                &servers[0]
            } else {
                return Err(AppError::Validation(
                    "Select a Plex server before saving this connection".into(),
                ));
            };
            return Ok(Some(selected.id.clone()));
        }

        if selected_server_id.is_some() {
            return Err(AppError::Validation(
                "Sign in with Plex before selecting a Plex server".into(),
            ));
        }

        Ok(normalize_optional_string(requested_machine_id)
            .or_else(|| existing_machine_id.map(ToString::to_string)))
    }
}

fn media_server_update_requires_manage_permissions(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    let effective_app_permissions = if provider.supports_external_auth() {
        patch
            .default_app_permissions
            .unwrap_or(existing.default_app_permissions)
    } else {
        AppPermissionMask::NONE
    };
    let effective_library_grants = if provider.supports_external_auth() {
        normalize_default_library_grants(
            patch
                .default_library_grants
                .clone()
                .unwrap_or_else(|| existing.default_library_grants.clone()),
        )
    } else {
        Vec::new()
    };

    if media_server_patch_changes_default_grants_to_non_empty(
        existing,
        patch,
        effective_app_permissions,
        &effective_library_grants,
    ) {
        return true;
    }

    if !media_server_default_grants_are_non_empty(
        effective_app_permissions,
        &effective_library_grants,
    ) {
        return false;
    }

    media_server_patch_activates_external_auth_surface(existing, patch, provider)
        || media_server_patch_changes_external_auth_identity(existing, patch, provider)
}

fn media_server_patch_changes_default_grants_to_non_empty(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    effective_app_permissions: AppPermissionMask,
    effective_library_grants: &[MediaServerDefaultLibraryGrant],
) -> bool {
    if patch.default_app_permissions.is_none() && patch.default_library_grants.is_none() {
        return false;
    }
    if !media_server_default_grants_are_non_empty(
        effective_app_permissions,
        effective_library_grants,
    ) {
        return false;
    }

    let existing_app_permissions = if existing.provider.supports_external_auth() {
        existing.default_app_permissions
    } else {
        AppPermissionMask::NONE
    };
    let existing_library_grants = if existing.provider.supports_external_auth() {
        normalize_default_library_grants(existing.default_library_grants.clone())
    } else {
        Vec::new()
    };

    effective_app_permissions != existing_app_permissions
        || !media_server_default_library_grants_equal(
            effective_library_grants,
            &existing_library_grants,
        )
}

fn media_server_patch_activates_external_auth_surface(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    let resulting_enabled = patch.enabled.unwrap_or(existing.enabled);
    let resulting_login_enabled = patch.login_enabled.unwrap_or(existing.login_enabled);
    let resulting_linking_enabled = patch.linking_enabled.unwrap_or(existing.linking_enabled);
    let resulting_auto_add_enabled = patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled);
    let existing_surface = media_server_external_auth_surface_usable(
        &existing.provider,
        existing.enabled,
        existing.login_enabled,
        existing.linking_enabled,
        existing.auto_add_enabled,
    );
    let resulting_surface = media_server_external_auth_surface_usable(
        provider,
        resulting_enabled,
        resulting_login_enabled,
        resulting_linking_enabled,
        resulting_auto_add_enabled,
    );

    if !resulting_surface {
        return false;
    }
    if !existing_surface {
        return true;
    }

    patch
        .enabled
        .is_some_and(|enabled| enabled && !existing.enabled)
        || patch
            .login_enabled
            .is_some_and(|enabled| enabled && !existing.login_enabled)
        || patch
            .linking_enabled
            .is_some_and(|enabled| enabled && !existing.linking_enabled)
        || patch
            .auto_add_enabled
            .is_some_and(|enabled| enabled && !existing.auto_add_enabled)
}

fn media_server_patch_changes_external_auth_identity(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
    provider: &MediaServerProvider,
) -> bool {
    if !media_server_external_auth_surface_usable(
        provider,
        patch.enabled.unwrap_or(existing.enabled),
        patch.login_enabled.unwrap_or(existing.login_enabled),
        patch.linking_enabled.unwrap_or(existing.linking_enabled),
        patch.auto_add_enabled.unwrap_or(existing.auto_add_enabled),
    ) {
        return false;
    }
    if patch
        .provider
        .as_ref()
        .is_some_and(|provider| provider != &existing.provider)
    {
        return true;
    }
    if patch.base_url.as_ref().is_some_and(|base_url| {
        media_server_base_url_changed(provider, &existing.base_url, base_url)
    }) {
        return true;
    }

    match provider {
        MediaServerProvider::Plex => media_server_plex_identity_changed(existing, patch),
        MediaServerProvider::Jellyfin => media_server_jellyfin_identity_changed(existing, patch),
        MediaServerProvider::Emby => false,
    }
}

fn media_server_external_auth_surface_usable(
    provider: &MediaServerProvider,
    enabled: bool,
    login_enabled: bool,
    linking_enabled: bool,
    auto_add_enabled: bool,
) -> bool {
    provider.supports_external_auth()
        && enabled
        && (login_enabled || linking_enabled || auto_add_enabled)
}

fn media_server_default_grants_are_non_empty(
    app_permissions: AppPermissionMask,
    library_grants: &[MediaServerDefaultLibraryGrant],
) -> bool {
    !app_permissions.is_empty()
        || library_grants
            .iter()
            .any(|grant| !grant.permissions.is_empty())
}

fn media_server_default_library_grants_equal(
    left: &[MediaServerDefaultLibraryGrant],
    right: &[MediaServerDefaultLibraryGrant],
) -> bool {
    let mut left = media_server_default_library_grant_entries(left);
    let mut right = media_server_default_library_grant_entries(right);
    left.sort_by(|a, b| a.0.cmp(&b.0));
    right.sort_by(|a, b| a.0.cmp(&b.0));
    left == right
}

fn media_server_default_library_grant_entries(
    grants: &[MediaServerDefaultLibraryGrant],
) -> Vec<(String, scryer_domain::LibraryPermissionMask)> {
    grants
        .iter()
        .filter(|grant| !grant.permissions.is_empty())
        .map(|grant| (grant.library_id.clone(), grant.permissions))
        .collect()
}

fn media_server_base_url_changed(
    provider: &MediaServerProvider,
    existing_base_url: &str,
    base_url: &str,
) -> bool {
    match normalize_media_server_base_url(provider, base_url.to_string()) {
        Ok(normalized) => normalized != existing_base_url,
        Err(_) => true,
    }
}

fn media_server_plex_identity_changed(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
) -> bool {
    (patch.clear_machine_id && existing.machine_id.is_some())
        || (patch.clear_api_key && existing.api_key.is_some())
        || patch.machine_id.as_ref().is_some_and(|machine_id| {
            normalize_optional_string(Some(machine_id.clone())) != existing.machine_id
        })
        || patch.api_key.as_ref().is_some_and(|api_key| {
            normalize_optional_string(Some(api_key.clone())) != existing.api_key
        })
        || option_has_non_empty_text(patch.plex_auth_token.as_deref())
        || option_has_non_empty_text(patch.plex_server_id.as_deref())
}

fn media_server_jellyfin_identity_changed(
    existing: &MediaServerConnection,
    patch: &MediaServerConnectionPatch,
) -> bool {
    (patch.clear_api_key && existing.api_key.is_some())
        || patch.api_key.as_ref().is_some_and(|api_key| {
            normalize_optional_string(Some(api_key.clone())) != existing.api_key
        })
        || option_has_non_empty_text(patch.admin_username.as_deref())
        || option_has_non_empty_text(patch.admin_password.as_deref())
}

fn option_has_non_empty_text(value: Option<&str>) -> bool {
    value.map(str::trim).is_some_and(|value| !value.is_empty())
}

fn default_media_server_display_name(provider: &MediaServerProvider) -> &'static str {
    match provider {
        MediaServerProvider::Jellyfin => "Jellyfin",
        MediaServerProvider::Plex => "Plex",
        MediaServerProvider::Emby => "Emby",
    }
}

async fn list_media_server_user_group_with_timeout(
    verifier: Arc<dyn ExternalIdentityVerifier>,
    connection: MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
    search: Option<String>,
) -> MediaServerUserGroup {
    let timeout_group = empty_media_server_user_group(&connection, provider.clone());
    match tokio::time::timeout(
        MEDIA_SERVER_USER_LIST_TIMEOUT,
        list_media_server_user_group(verifier, connection, provider, search),
    )
    .await
    {
        Ok(group) => group,
        Err(_) => {
            let mut group = timeout_group;
            group.status = MediaServerUserGroupStatus::Error;
            group.error_message = Some(format!(
                "Timed out after {} loading users from this server",
                media_server_user_list_timeout_label()
            ));
            group
        }
    }
}

fn media_server_user_list_timeout_label() -> String {
    let seconds = MEDIA_SERVER_USER_LIST_TIMEOUT.as_secs();
    if seconds > 0 {
        return format!("{seconds} seconds");
    }
    format!(
        "{} milliseconds",
        MEDIA_SERVER_USER_LIST_TIMEOUT.as_millis()
    )
}

async fn list_media_server_user_group(
    verifier: Arc<dyn ExternalIdentityVerifier>,
    connection: MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
    search: Option<String>,
) -> MediaServerUserGroup {
    let mut group = empty_media_server_user_group(&connection, provider.clone());
    match provider {
        scryer_domain::ExternalAccountProvider::Jellyfin => {
            let Some(api_key) = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                group.status = MediaServerUserGroupStatus::MissingCredentials;
                group.error_message =
                    Some("Save a Jellyfin API key to load users from this server".into());
                return group;
            };

            match verifier
                .list_jellyfin_users(&connection.base_url, api_key, search.as_deref())
                .await
            {
                Ok(users) => {
                    group.users = users.into_iter().map(MediaServerUser::from).collect();
                }
                Err(error) => {
                    group.status = MediaServerUserGroupStatus::Error;
                    group.error_message = Some(error.to_string());
                }
            }
        }
        scryer_domain::ExternalAccountProvider::Plex => {
            if connection
                .machine_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                group.status = MediaServerUserGroupStatus::Error;
                group.error_message =
                    Some("Discover and select a Plex server before loading users".into());
                return group;
            }

            let Some(plex_auth_token) = connection
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                group.status = MediaServerUserGroupStatus::MissingCredentials;
                group.error_message =
                    Some("Save a Plex token to load users from this server".into());
                return group;
            };

            match verifier
                .list_plex_users(plex_auth_token, search.as_deref())
                .await
            {
                Ok(users) => {
                    group.users = users.into_iter().map(MediaServerUser::from).collect();
                }
                Err(error) => {
                    group.status = MediaServerUserGroupStatus::Error;
                    group.error_message = Some(error.to_string());
                }
            }
        }
    }

    group.users.sort_by(|left, right| {
        left.username
            .to_ascii_lowercase()
            .cmp(&right.username.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    group
}

fn empty_media_server_user_group(
    connection: &MediaServerConnection,
    provider: scryer_domain::ExternalAccountProvider,
) -> MediaServerUserGroup {
    MediaServerUserGroup {
        connection_id: connection.id.clone(),
        connection_name: connection.display_name.clone(),
        provider,
        status: MediaServerUserGroupStatus::Ready,
        error_message: None,
        users: Vec::new(),
    }
}

fn external_account_provider_for_media_server(
    provider: &MediaServerProvider,
) -> Option<scryer_domain::ExternalAccountProvider> {
    match provider {
        MediaServerProvider::Jellyfin => Some(scryer_domain::ExternalAccountProvider::Jellyfin),
        MediaServerProvider::Plex => Some(scryer_domain::ExternalAccountProvider::Plex),
        MediaServerProvider::Emby => None,
    }
}

fn normalize_media_server_base_url(
    provider: &MediaServerProvider,
    value: String,
) -> AppResult<String> {
    let value = value.trim();
    let value = if value.is_empty() && *provider == MediaServerProvider::Plex {
        "https://plex.tv"
    } else {
        value
    };
    if value.is_empty() {
        return Err(AppError::Validation(
            "media server base URL is required".into(),
        ));
    }
    let parsed = url::Url::parse(value)
        .map_err(|_| AppError::Validation("media server base URL is invalid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Validation(
            "media server base URL must be an HTTP or HTTPS URL".into(),
        ));
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(AppError::Validation(
            "media server base URL must not include a query or fragment".into(),
        ));
    }
    Ok(parsed.as_str().trim_end_matches('/').to_string())
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_path_mappings(
    values: Vec<MediaServerPathMapping>,
) -> AppResult<Vec<MediaServerPathMapping>> {
    let mut normalized = Vec::new();
    for (index, mapping) in values.into_iter().enumerate() {
        let source_path = mapping.source_path.trim().to_string();
        let destination_path = mapping.destination_path.trim().to_string();
        if source_path.is_empty() || destination_path.is_empty() {
            continue;
        }
        if normalized
            .iter()
            .any(|existing: &MediaServerPathMapping| existing.source_path == source_path)
        {
            return Err(AppError::Validation(
                "media server path mappings must have unique source paths".into(),
            ));
        }
        normalized.push(MediaServerPathMapping {
            source_path,
            destination_path,
            sort_order: index as i64,
        });
    }
    Ok(normalized)
}

fn normalize_default_library_grants(
    values: Vec<MediaServerDefaultLibraryGrant>,
) -> Vec<MediaServerDefaultLibraryGrant> {
    let mut normalized = Vec::new();
    for grant in values {
        let library_id = grant.library_id.trim().to_string();
        if library_id.is_empty() {
            continue;
        }
        if let Some(existing) =
            normalized
                .iter_mut()
                .find(|existing: &&mut MediaServerDefaultLibraryGrant| {
                    existing.library_id == library_id
                })
        {
            existing.permissions = grant.permissions.normalized_for_storage();
        } else {
            normalized.push(MediaServerDefaultLibraryGrant {
                library_id,
                permissions: grant.permissions.normalized_for_storage(),
            });
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Mutex;

    use super::*;
    use crate::null_repositories::NullSettingsRepository;
    use crate::null_repositories::test_nulls::{
        NullDownloadClient, NullDownloadClientConfigRepository, NullIndexerClient,
        NullQualityProfileRepository, NullReleaseAttemptRepository, NullShowRepository,
        NullTitleRepository, NullUserRepository,
    };
    use crate::services::AppServices;
    use scryer_domain::{LibraryPermission, LibraryPermissionMask, UserAuthorization};

    #[derive(Default)]
    struct TestMediaServerConnectionRepository {
        connections: Mutex<Vec<MediaServerConnection>>,
    }

    impl TestMediaServerConnectionRepository {
        fn new(connections: Vec<MediaServerConnection>) -> Self {
            Self {
                connections: Mutex::new(connections),
            }
        }
    }

    #[async_trait::async_trait]
    impl MediaServerConnectionRepository for TestMediaServerConnectionRepository {
        async fn list(
            &self,
            provider: Option<MediaServerProvider>,
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

    struct TestIndexerConfigRepository;

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
            Err(AppError::Repository(
                "indexer config update is not configured".into(),
            ))
        }

        async fn delete(&self, _: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct NoopExternalIdentityVerifier;

    #[async_trait::async_trait]
    impl ExternalIdentityVerifier for NoopExternalIdentityVerifier {
        async fn verify_plex(
            &self,
            _: &str,
            _: Option<&str>,
            _: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            Err(AppError::Repository(
                "plex verification is not configured".into(),
            ))
        }

        async fn discover_plex_servers(&self, _: &str) -> AppResult<Vec<PlexServerDiscovery>> {
            Ok(vec![PlexServerDiscovery {
                id: "machine-2".to_string(),
                name: "Plex 2".to_string(),
            }])
        }

        async fn verify_jellyfin(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<VerifiedExternalIdentity> {
            Err(AppError::Repository(
                "jellyfin verification is not configured".into(),
            ))
        }

        async fn test_jellyfin_connection(&self, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn test_jellyfin_api_key(&self, _: &str, _: &str) -> AppResult<()> {
            Ok(())
        }

        async fn exchange_jellyfin_admin_api_key(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> AppResult<String> {
            Ok("generated-api-key".to_string())
        }

        async fn list_jellyfin_users(
            &self,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> AppResult<Vec<JellyfinServerUser>> {
            Ok(Vec::new())
        }

        async fn list_plex_users(
            &self,
            _: &str,
            _: Option<&str>,
        ) -> AppResult<Vec<PlexServerUser>> {
            Ok(Vec::new())
        }
    }

    struct TestNotificationPluginProvider;

    impl NotificationPluginProvider for TestNotificationPluginProvider {
        fn client_for_channel(
            &self,
            _: &scryer_domain::NotificationChannelConfig,
        ) -> Option<Arc<dyn NotificationClient>> {
            None
        }

        fn available_provider_types(&self) -> Vec<String> {
            vec!["plex".to_string()]
        }

        fn config_fields_for_provider(
            &self,
            provider_type: &str,
        ) -> Vec<scryer_domain::ConfigFieldDef> {
            if provider_type != "plex" {
                return Vec::new();
            }

            vec![scryer_domain::ConfigFieldDef {
                key: "base_url".to_string(),
                label: "Base URL".to_string(),
                field_type: scryer_domain::ConfigFieldType::String,
                required: true,
                default_value: None,
                value_source: Default::default(),
                role: None,
                host_binding: None,
                options: Vec::new(),
                help_text: None,
            }]
        }

        fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
            (provider_type == "plex").then(|| "Plex Media Server".to_string())
        }
    }

    struct TestDomainEventRepository;

    #[async_trait::async_trait]
    impl DomainEventRepository for TestDomainEventRepository {
        async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
            Ok(DomainEvent {
                sequence: 1,
                event_id: event.event_id,
                occurred_at: event.occurred_at,
                actor_user_id: event.actor_user_id,
                title_id: event.title_id,
                facet: event.facet,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                schema_version: event.schema_version,
                stream: event.stream,
                payload: event.payload,
            })
        }

        async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
            let mut appended = Vec::new();
            for event in events {
                appended.push(self.append(event).await?);
            }
            Ok(appended)
        }

        async fn list(&self, _: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
            Ok(Vec::new())
        }

        async fn count_title_history_page_events(
            &self,
            _: Option<&[TitleHistoryEventType]>,
            _: Option<&[String]>,
            _: Option<&str>,
        ) -> AppResult<i64> {
            Ok(0)
        }

        async fn list_title_history_page_events(
            &self,
            _: Option<&[TitleHistoryEventType]>,
            _: Option<&[String]>,
            _: Option<&str>,
            _: usize,
            _: usize,
        ) -> AppResult<Vec<DomainEvent>> {
            Ok(Vec::new())
        }

        async fn list_after_sequence(&self, _: i64, _: usize) -> AppResult<Vec<DomainEvent>> {
            Ok(Vec::new())
        }

        async fn delete_for_title_ids(&self, _: &[String]) -> AppResult<u32> {
            Ok(0)
        }

        async fn get_subscriber_offset(&self, _: &str) -> AppResult<i64> {
            Ok(0)
        }

        async fn set_subscriber_offset(&self, _: &str, _: i64) -> AppResult<()> {
            Ok(())
        }
    }

    fn app_with_connections(connections: Vec<MediaServerConnection>) -> AppUseCase {
        let services = AppServices::builder(
            Arc::new(NullTitleRepository),
            Arc::new(NullShowRepository),
            Arc::new(NullUserRepository),
            Arc::new(TestIndexerConfigRepository),
            Arc::new(NullIndexerClient),
            Arc::new(NullDownloadClient),
            Arc::new(NullDownloadClientConfigRepository),
            Arc::new(NullReleaseAttemptRepository),
            Arc::new(NullSettingsRepository),
            Arc::new(NullQualityProfileRepository),
            String::new(),
        )
        .with_external_identity_verifier(Arc::new(NoopExternalIdentityVerifier))
        .with_media_server_connection_store(Arc::new(TestMediaServerConnectionRepository::new(
            connections,
        )))
        .with_notification_provider(Arc::new(TestNotificationPluginProvider))
        .with_domain_events(Arc::new(TestDomainEventRepository))
        .build_partial_for_tests();

        AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(FacetRegistry::new()),
        )
    }

    fn app_with_connection(connection: MediaServerConnection) -> AppUseCase {
        app_with_connections(vec![connection])
    }

    fn user_with_permissions(username: &str, app: AppPermissionMask) -> User {
        User {
            id: username.to_string(),
            username: username.to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        }
    }

    fn system_settings_user() -> User {
        user_with_permissions(
            "system-settings",
            AppPermissionMask::from_permissions([AppPermission::ManageSystemSettings]),
        )
    }

    fn permission_manager_user() -> User {
        user_with_permissions(
            "permission-manager",
            AppPermissionMask::from_permissions([
                AppPermission::ManageSystemSettings,
                AppPermission::ManagePermissions,
            ]),
        )
    }

    fn grant_bearing_jellyfin_connection() -> MediaServerConnection {
        let now = Utc::now();
        MediaServerConnection {
            id: "jellyfin-main".to_string(),
            provider: MediaServerProvider::Jellyfin,
            display_name: "Jellyfin".to_string(),
            base_url: "https://jellyfin.example.test".to_string(),
            enabled: true,
            login_enabled: true,
            linking_enabled: false,
            auto_add_enabled: true,
            default_app_permissions: AppPermissionMask::from_permissions([
                AppPermission::ManageCatalogSettings,
            ]),
            default_library_grants: vec![MediaServerDefaultLibraryGrant {
                library_id: "movies".to_string(),
                permissions: LibraryPermissionMask::from_permissions([LibraryPermission::View]),
            }],
            machine_id: None,
            api_key: Some("api-key-1".to_string()),
            path_mappings: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    fn grant_bearing_plex_connection() -> MediaServerConnection {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.id = "plex-main".to_string();
        connection.provider = MediaServerProvider::Plex;
        connection.display_name = "Plex".to_string();
        connection.base_url = "https://plex.tv".to_string();
        connection.machine_id = Some("machine-1".to_string());
        connection.api_key = None;
        connection
    }

    fn empty_update_patch(id: &str) -> MediaServerConnectionPatch {
        MediaServerConnectionPatch {
            id: id.to_string(),
            ..Default::default()
        }
    }

    fn assert_unauthorized(error: AppError) {
        assert!(
            matches!(error, AppError::Unauthorized(_)),
            "expected unauthorized error, got {error:?}",
        );
    }

    #[tokio::test]
    async fn jellyfin_user_listing_without_api_key_points_to_picker_setup() {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.api_key = None;
        let app = app_with_connection(connection);
        let user = user_with_permissions(
            "manage-users",
            AppPermissionMask::from_permissions([AppPermission::ManageUsers]),
        );

        let error = app
            .list_jellyfin_server_users(&user, "jellyfin-main", None)
            .await
            .expect_err("missing Jellyfin API key should fail");
        let message = error.to_string();

        assert!(message.contains("save an API key to load Jellyfin users"));
        assert!(!message.contains("manually"));
    }

    #[tokio::test]
    async fn media_server_update_rejects_enabling_grant_bearing_connection_without_manage_permissions()
     {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.enabled = false;
        let app = app_with_connection(connection);
        let mut patch = empty_update_patch("jellyfin-main");
        patch.enabled = Some(true);

        let error = app
            .update_media_server_connection(&system_settings_user(), patch)
            .await
            .expect_err("system settings user should not activate preserved grants");

        assert_unauthorized(error);
    }

    #[tokio::test]
    async fn media_server_update_rejects_enabling_auth_flags_with_grants_without_manage_permissions()
     {
        let user = system_settings_user();
        for (name, patch) in [
            ("login", {
                let mut connection = grant_bearing_jellyfin_connection();
                connection.login_enabled = false;
                connection.linking_enabled = false;
                connection.auto_add_enabled = false;
                let mut patch = empty_update_patch("jellyfin-main");
                patch.login_enabled = Some(true);
                (connection, patch)
            }),
            ("linking", {
                let mut connection = grant_bearing_jellyfin_connection();
                connection.login_enabled = false;
                connection.linking_enabled = false;
                connection.auto_add_enabled = false;
                let mut patch = empty_update_patch("jellyfin-main");
                patch.linking_enabled = Some(true);
                (connection, patch)
            }),
            ("auto add", {
                let mut connection = grant_bearing_jellyfin_connection();
                connection.auto_add_enabled = false;
                let mut patch = empty_update_patch("jellyfin-main");
                patch.auto_add_enabled = Some(true);
                (connection, patch)
            }),
        ] {
            let app = app_with_connection(patch.0);
            let error = app
                .update_media_server_connection(&user, patch.1)
                .await
                .expect_err(&format!("{name} should require ManagePermissions"));
            assert_unauthorized(error);
        }
    }

    #[tokio::test]
    async fn media_server_update_rejects_auth_identity_changes_with_grants_without_manage_permissions()
     {
        let user = system_settings_user();
        for (name, connection, patch) in [
            ("base url", grant_bearing_jellyfin_connection(), {
                let mut patch = empty_update_patch("jellyfin-main");
                patch.base_url = Some("https://other-jellyfin.example.test".to_string());
                patch
            }),
            ("api key", grant_bearing_jellyfin_connection(), {
                let mut patch = empty_update_patch("jellyfin-main");
                patch.api_key = Some("api-key-2".to_string());
                patch
            }),
            ("admin credentials", grant_bearing_jellyfin_connection(), {
                let mut patch = empty_update_patch("jellyfin-main");
                patch.admin_username = Some("admin".to_string());
                patch.admin_password = Some("password".to_string());
                patch
            }),
            ("plex machine", grant_bearing_plex_connection(), {
                let mut patch = empty_update_patch("plex-main");
                patch.machine_id = Some("machine-2".to_string());
                patch
            }),
            ("plex api key", grant_bearing_plex_connection(), {
                let mut patch = empty_update_patch("plex-main");
                patch.api_key = Some("api-key-2".to_string());
                patch
            }),
            (
                "plex clear api key",
                {
                    let mut connection = grant_bearing_plex_connection();
                    connection.api_key = Some("api-key-1".to_string());
                    connection
                },
                {
                    let mut patch = empty_update_patch("plex-main");
                    patch.clear_api_key = true;
                    patch
                },
            ),
        ] {
            let app = app_with_connection(connection);
            let error = app
                .update_media_server_connection(&user, patch)
                .await
                .expect_err(&format!("{name} should require ManagePermissions"));
            assert_unauthorized(error);
        }
    }

    #[tokio::test]
    async fn media_server_update_rejects_adding_non_empty_default_grants_even_when_auth_disabled() {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.enabled = false;
        connection.login_enabled = false;
        connection.linking_enabled = false;
        connection.auto_add_enabled = false;
        connection.default_app_permissions = AppPermissionMask::NONE;
        connection.default_library_grants.clear();
        let app = app_with_connection(connection);
        let mut patch = empty_update_patch("jellyfin-main");
        patch.default_app_permissions = Some(AppPermissionMask::from_permissions([
            AppPermission::ManageCatalogSettings,
        ]));

        let error = app
            .update_media_server_connection(&system_settings_user(), patch)
            .await
            .expect_err("adding default grants should require ManagePermissions");

        assert_unauthorized(error);
    }

    #[tokio::test]
    async fn media_server_update_allows_permission_manager_to_activate_preserved_grants() {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.enabled = false;
        let app = app_with_connection(connection);
        let mut patch = empty_update_patch("jellyfin-main");
        patch.enabled = Some(true);

        let updated = app
            .update_media_server_connection(&permission_manager_user(), patch)
            .await
            .expect("permission manager should activate preserved grants");

        assert!(updated.enabled);
    }

    #[tokio::test]
    async fn media_server_update_allows_system_settings_user_to_deactivate_or_clear_grants() {
        let app = app_with_connection(grant_bearing_jellyfin_connection());
        let mut patch = empty_update_patch("jellyfin-main");
        patch.enabled = Some(false);
        let updated = app
            .update_media_server_connection(&system_settings_user(), patch)
            .await
            .expect("system settings user should be able to deactivate connection");
        assert!(!updated.enabled);

        let app = app_with_connection(grant_bearing_jellyfin_connection());
        let mut patch = empty_update_patch("jellyfin-main");
        patch.default_app_permissions = Some(AppPermissionMask::NONE);
        patch.default_library_grants = Some(Vec::new());
        let updated = app
            .update_media_server_connection(&system_settings_user(), patch)
            .await
            .expect("system settings user should be able to clear grants");
        assert!(updated.default_app_permissions.is_empty());
        assert!(updated.default_library_grants.is_empty());
    }

    #[tokio::test]
    async fn media_server_update_allows_harmless_save_with_unchanged_grants_without_manage_permissions()
     {
        let connection = grant_bearing_jellyfin_connection();
        let app = app_with_connection(connection.clone());
        let mut patch = empty_update_patch("jellyfin-main");
        patch.display_name = Some("Home Jellyfin".to_string());
        patch.base_url = Some(connection.base_url);
        patch.enabled = Some(connection.enabled);
        patch.login_enabled = Some(connection.login_enabled);
        patch.linking_enabled = Some(connection.linking_enabled);
        patch.auto_add_enabled = Some(connection.auto_add_enabled);
        patch.default_app_permissions = Some(connection.default_app_permissions);
        patch.default_library_grants = Some(connection.default_library_grants);

        let updated = app
            .update_media_server_connection(&system_settings_user(), patch)
            .await
            .expect("unchanged grant payload should not require ManagePermissions");

        assert_eq!(updated.display_name, "Home Jellyfin");
        assert!(!updated.default_app_permissions.is_empty());
        assert!(!updated.default_library_grants.is_empty());
    }

    #[tokio::test]
    async fn media_server_create_preserves_plex_token_and_path_mappings() {
        let app = app_with_connections(Vec::new());
        let created = app
            .create_media_server_connection(
                &system_settings_user(),
                MediaServerConnectionDraft {
                    provider: MediaServerProvider::Plex,
                    display_name: "Plex".to_string(),
                    base_url: "http://plex:32400".to_string(),
                    enabled: true,
                    login_enabled: false,
                    linking_enabled: false,
                    auto_add_enabled: false,
                    default_app_permissions: AppPermissionMask::NONE,
                    default_library_grants: Vec::new(),
                    machine_id: None,
                    plex_auth_token: Some(" plex-token ".to_string()),
                    plex_server_id: None,
                    api_key: None,
                    admin_username: None,
                    admin_password: None,
                    path_mappings: vec![MediaServerPathMapping {
                        source_path: "/mnt/plex".to_string(),
                        destination_path: "/data/media".to_string(),
                        sort_order: 0,
                    }],
                },
            )
            .await
            .expect("Plex connection should be created");

        assert_eq!(created.api_key.as_deref(), Some("plex-token"));
        assert_eq!(
            created.path_mappings,
            vec![MediaServerPathMapping {
                source_path: "/mnt/plex".to_string(),
                destination_path: "/data/media".to_string(),
                sort_order: 0,
            }]
        );
    }

    #[tokio::test]
    async fn media_server_update_preserves_existing_plex_token_and_replaces_from_oauth() {
        let mut connection = grant_bearing_plex_connection();
        connection.api_key = Some("old-token".to_string());
        let app = app_with_connection(connection);
        let updated = app
            .update_media_server_connection(&permission_manager_user(), {
                let mut patch = empty_update_patch("plex-main");
                patch.plex_auth_token = Some(" new-token ".to_string());
                patch.path_mappings = Some(vec![MediaServerPathMapping {
                    source_path: "/mnt/plex".to_string(),
                    destination_path: "/data/media".to_string(),
                    sort_order: 0,
                }]);
                patch
            })
            .await
            .expect("Plex connection should update");

        assert_eq!(updated.api_key.as_deref(), Some("new-token"));
        assert_eq!(updated.path_mappings.len(), 1);

        let app = app_with_connection(updated);
        let unchanged = app
            .update_media_server_connection(
                &permission_manager_user(),
                empty_update_patch("plex-main"),
            )
            .await
            .expect("empty update should preserve Plex token");

        assert_eq!(unchanged.api_key.as_deref(), Some("new-token"));
        assert_eq!(unchanged.path_mappings.len(), 1);
    }

    #[tokio::test]
    async fn media_server_update_does_not_carry_api_key_across_provider_change() {
        let mut connection = grant_bearing_jellyfin_connection();
        connection.login_enabled = false;
        connection.auto_add_enabled = false;
        connection.default_app_permissions = AppPermissionMask::NONE;
        connection.default_library_grants.clear();
        connection.api_key = Some("jellyfin-token".to_string());
        let app = app_with_connection(connection);

        let updated = app
            .update_media_server_connection(&system_settings_user(), {
                let mut patch = empty_update_patch("jellyfin-main");
                patch.provider = Some(MediaServerProvider::Plex);
                patch.display_name = Some("Plex".to_string());
                patch.base_url = Some("http://plex:32400".to_string());
                patch.login_enabled = Some(false);
                patch.linking_enabled = Some(false);
                patch.auto_add_enabled = Some(false);
                patch.default_app_permissions = Some(AppPermissionMask::NONE);
                patch.default_library_grants = Some(Vec::new());
                patch.path_mappings = Some(Vec::new());
                patch
            })
            .await
            .expect("provider change should not reuse old provider secret");

        assert_eq!(updated.provider, MediaServerProvider::Plex);
        assert_eq!(updated.api_key, None);
    }

    #[tokio::test]
    async fn plex_media_server_notification_channel_uses_facade_config() {
        let mut connection = grant_bearing_plex_connection();
        connection.api_key = Some("plex-token".to_string());
        connection.path_mappings = vec![MediaServerPathMapping {
            source_path: "/mnt/plex".to_string(),
            destination_path: "/data/media".to_string(),
            sort_order: 0,
        }];
        let app = app_with_connection(connection);

        let channel = app
            .notification_channel_for_media_server_target("plex-main")
            .await
            .expect("Plex media server notification channel should resolve");
        let config: serde_json::Value =
            serde_json::from_str(&channel.config_json).expect("config should be JSON");

        assert_eq!(channel.id, "media-server:plex-main");
        assert_eq!(channel.channel_type.as_str(), "plex");
        assert_eq!(config["base_url"], "https://plex.tv");
        assert_eq!(config["api_key"], "plex-token");
        assert_eq!(config["machine_id"], "machine-1");
        assert_eq!(config["path_mappings"], "/data/media => /mnt/plex");
    }
}
