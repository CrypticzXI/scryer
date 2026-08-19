// Seeding-profile CRUD, assignment, and the thin read API the grab-time
// resolver consumes. Resolution precedence itself lives with the acquisition
// pipeline, not here.

/// Facet routing scopes plus every library scope, i.e. every scope id that can
/// hold a download-client routing override.
const GLOBAL_ROUTING_SCOPE_IDS: &[&str] = &["movie", "series", "anime"];

impl AppUseCase {
    pub async fn list_seeding_profiles(
        &self,
        actor: &User,
    ) -> AppResult<Vec<scryer_domain::SeedingProfile>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.seeding_profiles().await
    }

    pub async fn get_seeding_profile(
        &self,
        actor: &User,
        id: &str,
    ) -> AppResult<Option<scryer_domain::SeedingProfile>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.seeding_profile(id).await
    }

    pub async fn create_seeding_profile(
        &self,
        actor: &User,
        input: NewSeedingProfile,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let now = Utc::now();
        let profile = scryer_domain::SeedingProfile {
            id: Id::new().0,
            name: input.name,
            ratio: input.ratio,
            seed_time_minutes: input.seed_time_minutes,
            season_pack_mode: input.season_pack_mode,
            season_pack_ratio: input.season_pack_ratio,
            season_pack_seed_time_minutes: input.season_pack_seed_time_minutes,
            honor_tracker_minimums: input.honor_tracker_minimums,
            goal_met_action: input.goal_met_action,
            never_remove: input.never_remove,
            created_at: now,
            updated_at: now,
        }
        .normalized();
        profile.validate().map_err(AppError::Validation)?;

        let created = self
            .services
            .integrations
            .seeding_profiles
            .create(profile)
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "seeding_profile",
            Some(created.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        Ok(created)
    }

    pub async fn update_seeding_profile(
        &self,
        actor: &User,
        update: SeedingProfileUpdate,
    ) -> AppResult<scryer_domain::SeedingProfile> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = update.id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("seeding profile id is required".into()));
        }
        if !update.has_changes() {
            return Err(AppError::Validation(
                "at least one seeding profile field must be provided".into(),
            ));
        }

        let mut profile = self
            .seeding_profile(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("seeding profile '{id}' not found")))?;
        if let Some(name) = update.name {
            profile.name = name;
        }
        if let Some(ratio) = update.ratio {
            profile.ratio = ratio;
        }
        if let Some(seed_time_minutes) = update.seed_time_minutes {
            profile.seed_time_minutes = seed_time_minutes;
        }
        if let Some(season_pack_mode) = update.season_pack_mode {
            profile.season_pack_mode = season_pack_mode;
        }
        if let Some(season_pack_ratio) = update.season_pack_ratio {
            profile.season_pack_ratio = season_pack_ratio;
        }
        if let Some(season_pack_seed_time_minutes) = update.season_pack_seed_time_minutes {
            profile.season_pack_seed_time_minutes = season_pack_seed_time_minutes;
        }
        if let Some(honor_tracker_minimums) = update.honor_tracker_minimums {
            profile.honor_tracker_minimums = honor_tracker_minimums;
        }
        if let Some(goal_met_action) = update.goal_met_action {
            profile.goal_met_action = goal_met_action;
        }
        if let Some(never_remove) = update.never_remove {
            profile.never_remove = never_remove;
        }
        profile.updated_at = Utc::now();
        let profile = profile.normalized();
        profile.validate().map_err(AppError::Validation)?;

        let updated = self
            .services
            .integrations
            .seeding_profiles
            .update(profile)
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "seeding_profile",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    /// Deleting a referenced profile fails rather than silently clearing the
    /// assignments; the error names every referrer so operators can unpick them.
    pub async fn delete_seeding_profile(&self, actor: &User, id: &str) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::Validation("seeding profile id is required".into()));
        }
        if self.seeding_profile(id).await?.is_none() {
            return Err(AppError::NotFound(format!(
                "seeding profile '{id}' not found"
            )));
        }

        let referrers = self.seeding_profile_referrers(id).await?;
        if !referrers.is_empty() {
            return Err(AppError::Validation(format!(
                "seeding profile is still assigned to {}",
                referrers.join(", ")
            )));
        }

        self.services
            .integrations
            .seeding_profiles
            .delete(id)
            .await?;
        self.emit_configuration_changed_event(
            actor,
            "seeding_profile",
            Some(id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        Ok(())
    }

    /// Set or clear the seeding profile for one indexer. Mirrors
    /// `set_indexer_download_client_mapping`, restricted to torrent-capable
    /// indexers because seeding goals are meaningless for Usenet.
    pub async fn set_indexer_seeding_profile(
        &self,
        actor: &User,
        indexer_id: &str,
        seeding_profile_id: Option<&str>,
    ) -> AppResult<IndexerConfig> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let indexer_id = indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation("indexer id is required".into()));
        }
        let normalized_profile_id = seeding_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let existing = self
            .services
            .integrations
            .indexer_configs
            .get_by_id(indexer_id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("indexer config '{indexer_id}' not found"))
            })?;
        if let Some(profile_id) = normalized_profile_id.as_deref() {
            let profile = self.seeding_profile(profile_id).await?.ok_or_else(|| {
                AppError::NotFound(format!("seeding profile '{profile_id}' not found"))
            })?;
            self.validate_indexer_seeding_profile_assignment(&existing, &profile)?;
        }
        if existing.seeding_profile_id == normalized_profile_id {
            return Ok(existing);
        }

        let updated = self
            .services
            .integrations
            .indexer_configs
            .set_seeding_profile_mapping(indexer_id, normalized_profile_id)
            .await?;
        self.publish_indexers_changed();
        self.emit_configuration_changed_event(
            actor,
            "indexer",
            Some(updated.id.clone()),
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        Ok(updated)
    }

    pub(crate) fn validate_indexer_seeding_profile_assignment(
        &self,
        indexer: &IndexerConfig,
        profile: &scryer_domain::SeedingProfile,
    ) -> AppResult<()> {
        if indexer.provider_type.eq_ignore_ascii_case("prowlarr")
            && indexer.managed_parent_config_id.is_none()
        {
            return Err(AppError::Validation(
                "Prowlarr management parents cannot be assigned a seeding profile".into(),
            ));
        }
        let families = self
            .indexer_download_mapping_families(&indexer.provider_type)
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "indexer provider '{}' does not declare a supported protocol family",
                    indexer.provider_type
                ))
            })?;
        if !families.contains(&"torrent") {
            return Err(AppError::Validation(format!(
                "indexer '{}' does not support torrents, so seeding profile '{}' cannot be assigned",
                indexer.name, profile.name
            )));
        }
        Ok(())
    }

    pub async fn get_default_seeding_profile_id(&self, actor: &User) -> AppResult<Option<String>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.default_seeding_profile_id().await
    }

    pub async fn set_default_seeding_profile(
        &self,
        actor: &User,
        seeding_profile_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        let normalized = seeding_profile_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let Some(profile_id) = normalized.as_deref()
            && self.seeding_profile(profile_id).await?.is_none()
        {
            return Err(AppError::NotFound(format!(
                "seeding profile '{profile_id}' not found"
            )));
        }

        let value_json = match normalized.as_deref() {
            Some(profile_id) => serde_json::Value::String(profile_id.to_string()).to_string(),
            None => "null".to_string(),
        };
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DEFAULT_SEEDING_PROFILE_SETTING_KEY,
                None,
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                (!actor.is_system_execution_actor()).then(|| actor.id.clone()),
            )
            .await?;
        self.emit_settings_saved(
            actor,
            "seeding_profile_default",
            None,
            vec![DEFAULT_SEEDING_PROFILE_SETTING_KEY.to_string()],
        )
        .await;
        Ok(normalized)
    }
}

impl AppUseCase {
    /// Read API for the grab-time resolver: no permission gate, no precedence.
    pub(crate) async fn seeding_profiles(
        &self,
    ) -> AppResult<Vec<scryer_domain::SeedingProfile>> {
        self.services.integrations.seeding_profiles.list().await
    }

    pub(crate) async fn seeding_profile(
        &self,
        id: &str,
    ) -> AppResult<Option<scryer_domain::SeedingProfile>> {
        let id = id.trim();
        if id.is_empty() {
            return Ok(None);
        }
        self.services
            .integrations
            .seeding_profiles
            .get_by_id(id)
            .await
    }

    pub async fn indexer_seeding_profile_id(&self, indexer_id: &str) -> AppResult<Option<String>> {
        Ok(self
            .services
            .integrations
            .indexer_configs
            .get_by_id(indexer_id)
            .await?
            .and_then(|indexer| indexer.seeding_profile_id))
    }

    pub async fn routing_seeding_profile_id(
        &self,
        scope_id: &str,
        client_id: &str,
    ) -> AppResult<Option<String>> {
        let Some(raw_json) = self.load_download_client_routing_json(scope_id).await? else {
            return Ok(None);
        };
        let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
        else {
            return Ok(None);
        };
        Ok(entries.get(client_id).and_then(|config| {
            crate::catalog_helpers::parse_download_client_routing_entry(config).seeding_profile_id
        }))
    }

    pub async fn default_seeding_profile_id(&self) -> AppResult<Option<String>> {
        let Some(raw_value) = self
            .read_setting_string_value(DEFAULT_SEEDING_PROFILE_SETTING_KEY, None)
            .await?
        else {
            return Ok(None);
        };
        let trimmed = raw_value.trim();
        if trimmed.is_empty() || trimmed == "null" {
            return Ok(None);
        }
        Ok(Some(trimmed.to_string()))
    }

    /// Human-readable descriptions of everything currently pointing at a
    /// profile, used to explain a blocked delete.
    async fn seeding_profile_referrers(&self, profile_id: &str) -> AppResult<Vec<String>> {
        let mut referrers = Vec::new();

        for indexer in self.services.integrations.indexer_configs.list(None).await? {
            if indexer.seeding_profile_id.as_deref() == Some(profile_id) {
                referrers.push(format!("indexer '{}'", indexer.name));
            }
        }

        let mut scope_ids = GLOBAL_ROUTING_SCOPE_IDS
            .iter()
            .map(|scope_id| (*scope_id).to_string())
            .collect::<Vec<_>>();
        for library in self.services.catalog.libraries.list(None).await? {
            scope_ids.push(library.id);
        }
        let routing_values = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &scope_ids,
            )
            .await?;
        for (scope_id, raw_json) in routing_values {
            let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
            else {
                continue;
            };
            for (client_id, config) in entries {
                let entry = crate::catalog_helpers::parse_download_client_routing_entry(&config);
                if entry.seeding_profile_id.as_deref() == Some(profile_id) {
                    referrers.push(format!(
                        "download-client routing entry '{client_id}' in scope '{scope_id}'"
                    ));
                }
            }
        }

        if self.default_seeding_profile_id().await?.as_deref() == Some(profile_id) {
            referrers.push("the global default seeding profile".to_string());
        }

        Ok(referrers)
    }
}
