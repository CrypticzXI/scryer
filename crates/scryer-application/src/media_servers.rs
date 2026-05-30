use super::*;

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
                draft.machine_id,
                draft.api_key,
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
            )
            .await?;
        self.test_media_server_connection_internal(&connection)
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
        if patch
            .default_app_permissions
            .is_some_and(|permissions| !permissions.is_empty())
            || patch
                .default_library_grants
                .as_ref()
                .is_some_and(|grants| grants.iter().any(|grant| !grant.permissions.is_empty()))
        {
            self.require_app_permission(actor, AppPermission::ManagePermissions)
                .await?;
        }
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

        let provider = patch.provider.unwrap_or_else(|| existing.provider.clone());
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
                if patch.clear_machine_id {
                    None
                } else {
                    patch.machine_id.or(existing.machine_id.clone())
                },
                if patch.clear_api_key {
                    None
                } else {
                    patch.api_key.or(existing.api_key.clone())
                },
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
            )
            .await?;
        self.test_media_server_connection_internal(&connection)
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

    pub async fn test_media_server_connection(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        let connection = self
            .services
            .integrations
            .media_server_connections
            .get_by_id(id.trim())
            .await?
            .ok_or_else(|| AppError::NotFound(format!("media server connection {}", id.trim())))?;
        self.test_media_server_connection_internal(&connection)
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
                "Jellyfin user listing requires a saved API key; enter the user manually instead"
                    .into(),
            )
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .list_jellyfin_users(&connection.base_url, api_key, search)
            .await
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
        if provider == MediaServerProvider::Plex && auto_add_enabled && machine_id.is_none() {
            return Err(AppError::Validation(
                "Plex auto-add requires a configured Plex machine identifier".into(),
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
                MediaServerProvider::Jellyfin | MediaServerProvider::Emby => api_key,
                MediaServerProvider::Plex => None,
            },
            path_mappings: match provider {
                MediaServerProvider::Jellyfin | MediaServerProvider::Emby => path_mappings,
                MediaServerProvider::Plex => Vec::new(),
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
            MediaServerProvider::Plex | MediaServerProvider::Emby => {}
        }
        Ok(())
    }
}

fn default_media_server_display_name(provider: &MediaServerProvider) -> &'static str {
    match provider {
        MediaServerProvider::Jellyfin => "Jellyfin",
        MediaServerProvider::Plex => "Plex",
        MediaServerProvider::Emby => "Emby",
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
            existing.permissions = grant.permissions;
        } else {
            normalized.push(MediaServerDefaultLibraryGrant {
                library_id,
                permissions: grant.permissions,
            });
        }
    }
    normalized
}
