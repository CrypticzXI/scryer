use super::*;
use crate::event_views::history_event_from_domain_event;
use scryer_domain::ConfigurationChangeAction;

#[cfg(unix)]
fn to_u64<T: Into<u64>>(value: T) -> u64 {
    value.into()
}

impl AppUseCase {
    async fn ensure_user_admin_permission_masks(&self, user: &User) -> AppResult<()> {
        let admin_authorization = scryer_domain::UserAuthorization::full_admin();
        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(&user.id, admin_authorization.app)
            .await?;
        let mut seen_library_ids = std::collections::HashSet::new();
        let grants = self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .filter_map(|library| {
                if !seen_library_ids.insert(library.id.clone()) {
                    return None;
                }
                Some(scryer_domain::LibraryGrant {
                    user_id: user.id.clone(),
                    library_id: library.id,
                    permissions: admin_authorization.default_library,
                })
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(&user.id, grants)
            .await
    }

    pub async fn system_health(&self, actor: &User) -> AppResult<SystemHealth> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let titles = self.services.catalog.titles.list(None, None).await?;
        let users = self.services.identity.users.list_all().await?;
        let recent_activity = self.recent_activity_page(12, 0).await?;

        let mut titles_movie = 0usize;
        let mut titles_series = 0usize;
        let mut titles_anime = 0usize;
        let titles_other = 0usize;
        let mut monitored_titles = 0usize;
        let mut recent_event_preview = Vec::with_capacity(std::cmp::min(3, recent_activity.len()));

        for title in &titles {
            if title.monitored {
                monitored_titles += 1;
            }

            match title.facet {
                MediaFacet::Movie => titles_movie += 1,
                MediaFacet::Series => titles_series += 1,
                MediaFacet::Anime => titles_anime += 1,
            }
        }

        for event in recent_activity.iter().take(3) {
            recent_event_preview.push(event.message.clone());
        }

        let datastore_info = self.services.config.system_info.datastore_info().await.ok();
        let db_migration_version = datastore_info
            .as_ref()
            .and_then(|info| info.current_migration_key.clone());
        let datastore_engine = datastore_info
            .map(|info| info.engine)
            .unwrap_or_else(|| "unknown".to_string());
        let indexer_stats = self.services.integrations.indexer_stats.all_stats();

        Ok(SystemHealth {
            service_ready: true,
            db_path: datastore_engine.clone(),
            datastore_engine,
            datastore_migration_key: db_migration_version.clone(),
            total_titles: titles.len(),
            monitored_titles,
            total_users: users.len(),
            titles_movie,
            titles_series,
            titles_anime,
            titles_other,
            recent_events: recent_activity.len(),
            recent_event_preview,
            db_migration_version,
            indexer_stats,
        })
    }

    pub async fn disk_space(&self, actor: &User) -> AppResult<Vec<DiskSpaceInfo>> {
        let libraries = self
            .list_libraries_for_permission(actor, None, scryer_domain::LibraryPermission::View)
            .await?;

        let mut seen_paths = std::collections::HashSet::new();
        #[cfg(unix)]
        let mut results = Vec::new();
        #[cfg(not(unix))]
        let results = Vec::new();

        for library in libraries {
            for root in library.roots {
                let path = root.path;
                if !seen_paths.insert(path.clone()) {
                    continue;
                }

                #[cfg(unix)]
                if let Some(stat) = statvfs_path(&path) {
                    let total = to_u64(stat.f_blocks) * to_u64(stat.f_frsize);
                    let free = to_u64(stat.f_bavail) * to_u64(stat.f_frsize);
                    let used = total.saturating_sub(free);
                    results.push(DiskSpaceInfo {
                        path,
                        label: library.name.clone(),
                        total_bytes: total,
                        free_bytes: free,
                        used_bytes: used,
                    });
                } else {
                    tracing::warn!(path = path.as_str(), "failed to query disk space");
                }
                #[cfg(not(unix))]
                {
                    tracing::debug!(
                        path = path.as_str(),
                        "disk space reporting not available on this platform"
                    );
                    let _ = path;
                }
            }
        }

        Ok(results)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<HistoryEvent> {
        let (tx, rx) = broadcast::channel(128);
        let app = self.clone();
        tokio::spawn(async move {
            let mut wake_rx = app.runtime.events.domain_event_broadcast.subscribe();
            let mut cursor = 0_i64;

            loop {
                let events = match app
                    .services
                    .events
                    .domain_events
                    .list_after_sequence(cursor, 100)
                    .await
                {
                    Ok(events) if !events.is_empty() => events,
                    Ok(_) => match wake_rx.recv().await {
                        Ok(sequence) => {
                            if sequence > cursor {
                                cursor = sequence.saturating_sub(1);
                            }
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::debug!(
                                "history event subscription lagged, skipped {n} wakeups"
                            );
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    Err(error) => {
                        tracing::warn!("history event subscription replay failed: {error}");
                        break;
                    }
                };

                for event in events {
                    cursor = event.sequence;
                    if let Some(history) = history_event_from_domain_event(&event)
                        && tx.send(history).is_err()
                    {
                        return;
                    }
                }
            }
        });
        rx
    }

    pub async fn ensure_default_admin(&self, username: &str, password: &str) -> AppResult<User> {
        let username = Self::normalize_local_username(username);
        if username.is_empty() {
            return Err(AppError::Validation("admin username is required".into()));
        }
        if password.is_empty() {
            return Err(AppError::Validation("admin password is required".into()));
        }
        if let Some(mut found) = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
        {
            self.ensure_user_admin_permission_masks(&found).await?;
            // Migration-seeded admin may lack a password hash — set one.
            if found.password_hash.is_none() {
                found = self
                    .services
                    .identity
                    .users
                    .update_password_hash(&found.id, self.hash_password(password)?)
                    .await?;
            }
            self.refresh_cached_jwt_signing_key(&found).await?;
            return Ok(found);
        }

        let user = User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: Some(self.hash_password(password)?),
            account_kind: Default::default(),
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.ensure_user_admin_permission_masks(&user).await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            None,
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    async fn ensure_default_admin_actor(&self) -> AppResult<User> {
        let username = "admin";
        if let Some(found) = self
            .services
            .identity
            .users
            .get_by_username(username)
            .await?
        {
            self.ensure_user_admin_permission_masks(&found).await?;
            self.refresh_cached_jwt_signing_key(&found).await?;
            return Ok(found);
        }

        let user = User {
            id: Id::new().0,
            username: username.to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.ensure_user_admin_permission_masks(&user).await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            None,
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    pub async fn find_or_create_default_user(&self) -> AppResult<User> {
        self.ensure_default_admin_actor().await
    }

    pub async fn find_default_user(&self) -> AppResult<Option<User>> {
        self.services.identity.users.get_by_username("admin").await
    }

    pub async fn list_users(&self, actor: &User) -> AppResult<Vec<User>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.services.identity.users.list_all().await
    }

    pub async fn user_auth_factor_status(&self, user_id: &str) -> AppResult<UserAuthFactorStatus> {
        let has_mfa = self
            .services
            .identity
            .totp
            .get_credential_for_user(user_id)
            .await?
            .is_some();
        let has_passkey = !self
            .services
            .identity
            .webauthn
            .list_credentials_for_user(user_id)
            .await?
            .is_empty();
        Ok(UserAuthFactorStatus {
            has_mfa,
            has_passkey,
        })
    }

    pub async fn get_user(&self, actor: &User, user_id: &str) -> AppResult<Option<User>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.services.identity.users.get_by_id(user_id).await
    }

    pub async fn create_user(
        &self,
        actor: &User,
        username: String,
        password: String,
        app_permissions: scryer_domain::AppPermissionMask,
        library_grants: Vec<scryer_domain::LibraryGrant>,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        if !app_permissions.is_empty()
            || library_grants
                .iter()
                .any(|grant| !grant.permissions.is_empty())
        {
            self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
                .await?;
        }

        let username = Self::normalize_local_username(&username).to_string();
        if username.is_empty() {
            return Err(AppError::Validation("username is required".to_string()));
        }
        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;

        if self
            .services
            .identity
            .users
            .get_by_username(&username)
            .await?
            .is_some()
        {
            return Err(AppError::Validation(format!(
                "user {} already exists",
                username
            )));
        }

        let user = User {
            id: Id::new().0,
            username: username.clone(),
            password_hash: Some(password_hash),
            account_kind: scryer_domain::UserAccountKind::Local,
            authorization: Default::default(),
        };

        let user = self.services.identity.users.create(user).await?;
        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(&user.id, app_permissions)
            .await?;
        let grants = library_grants
            .into_iter()
            .map(|mut grant| {
                grant.user_id = user.id.clone();
                grant.permissions = grant.permissions.normalized_for_storage();
                grant
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(&user.id, grants)
            .await?;
        self.cache_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user",
            Some(user.id.clone()),
            ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(user)
    }

    /// Set a user's password without actor checks. Used only for first-run bootstrap.
    pub async fn bootstrap_user_password(&self, user_id: &str, password: &str) -> AppResult<User> {
        let password_hash = self.hash_password(password)?;
        let user = self
            .services
            .identity
            .users
            .update_password_hash(user_id, password_hash)
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        Ok(user)
    }

    pub async fn change_own_password(
        &self,
        actor: &User,
        password: String,
        current_password: String,
    ) -> AppResult<User> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", actor.id)))?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        let hash = existing
            .password_hash
            .as_deref()
            .ok_or_else(|| AppError::Validation("account has no password set".into()))?;
        if !self.validate_password(&current_password, hash)? {
            return Err(AppError::Unauthorized(
                "current password is incorrect".into(),
            ));
        }

        self.update_user_password_hash(actor, &actor.id, password)
            .await
    }

    pub async fn set_initial_own_password(
        &self,
        actor: &User,
        password: String,
    ) -> AppResult<User> {
        let existing = self
            .services
            .identity
            .users
            .get_by_id(&actor.id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", actor.id)))?;

        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        if existing.password_hash.is_some() {
            return Err(AppError::Validation("current password is required".into()));
        }

        self.update_user_password_hash(actor, &actor.id, password)
            .await
    }

    pub async fn set_user_password(
        &self,
        actor: &User,
        user_id: &str,
        password: String,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        if user_id == actor.id {
            return Err(AppError::Validation(
                "use change_own_password to update your own password".into(),
            ));
        }

        self.update_user_password_hash(actor, user_id, password)
            .await
    }

    async fn update_user_password_hash(
        &self,
        actor: &User,
        user_id: &str,
        password: String,
    ) -> AppResult<User> {
        if password.is_empty() {
            return Err(AppError::Validation("password is required".into()));
        }

        let existing = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {user_id}")))?;
        if !existing.account_kind.allows_local_credentials() {
            return Err(AppError::Validation(
                "externally managed users cannot set a Scryer password".into(),
            ));
        }

        self.validate_new_local_password(&password).await?;
        let password_hash = self.hash_password(&password)?;
        let user = self
            .services
            .identity
            .users
            .update_password_hash(user_id, password_hash)
            .await?;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user_password",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }
    pub async fn set_user_app_permissions(
        &self,
        actor: &User,
        user_id: &str,
        permissions: scryer_domain::AppPermissionMask,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot modify own permissions".into()));
        }

        self.services
            .catalog
            .libraries
            .set_app_permission_mask_for_user(user_id, permissions)
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user_permissions",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }

    pub async fn set_user_library_permissions(
        &self,
        actor: &User,
        user_id: &str,
        grants: Vec<scryer_domain::LibraryGrant>,
    ) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.require_app_permission(actor, scryer_domain::AppPermission::ManagePermissions)
            .await?;
        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;
        let grants = grants
            .into_iter()
            .map(|mut grant| {
                grant.permissions = grant.permissions.normalized_for_storage();
                grant
            })
            .collect();
        self.services
            .catalog
            .libraries
            .set_grants_for_user(user_id, grants)
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user_permissions",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(user)
    }

    pub async fn delete_user(&self, actor: &User, user_id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot delete current user".into()));
        }

        self.services.identity.users.delete(user_id).await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user",
            Some(user.id),
            ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(())
    }

    pub async fn reset_user_mfa(&self, actor: &User, user_id: &str) -> AppResult<User> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        let user = self
            .services
            .identity
            .users
            .get_by_id(user_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {}", user_id)))?;

        if user.id == actor.id {
            return Err(AppError::Validation("cannot reset your own MFA".into()));
        }

        let auth_session_version = Id::new().0;
        self.services
            .identity
            .totp
            .reset_user_mfa_and_invalidate_sessions(user_id, &auth_session_version)
            .await?;
        self.evict_cached_jwt_signing_key(user_id).await;
        self.refresh_cached_jwt_signing_key(&user).await?;
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "user_mfa",
            Some(user.id.clone()),
            ConfigurationChangeAction::Updated,
        )
        .await;

        Ok(user)
    }
}
