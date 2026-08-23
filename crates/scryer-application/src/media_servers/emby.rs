use super::*;

impl AppUseCase {
    pub async fn discover_emby_connect_media_servers(
        &self,
        actor: &User,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<Vec<EmbyConnectServer>> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.services
            .integrations
            .external_identity_verifier
            .discover_emby_connect_servers(username_or_email, password)
            .await
    }

    pub async fn test_emby_connect(
        &self,
        actor: &User,
        connection_id: &str,
        username_or_email: &str,
        password: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
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
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::Validation(
                "Emby Connect test requires an Emby connection".into(),
            ));
        }
        if !connection.emby_connect_enabled {
            return Err(AppError::Validation(
                "Emby Connect sign-in is disabled for this connection".into(),
            ));
        }
        let server_id = connection.emby_server_id.as_deref().ok_or_else(|| {
            AppError::Validation("Emby connection has no verified server identity".into())
        })?;
        let verification = self
            .services
            .integrations
            .external_identity_verifier
            .test_emby_connect_identity(
                &connection.id,
                &connection.base_url,
                server_id,
                username_or_email,
                password,
            )
            .await?;
        if verification.resolved_api_base_url != connection.base_url
            && let Err(error) = self
                .services
                .integrations
                .media_server_connections
                .compare_and_set_emby_base_url(
                    &connection.id,
                    &connection.base_url,
                    server_id,
                    &verification.resolved_api_base_url,
                )
                .await
        {
            tracing::warn!(
                connection_id = connection.id,
                operation = "emby_connect_address_refresh",
                error_class = %error,
                "Emby Connect address refresh failed after successful connection test"
            );
        }
        Ok(())
    }

    pub async fn list_emby_server_users(
        &self,
        actor: &User,
        connection_id: &str,
        search: Option<&str>,
    ) -> AppResult<Vec<EmbyServerUser>> {
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
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::Validation(
                "Emby user listing requires an Emby connection".into(),
            ));
        }
        let api_key = connection.api_key.as_deref().ok_or_else(|| {
            AppError::Validation("Emby user listing requires a saved integration API key".into())
        })?;
        self.services
            .integrations
            .external_identity_verifier
            .list_emby_users(&connection.id, &connection.base_url, api_key, search)
            .await
    }

    pub async fn fetch_emby_server_user_avatar(
        &self,
        actor: &User,
        connection_id: &str,
        user_id: &str,
        image_tag: &str,
    ) -> AppResult<Option<EmbyAvatar>> {
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
        if connection.provider != MediaServerProvider::Emby {
            return Err(AppError::NotFound("Emby avatar was not found".into()));
        }
        let api_key = connection
            .api_key
            .as_deref()
            .ok_or_else(|| AppError::NotFound("Emby avatar was not found".into()))?;
        self.services
            .integrations
            .external_identity_verifier
            .fetch_emby_user_avatar(
                &connection.id,
                &connection.base_url,
                api_key,
                user_id,
                image_tag,
            )
            .await
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Emby setup accepts three mutually exclusive credential modes"
    )]
    pub(super) async fn resolve_emby_credentials_for_setup(
        &self,
        connection_id: &str,
        base_url: &str,
        mode: Option<EmbyConnectionMode>,
        local_setup_method: Option<EmbyLocalSetupMethod>,
        connect_enabled: Option<bool>,
        api_key: Option<&str>,
        admin_username: Option<&str>,
        admin_password: Option<&str>,
        connect_username_or_email: Option<&str>,
        connect_password: Option<&str>,
        connect_server_id: Option<&str>,
    ) -> AppResult<ResolvedEmbyCredentials> {
        fn present(value: Option<&str>) -> Option<&str> {
            value.map(str::trim).filter(|value| !value.is_empty())
        }
        fn present_secret(value: Option<&str>) -> Option<&str> {
            value.filter(|value| !value.is_empty())
        }
        let api_key = present(api_key);
        let admin_username = present(admin_username);
        let admin_password = present_secret(admin_password);
        let connect_username_or_email = present(connect_username_or_email);
        let connect_password = present_secret(connect_password);
        let connect_server_id = present(connect_server_id);
        let mode = mode.unwrap_or(EmbyConnectionMode::Local);

        let mut exchange = match mode {
            EmbyConnectionMode::Local => {
                if connect_username_or_email.is_some()
                    || connect_password.is_some()
                    || connect_server_id.is_some()
                {
                    return Err(AppError::Validation(
                        "local Emby setup does not accept Emby Connect credentials".into(),
                    ));
                }
                let method = local_setup_method.unwrap_or_else(|| {
                    if api_key.is_some() {
                        EmbyLocalSetupMethod::ApiKey
                    } else {
                        EmbyLocalSetupMethod::AdminCredentials
                    }
                });
                match method {
                    EmbyLocalSetupMethod::ApiKey => {
                        if admin_username.is_some() || admin_password.is_some() {
                            return Err(AppError::Validation(
                                "choose either an Emby API key or local administrator credentials, not both"
                                    .into(),
                            ));
                        }
                        let api_key = api_key.ok_or_else(|| {
                            AppError::Validation("Emby API key is required".into())
                        })?;
                        let identity = self
                            .services
                            .integrations
                            .external_identity_verifier
                            .test_emby_api_key(connection_id, base_url, api_key, None)
                            .await?;
                        EmbyApiKeyExchange {
                            api_key: api_key.to_string(),
                            server_identity: identity,
                            created_new_key: false,
                            cleanup: None,
                        }
                    }
                    EmbyLocalSetupMethod::AdminCredentials => {
                        if api_key.is_some() {
                            return Err(AppError::Validation(
                                "choose either an Emby API key or local administrator credentials, not both"
                                    .into(),
                            ));
                        }
                        let (Some(username), Some(password)) = (admin_username, admin_password)
                        else {
                            return Err(AppError::Validation(
                                "both Emby administrator username and password are required".into(),
                            ));
                        };
                        self.services
                            .integrations
                            .external_identity_verifier
                            .exchange_emby_local_admin_api_key(
                                connection_id,
                                base_url,
                                username,
                                password,
                            )
                            .await?
                    }
                }
            }
            EmbyConnectionMode::Connect => {
                if api_key.is_some() || admin_username.is_some() || admin_password.is_some() {
                    return Err(AppError::Validation(
                        "Emby Connect setup does not accept a pasted API key or local administrator credentials"
                            .into(),
                    ));
                }
                let username = connect_username_or_email.ok_or_else(|| {
                    AppError::Validation("Emby Connect username or email is required".into())
                })?;
                let password = connect_password.ok_or_else(|| {
                    AppError::Validation("Emby Connect password is required".into())
                })?;
                let server_id = connect_server_id
                    .ok_or_else(|| AppError::Validation("select an Emby Connect server".into()))?;
                self.services
                    .integrations
                    .external_identity_verifier
                    .exchange_emby_connect_admin_api_key(
                        connection_id,
                        base_url,
                        server_id,
                        username,
                        password,
                    )
                    .await?
            }
        };
        if let Some(expected) = connect_server_id
            && exchange.server_identity.server_id != expected
        {
            if let Some(cleanup) = exchange.cleanup.take() {
                self.services
                    .integrations
                    .external_identity_verifier
                    .finish_emby_api_key_exchange(connection_id, cleanup, true)
                    .await;
            }
            return Err(AppError::Validation(
                "selected Emby Connect server identity does not match the reachable server".into(),
            ));
        }
        Ok(ResolvedEmbyCredentials {
            base_url: exchange.server_identity.api_base_url,
            api_key: exchange.api_key,
            server_id: exchange.server_identity.server_id,
            connect_enabled: connect_enabled.unwrap_or(mode == EmbyConnectionMode::Connect),
            cleanup: exchange.cleanup,
        })
    }
}
