// Grab-time seeding-goal resolution.
//
// A single helper answers "what seeding goals does this grab get?" so the
// download-client choke point never has to know the precedence rules and no
// construction site has to duplicate them. Precedence mirrors the design doc
// (§4.2): the indexer that supplied the release wins, then the download-client
// routing entry the grab was routed through, then the global default. Nothing
// resolved means no goals at all — seeding profiles are opt-in, so an install
// with no profiles behaves exactly as it does today.

use std::sync::Arc;

use chrono::Utc;
use scryer_domain::{
    IndexerConfig, PostImportTracking, SeasonPackSeedMode, SeedGoalMetAction, SeedingProfile,
};
use serde_json::Value;

use crate::{
    AppResult, DEFAULT_SEEDING_PROFILE_SETTING_KEY, IndexerConfigRepository, SETTINGS_SCOPE_SYSTEM,
    SeedingProfileRepository, SettingsRepository,
};

/// Which assignment level supplied the profile. Persisted with the resolution
/// so later packages (and operators reading history) can explain a goal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeedGoalResolutionSource {
    /// No profile applied; the client's own global limits stay in charge.
    #[default]
    None,
    /// The indexer that supplied the release carries a profile.
    Indexer,
    /// Seed criteria imported from Prowlarr for a managed child indexer. Used
    /// only when that child has no seeding profile of its own, so assigning
    /// one is how an operator overrides what Prowlarr holds.
    ProwlarrManaged,
    /// The download-client routing entry the grab was routed through.
    RoutingEntry,
    /// The global default seeding profile setting.
    GlobalDefault,
}

impl SeedGoalResolutionSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Indexer => "indexer",
            Self::ProwlarrManaged => "prowlarr_managed",
            Self::RoutingEntry => "routing_entry",
            Self::GlobalDefault => "global_default",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "none" => Some(Self::None),
            "indexer" => Some(Self::Indexer),
            "prowlarr_managed" => Some(Self::ProwlarrManaged),
            "routing_entry" => Some(Self::RoutingEntry),
            "global_default" => Some(Self::GlobalDefault),
            _ => None,
        }
    }
}

/// Everything the resolver needs about one grab. Tracker minimums come off the
/// release `extra` map the indexer adapter populates
/// (`minimum_seed_ratio` / `minimum_seed_time_minutes` and the season-pack
/// twins); construction sites that have no release object pass `None` and the
/// resolver simply skips the clamp.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SeedGoalRequest {
    /// Indexer that supplied the release, when known.
    pub indexer_id: Option<String>,
    /// `seedingProfileId` from the routing entry of the client this grab was
    /// actually routed to.
    pub routing_seeding_profile_id: Option<String>,
    /// Whether the release is a season pack (drives the profile's season-pack
    /// override and which tracker minimum applies).
    pub season_pack: bool,
    pub tracker_min_seed_ratio: Option<f64>,
    pub tracker_min_seed_time_minutes: Option<i64>,
    pub season_pack_min_seed_ratio: Option<f64>,
    pub season_pack_min_seed_time_minutes: Option<i64>,
}

impl SeedGoalRequest {
    /// Tracker minimum for the ratio axis, preferring the season-pack minimum
    /// on season-pack releases and falling back to the per-release minimum.
    fn effective_min_ratio(&self) -> Option<f64> {
        if self.season_pack {
            self.season_pack_min_seed_ratio
                .or(self.tracker_min_seed_ratio)
        } else {
            self.tracker_min_seed_ratio
        }
    }

    fn effective_min_seed_time_minutes(&self) -> Option<i64> {
        if self.season_pack {
            self.season_pack_min_seed_time_minutes
                .or(self.tracker_min_seed_time_minutes)
        } else {
            self.tracker_min_seed_time_minutes
        }
    }
}

/// The resolved policy for one grab. `seeding_profile_id` is `None` exactly
/// when `resolution_source` is `None`, and in that case every goal is `None`
/// too.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResolvedSeedGoals {
    pub seeding_profile_id: Option<String>,
    pub seed_goal_ratio: Option<f64>,
    pub seed_goal_seconds: Option<i64>,
    pub never_remove: bool,
    pub goal_met_action: Option<SeedGoalMetAction>,
    /// Whether Scryer keeps managing the torrent after import. Frozen with the
    /// goals, so a torrent keeps the tracking mode it was grabbed under.
    pub post_import_tracking: PostImportTracking,
    pub resolution_source: SeedGoalResolutionSource,
}

impl ResolvedSeedGoals {
    /// Whether anything was resolved at all. A profile with no goals on either
    /// axis still counts — `never_remove` and `goal_met_action` are policy even
    /// without a numeric goal.
    pub fn is_resolved(&self) -> bool {
        self.resolution_source != SeedGoalResolutionSource::None
    }

    /// Whether either numeric goal is present (i.e. there is something to push
    /// to a Tier-A client or evaluate in Tier B).
    pub fn has_goals(&self) -> bool {
        self.seed_goal_ratio.is_some() || self.seed_goal_seconds.is_some()
    }
}

/// Resolves seeding goals from the three assignment levels plus tracker
/// minimums. Cheap to clone; every lookup goes straight to the repositories so
/// a profile edit takes effect on the next grab.
#[derive(Clone)]
pub struct SeedGoalResolver {
    seeding_profiles: Arc<dyn SeedingProfileRepository>,
    indexer_configs: Option<Arc<dyn IndexerConfigRepository>>,
    settings: Arc<dyn SettingsRepository>,
}

impl SeedGoalResolver {
    pub fn new(
        seeding_profiles: Arc<dyn SeedingProfileRepository>,
        indexer_configs: Option<Arc<dyn IndexerConfigRepository>>,
        settings: Arc<dyn SettingsRepository>,
    ) -> Self {
        Self {
            seeding_profiles,
            indexer_configs,
            settings,
        }
    }

    /// Resolve the applicable profile and compute its goals for one grab.
    pub async fn resolve(&self, request: &SeedGoalRequest) -> AppResult<ResolvedSeedGoals> {
        let Some((profile, source)) = self.resolve_profile(request).await? else {
            return Ok(ResolvedSeedGoals::default());
        };
        Ok(apply_profile(&profile, request, source))
    }

    /// Walk the precedence chain. A level that names a profile which no longer
    /// exists falls through to the next level rather than failing the grab —
    /// a dangling assignment must never block a download.
    async fn resolve_profile(
        &self,
        request: &SeedGoalRequest,
    ) -> AppResult<Option<(SeedingProfile, SeedGoalResolutionSource)>> {
        if let Some(indexer_id) = trimmed(request.indexer_id.as_deref())
            && let Some(repository) = self.indexer_configs.as_ref()
            && let Some(indexer) = repository.get_by_id(&indexer_id).await?
        {
            // An assigned profile always wins: choosing one is exactly how an
            // operator overrides the criteria Prowlarr holds for this tracker.
            if let Some(profile_id) = trimmed(indexer.seeding_profile_id.as_deref())
                && let Some(profile) = self.load_profile(&profile_id).await?
            {
                return Ok(Some((profile, SeedGoalResolutionSource::Indexer)));
            }
            if let Some(profile) = prowlarr_managed_profile(&indexer) {
                return Ok(Some((profile, SeedGoalResolutionSource::ProwlarrManaged)));
            }
        }

        if let Some(profile_id) = trimmed(request.routing_seeding_profile_id.as_deref())
            && let Some(profile) = self.load_profile(&profile_id).await?
        {
            return Ok(Some((profile, SeedGoalResolutionSource::RoutingEntry)));
        }

        if let Some(profile_id) = self.default_seeding_profile_id().await?
            && let Some(profile) = self.load_profile(&profile_id).await?
        {
            return Ok(Some((profile, SeedGoalResolutionSource::GlobalDefault)));
        }

        Ok(None)
    }

    async fn load_profile(&self, profile_id: &str) -> AppResult<Option<SeedingProfile>> {
        self.seeding_profiles.get_by_id(profile_id).await
    }

    /// Global default profile id from the nullable settings key. Mirrors
    /// `AppUseCase::default_seeding_profile_id`, reading the repository
    /// directly so the resolver works outside the use-case facade.
    async fn default_seeding_profile_id(&self) -> AppResult<Option<String>> {
        let Some(raw_value) = self
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DEFAULT_SEEDING_PROFILE_SETTING_KEY,
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(parse_setting_string(&raw_value))
    }
}

/// Compute the goals for a resolved profile: season-pack overrides first, then
/// the tracker-minimum clamp.
/// Seed criteria Scryer imported from Prowlarr for a managed child indexer.
///
/// Stored inside the child's managed-metadata blob rather than as a seeding
/// profile row, so these never appear in the profile manager and cannot be
/// edited, deleted, or assigned to another indexer — they belong to Prowlarr
/// and are refreshed by the next sync.
#[derive(Debug, Clone, Default, serde::Deserialize)]
struct ProwlarrManagedSeedCriteria {
    #[serde(default)]
    seed_ratio: Option<f64>,
    #[serde(default)]
    seed_time_minutes: Option<i64>,
    #[serde(default)]
    season_pack_seed_time_minutes: Option<i64>,
}

/// Builds the throwaway profile that carries Prowlarr's criteria through the
/// normal resolution path. The id is empty because no such profile exists;
/// callers key the resolution off `SeedGoalResolutionSource::ProwlarrManaged`.
pub fn prowlarr_managed_profile(indexer: &IndexerConfig) -> Option<SeedingProfile> {
    // Only a Prowlarr-managed child can carry these; a standalone indexer with
    // a stray blob is not Prowlarr's to speak for.
    indexer.managed_parent_config_id.as_deref()?;
    let criteria: ProwlarrManagedSeedCriteria =
        serde_json::from_str(indexer.managed_metadata_json.as_deref()?).ok()?;
    let ratio = criteria.seed_ratio.filter(|value| value.is_finite() && *value > 0.0);
    let seed_time_minutes = criteria.seed_time_minutes.filter(|value| *value > 0);
    let season_pack_seed_time_minutes = criteria
        .season_pack_seed_time_minutes
        .filter(|value| *value > 0);
    if ratio.is_none() && seed_time_minutes.is_none() && season_pack_seed_time_minutes.is_none() {
        return None;
    }

    let now = Utc::now();
    Some(SeedingProfile {
        id: String::new(),
        name: "Managed by Prowlarr".to_string(),
        ratio,
        seed_time_minutes,
        // Prowlarr carries a season-pack seed time but no season-pack ratio, so
        // an override only kicks in when it actually set one.
        season_pack_mode: if season_pack_seed_time_minutes.is_some() {
            SeasonPackSeedMode::Override
        } else {
            SeasonPackSeedMode::Inherit
        },
        season_pack_ratio: None,
        season_pack_seed_time_minutes,
        // The operator's Prowlarr goals are still a floor, not a ceiling: a
        // tracker that declares a higher minimum wins, same as for a profile.
        honor_tracker_minimums: true,
        goal_met_action: SeedGoalMetAction::default(),
        never_remove: false,
        post_import_tracking: PostImportTracking::default(),
        created_at: now,
        updated_at: now,
    })
}

fn apply_profile(
    profile: &SeedingProfile,
    request: &SeedGoalRequest,
    source: SeedGoalResolutionSource,
) -> ResolvedSeedGoals {
    let mut ratio = profile.effective_ratio(request.season_pack);
    let mut seed_time_minutes = profile.effective_seed_time_minutes(request.season_pack);

    if profile.honor_tracker_minimums {
        // Clamp UP only: a tracker minimum can raise a goal but never lower it,
        // and a minimum on an axis the profile leaves unset becomes the goal on
        // that axis (otherwise the tracker's H&R rule would go unenforced).
        let profile_ratio = ratio;
        let profile_seed_time_minutes = seed_time_minutes;
        let min_ratio = request.effective_min_ratio();
        let min_seed_time_minutes = request.effective_min_seed_time_minutes();
        ratio = clamp_up_f64(profile_ratio, min_ratio);
        seed_time_minutes = clamp_up_i64(profile_seed_time_minutes, min_seed_time_minutes);
        log_tracker_minimum_clamp(
            profile,
            request,
            source,
            TrackerMinimumClamp {
                profile_ratio,
                min_ratio,
                resolved_ratio: ratio,
                profile_seed_time_minutes,
                min_seed_time_minutes,
                resolved_seed_time_minutes: seed_time_minutes,
            },
        );
    }

    ResolvedSeedGoals {
        // Prowlarr-managed criteria are synthesized, so there is no profile row
        // to point at; the resolution source records where they came from.
        seeding_profile_id: (!profile.id.is_empty()).then(|| profile.id.clone()),
        seed_goal_ratio: ratio.filter(|value| value.is_finite() && *value > 0.0),
        seed_goal_seconds: seed_time_minutes
            .filter(|minutes| *minutes > 0)
            .and_then(|minutes| minutes.checked_mul(60)),
        never_remove: profile.never_remove,
        goal_met_action: Some(profile.goal_met_action),
        post_import_tracking: profile.post_import_tracking,
        resolution_source: source,
    }
}

/// Which axes a tracker minimum actually raised, as the single `axes` field of
/// the breadcrumb — `None` when nothing was raised and there is nothing to say.
///
/// Split out from the log call so the decision is unit-testable without a
/// subscriber: `scryer-application` has no log-capture harness, and the value
/// worth pinning is which axes count as clamped, not that `tracing` works.
///
/// Derived from the inputs rather than by comparing the goal before and after,
/// so a non-finite or non-positive profile value can never read as "clamped".
fn clamped_axes(clamp: &TrackerMinimumClamp) -> Option<&'static str> {
    let ratio_clamped = clamp
        .min_ratio
        .filter(|value| value.is_finite() && *value > 0.0)
        .is_some_and(|minimum| {
            clamp
                .profile_ratio
                .filter(|value| value.is_finite())
                .is_none_or(|value| minimum > value)
        });
    let seed_time_clamped = clamp
        .min_seed_time_minutes
        .filter(|minutes| *minutes > 0)
        .is_some_and(|minimum| {
            clamp
                .profile_seed_time_minutes
                .is_none_or(|value| minimum > value)
        });

    match (ratio_clamped, seed_time_clamped) {
        (true, true) => Some("ratio,seed_time"),
        (true, false) => Some("ratio"),
        (false, true) => Some("seed_time"),
        (false, false) => None,
    }
}

/// Both axes of one clamp, before and after, for the operator breadcrumb.
struct TrackerMinimumClamp {
    profile_ratio: Option<f64>,
    min_ratio: Option<f64>,
    resolved_ratio: Option<f64>,
    profile_seed_time_minutes: Option<i64>,
    min_seed_time_minutes: Option<i64>,
    resolved_seed_time_minutes: Option<i64>,
}

/// One structured line per grab when a tracker-declared minimum raised a goal
/// above the profile's value, or supplied a goal on an axis the profile leaves
/// unset.
///
/// This is the operator-facing evidence that hit-and-run protection engaged:
/// the goals a torrent is actually seeding to are frozen at grab time, so
/// without this line the only way to explain a goal that does not match the
/// profile is to read the submission row. Deliberately one event covering both
/// axes — a clamp is a single decision about one grab, not one per axis — and
/// silent when nothing was raised, so it stays a signal rather than per-grab
/// noise.
fn log_tracker_minimum_clamp(
    profile: &SeedingProfile,
    request: &SeedGoalRequest,
    source: SeedGoalResolutionSource,
    clamp: TrackerMinimumClamp,
) {
    let Some(axes) = clamped_axes(&clamp) else {
        return;
    };

    tracing::info!(
        indexer_id = request.indexer_id.as_deref().unwrap_or("unknown"),
        seeding_profile_id = profile.id.as_str(),
        seeding_profile = profile.name.as_str(),
        resolution_source = source.as_str(),
        season_pack = request.season_pack,
        axes,
        profile_ratio = ?clamp.profile_ratio,
        tracker_min_ratio = ?clamp.min_ratio,
        resolved_ratio = ?clamp.resolved_ratio,
        profile_seed_time_minutes = ?clamp.profile_seed_time_minutes,
        tracker_min_seed_time_minutes = ?clamp.min_seed_time_minutes,
        resolved_seed_time_minutes = ?clamp.resolved_seed_time_minutes,
        "seeding goal raised to the tracker-declared minimum (hit-and-run protection)"
    );
}

fn clamp_up_f64(value: Option<f64>, minimum: Option<f64>) -> Option<f64> {
    let minimum = minimum.filter(|value| value.is_finite() && *value > 0.0);
    match (value, minimum) {
        (Some(value), Some(minimum)) => Some(value.max(minimum)),
        (Some(value), None) => Some(value),
        (None, minimum) => minimum,
    }
}

fn clamp_up_i64(value: Option<i64>, minimum: Option<i64>) -> Option<i64> {
    let minimum = minimum.filter(|minutes| *minutes > 0);
    match (value, minimum) {
        (Some(value), Some(minimum)) => Some(value.max(minimum)),
        (Some(value), None) => Some(value),
        (None, minimum) => minimum,
    }
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// Settings values are stored as JSON; the key holds either `null` or a quoted
/// id. Tolerate a bare (unquoted) id too, the way the quality-profile reader
/// does.
fn parse_setting_string(raw_value: &str) -> Option<String> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return None;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Null) => None,
        Ok(Value::String(value)) => {
            let normalized = value.trim();
            (!normalized.is_empty()).then(|| normalized.to_string())
        }
        Ok(_) => Some(trimmed.to_string()),
        Err(_) => Some(trimmed.to_string()),
    }
}

/// Read a tracker-declared minimum out of a release `extra` map. The indexer
/// adapter writes these as JSON numbers, but Torznab feeds proxied through
/// plugins sometimes stringify them, so both shapes are accepted.
pub fn release_extra_f64(
    extra: &std::collections::HashMap<String, Value>,
    key: &str,
) -> Option<f64> {
    match extra.get(key)? {
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite() && *value > 0.0)
}

pub fn release_extra_i64(
    extra: &std::collections::HashMap<String, Value>,
    key: &str,
) -> Option<i64> {
    match extra.get(key)? {
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_f64().map(|value| value.round() as i64)),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
    .filter(|value| *value > 0)
}

/// Tracker minimums lifted off a release `extra` map, in the order the indexer
/// adapter writes them (`indexer_adapter.rs`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReleaseSeedMinimums {
    pub min_seed_ratio: Option<f64>,
    pub min_seed_time_minutes: Option<i64>,
    pub season_pack_seed_ratio: Option<f64>,
    pub season_pack_seed_time_minutes: Option<i64>,
}

impl ReleaseSeedMinimums {
    pub fn from_release_extra(extra: &std::collections::HashMap<String, Value>) -> Self {
        Self {
            min_seed_ratio: release_extra_f64(extra, "minimum_seed_ratio"),
            min_seed_time_minutes: release_extra_i64(extra, "minimum_seed_time_minutes"),
            season_pack_seed_ratio: release_extra_f64(extra, "season_pack_seed_ratio"),
            season_pack_seed_time_minutes: release_extra_i64(
                extra,
                "season_pack_seed_time_minutes",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use async_trait::async_trait;
    use chrono::Utc;
    use scryer_domain::{IndexerConfig, SeasonPackSeedMode};

    use super::*;
    use crate::{AppError, IndexerConfigUpdate, IndexerSystemBackoff};

    struct FakeSeedingProfiles {
        profiles: Vec<SeedingProfile>,
    }

    #[async_trait]
    impl SeedingProfileRepository for FakeSeedingProfiles {
        async fn list(&self) -> AppResult<Vec<SeedingProfile>> {
            Ok(self.profiles.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<SeedingProfile>> {
            Ok(self
                .profiles
                .iter()
                .find(|profile| profile.id == id)
                .cloned())
        }

        async fn create(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
            Ok(profile)
        }

        async fn update(&self, profile: SeedingProfile) -> AppResult<SeedingProfile> {
            Ok(profile)
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct FakeIndexerConfigs {
        indexers: Vec<IndexerConfig>,
    }

    #[async_trait]
    impl IndexerConfigRepository for FakeIndexerConfigs {
        async fn list(&self, _provider_type: Option<String>) -> AppResult<Vec<IndexerConfig>> {
            Ok(self.indexers.clone())
        }

        async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
            Ok(self
                .indexers
                .iter()
                .find(|indexer| indexer.id == id)
                .cloned())
        }

        async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
            Ok(config)
        }

        async fn touch_last_error(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }

        async fn list_system_backoffs(&self) -> AppResult<HashMap<String, IndexerSystemBackoff>> {
            Ok(HashMap::new())
        }

        async fn update(&self, _update: IndexerConfigUpdate) -> AppResult<IndexerConfig> {
            Err(AppError::NotFound("not implemented".into()))
        }

        async fn delete(&self, _id: &str) -> AppResult<()> {
            Ok(())
        }
    }

    struct FakeSettings {
        values: HashMap<String, String>,
    }

    #[async_trait]
    impl SettingsRepository for FakeSettings {
        async fn get_setting_json(
            &self,
            _scope: &str,
            key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            Ok(self.values.get(key_name).cloned())
        }

        async fn get_setting_json_explicit(
            &self,
            scope: &str,
            key_name: &str,
            scope_id: Option<String>,
        ) -> AppResult<Option<String>> {
            self.get_setting_json(scope, key_name, scope_id).await
        }

        async fn list_setting_json_explicit_for_scope_ids(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_ids: &[String],
        ) -> AppResult<Vec<(String, String)>> {
            Ok(Vec::new())
        }

        async fn upsert_setting_json(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
            _value_json: String,
            _source: &str,
            _updated_by: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_setting_value(
            &self,
            _scope: &str,
            _key_name: &str,
            _scope_id: Option<String>,
        ) -> AppResult<()> {
            Ok(())
        }

        async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
            Ok(0)
        }
    }

    fn profile(id: &str, ratio: Option<f64>, seed_time_minutes: Option<i64>) -> SeedingProfile {
        let now = Utc::now();
        SeedingProfile {
            id: id.to_string(),
            name: id.to_string(),
            ratio,
            seed_time_minutes,
            season_pack_mode: SeasonPackSeedMode::Inherit,
            season_pack_ratio: None,
            season_pack_seed_time_minutes: None,
            honor_tracker_minimums: true,
            goal_met_action: SeedGoalMetAction::RemoveEntry,
            never_remove: false,
            post_import_tracking: PostImportTracking::Park,
            created_at: now,
            updated_at: now,
        }
    }

    fn indexer(id: &str, seeding_profile_id: Option<&str>) -> IndexerConfig {
        let now = Utc::now();
        IndexerConfig {
            id: id.to_string(),
            name: id.to_string(),
            provider_type: "torznab".to_string(),
            base_url: "https://example.invalid".to_string(),
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            indexer_proxy_config_id: None,
            download_client_id: None,
            seeding_profile_id: seeding_profile_id.map(str::to_string),
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_message: None,
            last_error_at: None,
            config_json: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn resolver(
        profiles: Vec<SeedingProfile>,
        indexers: Vec<IndexerConfig>,
        default_profile_id: Option<&str>,
    ) -> SeedGoalResolver {
        let mut values = HashMap::new();
        if let Some(profile_id) = default_profile_id {
            values.insert(
                DEFAULT_SEEDING_PROFILE_SETTING_KEY.to_string(),
                serde_json::Value::String(profile_id.to_string()).to_string(),
            );
        }
        SeedGoalResolver::new(
            Arc::new(FakeSeedingProfiles { profiles }),
            Some(Arc::new(FakeIndexerConfigs { indexers })),
            Arc::new(FakeSettings { values }),
        )
    }

    fn request(indexer_id: Option<&str>, routing_profile_id: Option<&str>) -> SeedGoalRequest {
        SeedGoalRequest {
            indexer_id: indexer_id.map(str::to_string),
            routing_seeding_profile_id: routing_profile_id.map(str::to_string),
            ..SeedGoalRequest::default()
        }
    }

    /// A Prowlarr-managed child carrying imported seed criteria.
    fn prowlarr_child(
        id: &str,
        seeding_profile_id: Option<&str>,
        metadata: serde_json::Value,
    ) -> IndexerConfig {
        let mut config = indexer(id, seeding_profile_id);
        config.managed_parent_config_id = Some("prowlarr-parent".to_string());
        config.managed_metadata_json = Some(metadata.to_string());
        config
    }

    #[tokio::test]
    async fn prowlarr_seed_criteria_apply_when_the_child_has_no_profile() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({
                    "indexer_id": 7,
                    "seed_ratio": 1.5,
                    "seed_time_minutes": 4320,
                }),
            )],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::ProwlarrManaged
        );
        // Synthesized, so there is no profile row to point at.
        assert_eq!(resolved.seeding_profile_id, None);
        assert_eq!(resolved.seed_goal_ratio, Some(1.5));
        assert_eq!(resolved.seed_goal_seconds, Some(4320 * 60));
    }

    #[tokio::test]
    async fn a_scryer_profile_overrides_the_prowlarr_criteria() {
        let resolver = resolver(
            vec![profile("indexer-profile", Some(3.0), None)],
            vec![prowlarr_child(
                "idx-managed",
                Some("indexer-profile"),
                serde_json::json!({ "indexer_id": 7, "seed_ratio": 1.5 }),
            )],
            None,
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.resolution_source, SeedGoalResolutionSource::Indexer);
        assert_eq!(resolved.seed_goal_ratio, Some(3.0));
    }

    #[tokio::test]
    async fn a_managed_child_without_prowlarr_goals_falls_through_to_the_default() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![prowlarr_child(
                "idx-managed",
                None,
                // Prowlarr left the seed criteria blank for this tracker.
                serde_json::json!({ "indexer_id": 7 }),
            )],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-managed"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
        assert_eq!(resolved.seed_goal_ratio, Some(0.5));
    }

    #[tokio::test]
    async fn seed_criteria_on_a_standalone_indexer_are_ignored() {
        // Only Prowlarr speaks for a managed child; a stray blob on an
        // unmanaged indexer is not Prowlarr's to interpret.
        let mut standalone = indexer("idx-standalone", None);
        standalone.managed_metadata_json =
            Some(serde_json::json!({ "seed_ratio": 9.0 }).to_string());
        let resolver = resolver(Vec::new(), vec![standalone], None);

        let resolved = resolver
            .resolve(&request(Some("idx-standalone"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.resolution_source, SeedGoalResolutionSource::None);
        assert_eq!(resolved.seed_goal_ratio, None);
    }

    #[tokio::test]
    async fn tracker_minimums_still_raise_prowlarr_criteria() {
        let resolver = resolver(
            Vec::new(),
            vec![prowlarr_child(
                "idx-managed",
                None,
                serde_json::json!({ "indexer_id": 7, "seed_ratio": 1.0 }),
            )],
            None,
        );
        let mut req = request(Some("idx-managed"), None);
        req.tracker_min_seed_ratio = Some(2.0);

        let resolved = resolver
            .resolve(&req)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(2.0));
    }

    #[tokio::test]
    async fn indexer_assignment_beats_routing_entry_and_global_default() {
        let resolver = resolver(
            vec![
                profile("indexer-profile", Some(2.0), None),
                profile("routing-profile", Some(1.0), None),
                profile("global-profile", Some(0.5), None),
            ],
            vec![indexer("idx-1", Some("indexer-profile"))],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("routing-profile")))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.seeding_profile_id.as_deref(),
            Some("indexer-profile")
        );
        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::Indexer
        );
        assert_eq!(resolved.seed_goal_ratio, Some(2.0));
    }

    #[tokio::test]
    async fn routing_entry_beats_global_default_when_the_indexer_has_no_profile() {
        let resolver = resolver(
            vec![
                profile("routing-profile", Some(1.0), None),
                profile("global-profile", Some(0.5), None),
            ],
            vec![indexer("idx-1", None)],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("routing-profile")))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.seeding_profile_id.as_deref(),
            Some("routing-profile")
        );
        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::RoutingEntry
        );
    }

    #[tokio::test]
    async fn global_default_applies_when_nothing_else_is_assigned() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), Some(60))],
            vec![indexer("idx-1", None)],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
        assert_eq!(resolved.seed_goal_ratio, Some(0.5));
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn no_assignment_anywhere_resolves_to_no_goals() {
        let resolver = resolver(
            vec![profile("unused", Some(3.0), Some(120))],
            vec![indexer("idx-1", None)],
            None,
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert!(!resolved.is_resolved());
        assert_eq!(resolved, ResolvedSeedGoals::default());
        assert_eq!(resolved.seeding_profile_id, None);
        assert_eq!(resolved.seed_goal_ratio, None);
        assert_eq!(resolved.seed_goal_seconds, None);
        assert_eq!(resolved.goal_met_action, None);
        assert!(!resolved.never_remove);
        // No profile means Scryer keeps managing the torrent — the fail-closed
        // direction, and what every install did before this feature existed.
        assert_eq!(resolved.post_import_tracking, PostImportTracking::Park);
    }

    #[tokio::test]
    async fn a_dangling_assignment_falls_through_to_the_next_level() {
        let resolver = resolver(
            vec![profile("global-profile", Some(0.5), None)],
            vec![indexer("idx-1", Some("deleted-profile"))],
            Some("global-profile"),
        );

        let resolved = resolver
            .resolve(&request(Some("idx-1"), Some("also-deleted")))
            .await
            .expect("a dangling assignment must not fail the grab");

        assert_eq!(
            resolved.resolution_source,
            SeedGoalResolutionSource::GlobalDefault
        );
    }

    #[tokio::test]
    async fn tracker_minimums_clamp_goals_up_but_never_down() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), Some(60))],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_ratio = Some(2.5);
        goal_request.tracker_min_seed_time_minutes = Some(30);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(2.5));
        // The profile's 60 minutes already clears the 30-minute minimum.
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn a_tracker_minimum_becomes_the_goal_on_an_axis_the_profile_leaves_unset() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), None)],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_time_minutes = Some(4320);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, Some(4320 * 60));
    }

    #[tokio::test]
    async fn tracker_minimums_are_ignored_when_the_profile_opts_out() {
        let mut opted_out = profile("p", Some(1.0), None);
        opted_out.honor_tracker_minimums = false;
        let resolver = resolver(vec![opted_out], vec![indexer("idx-1", Some("p"))], None);

        let mut goal_request = request(Some("idx-1"), None);
        goal_request.tracker_min_seed_ratio = Some(2.5);
        goal_request.tracker_min_seed_time_minutes = Some(4320);

        let resolved = resolver
            .resolve(&goal_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, None);
    }

    #[tokio::test]
    async fn season_pack_override_selects_the_pack_goals_and_pack_minimums() {
        let mut pack_profile = profile("p", Some(1.0), Some(60));
        pack_profile.season_pack_mode = SeasonPackSeedMode::Override;
        pack_profile.season_pack_ratio = Some(2.0);
        pack_profile.season_pack_seed_time_minutes = Some(120);
        let resolver = resolver(vec![pack_profile], vec![indexer("idx-1", Some("p"))], None);

        let mut episode_request = request(Some("idx-1"), None);
        episode_request.tracker_min_seed_ratio = Some(0.1);
        episode_request.season_pack_min_seed_ratio = Some(9.0);
        let episode = resolver
            .resolve(&episode_request)
            .await
            .expect("resolution should succeed");
        assert_eq!(episode.seed_goal_ratio, Some(1.0));
        assert_eq!(episode.seed_goal_seconds, Some(3600));

        let mut pack_request = episode_request.clone();
        pack_request.season_pack = true;
        let pack = resolver
            .resolve(&pack_request)
            .await
            .expect("resolution should succeed");
        // Pack goals win over the base goals, then the pack minimum clamps up.
        assert_eq!(pack.seed_goal_ratio, Some(9.0));
        assert_eq!(pack.seed_goal_seconds, Some(120 * 60));
    }

    #[tokio::test]
    async fn season_pack_inherit_mode_keeps_the_base_goals() {
        let resolver = resolver(
            vec![profile("p", Some(1.0), Some(60))],
            vec![indexer("idx-1", Some("p"))],
            None,
        );

        let mut pack_request = request(Some("idx-1"), None);
        pack_request.season_pack = true;
        let resolved = resolver
            .resolve(&pack_request)
            .await
            .expect("resolution should succeed");

        assert_eq!(resolved.seed_goal_ratio, Some(1.0));
        assert_eq!(resolved.seed_goal_seconds, Some(3600));
    }

    #[tokio::test]
    async fn profile_policy_flags_ride_along_with_the_goals() {
        let mut kept = profile("p", None, None);
        kept.never_remove = true;
        kept.goal_met_action = SeedGoalMetAction::StopSeeding;
        kept.post_import_tracking = PostImportTracking::HandOff;
        let resolver = resolver(vec![kept], vec![indexer("idx-1", Some("p"))], None);

        let resolved = resolver
            .resolve(&request(Some("idx-1"), None))
            .await
            .expect("resolution should succeed");

        assert!(resolved.is_resolved());
        assert!(!resolved.has_goals());
        assert!(resolved.never_remove);
        assert_eq!(
            resolved.goal_met_action,
            Some(SeedGoalMetAction::StopSeeding)
        );
        // Frozen with the goals: a torrent keeps the tracking mode it was
        // grabbed under even if the profile is later edited.
        assert_eq!(resolved.post_import_tracking, PostImportTracking::HandOff);
    }

    #[test]
    fn release_minimums_are_read_from_the_indexer_extra_map() {
        let mut extra = HashMap::new();
        extra.insert("minimum_seed_ratio".to_string(), serde_json::json!(1.25));
        extra.insert(
            "minimum_seed_time_minutes".to_string(),
            serde_json::json!(4320),
        );
        // Some plugins stringify torznab attrs; both shapes must read.
        extra.insert(
            "season_pack_seed_ratio".to_string(),
            serde_json::json!("2.5"),
        );
        extra.insert(
            "season_pack_seed_time_minutes".to_string(),
            serde_json::json!("10080"),
        );
        extra.insert(
            "minimum_seed_ratio_unused".to_string(),
            serde_json::json!(0),
        );

        let minimums = ReleaseSeedMinimums::from_release_extra(&extra);
        assert_eq!(minimums.min_seed_ratio, Some(1.25));
        assert_eq!(minimums.min_seed_time_minutes, Some(4320));
        assert_eq!(minimums.season_pack_seed_ratio, Some(2.5));
        assert_eq!(minimums.season_pack_seed_time_minutes, Some(10080));

        assert_eq!(
            ReleaseSeedMinimums::from_release_extra(&HashMap::new()),
            ReleaseSeedMinimums::default()
        );
    }

    fn clamp(
        profile_ratio: Option<f64>,
        min_ratio: Option<f64>,
        profile_seed_time_minutes: Option<i64>,
        min_seed_time_minutes: Option<i64>,
    ) -> TrackerMinimumClamp {
        TrackerMinimumClamp {
            profile_ratio,
            min_ratio,
            resolved_ratio: clamp_up_f64(profile_ratio, min_ratio),
            profile_seed_time_minutes,
            min_seed_time_minutes,
            resolved_seed_time_minutes: clamp_up_i64(
                profile_seed_time_minutes,
                min_seed_time_minutes,
            ),
        }
    }

    #[test]
    fn a_raised_axis_is_reported_as_a_clamp() {
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(2.0), None, None)),
            Some("ratio")
        );
        assert_eq!(
            clamped_axes(&clamp(None, None, Some(60), Some(4_320))),
            Some("seed_time")
        );
    }

    #[test]
    fn an_axis_the_profile_leaves_unset_is_reported_when_the_tracker_fills_it() {
        // The tracker minimum *becomes* the goal on that axis, which is a
        // policy change the operator has to be able to see.
        assert_eq!(
            clamped_axes(&clamp(None, Some(1.5), None, None)),
            Some("ratio")
        );
        assert_eq!(
            clamped_axes(&clamp(None, None, None, Some(4_320))),
            Some("seed_time")
        );
    }

    #[test]
    fn both_axes_combine_into_one_event() {
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(2.0), Some(60), Some(4_320))),
            Some("ratio,seed_time")
        );
    }

    #[test]
    fn nothing_is_reported_when_the_profile_already_covers_the_minimum() {
        // Equal is not raised, and a profile above the minimum is not raised.
        assert_eq!(clamped_axes(&clamp(Some(2.0), Some(2.0), None, None)), None);
        assert_eq!(clamped_axes(&clamp(Some(3.0), Some(2.0), None, None)), None);
        assert_eq!(
            clamped_axes(&clamp(None, None, Some(4_320), Some(4_320))),
            None
        );
        // No release minimums at all: the common case, and silent.
        assert_eq!(clamped_axes(&clamp(Some(1.0), None, Some(60), None)), None);
    }

    #[test]
    fn non_positive_minimums_are_never_reported_as_a_clamp() {
        assert_eq!(clamped_axes(&clamp(Some(1.0), Some(0.0), None, None)), None);
        assert_eq!(
            clamped_axes(&clamp(Some(1.0), Some(f64::NAN), None, None)),
            None
        );
        assert_eq!(clamped_axes(&clamp(None, None, Some(60), Some(0))), None);
    }

    #[test]
    fn resolution_sources_round_trip_through_their_persisted_labels() {
        for source in [
            SeedGoalResolutionSource::None,
            SeedGoalResolutionSource::Indexer,
            SeedGoalResolutionSource::RoutingEntry,
            SeedGoalResolutionSource::GlobalDefault,
        ] {
            assert_eq!(
                SeedGoalResolutionSource::parse(source.as_str()),
                Some(source)
            );
        }
        assert_eq!(SeedGoalResolutionSource::parse("nope"), None);
    }
}
