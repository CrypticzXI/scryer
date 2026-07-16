#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityProfileSelection {
    pub facet: MediaFacet,
    pub override_profile_id: Option<String>,
    pub effective_profile_id: String,
    pub inherits_global: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FacetScoringPersonaSelection {
    pub facet: MediaFacet,
    pub override_persona: Option<ScoringPersona>,
    pub effective_persona: ScoringPersona,
    pub inherits_global: bool,
}
#[derive(Debug, Clone)]
pub struct QualityProfileSettings {
    pub profiles: Vec<crate::QualityProfile>,
    pub global_profile_id: String,
    pub global_scoring_persona: ScoringPersona,
    pub category_selections: Vec<QualityProfileSelection>,
    pub category_persona_selections: Vec<FacetScoringPersonaSelection>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateQualityProfileSelection {
    pub facet: MediaFacet,
    pub inherit_global: bool,
    pub profile_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFacetScoringPersonaSelection {
    pub facet: MediaFacet,
    pub inherit_global: bool,
    pub persona: Option<ScoringPersona>,
}
#[derive(Debug, Clone)]
pub struct SaveQualityProfileSettings {
    pub profiles: Vec<crate::QualityProfile>,
    pub replace_existing: bool,
    pub global_profile_id: Option<String>,
    pub category_selections: Vec<UpdateQualityProfileSelection>,
    pub global_scoring_persona: Option<ScoringPersona>,
    pub category_persona_selections: Vec<UpdateFacetScoringPersonaSelection>,
}
fn ensure_quality_profiles_exist(
    mut profiles: Vec<crate::QualityProfile>,
) -> Vec<crate::QualityProfile> {
    if profiles.is_empty() {
        profiles.push(crate::default_quality_profile_for_search());
        profiles.push(crate::default_quality_profile_1080p_for_search());
    }

    profiles
}
fn resolve_global_profile_id(
    profiles: &[crate::QualityProfile],
    candidate: Option<String>,
) -> String {
    let trimmed = candidate.unwrap_or_default();
    if profiles.iter().any(|profile| profile.id == trimmed) {
        return trimmed;
    }

    profiles
        .first()
        .map(|profile| profile.id.clone())
        .unwrap_or_else(|| "default".to_string())
}
fn merge_quality_profiles(
    existing: Vec<crate::QualityProfile>,
    updates: Vec<crate::QualityProfile>,
) -> Vec<crate::QualityProfile> {
    let mut merged = existing;
    for update in updates {
        if let Some(index) = merged.iter().position(|profile| profile.id == update.id) {
            merged[index] = update;
        } else {
            merged.push(update);
        }
    }
    merged
}
fn normalize_delay_profile(mut profile: crate::DelayProfile) -> crate::DelayProfile {
    profile.id = profile.id.trim().to_string();
    profile.name = profile.name.trim().to_string();

    let mut seen_facets = HashSet::new();
    profile.applies_to_facets = profile
        .applies_to_facets
        .into_iter()
        .filter_map(|facet| MediaFacet::parse(&facet).map(|parsed| parsed.as_str().to_string()))
        .filter(|facet| seen_facets.insert(facet.clone()))
        .collect();

    let mut seen_tags = HashSet::new();
    profile.tags = profile
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .filter(|tag| seen_tags.insert(tag.to_ascii_lowercase()))
        .collect();

    profile
}
fn parse_scoring_persona_setting(value: Option<String>) -> Option<ScoringPersona> {
    match value?.trim() {
        "Balanced" | "balanced" => Some(ScoringPersona::Balanced),
        "Audiophile" | "audiophile" => Some(ScoringPersona::Audiophile),
        "Efficient" | "efficient" => Some(ScoringPersona::Efficient),
        "Compatible" | "compatible" => Some(ScoringPersona::Compatible),
        _ => None,
    }
}
fn global_persona_as_setting(persona: &ScoringPersona) -> &'static str {
    match persona {
        ScoringPersona::Balanced => "balanced",
        ScoringPersona::Audiophile => "audiophile",
        ScoringPersona::Efficient => "efficient",
        ScoringPersona::Compatible => "compatible",
    }
}
impl AppUseCase {
    async fn resolve_quality_profile_id(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<String> {
        if let Some(library_id) = library_id
            && let Some(profile_id) = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(library_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(profile_id);
        }
        if let Some(scope_id) = scope_id
            && let Some(profile_id) = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(scope_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(profile_id);
        }
        if let Some(profile_id) = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?
            .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(profile_id);
        }
        Ok(crate::default_quality_profile_for_search().id)
    }
}
impl AppUseCase {
    /// Resolves the effective quality-profile label for a catalog page without
    /// fetching per-title settings or profile data.
    pub async fn list_title_effective_quality_summaries(
        &self,
        actor: &User,
        title_ids: &[String],
    ) -> AppResult<Vec<crate::TitleQualitySummary>> {
        let titles = self.get_titles_by_ids(actor, title_ids).await?;
        if titles.is_empty() {
            return Ok(Vec::new());
        }

        let settings = self.load_quality_profile_settings().await?;
        let profile_names = settings
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), profile.name.clone()))
            .collect::<HashMap<_, _>>();
        let facet_profile_ids = settings
            .category_selections
            .into_iter()
            .map(|selection| (selection.facet, selection.effective_profile_id))
            .collect::<HashMap<_, _>>();
        let library_ids = titles
            .iter()
            .map(|title| title.library_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let library_profile_ids = self
            .services
            .config
            .settings
            .list_setting_json_explicit_for_scope_ids(
                SETTINGS_SCOPE_SYSTEM,
                QUALITY_PROFILE_ID_KEY,
                &library_ids,
            )
            .await?
            .into_iter()
            .filter_map(|(library_id, raw_value)| {
                serde_json::from_str::<String>(&raw_value)
                    .ok()
                    .and_then(|profile_id| normalize_optional_string(Some(profile_id)))
                    .filter(|profile_id| profile_names.contains_key(profile_id))
                    .map(|profile_id| (library_id, profile_id))
            })
            .collect::<HashMap<_, _>>();

        Ok(titles
            .into_iter()
            .filter_map(|title| {
                let title_profile_id = title
                    .tags
                    .iter()
                    .find_map(|tag| tag.strip_prefix("scryer:quality-profile:"))
                    .map(str::trim)
                    .filter(|profile_id| !profile_id.is_empty())
                    .filter(|profile_id| profile_names.contains_key(*profile_id));
                let profile_id = title_profile_id
                    .or_else(|| {
                        library_profile_ids
                            .get(&title.library_id)
                            .map(String::as_str)
                    })
                    .or_else(|| facet_profile_ids.get(&title.facet).map(String::as_str))
                    .unwrap_or(settings.global_profile_id.as_str());
                profile_names.get(profile_id).cloned().map(|quality_tier| {
                    crate::TitleQualitySummary {
                        title_id: title.id,
                        quality_tier,
                    }
                })
            })
            .collect())
    }
}
impl AppUseCase {
    pub(crate) async fn delay_profiles(&self) -> AppResult<Vec<crate::DelayProfile>> {
        let profiles = self
            .read_setting_json_value::<Vec<crate::DelayProfile>>(
                crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
                None,
            )
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(normalize_delay_profile)
            .collect::<Vec<_>>();

        crate::validate_delay_profile_catalog(&profiles).map_err(AppError::Validation)?;

        Ok(profiles)
    }
}
impl AppUseCase {
    pub async fn get_delay_profiles(&self, actor: &User) -> AppResult<Vec<crate::DelayProfile>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.delay_profiles().await
    }
}
impl AppUseCase {
    pub(crate) async fn load_quality_profile_settings(&self) -> AppResult<QualityProfileSettings> {
        let profiles = ensure_quality_profiles_exist(
            self.services
                .config
                .quality_profiles
                .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
                .await?,
        );
        let global_profile_id = resolve_global_profile_id(
            &profiles,
            self.read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
                .await?,
        );
        let global_scoring_persona = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        )
        .unwrap_or_default();

        let mut category_selections = Vec::with_capacity(3);
        let mut category_persona_selections = Vec::with_capacity(3);
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let override_profile_id = self
                .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(facet.as_str()))
                .await?
                .filter(|value| profiles.iter().any(|profile| profile.id == *value));
            let effective_profile_id = override_profile_id
                .clone()
                .unwrap_or_else(|| global_profile_id.clone());
            category_selections.push(QualityProfileSelection {
                facet: facet.clone(),
                inherits_global: override_profile_id.is_none(),
                override_profile_id,
                effective_profile_id,
            });

            let override_persona = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(facet.as_str()))
                    .await?,
            );
            let effective_persona = override_persona
                .clone()
                .unwrap_or_else(|| global_scoring_persona.clone());
            category_persona_selections.push(FacetScoringPersonaSelection {
                facet,
                inherits_global: override_persona.is_none(),
                override_persona,
                effective_persona,
            });
        }

        Ok(QualityProfileSettings {
            profiles,
            global_profile_id,
            global_scoring_persona,
            category_selections,
            category_persona_selections,
        })
    }
}
impl AppUseCase {
    pub async fn get_quality_profile_settings(
        &self,
        actor: &User,
    ) -> AppResult<QualityProfileSettings> {
        let can_manage_catalog = self
            .has_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let can_manage_titles = self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::ManageTitles)
            .await?;
        let can_manage_library = self
            .has_any_granted_library_permission(
                actor,
                scryer_domain::LibraryPermission::ManageLibrary,
            )
            .await?;
        let can_request = self
            .has_any_library_permission(actor, scryer_domain::LibraryPermission::Request)
            .await?;
        if !can_manage_catalog && !can_manage_titles && !can_manage_library && !can_request {
            return Err(AppError::Unauthorized(
                "You do not have permission to view quality profiles".to_string(),
            ));
        }
        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn save_quality_profile_settings(
        &self,
        actor: &User,
        input: SaveQualityProfileSettings,
    ) -> AppResult<QualityProfileSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profiles = if input.replace_existing {
            input.profiles
        } else {
            merge_quality_profiles(
                self.services
                    .config
                    .quality_profiles
                    .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
                    .await?,
                input.profiles,
            )
        };

        let mut changed_keys = Vec::new();
        if !profiles.is_empty() {
            self.services
                .config
                .quality_profiles
                .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, profiles.clone())
                .await?;
            self.upsert_system_setting_json(
                QUALITY_PROFILE_CATALOG_KEY,
                &profiles,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(QUALITY_PROFILE_CATALOG_KEY.to_string());
        }

        let current_profiles = ensure_quality_profiles_exist(
            self.services
                .config
                .quality_profiles
                .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
                .await?,
        );
        let valid_profile_ids = current_profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<HashSet<_>>();

        if let Some(global_profile_id) = input.global_profile_id {
            let global_profile_id = global_profile_id.trim();
            if !global_profile_id.is_empty() {
                if !valid_profile_ids.contains(global_profile_id) {
                    return Err(AppError::Validation(format!(
                        "unknown quality profile '{global_profile_id}'"
                    )));
                }
                self.upsert_system_setting_json(
                    QUALITY_PROFILE_ID_KEY,
                    &global_profile_id,
                    Some(actor.id.clone()),
                )
                .await?;
                if !changed_keys.iter().any(|key| key == QUALITY_PROFILE_ID_KEY) {
                    changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
                }
            }
        }

        for selection in input.category_selections {
            let value = if selection.inherit_global {
                QUALITY_PROFILE_INHERIT_VALUE.to_string()
            } else {
                let profile_id = selection
                    .profile_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        AppError::Validation(
                            "profile_id is required when inherit_global is false".to_string(),
                        )
                    })?;
                if !valid_profile_ids.contains(profile_id) {
                    return Err(AppError::Validation(format!(
                        "unknown quality profile '{profile_id}'"
                    )));
                }
                profile_id.to_string()
            };

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    QUALITY_PROFILE_ID_KEY,
                    Some(selection.facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            if !changed_keys.iter().any(|key| key == QUALITY_PROFILE_ID_KEY) {
                changed_keys.push(QUALITY_PROFILE_ID_KEY.to_string());
            }
        }

        if let Some(global_scoring_persona) = input.global_scoring_persona {
            self.upsert_system_setting_json(
                SCORING_PERSONA_KEY,
                &global_persona_as_setting(&global_scoring_persona),
                Some(actor.id.clone()),
            )
            .await?;
            if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                changed_keys.push(SCORING_PERSONA_KEY.to_string());
            }
        }

        for selection in input.category_persona_selections {
            let value = if selection.inherit_global {
                QUALITY_PROFILE_INHERIT_VALUE.to_string()
            } else {
                global_persona_as_setting(&selection.persona.ok_or_else(|| {
                    AppError::Validation(
                        "persona is required when inherit_global is false".to_string(),
                    )
                })?)
                .to_string()
            };

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    SCORING_PERSONA_KEY,
                    Some(selection.facet.as_str().to_string()),
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                changed_keys.push(SCORING_PERSONA_KEY.to_string());
            }
        }

        self.emit_configuration_changed_event(
            actor,
            "quality_profiles".to_string(),
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        if !changed_keys.is_empty() {
            self.publish_settings_changed(changed_keys);
        }

        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn delete_quality_profile(
        &self,
        actor: &User,
        profile_id: &str,
    ) -> AppResult<QualityProfileSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Err(AppError::Validation("profile_id is required".to_string()));
        }

        let current = self.load_quality_profile_settings().await?;
        if current.global_profile_id == profile_id {
            return Err(AppError::Validation(
                "cannot delete this profile because it is set as the global default quality profile"
                    .to_string(),
            ));
        }

        for selection in &current.category_selections {
            if selection.override_profile_id.as_deref() == Some(profile_id) {
                return Err(AppError::Validation(format!(
                    "cannot delete this profile because it is set as the quality profile override for {}",
                    selection.facet.as_str(),
                )));
            }
        }

        let remaining_profiles = current
            .profiles
            .into_iter()
            .filter(|profile| profile.id != profile_id)
            .collect::<Vec<_>>();
        self.services
            .config
            .quality_profiles
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, remaining_profiles.clone())
            .await?;
        self.upsert_system_setting_json(
            QUALITY_PROFILE_CATALOG_KEY,
            &remaining_profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "quality_profile".to_string(),
            Some(profile_id.to_string()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        self.publish_settings_changed(vec![
            QUALITY_PROFILE_CATALOG_KEY.to_string(),
            QUALITY_PROFILE_ID_KEY.to_string(),
        ]);

        self.load_quality_profile_settings().await
    }
}
impl AppUseCase {
    pub async fn upsert_delay_profile(
        &self,
        actor: &User,
        profile: crate::DelayProfile,
    ) -> AppResult<crate::DelayProfile> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profile = normalize_delay_profile(profile);
        if profile.id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let mut profiles = self.delay_profiles().await?;
        if let Some(existing) = profiles
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }

        crate::validate_delay_profile_catalog(&profiles).map_err(AppError::Validation)?;
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "delay_profile",
            Some(profile.id.clone()),
            scryer_domain::ConfigurationChangeAction::Saved,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        Ok(profile)
    }
}
impl AppUseCase {
    pub async fn delete_delay_profile(&self, actor: &User, profile_id: &str) -> AppResult<String> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let profile_id = profile_id.trim().to_string();
        if profile_id.is_empty() {
            return Err(AppError::Validation(
                "delay profile id is required".to_string(),
            ));
        }

        let profiles = self.delay_profiles().await?;
        if !profiles.iter().any(|profile| profile.id == profile_id) {
            return Err(AppError::NotFound(format!("delay profile {profile_id}")));
        }

        let next_profiles: Vec<crate::DelayProfile> = profiles
            .into_iter()
            .filter(|profile| profile.id != profile_id)
            .collect();
        self.upsert_system_setting_json(
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY,
            &next_profiles,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            actor,
            "delay_profile",
            Some(profile_id.clone()),
            scryer_domain::ConfigurationChangeAction::Deleted,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            crate::delay_profile::DELAY_PROFILE_CATALOG_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        Ok(profile_id)
    }
}
