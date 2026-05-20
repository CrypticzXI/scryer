use std::collections::{HashMap, HashSet};
use std::path::Path;

use scryer_domain::RootFolderEntry;
use serde::{Serialize, de::DeserializeOwned};
use tracing::{info, warn};

use super::*;
use crate::acquisition_policy::AcquisitionThresholds;
use crate::scoring_weights::ScoringPersona;
use crate::subtitles::{normalize_subtitle_language_code, wanted::SubtitleLanguagePref};
use crate::{
    AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
    AUTO_BACKUP_ENABLED_KEY, AUTO_BACKUP_KEY_KEY, DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL,
    FORM_LOGIN_ENABLED_KEY, HISTORY_KEEP_FOREVER_KEY, HISTORY_RETENTION_DAYS_KEY, LibraryRootDraft,
    REQUIRED_AUDIO_LANGUAGES_KEY, SCORING_PERSONA_KEY, SETTINGS_SOURCE_TYPED_GRAPHQL,
    SETUP_COMPLETE_KEY, SKIP_LOGIN_FOR_LOCAL_IPS_KEY, TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
};

use super::keys::default_indexer_routing_categories_for_scope;

const ACQUISITION_ENABLED_KEY: &str = "acquisition.enabled";
const ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY: &str = "acquisition.upgrade_cooldown_hours";
const ACQUISITION_SAME_TIER_MIN_DELTA_KEY: &str = "acquisition.same_tier_min_delta";
const ACQUISITION_CROSS_TIER_MIN_DELTA_KEY: &str = "acquisition.cross_tier_min_delta";
const ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY: &str = "acquisition.forced_upgrade_delta_bypass";
const ACQUISITION_POLL_INTERVAL_SECONDS_KEY: &str = "acquisition.poll_interval_seconds";
const ACQUISITION_SYNC_INTERVAL_SECONDS_KEY: &str = "acquisition.sync_interval_seconds";
const ACQUISITION_BATCH_SIZE_KEY: &str = "acquisition.batch_size";

const SUBTITLES_ENABLED_KEY: &str = "subtitles.enabled";
const SUBTITLES_LANGUAGES_KEY: &str = "subtitles.languages";
const SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY: &str = "subtitles.auto_download_on_import";
const SUBTITLES_MINIMUM_SCORE_SERIES_KEY: &str = "subtitles.minimum_score_series";
const SUBTITLES_MINIMUM_SCORE_MOVIE_KEY: &str = "subtitles.minimum_score_movie";
const SUBTITLES_SEARCH_INTERVAL_HOURS_KEY: &str = "subtitles.search_interval_hours";
const SUBTITLES_INCLUDE_AI_TRANSLATED_KEY: &str = "subtitles.include_ai_translated";
const SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY: &str = "subtitles.include_machine_translated";
const SUBTITLES_SYNC_ENABLED_KEY: &str = "subtitles.sync_enabled";
const SUBTITLES_SYNC_THRESHOLD_SERIES_KEY: &str = "subtitles.sync_threshold_series";
const SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY: &str = "subtitles.sync_threshold_movie";
const SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY: &str = "subtitles.sync_max_offset_seconds";
const SMG_VERSION_COMPATIBILITY_NOTICE_KEY: &str = "smg.version_compatibility_notice";

fn normalize_auto_backup_daily_time_local(value: &str) -> AppResult<String> {
    let value = value.trim();
    let (hour, minute) = value
        .split_once(':')
        .ok_or_else(|| AppError::Validation("daily time must use HH:MM format".to_string()))?;
    let hour = hour
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time hour must be numeric".to_string()))?;
    let minute = minute
        .parse::<u32>()
        .map_err(|_| AppError::Validation("daily time minute must be numeric".to_string()))?;
    if hour > 23 || minute > 59 {
        return Err(AppError::Validation(
            "daily time must be between 00:00 and 23:59".to_string(),
        ));
    }
    Ok(format!("{hour:02}:{minute:02}"))
}

fn validate_auto_backup_key_update(
    set_auto_backup_key: Option<&str>,
    clear_auto_backup_key: bool,
) -> AppResult<()> {
    if clear_auto_backup_key && set_auto_backup_key.is_some_and(|value| !value.is_empty()) {
        return Err(AppError::Validation(
            "automatic backup key cannot be replaced and cleared in the same request".to_string(),
        ));
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct SubtitleSettings {
    pub enabled: bool,
    pub languages: Vec<SubtitleLanguagePref>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct UpdateSubtitleSettings {
    pub enabled: bool,
    pub languages: Vec<SubtitleLanguagePref>,
    pub auto_download_on_import: bool,
    pub minimum_score_series: i32,
    pub minimum_score_movie: i32,
    pub search_interval_hours: i32,
    pub include_ai_translated: bool,
    pub include_machine_translated: bool,
    pub sync_enabled: bool,
    pub sync_threshold_series: i32,
    pub sync_threshold_movie: i32,
    pub sync_max_offset_seconds: i32,
}

#[derive(Debug, Clone)]
pub struct AcquisitionSettings {
    pub enabled: bool,
    pub upgrade_cooldown_hours: i32,
    pub same_tier_min_delta: i32,
    pub cross_tier_min_delta: i32,
    pub forced_upgrade_delta_bypass: i32,
    pub poll_interval_seconds: i32,
    pub sync_interval_seconds: i32,
    pub batch_size: i32,
}

impl AcquisitionSettings {
    pub fn thresholds(&self) -> AcquisitionThresholds {
        AcquisitionThresholds {
            upgrade_cooldown_hours: self.upgrade_cooldown_hours as i64,
            same_tier_min_delta: self.same_tier_min_delta,
            cross_tier_min_delta: self.cross_tier_min_delta,
            forced_upgrade_delta_bypass: self.forced_upgrade_delta_bypass,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPathsSettings {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateLibraryPaths {
    pub movie_path: String,
    pub series_path: String,
    pub anime_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalImportLibraryPathsSelection {
    pub movie_paths: Vec<String>,
    pub series_paths: Vec<String>,
    pub anime_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralSettings {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub auto_backup_key_present: bool,
    pub next_run_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateGeneralSettings {
    pub keep_history_forever: bool,
    pub history_retention_days: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAutoBackupSettings {
    pub enabled: bool,
    pub daily_time_local: String,
    pub set_auto_backup_key: Option<String>,
    pub clear_auto_backup_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecuritySettings {
    pub form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSecuritySettings {
    pub form_login_enabled: bool,
    pub skip_login_for_local_ips: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateServiceSettings {
    pub tls_cert_path: String,
    pub tls_key_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadClientRoutingSettingsEntry {
    pub client_id: String,
    pub enabled: bool,
    pub category: Option<String>,
    pub recent_queue_priority: Option<String>,
    pub older_queue_priority: Option<String>,
    pub remove_completed: bool,
    pub remove_failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexerRoutingSettingsEntry {
    pub indexer_id: String,
    pub enabled: bool,
    pub categories: Vec<String>,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LibrarySettingsOverrideDraft {
    pub required_audio_languages: Option<Vec<String>>,
    pub quality_profile_id: Option<String>,
    pub scoring_persona: Option<ScoringPersona>,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub indexer_routing: Option<Vec<IndexerRoutingSettingsEntry>>,
    pub download_client_routing: Option<Vec<DownloadClientRoutingSettingsEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibrarySettings {
    pub required_audio_languages_override: Option<Vec<String>>,
    pub required_audio_languages: Vec<String>,
    pub quality_profile_id_override: Option<String>,
    pub quality_profile_id: String,
    pub scoring_persona_override: Option<ScoringPersona>,
    pub scoring_persona: ScoringPersona,
    pub filler_policy_override: Option<String>,
    pub filler_policy: Option<String>,
    pub recap_policy_override: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials_override: Option<bool>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies_override: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies_override: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import_override: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import_override: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
    pub indexer_routing_override: Option<Vec<IndexerRoutingSettingsEntry>>,
    pub download_client_routing_override: Option<Vec<DownloadClientRoutingSettingsEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSettings {
    pub library_path: String,
    pub root_folders: Vec<RootFolderEntry>,
    pub required_audio_languages: Vec<String>,
    pub rename_template: String,
    pub rename_collision_policy: String,
    pub rename_missing_metadata_policy: String,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: bool,
    pub plexmatch_write_on_import: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateMediaSettings {
    pub library_path: Option<String>,
    pub root_folders: Option<Vec<RootFolderEntry>>,
    pub required_audio_languages: Option<Vec<String>>,
    pub rename_template: Option<String>,
    pub rename_collision_policy: Option<String>,
    pub rename_missing_metadata_policy: Option<String>,
    pub filler_policy: Option<String>,
    pub recap_policy: Option<String>,
    pub monitor_specials: Option<bool>,
    pub inter_season_movies: Option<bool>,
    pub monitor_filler_movies: Option<bool>,
    pub nfo_write_on_import: Option<bool>,
    pub plexmatch_write_on_import: Option<bool>,
}

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

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn download_client_routing_payload(
    entries: Vec<DownloadClientRoutingSettingsEntry>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let client_id = entry.client_id.trim();
        if client_id.is_empty() {
            return Err(AppError::Validation(
                "download client routing entry requires client_id".to_string(),
            ));
        }

        payload.insert(
            client_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "category": normalize_optional_string(entry.category),
                "recentQueuePriority": normalize_optional_string(entry.recent_queue_priority),
                "olderQueuePriority": normalize_optional_string(entry.older_queue_priority),
                "removeCompleted": entry.remove_completed,
                "removeFailed": entry.remove_failed,
            }),
        );
    }
    Ok(payload)
}

fn download_client_routing_settings_entry_from_domain(
    client_id: String,
    entry: crate::catalog_helpers::DownloadClientRoutingEntry,
) -> DownloadClientRoutingSettingsEntry {
    DownloadClientRoutingSettingsEntry {
        client_id,
        enabled: entry.enabled,
        category: entry.category,
        recent_queue_priority: entry.recent_queue_priority,
        older_queue_priority: entry.older_queue_priority,
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
    }
}

fn disabled_download_client_routing_settings_entry(
    client_id: String,
) -> DownloadClientRoutingSettingsEntry {
    let mut entry = crate::catalog_helpers::default_download_client_routing_entry();
    entry.enabled = false;
    download_client_routing_settings_entry_from_domain(client_id, entry)
}

fn normalize_download_client_routing_settings_entry(
    entry: DownloadClientRoutingSettingsEntry,
) -> AppResult<DownloadClientRoutingSettingsEntry> {
    let client_id = entry.client_id.trim().to_string();
    if client_id.is_empty() {
        return Err(AppError::Validation(
            "download client routing entry requires client_id".to_string(),
        ));
    }

    Ok(DownloadClientRoutingSettingsEntry {
        client_id,
        enabled: entry.enabled,
        category: normalize_optional_string(entry.category),
        recent_queue_priority: normalize_optional_string(entry.recent_queue_priority),
        older_queue_priority: normalize_optional_string(entry.older_queue_priority),
        remove_completed: entry.remove_completed,
        remove_failed: entry.remove_failed,
    })
}

fn indexer_routing_payload(
    entries: Vec<IndexerRoutingSettingsEntry>,
) -> AppResult<serde_json::Map<String, serde_json::Value>> {
    let mut payload = serde_json::Map::new();
    for entry in entries {
        let indexer_id = entry.indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation(
                "indexer routing entry requires indexer_id".to_string(),
            ));
        }

        let categories = entry
            .categories
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();

        payload.insert(
            indexer_id.to_string(),
            serde_json::json!({
                "enabled": entry.enabled,
                "categories": categories,
                "priority": entry.priority,
            }),
        );
    }
    Ok(payload)
}

fn parse_json_object(raw_json: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    serde_json::from_str::<serde_json::Value>(raw_json)
        .ok()?
        .as_object()
        .cloned()
}

fn encode_setting_json<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(|error| AppError::Repository(error.to_string()))
}

fn parse_routing_priority(raw_priority: &serde_json::Value) -> Option<i64> {
    match raw_priority {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(value) => value.parse::<i64>().ok(),
        _ => None,
    }
}

fn next_routing_priority(routing_by_id: &serde_json::Map<String, serde_json::Value>) -> i64 {
    let max_explicit_priority = routing_by_id
        .values()
        .filter_map(|value| value.get("priority"))
        .filter_map(parse_routing_priority)
        .max();

    match max_explicit_priority {
        Some(max_priority) => max_priority + 1,
        None => routing_by_id.len() as i64 + 1,
    }
}

fn default_download_client_routing_entry_json(priority: i64) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "category": "",
        "recentQueuePriority": "",
        "olderQueuePriority": "",
        "removeCompleted": true,
        "removeFailed": false,
        "priority": priority,
    })
}

fn default_indexer_routing_entry_json(scope_id: &str, priority: i64) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "categories": default_indexer_routing_categories_for_scope(scope_id),
        "priority": priority,
    })
}

/// Fill in any fields missing from a stored download-client routing entry with
/// canonical defaults. Returns `true` if the entry was modified. This is the
/// single source of truth for what a "complete" entry looks like at rest, used
/// by both the per-client ensure path and the startup normalization migration.
fn normalize_download_client_routing_entry_in_place(
    entry: &mut serde_json::Map<String, serde_json::Value>,
    fallback_priority: i64,
) -> bool {
    let mut changed = false;
    if !entry.contains_key("enabled") {
        entry.insert("enabled".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("category") {
        entry.insert(
            "category".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("recentQueuePriority") {
        entry.insert(
            "recentQueuePriority".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("olderQueuePriority") {
        entry.insert(
            "olderQueuePriority".to_string(),
            serde_json::Value::String(String::new()),
        );
        changed = true;
    }
    if !entry.contains_key("removeCompleted") {
        entry.insert("removeCompleted".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("removeFailed") {
        entry.insert("removeFailed".to_string(), serde_json::Value::Bool(false));
        changed = true;
    }
    if !entry.contains_key("priority") {
        entry.insert(
            "priority".to_string(),
            serde_json::Value::Number(fallback_priority.into()),
        );
        changed = true;
    }
    changed
}

/// Fill in any fields missing from a stored indexer routing entry with
/// canonical defaults. Returns `true` if the entry was modified.
fn normalize_indexer_routing_entry_in_place(
    scope_id: &str,
    entry: &mut serde_json::Map<String, serde_json::Value>,
    fallback_priority: i64,
) -> bool {
    let mut changed = false;
    if !entry.contains_key("enabled") {
        entry.insert("enabled".to_string(), serde_json::Value::Bool(true));
        changed = true;
    }
    if !entry.contains_key("categories") {
        entry.insert(
            "categories".to_string(),
            serde_json::json!(default_indexer_routing_categories_for_scope(scope_id)),
        );
        changed = true;
    }
    if !entry.contains_key("priority") {
        entry.insert(
            "priority".to_string(),
            serde_json::Value::Number(fallback_priority.into()),
        );
        changed = true;
    }
    changed
}

fn library_path_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => MOVIES_PATH_KEY,
        MediaFacet::Series => SERIES_PATH_KEY,
        MediaFacet::Anime => ANIME_PATH_KEY,
    }
}

fn root_folders_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => MOVIES_ROOT_FOLDERS_KEY,
        MediaFacet::Series => SERIES_ROOT_FOLDERS_KEY,
        MediaFacet::Anime => ANIME_ROOT_FOLDERS_KEY,
    }
}

fn default_library_path(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => DEFAULT_MOVIE_LIBRARY_PATH,
        MediaFacet::Series => DEFAULT_SERIES_LIBRARY_PATH,
        MediaFacet::Anime => DEFAULT_ANIME_LIBRARY_PATH,
    }
}

fn default_library_name(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "Movies",
        MediaFacet::Series => "Series",
        MediaFacet::Anime => "Anime",
    }
}

fn rename_template_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_TEMPLATE_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_TEMPLATE_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_TEMPLATE_ANIME_GLOBAL_KEY,
    }
}

fn default_rename_template(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => DEFAULT_RENAME_TEMPLATE_MOVIE,
        MediaFacet::Series => DEFAULT_RENAME_TEMPLATE_SERIES,
        MediaFacet::Anime => DEFAULT_RENAME_TEMPLATE_ANIME,
    }
}

fn legacy_collision_policy_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_COLLISION_POLICY_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_COLLISION_POLICY_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_COLLISION_POLICY_ANIME_GLOBAL_KEY,
    }
}

fn legacy_missing_metadata_policy_global_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => RENAME_MISSING_METADATA_POLICY_MOVIE_GLOBAL_KEY,
        MediaFacet::Series => RENAME_MISSING_METADATA_POLICY_SERIES_GLOBAL_KEY,
        MediaFacet::Anime => RENAME_MISSING_METADATA_POLICY_ANIME_GLOBAL_KEY,
    }
}

fn nfo_write_on_import_key(facet: &MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => NFO_WRITE_ON_IMPORT_MOVIE_KEY,
        MediaFacet::Series => NFO_WRITE_ON_IMPORT_SERIES_KEY,
        MediaFacet::Anime => NFO_WRITE_ON_IMPORT_ANIME_KEY,
    }
}

fn plexmatch_write_on_import_key(facet: &MediaFacet) -> Option<&'static str> {
    match facet {
        MediaFacet::Movie => None,
        MediaFacet::Series => Some(PLEXMATCH_WRITE_ON_IMPORT_SERIES_KEY),
        MediaFacet::Anime => Some(PLEXMATCH_WRITE_ON_IMPORT_ANIME_KEY),
    }
}

fn normalize_root_folders(entries: Vec<RootFolderEntry>) -> AppResult<Vec<RootFolderEntry>> {
    let mut normalized = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut default_index = None;

    for entry in entries {
        let path = entry.path.trim().to_string();
        if path.is_empty() {
            return Err(AppError::Validation(
                "root folder path is required".to_string(),
            ));
        }
        if !seen_paths.insert(path.clone()) {
            continue;
        }
        if entry.is_default && default_index.is_none() {
            default_index = Some(normalized.len());
        }
        normalized.push(RootFolderEntry {
            path,
            is_default: false,
        });
    }

    if normalized.is_empty() {
        return Err(AppError::Validation(
            "at least one root folder is required".to_string(),
        ));
    }

    let default_index = default_index.unwrap_or(0);
    for (index, entry) in normalized.iter_mut().enumerate() {
        entry.is_default = index == default_index;
    }

    Ok(normalized)
}

pub(crate) fn root_folder_entries_from_library_roots(
    roots: &[scryer_domain::LibraryRoot],
) -> Vec<RootFolderEntry> {
    let mut entries = roots
        .iter()
        .filter_map(|root| {
            let path = root.path.trim();
            if path.is_empty() {
                None
            } else {
                Some(RootFolderEntry {
                    path: path.to_string(),
                    is_default: root.is_default,
                })
            }
        })
        .collect::<Vec<_>>();

    if !entries.iter().any(|entry| entry.is_default)
        && let Some(first) = entries.first_mut()
    {
        first.is_default = true;
    }

    entries
}

fn root_folder_entries_to_library_root_drafts(
    entries: &[RootFolderEntry],
) -> AppResult<Vec<LibraryRootDraft>> {
    crate::library::workflow::normalize_library_root_drafts(
        entries
            .iter()
            .map(|entry| LibraryRootDraft {
                path: entry.path.clone(),
                is_default: entry.is_default,
            })
            .collect(),
    )
}

fn default_root_folder_entry(facet: &MediaFacet) -> RootFolderEntry {
    RootFolderEntry {
        path: default_library_path(facet).to_string(),
        is_default: true,
    }
}

fn default_path_from_root_folders(facet: &MediaFacet, root_folders: &[RootFolderEntry]) -> String {
    root_folders
        .iter()
        .find(|entry| entry.is_default)
        .or_else(|| root_folders.first())
        .map(|entry| entry.path.clone())
        .unwrap_or_else(|| default_library_path(facet).to_string())
}

fn normalize_root_path_for_compare(path: &str) -> String {
    path.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn is_bootstrap_default_root_set(facet: &MediaFacet, root_folders: &[RootFolderEntry]) -> bool {
    root_folders.len() == 1
        && normalize_root_path_for_compare(&root_folders[0].path)
            == normalize_root_path_for_compare(default_library_path(facet))
}

fn normalize_external_import_root_folders(
    paths: Vec<String>,
) -> AppResult<Option<Vec<RootFolderEntry>>> {
    let normalized_paths = paths
        .into_iter()
        .filter_map(|path| normalize_optional_string(Some(path)))
        .collect::<Vec<_>>();

    if normalized_paths.is_empty() {
        return Ok(None);
    }

    let entries = normalized_paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| RootFolderEntry {
            path,
            is_default: index == 0,
        })
        .collect::<Vec<_>>();

    normalize_root_folders(entries).map(Some)
}

fn normalize_effective_scan_root(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(Path::new(trimmed).to_string_lossy().trim().to_string())
}

pub(crate) fn effective_scan_roots_from_root_folders(
    root_folders: &[RootFolderEntry],
) -> Vec<String> {
    let mut roots = Vec::with_capacity(root_folders.len());
    let mut seen = HashSet::new();

    for entry in root_folders {
        let Some(root) = normalize_effective_scan_root(&entry.path) else {
            continue;
        };
        if seen.insert(root.clone()) {
            roots.push(root);
        }
    }

    roots
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

fn normalize_subtitle_languages(languages: Vec<SubtitleLanguagePref>) -> Vec<SubtitleLanguagePref> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::with_capacity(languages.len());

    for language in languages {
        let Some(code) = normalize_subtitle_language_code(&language.code) else {
            continue;
        };
        let key = format!("{}:{}:{}", code, language.hearing_impaired, language.forced);
        if seen.insert(key) {
            normalized.push(SubtitleLanguagePref {
                code,
                hearing_impaired: language.hearing_impaired,
                forced: language.forced,
            });
        }
    }

    normalized
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

fn extract_languages_from_required_audio_rego(rego: &str) -> Vec<String> {
    for line in rego.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("_required_langs := {")
            && let Some(set_body) = rest.strip_suffix('}')
        {
            return normalize_required_audio_languages(
                set_body
                    .split(',')
                    .map(|value| value.trim().trim_matches('"').to_string()),
            );
        }
    }

    Vec::new()
}

impl AppUseCase {
    async fn effective_scan_roots_for_facet(&self, facet: &MediaFacet) -> AppResult<Vec<String>> {
        let root_folders = self.root_folders_for_facet(facet).await?;
        Ok(effective_scan_roots_from_root_folders(&root_folders))
    }

    pub(crate) async fn clear_pending_imports_for_removed_roots(
        &self,
        facet: &MediaFacet,
        previous_roots: &[String],
        current_roots: &[String],
    ) -> AppResult<()> {
        let current = current_roots.iter().cloned().collect::<HashSet<_>>();

        for removed_root in previous_roots
            .iter()
            .filter(|root| !current.contains(root.as_str()))
        {
            let count = self
                .services
                .library
                .library_scan_unmatched_items
                .count_library_scan_unmatched_items(Some(facet.clone()), Some(removed_root), None)
                .await?;
            if count <= 0 {
                continue;
            }

            let items = self
                .services
                .library
                .library_scan_unmatched_items
                .list_library_scan_unmatched_items(
                    Some(facet.clone()),
                    Some(removed_root),
                    None,
                    count,
                    0,
                )
                .await?;

            for item in items {
                self.services
                    .library
                    .library_scan_unmatched_items
                    .delete_library_scan_unmatched_item(
                        &item.library_id,
                        item.facet.clone(),
                        &item.item_path,
                    )
                    .await?;
            }
        }

        Ok(())
    }

    pub(crate) async fn mirror_default_library_roots_to_legacy_settings(
        &self,
        facet: &MediaFacet,
        root_folders: &[RootFolderEntry],
        source: &str,
        actor_id: Option<String>,
    ) -> AppResult<Vec<String>> {
        let normalized = normalize_root_folders(root_folders.to_vec())?;
        let default_path = default_path_from_root_folders(facet, &normalized);

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_MEDIA,
                root_folders_key(facet),
                None,
                encode_setting_json(&normalized)?,
                source,
                actor_id.clone(),
            )
            .await?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_MEDIA,
                library_path_key(facet),
                None,
                encode_setting_json(&default_path)?,
                source,
                actor_id,
            )
            .await?;

        Ok(vec![
            root_folders_key(facet).to_string(),
            library_path_key(facet).to_string(),
        ])
    }

    async fn ensure_default_facet_libraries(&self) -> AppResult<()> {
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            self.ensure_default_facet_library(&facet).await?;
        }

        Ok(())
    }

    async fn ensure_default_facet_library(&self, facet: &MediaFacet) -> AppResult<()> {
        let bootstrap_roots =
            root_folder_entries_to_library_root_drafts(&[default_root_folder_entry(facet)])?;
        let library = match self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        {
            Some(library) => library,
            None => {
                self.validate_library_root_conflicts(None, &bootstrap_roots)
                    .await?;
                let now = chrono::Utc::now();
                let library = scryer_domain::Library {
                    id: scryer_domain::default_library_id_for_facet(facet),
                    facet: facet.clone(),
                    name: default_library_name(facet).to_string(),
                    slug: scryer_domain::default_library_slug_for_facet(facet).to_string(),
                    is_default: true,
                    roots: Vec::new(),
                    created_at: now,
                    updated_at: now,
                };
                let created = self
                    .services
                    .catalog
                    .libraries
                    .create(library, bootstrap_roots.clone())
                    .await?;
                info!(
                    facet = facet.as_str(),
                    library_id = created.id.as_str(),
                    "recreated missing default library during settings repair"
                );
                created
            }
        };

        if !library.roots.is_empty() {
            return Ok(());
        }

        self.validate_library_root_conflicts(Some(&library.id), &bootstrap_roots)
            .await?;
        self.services
            .catalog
            .libraries
            .update(
                &library.id,
                library.name.clone(),
                library.slug.clone(),
                bootstrap_roots,
            )
            .await?;
        info!(
            facet = facet.as_str(),
            library_id = library.id.as_str(),
            "restored bootstrap root for default library during settings repair"
        );
        Ok(())
    }

    async fn update_default_library_roots_from_entries(
        &self,
        facet: &MediaFacet,
        root_folders: &[RootFolderEntry],
        source: &str,
        actor_id: Option<String>,
    ) -> AppResult<Vec<String>> {
        let Some(library) = self
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await?
        else {
            return Err(AppError::NotFound(format!(
                "default {} library",
                facet.as_str()
            )));
        };

        let roots = root_folder_entries_to_library_root_drafts(root_folders)?;
        self.validate_library_root_conflicts(Some(&library.id), &roots)
            .await?;

        let library = self
            .services
            .catalog
            .libraries
            .update(&library.id, library.name, library.slug, roots)
            .await?;
        let canonical_roots = root_folder_entries_from_library_roots(&library.roots);
        self.mirror_default_library_roots_to_legacy_settings(
            facet,
            &canonical_roots,
            source,
            actor_id,
        )
        .await
    }

    async fn read_legacy_root_folders_for_facet(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<Vec<RootFolderEntry>>> {
        if let Some(raw) = self
            .read_setting_string_value_for_scope_explicit(
                SETTINGS_SCOPE_MEDIA,
                root_folders_key(facet),
                None,
            )
            .await?
        {
            let trimmed = raw.trim();
            if !trimmed.is_empty() && trimmed != "[]" {
                match serde_json::from_str::<Vec<RootFolderEntry>>(trimmed) {
                    Ok(entries) if !entries.is_empty() => {
                        return normalize_root_folders(entries).map(Some);
                    }
                    Ok(_) => {}
                    Err(error) => warn!(
                        facet = facet.as_str(),
                        error = %error,
                        "failed to parse legacy root_folders setting during root reconciliation"
                    ),
                }
            }
        }

        let Some(path) = self
            .read_setting_string_value_for_scope_explicit(
                SETTINGS_SCOPE_MEDIA,
                library_path_key(facet),
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        let path = path.trim();
        if path.is_empty() {
            return Ok(None);
        }

        normalize_root_folders(vec![RootFolderEntry {
            path: path.to_string(),
            is_default: true,
        }])
        .map(Some)
    }

    pub async fn reconcile_default_library_roots(&self) -> AppResult<()> {
        self.ensure_default_facet_libraries().await?;

        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            let library = self
                .services
                .catalog
                .libraries
                .default_for_facet(facet.clone())
                .await?
                .ok_or_else(|| AppError::NotFound(format!("default {} library", facet.as_str())))?;

            let canonical_roots = root_folder_entries_from_library_roots(&library.roots);
            let legacy_roots = self.read_legacy_root_folders_for_facet(&facet).await?;
            let canonical_is_empty_or_bootstrap = canonical_roots.is_empty()
                || is_bootstrap_default_root_set(&facet, &canonical_roots);
            let legacy_roots_are_non_bootstrap = legacy_roots.as_ref().is_some_and(|roots| {
                !roots.is_empty() && !is_bootstrap_default_root_set(&facet, roots)
            });

            if canonical_is_empty_or_bootstrap && legacy_roots_are_non_bootstrap {
                let legacy_roots = legacy_roots.expect("checked legacy root presence");
                self.update_default_library_roots_from_entries(
                    &facet,
                    &legacy_roots,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    root_count = legacy_roots.len(),
                    "backfilled default library roots from legacy facet root settings"
                );
                continue;
            }

            let roots_to_mirror = if canonical_roots.is_empty() {
                vec![default_root_folder_entry(&facet)]
            } else {
                canonical_roots
            };

            if roots_to_mirror != root_folder_entries_from_library_roots(&library.roots) {
                self.update_default_library_roots_from_entries(
                    &facet,
                    &roots_to_mirror,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    "initialized empty default library roots from the bootstrap default"
                );
            } else {
                self.mirror_default_library_roots_to_legacy_settings(
                    &facet,
                    &roots_to_mirror,
                    "startup_reconciliation",
                    None,
                )
                .await?;
                info!(
                    facet = facet.as_str(),
                    root_count = roots_to_mirror.len(),
                    "mirrored canonical default library roots to legacy facet settings"
                );
            }
        }

        Ok(())
    }

    async fn load_download_client_routing_json(&self, scope_id: &str) -> AppResult<Option<String>> {
        if let Some(raw_json) = self
            .read_setting_string_value(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await?
        {
            return Ok(Some(raw_json));
        }

        self.read_setting_string_value(LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
    }

    async fn load_explicit_download_client_routing_json(
        &self,
        scope_id: &str,
    ) -> AppResult<Option<String>> {
        if let Some(raw_json) = self
            .read_setting_string_value_explicit(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id),
            )
            .await?
        {
            return Ok(Some(raw_json));
        }

        self.read_setting_string_value_explicit(
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            Some(scope_id),
        )
        .await
    }

    async fn emit_settings_saved(
        &self,
        actor: &User,
        resource_type: &str,
        resource_id: Option<String>,
        changed_keys: Vec<String>,
    ) {
        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            resource_type.to_string(),
            resource_id,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;

        self.publish_settings_changed(changed_keys);
    }

    pub(crate) async fn read_setting_bool_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }))
    }

    pub(crate) async fn read_setting_bool_value_explicit(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<bool>> {
        Ok(self
            .read_setting_string_value_explicit(key_name, scope_id)
            .await?
            .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            }))
    }

    pub(crate) async fn read_setting_i64_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<i64>> {
        Ok(self
            .read_setting_string_value(key_name, scope_id)
            .await?
            .and_then(|value| value.parse::<i64>().ok()))
    }

    pub(crate) async fn read_setting_json_value<T: DeserializeOwned>(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<T>> {
        let Some(raw_value) = self.read_setting_string_value(key_name, scope_id).await? else {
            return Ok(None);
        };
        serde_json::from_str::<T>(&raw_value)
            .map(Some)
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{key_name}' JSON value: {error}"
                ))
            })
    }

    pub async fn smg_version_compatibility_notice(
        &self,
    ) -> AppResult<Option<crate::SmgVersionCompatibilityNotice>> {
        self.read_setting_json_value(SMG_VERSION_COMPATIBILITY_NOTICE_KEY, None)
            .await
    }

    async fn upsert_system_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                key_name,
                None,
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }

    async fn delete_system_setting(&self, key_name: &str) -> AppResult<()> {
        self.services
            .config
            .settings
            .delete_setting_value(SETTINGS_SCOPE_SYSTEM, key_name, None)
            .await
    }

    async fn upsert_scoped_system_setting_json<T: Serialize>(
        &self,
        key_name: &str,
        scope_id: &str,
        value: &T,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let value_json = serde_json::to_string(value)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                key_name,
                Some(scope_id.to_string()),
                value_json,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                updated_by_user_id,
            )
            .await
    }

    async fn delete_scoped_system_setting(&self, key_name: &str, scope_id: &str) -> AppResult<()> {
        self.services
            .config
            .settings
            .delete_setting_value(SETTINGS_SCOPE_SYSTEM, key_name, Some(scope_id.to_string()))
            .await
    }

    pub async fn load_facet_required_audio_languages(
        &self,
        scope_id: &str,
    ) -> AppResult<Vec<String>> {
        Ok(normalize_required_audio_languages(
            self.read_setting_json_value::<Vec<String>>(
                REQUIRED_AUDIO_LANGUAGES_KEY,
                Some(scope_id),
            )
            .await?
            .unwrap_or_default(),
        ))
    }

    pub async fn load_title_required_audio_override(
        &self,
        title_id: &str,
    ) -> AppResult<Option<Vec<String>>> {
        let raw_value = self
            .services
            .config
            .settings
            .get_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                Some(title_id.to_string()),
            )
            .await?;

        let Some(raw_value) = raw_value else {
            return Ok(None);
        };

        serde_json::from_str::<Option<Vec<String>>>(&raw_value)
            .map(|value| value.map(normalize_required_audio_languages))
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to parse setting '{TITLE_REQUIRED_AUDIO_OVERRIDE_KEY}' JSON value: {error}"
                ))
            })
    }

    pub(crate) async fn resolve_required_audio_languages(
        &self,
        title_id: Option<&str>,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<Vec<String>> {
        if let Some(title_id) = title_id
            && let Some(languages) = self.load_title_required_audio_override(title_id).await?
        {
            return Ok(languages);
        }

        if let Some(library_id) = library_id {
            let languages = self.load_facet_required_audio_languages(library_id).await?;
            if !languages.is_empty() {
                return Ok(languages);
            }
        }

        if let Some(scope_id) = scope_id {
            let languages = self.load_facet_required_audio_languages(scope_id).await?;
            if !languages.is_empty() {
                return Ok(languages);
            }
        }

        Ok(Vec::new())
    }

    pub(crate) async fn resolve_scoring_persona(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> AppResult<ScoringPersona> {
        if let Some(library_id) = library_id
            && let Some(persona) = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(library_id))
                    .await?,
            )
        {
            return Ok(persona);
        }

        if let Some(scope_id) = scope_id
            && let Some(persona) = parse_scoring_persona_setting(
                self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(scope_id))
                    .await?,
            )
        {
            return Ok(persona);
        }

        if let Some(persona) = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        ) {
            return Ok(persona);
        }

        Ok(ScoringPersona::default())
    }

    pub(crate) async fn resolve_library_string_setting(
        &self,
        key_name: &str,
        library_id: Option<&str>,
        scope_id: Option<&str>,
        default: &str,
    ) -> AppResult<String> {
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_string_value_explicit(key_name, Some(library_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(value);
        }

        if let Some(scope_id) = scope_id
            && let Some(value) = self
                .read_setting_string_value_explicit(key_name, Some(scope_id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        {
            return Ok(value);
        }

        Ok(default.to_string())
    }

    pub(crate) async fn resolve_library_bool_setting(
        &self,
        key_name: &str,
        library_id: Option<&str>,
        scope_id: Option<&str>,
        default: bool,
    ) -> AppResult<bool> {
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(value);
        }

        if let Some(scope_id) = scope_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(scope_id))
                .await?
        {
            return Ok(value);
        }

        Ok(default)
    }

    pub(crate) async fn resolve_nfo_write_on_import(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<bool> {
        let key_name = nfo_write_on_import_key(facet);
        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(value);
        }

        Ok(self
            .read_setting_bool_value(key_name, None)
            .await?
            .unwrap_or(false))
    }

    pub(crate) async fn resolve_plexmatch_write_on_import(
        &self,
        library_id: Option<&str>,
        facet: &MediaFacet,
    ) -> AppResult<Option<bool>> {
        let Some(key_name) = plexmatch_write_on_import_key(facet) else {
            return Ok(None);
        };

        if let Some(library_id) = library_id
            && let Some(value) = self
                .read_setting_bool_value_explicit(key_name, Some(library_id))
                .await?
        {
            return Ok(Some(value));
        }

        Ok(Some(
            self.read_setting_bool_value(key_name, None)
                .await?
                .unwrap_or(false),
        ))
    }

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

    async fn load_download_client_routing_override(
        &self,
        library_id: &str,
    ) -> AppResult<Option<Vec<DownloadClientRoutingSettingsEntry>>> {
        let Some(raw_json) = self
            .load_explicit_download_client_routing_json(library_id)
            .await?
        else {
            return Ok(None);
        };
        let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
        else {
            warn!(
                library_id,
                "ignoring invalid library-scoped download client routing override in settings"
            );
            return Ok(None);
        };
        let entries = entries
            .into_iter()
            .map(|(client_id, config)| {
                let entry = crate::catalog_helpers::parse_download_client_routing_entry(&config);
                download_client_routing_settings_entry_from_domain(client_id, entry)
            })
            .collect::<Vec<_>>();
        let routing = self
            .complete_library_download_client_routing_entries(entries)
            .await?;
        Ok(Some(routing))
    }

    async fn complete_library_download_client_routing_entries(
        &self,
        entries: Vec<DownloadClientRoutingSettingsEntry>,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        let mut completed = Vec::new();
        let mut seen = HashSet::new();

        for entry in entries {
            let entry = normalize_download_client_routing_settings_entry(entry)?;
            if seen.insert(entry.client_id.clone()) {
                completed.push(entry);
            }
        }

        for config in self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await?
        {
            if seen.insert(config.id.clone()) {
                completed.push(disabled_download_client_routing_settings_entry(config.id));
            }
        }

        Ok(completed)
    }

    async fn load_indexer_routing_override(
        &self,
        library_id: &str,
    ) -> AppResult<Option<Vec<IndexerRoutingSettingsEntry>>> {
        let Some(raw_json) = self
            .read_setting_string_value_explicit(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
            .await?
        else {
            return Ok(None);
        };
        let Some(plan) = self.parse_indexer_routing_plan(library_id, &raw_json) else {
            return Ok(Some(Vec::new()));
        };
        let mut routing = plan
            .entries
            .into_iter()
            .map(|(indexer_id, entry)| IndexerRoutingSettingsEntry {
                indexer_id,
                enabled: entry.enabled,
                categories: entry.categories,
                priority: entry.priority as i32,
            })
            .collect::<Vec<_>>();
        routing.sort_by_key(|entry| (entry.priority, entry.indexer_id.clone()));
        Ok(Some(routing))
    }

    pub async fn get_library_settings(
        &self,
        actor: &User,
        library_id: &str,
    ) -> AppResult<LibrarySettings> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;

        let scope_id = library.facet.as_str();
        let required_audio_languages_override = self
            .load_facet_required_audio_languages(&library.id)
            .await?;
        let required_audio_languages_override = (!required_audio_languages_override.is_empty())
            .then_some(required_audio_languages_override);
        let required_audio_languages = self
            .resolve_required_audio_languages(None, Some(&library.id), Some(scope_id))
            .await?;
        let quality_profile_id_override = self
            .read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(&library.id))
            .await?
            .and_then(|value| normalize_optional_string(Some(value)));
        let quality_profile_id = self
            .resolve_quality_profile_id(Some(&library.id), Some(scope_id))
            .await?;
        let scoring_persona_override = parse_scoring_persona_setting(
            self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(&library.id))
                .await?,
        );
        let scoring_persona = self
            .resolve_scoring_persona(Some(&library.id), Some(scope_id))
            .await?;
        let filler_policy_override = if library.facet == MediaFacet::Anime {
            self.read_setting_string_value_explicit(ANIME_FILLER_POLICY_KEY, Some(&library.id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        } else {
            None
        };
        let filler_policy = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_string_setting(
                    ANIME_FILLER_POLICY_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    DEFAULT_FILLER_POLICY,
                )
                .await?,
            )
        } else {
            None
        };
        let recap_policy_override = if library.facet == MediaFacet::Anime {
            self.read_setting_string_value_explicit(ANIME_RECAP_POLICY_KEY, Some(&library.id))
                .await?
                .and_then(|value| normalize_optional_string(Some(value)))
        } else {
            None
        };
        let recap_policy = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_string_setting(
                    ANIME_RECAP_POLICY_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    DEFAULT_RECAP_POLICY,
                )
                .await?,
            )
        } else {
            None
        };
        let monitor_specials_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(ANIME_MONITOR_SPECIALS_KEY, Some(&library.id))
                .await?
        } else {
            None
        };
        let monitor_specials = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_MONITOR_SPECIALS_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    false,
                )
                .await?,
            )
        } else {
            None
        };
        let inter_season_movies_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(ANIME_INTER_SEASON_MOVIES_KEY, Some(&library.id))
                .await?
        } else {
            None
        };
        let inter_season_movies = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_INTER_SEASON_MOVIES_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    true,
                )
                .await?,
            )
        } else {
            None
        };
        let monitor_filler_movies_override = if library.facet == MediaFacet::Anime {
            self.read_setting_bool_value_explicit(
                ANIME_MONITOR_FILLER_MOVIES_KEY,
                Some(&library.id),
            )
            .await?
        } else {
            None
        };
        let monitor_filler_movies = if library.facet == MediaFacet::Anime {
            Some(
                self.resolve_library_bool_setting(
                    ANIME_MONITOR_FILLER_MOVIES_KEY,
                    Some(&library.id),
                    Some(scope_id),
                    false,
                )
                .await?,
            )
        } else {
            None
        };
        let nfo_write_on_import_override = self
            .read_setting_bool_value_explicit(
                nfo_write_on_import_key(&library.facet),
                Some(&library.id),
            )
            .await?;
        let nfo_write_on_import = self
            .resolve_nfo_write_on_import(Some(&library.id), &library.facet)
            .await?;
        let plexmatch_write_on_import_override = match plexmatch_write_on_import_key(&library.facet)
        {
            Some(key_name) => {
                self.read_setting_bool_value_explicit(key_name, Some(&library.id))
                    .await?
            }
            None => None,
        };
        let plexmatch_write_on_import = self
            .resolve_plexmatch_write_on_import(Some(&library.id), &library.facet)
            .await?;
        let indexer_routing_override = self.load_indexer_routing_override(&library.id).await?;
        let download_client_routing_override = self
            .load_download_client_routing_override(&library.id)
            .await?;

        Ok(LibrarySettings {
            required_audio_languages_override,
            required_audio_languages,
            quality_profile_id_override,
            quality_profile_id,
            scoring_persona_override,
            scoring_persona,
            filler_policy_override,
            filler_policy,
            recap_policy_override,
            recap_policy,
            monitor_specials_override,
            monitor_specials,
            inter_season_movies_override,
            inter_season_movies,
            monitor_filler_movies_override,
            monitor_filler_movies,
            nfo_write_on_import_override,
            nfo_write_on_import,
            plexmatch_write_on_import_override,
            plexmatch_write_on_import,
            indexer_routing_override,
            download_client_routing_override,
        })
    }

    pub async fn update_library_settings(
        &self,
        actor: &User,
        library_id: &str,
        settings: LibrarySettingsOverrideDraft,
    ) -> AppResult<LibrarySettings> {
        let library = self
            .services
            .catalog
            .libraries
            .get_by_id(library_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        self.require_library_management_permission(actor, &library.id)
            .await?;
        let is_anime_library = library.facet == MediaFacet::Anime;
        if !is_anime_library
            && (settings.filler_policy.is_some()
                || settings.recap_policy.is_some()
                || settings.monitor_specials.is_some()
                || settings.inter_season_movies.is_some()
                || settings.monitor_filler_movies.is_some())
        {
            return Err(AppError::Validation(
                "anime-specific settings require an anime library".to_string(),
            ));
        }
        if library.facet == MediaFacet::Movie && settings.plexmatch_write_on_import.is_some() {
            return Err(AppError::Validation(
                "plexmatch_write_on_import is only valid for series and anime libraries"
                    .to_string(),
            ));
        }

        if let Some(languages) = settings.required_audio_languages {
            let languages = normalize_required_audio_languages(languages);
            if languages.is_empty() {
                self.delete_scoped_system_setting(REQUIRED_AUDIO_LANGUAGES_KEY, &library.id)
                    .await?;
            } else {
                self.upsert_scoped_system_setting_json(
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    &library.id,
                    &languages,
                    Some(actor.id.clone()),
                )
                .await?;
            }
        } else {
            self.delete_scoped_system_setting(REQUIRED_AUDIO_LANGUAGES_KEY, &library.id)
                .await?;
        }

        if let Some(profile_id) = normalize_optional_string(settings.quality_profile_id) {
            self.upsert_scoped_system_setting_json(
                QUALITY_PROFILE_ID_KEY,
                &library.id,
                &profile_id,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(QUALITY_PROFILE_ID_KEY, &library.id)
                .await?;
        }

        if let Some(persona) = settings.scoring_persona {
            let persona = global_persona_as_setting(&persona).to_string();
            self.upsert_scoped_system_setting_json(
                SCORING_PERSONA_KEY,
                &library.id,
                &persona,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(SCORING_PERSONA_KEY, &library.id)
                .await?;
        }

        if is_anime_library {
            if let Some(policy) = normalize_optional_string(settings.filler_policy) {
                self.upsert_scoped_system_setting_json(
                    ANIME_FILLER_POLICY_KEY,
                    &library.id,
                    &policy,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_FILLER_POLICY_KEY, &library.id)
                    .await?;
            }

            if let Some(policy) = normalize_optional_string(settings.recap_policy) {
                self.upsert_scoped_system_setting_json(
                    ANIME_RECAP_POLICY_KEY,
                    &library.id,
                    &policy,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_RECAP_POLICY_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.monitor_specials {
                self.upsert_scoped_system_setting_json(
                    ANIME_MONITOR_SPECIALS_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_MONITOR_SPECIALS_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.inter_season_movies {
                self.upsert_scoped_system_setting_json(
                    ANIME_INTER_SEASON_MOVIES_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_INTER_SEASON_MOVIES_KEY, &library.id)
                    .await?;
            }

            if let Some(value) = settings.monitor_filler_movies {
                self.upsert_scoped_system_setting_json(
                    ANIME_MONITOR_FILLER_MOVIES_KEY,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(ANIME_MONITOR_FILLER_MOVIES_KEY, &library.id)
                    .await?;
            }
        }

        if let Some(value) = settings.nfo_write_on_import {
            self.upsert_scoped_system_setting_json(
                nfo_write_on_import_key(&library.facet),
                &library.id,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(nfo_write_on_import_key(&library.facet), &library.id)
                .await?;
        }

        if let Some(key_name) = plexmatch_write_on_import_key(&library.facet) {
            if let Some(value) = settings.plexmatch_write_on_import {
                self.upsert_scoped_system_setting_json(
                    key_name,
                    &library.id,
                    &value,
                    Some(actor.id.clone()),
                )
                .await?;
            } else {
                self.delete_scoped_system_setting(key_name, &library.id)
                    .await?;
            }
        }

        if let Some(entries) = settings.indexer_routing {
            let payload = indexer_routing_payload(entries)?;
            self.upsert_scoped_system_setting_json(
                INDEXER_ROUTING_SETTINGS_KEY,
                &library.id,
                &serde_json::Value::Object(payload),
                Some(actor.id.clone()),
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(INDEXER_ROUTING_SETTINGS_KEY, &library.id)
                .await?;
        }

        if let Some(entries) = settings.download_client_routing {
            let entries = self
                .complete_library_download_client_routing_entries(entries)
                .await?;
            let payload = download_client_routing_payload(entries)?;
            self.upsert_scoped_system_setting_json(
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
                &serde_json::Value::Object(payload),
                Some(actor.id.clone()),
            )
            .await?;
            self.delete_scoped_system_setting(
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
            )
            .await?;
        } else {
            self.delete_scoped_system_setting(DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY, &library.id)
                .await?;
            self.delete_scoped_system_setting(
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &library.id,
            )
            .await?;
        }

        let mut changed_keys = vec![
            REQUIRED_AUDIO_LANGUAGES_KEY.to_string(),
            QUALITY_PROFILE_ID_KEY.to_string(),
            SCORING_PERSONA_KEY.to_string(),
            ANIME_FILLER_POLICY_KEY.to_string(),
            ANIME_RECAP_POLICY_KEY.to_string(),
            ANIME_MONITOR_SPECIALS_KEY.to_string(),
            ANIME_INTER_SEASON_MOVIES_KEY.to_string(),
            ANIME_MONITOR_FILLER_MOVIES_KEY.to_string(),
            nfo_write_on_import_key(&library.facet).to_string(),
            INDEXER_ROUTING_SETTINGS_KEY.to_string(),
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY.to_string(),
        ];
        if let Some(key_name) = plexmatch_write_on_import_key(&library.facet) {
            changed_keys.push(key_name.to_string());
        }

        self.emit_settings_saved(
            actor,
            "library_settings",
            Some(library.id.clone()),
            changed_keys,
        )
        .await;

        self.get_library_settings(actor, &library.id).await
    }

    pub async fn set_title_required_audio_override(
        &self,
        actor: &User,
        title_id: &str,
        languages: Option<Vec<String>>,
    ) -> AppResult<()> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;

        let payload = languages.map(normalize_required_audio_languages);
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                Some(title_id.trim().to_string()),
                serde_json::to_string(&payload)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await
    }

    pub async fn set_facet_required_audio_languages(
        &self,
        actor: &User,
        scope_id: &str,
        languages: Vec<String>,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let normalized = normalize_required_audio_languages(languages);
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                REQUIRED_AUDIO_LANGUAGES_KEY,
                Some(scope_id.trim().to_string()),
                serde_json::to_string(&normalized)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await
    }

    pub async fn migrate_canonical_audio_persona_settings(&self) -> AppResult<()> {
        if self
            .read_setting_bool_value(AUDIO_PERSONA_MIGRATION_SENTINEL_KEY, None)
            .await?
            == Some(true)
        {
            return Ok(());
        }

        let mut changed_keys = Vec::new();

        let existing_global_persona = parse_scoring_persona_setting(
            self.read_setting_string_value(SCORING_PERSONA_KEY, None)
                .await?,
        );
        let existing_facet_personas = {
            let mut values = HashMap::new();
            for scope_id in ["movie", "series", "anime"] {
                if let Some(persona) = parse_scoring_persona_setting(
                    self.read_setting_string_value_explicit(SCORING_PERSONA_KEY, Some(scope_id))
                        .await?,
                ) {
                    values.insert(scope_id.to_string(), persona);
                }
            }
            values
        };

        let profiles = self
            .services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
            .unwrap_or_default();
        let mut selected_profile_ids_by_scope = HashMap::new();
        for scope_id in ["movie", "series", "anime"] {
            selected_profile_ids_by_scope.insert(
                scope_id.to_string(),
                self.read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(scope_id))
                    .await?,
            );
        }

        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;
        let selected_global_profile = global_profile_id
            .as_deref()
            .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
            .or_else(|| profiles.first());
        let global_persona = existing_global_persona.unwrap_or_else(|| {
            selected_global_profile
                .map(|profile| profile.criteria.scoring_persona.clone())
                .unwrap_or_default()
        });

        self.upsert_system_setting_json(
            SCORING_PERSONA_KEY,
            &global_persona_as_setting(&global_persona),
            None,
        )
        .await?;
        changed_keys.push(SCORING_PERSONA_KEY.to_string());

        for scope_id in ["movie", "series", "anime"] {
            let selected_profile_id = selected_profile_ids_by_scope
                .get(scope_id)
                .cloned()
                .flatten();
            let profile = selected_profile_id
                .as_deref()
                .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
                .or(selected_global_profile);
            let effective_persona = existing_facet_personas
                .get(scope_id)
                .cloned()
                .or_else(|| {
                    profile.and_then(|profile| {
                        profile
                            .criteria
                            .facet_persona_overrides
                            .get(scope_id)
                            .cloned()
                            .or_else(|| Some(profile.criteria.scoring_persona.clone()))
                    })
                })
                .unwrap_or_else(|| global_persona.clone());

            if effective_persona != global_persona {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        SCORING_PERSONA_KEY,
                        Some(scope_id.to_string()),
                        serde_json::to_string(&global_persona_as_setting(&effective_persona))
                            .map_err(|error| AppError::Repository(error.to_string()))?,
                        "startup-migration",
                        None,
                    )
                    .await?;
                if !changed_keys.iter().any(|key| key == SCORING_PERSONA_KEY) {
                    changed_keys.push(SCORING_PERSONA_KEY.to_string());
                }
            }
        }

        let managed_required_audio = self
            .services
            .customization
            .rule_sets
            .list_rule_sets_by_managed_key_prefix("convenience:required-audio:")
            .await
            .unwrap_or_default();
        let mut global_required_audio = Vec::new();
        let mut facet_required_audio = HashMap::<String, Vec<String>>::new();
        let mut title_overrides = Vec::<(String, Vec<String>)>::new();

        for rule_set in &managed_required_audio {
            let Some(managed_key) = rule_set.managed_key.as_deref() else {
                continue;
            };
            let languages = extract_languages_from_required_audio_rego(&rule_set.rego_source);
            if let Some(title_id) = managed_key.strip_prefix("convenience:required-audio:title:") {
                title_overrides.push((title_id.to_string(), languages));
            } else if let Some(scope_id) = managed_key.strip_prefix("convenience:required-audio:") {
                if scope_id == "global" {
                    global_required_audio = languages;
                } else {
                    facet_required_audio.insert(scope_id.to_string(), languages);
                }
            }
        }

        for scope_id in ["movie", "series", "anime"] {
            let current = self.load_facet_required_audio_languages(scope_id).await?;
            if !current.is_empty() {
                continue;
            }

            let migrated = facet_required_audio
                .get(scope_id)
                .cloned()
                .or_else(|| {
                    (!global_required_audio.is_empty()).then(|| global_required_audio.clone())
                })
                .or_else(|| {
                    let selected_profile_id = selected_profile_ids_by_scope
                        .get(scope_id)
                        .cloned()
                        .flatten();
                    selected_profile_id
                        .as_deref()
                        .and_then(|profile_id| {
                            profiles.iter().find(|profile| profile.id == profile_id)
                        })
                        .or(selected_global_profile)
                        .map(|profile| {
                            normalize_required_audio_languages(
                                profile.criteria.required_audio_languages.clone(),
                            )
                        })
                })
                .unwrap_or_default();

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    Some(scope_id.to_string()),
                    serde_json::to_string(&migrated)
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    "startup-migration",
                    None,
                )
                .await?;
            if !changed_keys
                .iter()
                .any(|key| key == REQUIRED_AUDIO_LANGUAGES_KEY)
            {
                changed_keys.push(REQUIRED_AUDIO_LANGUAGES_KEY.to_string());
            }
        }

        for (title_id, languages) in title_overrides {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    TITLE_REQUIRED_AUDIO_OVERRIDE_KEY,
                    Some(title_id),
                    serde_json::to_string(&Some(languages))
                        .map_err(|error| AppError::Repository(error.to_string()))?,
                    "startup-migration",
                    None,
                )
                .await?;
            if !changed_keys
                .iter()
                .any(|key| key == TITLE_REQUIRED_AUDIO_OVERRIDE_KEY)
            {
                changed_keys.push(TITLE_REQUIRED_AUDIO_OVERRIDE_KEY.to_string());
            }
        }

        for rule_set in managed_required_audio {
            self.services
                .customization
                .rule_sets
                .delete_rule_set(&rule_set.id)
                .await?;
        }

        let legacy_dual_rules = self
            .services
            .customization
            .rule_sets
            .list_rule_sets()
            .await?;
        for rule_set in legacy_dual_rules {
            let managed_key = rule_set.managed_key.as_deref().unwrap_or_default();
            let description = rule_set.description.as_str();
            let rego_source = rule_set.rego_source.as_str();
            if managed_key.starts_with("convenience:prefer-dual-audio:")
                || description.contains("legacy-prefer-dual-audio:")
                || rego_source.contains("legacy-prefer-dual-audio:")
            {
                self.services
                    .customization
                    .rule_sets
                    .delete_rule_set(&rule_set.id)
                    .await?;
            }
        }

        let scrubbed_profiles: Vec<crate::QualityProfile> = profiles
            .into_iter()
            .map(|mut profile| {
                profile.criteria.prefer_dual_audio = false;
                profile.criteria.required_audio_languages.clear();
                profile.criteria.scoring_persona = ScoringPersona::Balanced;
                profile.criteria.facet_persona_overrides.clear();
                profile
            })
            .collect();
        self.services
            .config
            .quality_profiles
            .replace_quality_profiles(SETTINGS_SCOPE_SYSTEM, None, scrubbed_profiles)
            .await?;
        if !changed_keys
            .iter()
            .any(|key| key == QUALITY_PROFILE_CATALOG_KEY)
        {
            changed_keys.push(QUALITY_PROFILE_CATALOG_KEY.to_string());
        }

        if !changed_keys.is_empty() {
            let _ = self
                .runtime
                .events
                .settings_changed_broadcast
                .send(changed_keys);
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                AUDIO_PERSONA_MIGRATION_SENTINEL_KEY,
                None,
                serde_json::to_string(&true)
                    .map_err(|error| AppError::Repository(error.to_string()))?,
                "startup-migration",
                None,
            )
            .await?;

        Ok(())
    }

    async fn load_subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        Ok(SubtitleSettings {
            enabled: self
                .read_setting_bool_value(SUBTITLES_ENABLED_KEY, None)
                .await?
                .unwrap_or(false),
            languages: normalize_subtitle_languages(
                self.read_setting_json_value::<Vec<SubtitleLanguagePref>>(
                    SUBTITLES_LANGUAGES_KEY,
                    None,
                )
                .await?
                .unwrap_or_default(),
            ),
            auto_download_on_import: self
                .read_setting_bool_value(SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY, None)
                .await?
                .unwrap_or(false),
            minimum_score_series: self
                .read_setting_i64_value(SUBTITLES_MINIMUM_SCORE_SERIES_KEY, None)
                .await?
                .unwrap_or(240) as i32,
            minimum_score_movie: self
                .read_setting_i64_value(SUBTITLES_MINIMUM_SCORE_MOVIE_KEY, None)
                .await?
                .unwrap_or(70) as i32,
            search_interval_hours: self
                .read_setting_i64_value(SUBTITLES_SEARCH_INTERVAL_HOURS_KEY, None)
                .await?
                .unwrap_or(6) as i32,
            include_ai_translated: self
                .read_setting_bool_value(SUBTITLES_INCLUDE_AI_TRANSLATED_KEY, None)
                .await?
                .unwrap_or(false),
            include_machine_translated: self
                .read_setting_bool_value(SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY, None)
                .await?
                .unwrap_or(false),
            sync_enabled: self
                .read_setting_bool_value(SUBTITLES_SYNC_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
            sync_threshold_series: self
                .read_setting_i64_value(SUBTITLES_SYNC_THRESHOLD_SERIES_KEY, None)
                .await?
                .unwrap_or(90) as i32,
            sync_threshold_movie: self
                .read_setting_i64_value(SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY, None)
                .await?
                .unwrap_or(70) as i32,
            sync_max_offset_seconds: self
                .read_setting_i64_value(SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY, None)
                .await?
                .unwrap_or(60) as i32,
        })
    }

    async fn load_acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        Ok(AcquisitionSettings {
            enabled: self
                .read_setting_bool_value(ACQUISITION_ENABLED_KEY, None)
                .await?
                .unwrap_or(true),
            upgrade_cooldown_hours: self
                .read_setting_i64_value(ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY, None)
                .await?
                .unwrap_or(24) as i32,
            same_tier_min_delta: self
                .read_setting_i64_value(ACQUISITION_SAME_TIER_MIN_DELTA_KEY, None)
                .await?
                .unwrap_or(120) as i32,
            cross_tier_min_delta: self
                .read_setting_i64_value(ACQUISITION_CROSS_TIER_MIN_DELTA_KEY, None)
                .await?
                .unwrap_or(30) as i32,
            forced_upgrade_delta_bypass: self
                .read_setting_i64_value(ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY, None)
                .await?
                .unwrap_or(400) as i32,
            poll_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_POLL_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(60) as i32,
            sync_interval_seconds: self
                .read_setting_i64_value(ACQUISITION_SYNC_INTERVAL_SECONDS_KEY, None)
                .await?
                .unwrap_or(3600) as i32,
            batch_size: self
                .read_setting_i64_value(ACQUISITION_BATCH_SIZE_KEY, None)
                .await?
                .unwrap_or(50) as i32,
        })
    }

    async fn load_general_settings(&self) -> AppResult<GeneralSettings> {
        let keep_history_forever = self
            .read_setting_bool_value(HISTORY_KEEP_FOREVER_KEY, None)
            .await?
            .unwrap_or(false);
        let history_retention_days = self
            .read_setting_i64_value(HISTORY_RETENTION_DAYS_KEY, None)
            .await?
            .map(|value| value.max(1) as i32)
            .unwrap_or(180);

        Ok(GeneralSettings {
            keep_history_forever,
            history_retention_days,
        })
    }

    pub(crate) async fn load_auto_backup_settings(&self) -> AppResult<AutoBackupSettings> {
        let enabled = self
            .read_setting_bool_value(AUTO_BACKUP_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let daily_time_local = normalize_auto_backup_daily_time_local(
            &self
                .read_setting_string_value(AUTO_BACKUP_DAILY_TIME_LOCAL_KEY, None)
                .await?
                .unwrap_or_else(|| DEFAULT_AUTO_BACKUP_DAILY_TIME_LOCAL.to_string()),
        )?;
        let auto_backup_key_present = self
            .read_setting_string_value(AUTO_BACKUP_KEY_KEY, None)
            .await?
            .is_some_and(|value| !value.is_empty());
        let next_run_at = if enabled {
            Some(
                crate::security::backup::compute_next_auto_backup_run_at(
                    &daily_time_local,
                    chrono::Utc::now(),
                )?
                .to_rfc3339(),
            )
        } else {
            None
        };

        Ok(AutoBackupSettings {
            enabled,
            daily_time_local,
            auto_backup_key_present,
            next_run_at,
        })
    }

    async fn load_security_settings(&self) -> AppResult<SecuritySettings> {
        let form_login_enabled = self
            .read_setting_bool_value(FORM_LOGIN_ENABLED_KEY, None)
            .await?
            .unwrap_or(false);
        let skip_login_for_local_ips = self
            .read_setting_bool_value(SKIP_LOGIN_FOR_LOCAL_IPS_KEY, None)
            .await?
            .unwrap_or(false);

        Ok(SecuritySettings {
            form_login_enabled,
            skip_login_for_local_ips,
        })
    }

    pub(crate) async fn subtitle_settings(&self) -> AppResult<SubtitleSettings> {
        self.load_subtitle_settings().await
    }

    pub(crate) async fn acquisition_settings(&self) -> AppResult<AcquisitionSettings> {
        self.load_acquisition_settings().await
    }

    pub(crate) async fn general_settings(&self) -> AppResult<GeneralSettings> {
        self.load_general_settings().await
    }

    pub async fn security_settings(&self) -> AppResult<SecuritySettings> {
        self.load_security_settings().await
    }

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

    pub async fn get_subtitle_settings(&self, actor: &User) -> AppResult<SubtitleSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.load_subtitle_settings().await
    }

    pub async fn get_acquisition_settings(&self, actor: &User) -> AppResult<AcquisitionSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.load_acquisition_settings().await
    }

    pub async fn get_general_settings(&self, actor: &User) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_general_settings().await
    }

    pub async fn get_auto_backup_settings(&self, actor: &User) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;
        self.load_auto_backup_settings().await
    }

    pub async fn get_security_settings(&self, actor: &User) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;
        self.load_security_settings().await
    }

    pub async fn setup_complete(&self) -> AppResult<bool> {
        Ok(self
            .read_setting_bool_value(SETUP_COMPLETE_KEY, None)
            .await?
            .unwrap_or(false))
    }

    pub async fn complete_setup(&self, actor: &User) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                SETUP_COMPLETE_KEY,
                None,
                encode_setting_json(&true)?,
                "setup-wizard",
                Some(actor.id.clone()),
            )
            .await?;

        Ok(true)
    }

    pub async fn queue_tvdb_movies_scan(
        &self,
        actor: &User,
        limit: i64,
        source: &str,
    ) -> AppResult<WorkflowOperationInfo> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if limit <= 0 {
            return Err(AppError::Validation(
                "limit is required and must be greater than zero".into(),
            ));
        }

        let source = source.trim();
        if source.is_empty() {
            return Err(AppError::Validation("source is required".into()));
        }

        self.services
            .workflow
            .workflow_operations
            .create_workflow_operation(
                "tvdb_movies_scan".to_string(),
                "queued".to_string(),
                Some(actor.id.clone()),
                Some(
                    serde_json::json!({
                        "type": "tvdb_movies_scan",
                        "limit": limit,
                        "source": source,
                    })
                    .to_string(),
                ),
                None,
                None,
            )
            .await
    }

    pub async fn get_delay_profiles(&self, actor: &User) -> AppResult<Vec<crate::DelayProfile>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.delay_profiles().await
    }

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

    pub async fn get_quality_profile_settings(
        &self,
        actor: &User,
    ) -> AppResult<QualityProfileSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.load_quality_profile_settings().await
    }

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
            Some(actor.id.clone()),
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
            Some(actor.id.clone()),
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

    pub async fn get_media_settings(
        &self,
        actor: &User,
        facet: MediaFacet,
    ) -> AppResult<MediaSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let root_folders = self.root_folders_for_facet(&facet).await?;
        let library_path = default_path_from_root_folders(&facet, &root_folders);
        let scoped_rename_template = self
            .read_setting_string_value(RENAME_TEMPLATE_KEY, Some(facet.as_str()))
            .await?;
        let global_rename_template = self
            .read_setting_string_value(rename_template_global_key(&facet), None)
            .await?;
        let rename_template = scoped_rename_template
            .or(global_rename_template)
            .unwrap_or_else(|| default_rename_template(&facet).to_string());
        let scoped_collision_policy = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_KEY, Some(facet.as_str()))
            .await?;
        let global_collision_policy = self
            .read_setting_string_value(RENAME_COLLISION_POLICY_GLOBAL_KEY, None)
            .await?;
        let legacy_collision_policy = self
            .read_setting_string_value(legacy_collision_policy_global_key(&facet), None)
            .await?;
        let rename_collision_policy = scoped_collision_policy
            .or(global_collision_policy)
            .or(legacy_collision_policy)
            .unwrap_or_else(|| DEFAULT_RENAME_COLLISION_POLICY.to_string());
        let scoped_missing_metadata_policy = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_KEY, Some(facet.as_str()))
            .await?;
        let global_missing_metadata_policy = self
            .read_setting_string_value(RENAME_MISSING_METADATA_POLICY_GLOBAL_KEY, None)
            .await?;
        let legacy_missing_metadata_policy = self
            .read_setting_string_value(legacy_missing_metadata_policy_global_key(&facet), None)
            .await?;
        let rename_missing_metadata_policy = scoped_missing_metadata_policy
            .or(global_missing_metadata_policy)
            .or(legacy_missing_metadata_policy)
            .unwrap_or_else(|| DEFAULT_RENAME_MISSING_METADATA_POLICY.to_string());

        Ok(MediaSettings {
            library_path,
            root_folders,
            required_audio_languages: self
                .load_facet_required_audio_languages(facet.as_str())
                .await?,
            rename_template,
            rename_collision_policy,
            rename_missing_metadata_policy,
            filler_policy: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_string_value(ANIME_FILLER_POLICY_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or_else(|| DEFAULT_FILLER_POLICY.to_string()),
                )
            } else {
                None
            },
            recap_policy: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_string_value(ANIME_RECAP_POLICY_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or_else(|| DEFAULT_RECAP_POLICY.to_string()),
                )
            } else {
                None
            },
            monitor_specials: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(ANIME_MONITOR_SPECIALS_KEY, Some(facet.as_str()))
                        .await?
                        .unwrap_or(false),
                )
            } else {
                None
            },
            inter_season_movies: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(
                        ANIME_INTER_SEASON_MOVIES_KEY,
                        Some(facet.as_str()),
                    )
                    .await?
                    .unwrap_or(true),
                )
            } else {
                None
            },
            monitor_filler_movies: if facet == MediaFacet::Anime {
                Some(
                    self.read_setting_bool_value(
                        ANIME_MONITOR_FILLER_MOVIES_KEY,
                        Some(facet.as_str()),
                    )
                    .await?
                    .unwrap_or(false),
                )
            } else {
                None
            },
            nfo_write_on_import: self
                .read_setting_bool_value(nfo_write_on_import_key(&facet), None)
                .await?
                .unwrap_or(false),
            plexmatch_write_on_import: match plexmatch_write_on_import_key(&facet) {
                Some(key) => Some(
                    self.read_setting_bool_value(key, None)
                        .await?
                        .unwrap_or(false),
                ),
                None => None,
            },
        })
    }

    pub async fn update_media_settings(
        &self,
        actor: &User,
        facet: MediaFacet,
        input: UpdateMediaSettings,
    ) -> AppResult<MediaSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let previous_roots = self.effective_scan_roots_for_facet(&facet).await?;
        let root_folder_update = input
            .root_folders
            .clone()
            .map(normalize_root_folders)
            .transpose()?;

        let mut changed_keys = Vec::new();

        if let Some(normalized) = root_folder_update {
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &facet,
                    &normalized,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        } else if let Some(library_path) = normalize_optional_string(input.library_path) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: library_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &facet,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(rename_template) = normalize_optional_string(input.rename_template) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_TEMPLATE_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&rename_template)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_TEMPLATE_KEY.to_string());
        }

        if let Some(required_audio_languages) = input.required_audio_languages {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    REQUIRED_AUDIO_LANGUAGES_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&normalize_required_audio_languages(
                        required_audio_languages,
                    ))?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(REQUIRED_AUDIO_LANGUAGES_KEY.to_string());
        }

        if let Some(policy) = normalize_optional_string(input.rename_collision_policy) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_COLLISION_POLICY_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&policy)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_COLLISION_POLICY_KEY.to_string());
        }

        if let Some(policy) = normalize_optional_string(input.rename_missing_metadata_policy) {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    RENAME_MISSING_METADATA_POLICY_KEY,
                    Some(facet.as_str().to_string()),
                    encode_setting_json(&policy)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(RENAME_MISSING_METADATA_POLICY_KEY.to_string());
        }

        if let Some(value) = input.nfo_write_on_import {
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    nfo_write_on_import_key(&facet),
                    None,
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(nfo_write_on_import_key(&facet).to_string());
        }

        if let Some(value) = input.plexmatch_write_on_import {
            let Some(key) = plexmatch_write_on_import_key(&facet) else {
                return Err(AppError::Validation(
                    "plexmatch_write_on_import is only valid for series and anime".to_string(),
                ));
            };
            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    key,
                    None,
                    encode_setting_json(&value)?,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?;
            changed_keys.push(key.to_string());
        }

        if facet == MediaFacet::Anime {
            if let Some(value) = normalize_optional_string(input.filler_policy) {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_FILLER_POLICY_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_FILLER_POLICY_KEY.to_string());
            }
            if let Some(value) = normalize_optional_string(input.recap_policy) {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_RECAP_POLICY_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_RECAP_POLICY_KEY.to_string());
            }
            if let Some(value) = input.monitor_specials {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_MONITOR_SPECIALS_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_MONITOR_SPECIALS_KEY.to_string());
            }
            if let Some(value) = input.inter_season_movies {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_INTER_SEASON_MOVIES_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_INTER_SEASON_MOVIES_KEY.to_string());
            }
            if let Some(value) = input.monitor_filler_movies {
                self.services
                    .config
                    .settings
                    .upsert_setting_json(
                        SETTINGS_SCOPE_SYSTEM,
                        ANIME_MONITOR_FILLER_MOVIES_KEY,
                        Some(facet.as_str().to_string()),
                        encode_setting_json(&value)?,
                        SETTINGS_SOURCE_TYPED_GRAPHQL,
                        Some(actor.id.clone()),
                    )
                    .await?;
                changed_keys.push(ANIME_MONITOR_FILLER_MOVIES_KEY.to_string());
            }
        } else if input.filler_policy.is_some()
            || input.recap_policy.is_some()
            || input.monitor_specials.is_some()
            || input.inter_season_movies.is_some()
            || input.monitor_filler_movies.is_some()
        {
            return Err(AppError::Validation(
                "anime-specific settings require scope anime".to_string(),
            ));
        }

        if changed_keys.is_empty() {
            return Err(AppError::Validation(
                "at least one media setting change is required".to_string(),
            ));
        }

        let current_roots = self.effective_scan_roots_for_facet(&facet).await?;
        self.clear_pending_imports_for_removed_roots(&facet, &previous_roots, &current_roots)
            .await?;

        self.emit_settings_saved(
            actor,
            "media_settings",
            Some(facet.as_str().to_string()),
            changed_keys,
        )
        .await;

        self.get_media_settings(actor, facet).await
    }

    pub async fn get_library_paths(&self, actor: &User) -> AppResult<LibraryPathsSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        let movie_roots = self.root_folders_for_facet(&MediaFacet::Movie).await?;
        let series_roots = self.root_folders_for_facet(&MediaFacet::Series).await?;
        let anime_roots = self.root_folders_for_facet(&MediaFacet::Anime).await?;

        Ok(LibraryPathsSettings {
            movie_path: default_path_from_root_folders(&MediaFacet::Movie, &movie_roots),
            series_path: default_path_from_root_folders(&MediaFacet::Series, &series_roots),
            anime_path: default_path_from_root_folders(&MediaFacet::Anime, &anime_roots),
        })
    }

    pub async fn update_library_paths(
        &self,
        actor: &User,
        input: UpdateLibraryPaths,
    ) -> AppResult<LibraryPathsSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;
        self.ensure_default_facet_libraries().await?;
        let previous_roots = [
            (
                MediaFacet::Movie,
                self.effective_scan_roots_for_facet(&MediaFacet::Movie)
                    .await?,
            ),
            (
                MediaFacet::Series,
                self.effective_scan_roots_for_facet(&MediaFacet::Series)
                    .await?,
            ),
            (
                MediaFacet::Anime,
                self.effective_scan_roots_for_facet(&MediaFacet::Anime)
                    .await?,
            ),
        ];

        let mut changed_keys = Vec::new();
        if let Some(movie_path) = normalize_optional_string(Some(input.movie_path)) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: movie_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Movie,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(series_path) = normalize_optional_string(Some(input.series_path)) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: series_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Series,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if let Some(anime_path) = normalize_optional_string(input.anime_path) {
            let root_folders = normalize_root_folders(vec![RootFolderEntry {
                path: anime_path,
                is_default: true,
            }])?;
            changed_keys.extend(
                self.update_default_library_roots_from_entries(
                    &MediaFacet::Anime,
                    &root_folders,
                    SETTINGS_SOURCE_TYPED_GRAPHQL,
                    Some(actor.id.clone()),
                )
                .await?,
            );
        }

        if changed_keys.is_empty() {
            return self.get_library_paths(actor).await;
        }
        warn!("updateLibraryPaths is deprecated; updated default library roots instead");

        for (facet, previous) in previous_roots {
            let current = self.effective_scan_roots_for_facet(&facet).await?;
            self.clear_pending_imports_for_removed_roots(&facet, &previous, &current)
                .await?;
        }

        self.emit_settings_saved(actor, "library_paths", None, changed_keys)
            .await;
        self.get_library_paths(actor).await
    }

    pub async fn save_external_import_library_paths(
        &self,
        actor: &User,
        selection: ExternalImportLibraryPathsSelection,
    ) -> AppResult<bool> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut saved_any = false;
        for (facet, paths) in [
            (MediaFacet::Movie, selection.movie_paths),
            (MediaFacet::Series, selection.series_paths),
            (MediaFacet::Anime, selection.anime_paths),
        ] {
            let Some(root_folders) = normalize_external_import_root_folders(paths)? else {
                continue;
            };

            self.update_media_settings(
                actor,
                facet,
                UpdateMediaSettings {
                    library_path: None,
                    root_folders: Some(root_folders),
                    required_audio_languages: None,
                    rename_template: None,
                    rename_collision_policy: None,
                    rename_missing_metadata_policy: None,
                    filler_policy: None,
                    recap_policy: None,
                    monitor_specials: None,
                    inter_season_movies: None,
                    monitor_filler_movies: None,
                    nfo_write_on_import: None,
                    plexmatch_write_on_import: None,
                },
            )
            .await?;
            saved_any = true;
        }

        if !saved_any {
            return Ok(false);
        }

        Ok(true)
    }

    pub async fn get_service_settings(&self, actor: &User) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        Ok(ServiceSettings {
            tls_cert_path: self
                .read_setting_string_value(TLS_CERT_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
            tls_key_path: self
                .read_setting_string_value(TLS_KEY_PATH_KEY, None)
                .await?
                .unwrap_or_default(),
        })
    }

    pub async fn update_general_settings(
        &self,
        actor: &User,
        input: UpdateGeneralSettings,
    ) -> AppResult<GeneralSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let current = self.load_general_settings().await?;
        let history_retention_days =
            if input.keep_history_forever && input.history_retention_days < 1 {
                current.history_retention_days
            } else {
                input.history_retention_days
            };

        if history_retention_days < 1 {
            return Err(AppError::Validation(
                "history retention days must be at least 1".to_string(),
            ));
        }

        self.upsert_system_setting_json(
            HISTORY_KEEP_FOREVER_KEY,
            &input.keep_history_forever,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            HISTORY_RETENTION_DAYS_KEY,
            &history_retention_days,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_settings_saved(
            actor,
            "general_settings",
            None,
            vec![
                HISTORY_KEEP_FOREVER_KEY.to_string(),
                HISTORY_RETENTION_DAYS_KEY.to_string(),
            ],
        )
        .await;

        Ok(GeneralSettings {
            keep_history_forever: input.keep_history_forever,
            history_retention_days,
        })
    }

    pub async fn update_auto_backup_settings(
        &self,
        actor: &User,
        input: UpdateAutoBackupSettings,
    ) -> AppResult<AutoBackupSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        validate_auto_backup_key_update(
            input.set_auto_backup_key.as_deref(),
            input.clear_auto_backup_key,
        )?;
        let daily_time_local = normalize_auto_backup_daily_time_local(&input.daily_time_local)?;

        self.upsert_system_setting_json(
            AUTO_BACKUP_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY,
            &daily_time_local,
            Some(actor.id.clone()),
        )
        .await?;

        let mut changed_keys = vec![
            AUTO_BACKUP_ENABLED_KEY.to_string(),
            AUTO_BACKUP_DAILY_TIME_LOCAL_KEY.to_string(),
        ];

        if input.clear_auto_backup_key {
            self.delete_system_setting(AUTO_BACKUP_KEY_KEY).await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        } else if let Some(set_auto_backup_key) = input.set_auto_backup_key
            && !set_auto_backup_key.is_empty()
        {
            self.upsert_system_setting_json(
                AUTO_BACKUP_KEY_KEY,
                &set_auto_backup_key,
                Some(actor.id.clone()),
            )
            .await?;
            changed_keys.push(AUTO_BACKUP_KEY_KEY.to_string());
        }

        self.emit_settings_saved(actor, "auto_backup_settings", None, changed_keys)
            .await;

        self.load_auto_backup_settings().await
    }

    pub async fn update_security_settings(
        &self,
        actor: &User,
        input: UpdateSecuritySettings,
    ) -> AppResult<SecuritySettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageUsers)
            .await?;

        self.upsert_system_setting_json(
            FORM_LOGIN_ENABLED_KEY,
            &input.form_login_enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SKIP_LOGIN_FOR_LOCAL_IPS_KEY,
            &input.skip_login_for_local_ips,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_settings_saved(
            actor,
            "security_settings",
            None,
            vec![
                FORM_LOGIN_ENABLED_KEY.to_string(),
                SKIP_LOGIN_FOR_LOCAL_IPS_KEY.to_string(),
            ],
        )
        .await;

        Ok(SecuritySettings {
            form_login_enabled: input.form_login_enabled,
            skip_login_for_local_ips: input.skip_login_for_local_ips,
        })
    }

    pub async fn update_service_settings(
        &self,
        actor: &User,
        input: UpdateServiceSettings,
    ) -> AppResult<ServiceSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageSystemSettings)
            .await?;

        let tls_cert_path = input.tls_cert_path.trim().to_string();
        let tls_key_path = input.tls_key_path.trim().to_string();

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_CERT_PATH_KEY,
                None,
                encode_setting_json(&tls_cert_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;
        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                TLS_KEY_PATH_KEY,
                None,
                encode_setting_json(&tls_key_path)?,
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "service_settings",
            None,
            vec![TLS_CERT_PATH_KEY.to_string(), TLS_KEY_PATH_KEY.to_string()],
        )
        .await;

        self.get_service_settings(actor).await
    }

    pub async fn get_download_client_routing(
        &self,
        actor: &User,
        scope_id: &str,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let raw_json = self.load_download_client_routing_json(scope_id).await?;
        let Some(raw_json) = raw_json else {
            return Ok(Vec::new());
        };
        let Some(entries) = crate::catalog_helpers::parse_download_client_routing_map(&raw_json)
        else {
            return Ok(Vec::new());
        };

        let mut routing = entries
            .into_iter()
            .map(|(client_id, config)| {
                let entry = crate::catalog_helpers::parse_download_client_routing_entry(&config);
                DownloadClientRoutingSettingsEntry {
                    client_id,
                    enabled: entry.enabled,
                    category: entry.category,
                    recent_queue_priority: entry.recent_queue_priority,
                    older_queue_priority: entry.older_queue_priority,
                    remove_completed: entry.remove_completed,
                    remove_failed: entry.remove_failed,
                }
            })
            .collect::<Vec<_>>();
        routing.sort_by(|left, right| left.client_id.cmp(&right.client_id));
        Ok(routing)
    }

    pub async fn update_download_client_routing(
        &self,
        actor: &User,
        scope_id: &str,
        entries: Vec<DownloadClientRoutingSettingsEntry>,
    ) -> AppResult<Vec<DownloadClientRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut payload = serde_json::Map::new();
        for entry in entries {
            let client_id = entry.client_id.trim();
            if client_id.is_empty() {
                return Err(AppError::Validation(
                    "download client routing entry requires client_id".to_string(),
                ));
            }

            payload.insert(
                client_id.to_string(),
                serde_json::json!({
                    "enabled": entry.enabled,
                    "category": normalize_optional_string(entry.category),
                    "recentQueuePriority": normalize_optional_string(entry.recent_queue_priority),
                    "olderQueuePriority": normalize_optional_string(entry.older_queue_priority),
                    "removeCompleted": entry.remove_completed,
                    "removeFailed": entry.remove_failed,
                }),
            );
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
                serde_json::Value::Object(payload).to_string(),
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "download_client_routing",
            Some(scope_id.to_string()),
            vec![DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY.to_string()],
        )
        .await;

        self.get_download_client_routing(actor, scope_id).await
    }

    pub async fn ensure_download_client_routing_entry_for_client(
        &self,
        actor: &User,
        client_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        for scope_id in ["movie", "series", "anime"] {
            let current = self.load_download_client_routing_json(scope_id).await?;
            let mut payload = current
                .as_deref()
                .and_then(parse_json_object)
                .unwrap_or_default();

            if payload.contains_key(client_id) {
                continue;
            }

            let next_priority = next_routing_priority(&payload);
            payload.insert(
                client_id.to_string(),
                default_download_client_routing_entry_json(next_priority),
            );

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                    Some(scope_id.to_string()),
                    serde_json::Value::Object(payload).to_string(),
                    "admin_graphql",
                    Some(actor.id.clone()),
                )
                .await?;
        }

        Ok(())
    }

    pub async fn ensure_indexer_routing_entry_for_indexer(
        &self,
        actor: &User,
        indexer_id: &str,
    ) -> AppResult<()> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        self.ensure_indexer_routing_entry_for_indexer_internal(
            indexer_id,
            "admin_graphql",
            Some(actor.id.clone()),
        )
        .await
    }

    async fn ensure_indexer_routing_entry_for_indexer_internal(
        &self,
        indexer_id: &str,
        source: &str,
        updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        let indexer_id = indexer_id.trim();
        if indexer_id.is_empty() {
            return Err(AppError::Validation(
                "indexer routing entry requires indexer_id".to_string(),
            ));
        }

        for scope_id in ["movie", "series", "anime"] {
            let current = self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
                .await?;
            let mut payload = current
                .as_deref()
                .and_then(parse_json_object)
                .unwrap_or_default();

            if payload.contains_key(indexer_id) {
                continue;
            }

            let next_priority = next_routing_priority(&payload);
            payload.insert(
                indexer_id.to_string(),
                default_indexer_routing_entry_json(scope_id, next_priority),
            );

            self.services
                .config
                .settings
                .upsert_setting_json(
                    SETTINGS_SCOPE_SYSTEM,
                    INDEXER_ROUTING_SETTINGS_KEY,
                    Some(scope_id.to_string()),
                    serde_json::Value::Object(payload).to_string(),
                    source,
                    updated_by_user_id.clone(),
                )
                .await?;
        }

        Ok(())
    }

    pub async fn ensure_indexer_routing_entries_for_existing_indexers(&self) -> AppResult<()> {
        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await?;
        for config in configs {
            self.ensure_indexer_routing_entry_for_indexer_internal(
                &config.id,
                "startup_reconcile",
                None,
            )
            .await?;
        }
        Ok(())
    }

    pub async fn get_indexer_routing(
        &self,
        actor: &User,
        scope_id: &str,
    ) -> AppResult<Vec<IndexerRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let Some(plan) = self.resolve_indexer_routing(None, Some(scope_id)).await else {
            return Ok(Vec::new());
        };

        let mut routing = plan
            .entries
            .into_iter()
            .map(|(indexer_id, entry)| IndexerRoutingSettingsEntry {
                indexer_id,
                enabled: entry.enabled,
                categories: entry.categories,
                priority: entry.priority as i32,
            })
            .collect::<Vec<_>>();
        routing.sort_by_key(|entry| (entry.priority, entry.indexer_id.clone()));
        Ok(routing)
    }

    pub async fn update_indexer_routing(
        &self,
        actor: &User,
        scope_id: &str,
        entries: Vec<IndexerRoutingSettingsEntry>,
    ) -> AppResult<Vec<IndexerRoutingSettingsEntry>> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        let mut payload = serde_json::Map::new();
        for entry in entries {
            let indexer_id = entry.indexer_id.trim();
            if indexer_id.is_empty() {
                return Err(AppError::Validation(
                    "indexer routing entry requires indexer_id".to_string(),
                ));
            }

            let categories = entry
                .categories
                .into_iter()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>();

            payload.insert(
                indexer_id.to_string(),
                serde_json::json!({
                    "enabled": entry.enabled,
                    "categories": categories,
                    "priority": entry.priority,
                }),
            );
        }

        self.services
            .config
            .settings
            .upsert_setting_json(
                SETTINGS_SCOPE_SYSTEM,
                INDEXER_ROUTING_SETTINGS_KEY,
                Some(scope_id.to_string()),
                serde_json::Value::Object(payload).to_string(),
                SETTINGS_SOURCE_TYPED_GRAPHQL,
                Some(actor.id.clone()),
            )
            .await?;

        self.emit_settings_saved(
            actor,
            "indexer_routing",
            Some(scope_id.to_string()),
            vec![INDEXER_ROUTING_SETTINGS_KEY.to_string()],
        )
        .await;

        self.get_indexer_routing(actor, scope_id).await
    }

    /// Idempotent backfill: walks all persisted routing settings and rewrites
    /// any entry that is missing canonical fields with explicit defaults.
    /// Intended to run once per startup so legacy installs converge on the
    /// fully-materialized JSON shape that the typed write paths now produce.
    /// Reads stay read-only — this is the single explicit write boundary for
    /// the migration.
    pub async fn normalize_routing_settings(&self) -> AppResult<()> {
        const NORMALIZE_SOURCE: &str = "startup_normalize_routing";

        for scope_id in ["movie", "series", "anime"] {
            if let Some(raw_json) = self.load_download_client_routing_json(scope_id).await?
                && let Some(mut payload) = parse_json_object(&raw_json)
            {
                let mut changed = false;
                let mut next_priority = next_routing_priority(&payload);
                for (_, value) in payload.iter_mut() {
                    if let Some(entry) = value.as_object_mut() {
                        let missing_priority = !entry.contains_key("priority");
                        if normalize_download_client_routing_entry_in_place(entry, next_priority) {
                            changed = true;
                            if missing_priority {
                                next_priority += 1;
                            }
                        }
                    }
                }
                if changed {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                            Some(scope_id.to_string()),
                            serde_json::Value::Object(payload).to_string(),
                            NORMALIZE_SOURCE,
                            None,
                        )
                        .await?;
                }
            }
        }

        for scope_id in ["movie", "series", "anime"] {
            if let Some(raw_json) = self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
                .await?
                && let Some(mut payload) = parse_json_object(&raw_json)
            {
                let mut changed = false;
                let mut next_priority = next_routing_priority(&payload);
                for (_, value) in payload.iter_mut() {
                    if let Some(entry) = value.as_object_mut() {
                        let missing_priority = !entry.contains_key("priority");
                        if normalize_indexer_routing_entry_in_place(scope_id, entry, next_priority)
                        {
                            changed = true;
                            if missing_priority {
                                next_priority += 1;
                            }
                        }
                    }
                }
                if changed {
                    self.services
                        .config
                        .settings
                        .upsert_setting_json(
                            SETTINGS_SCOPE_SYSTEM,
                            INDEXER_ROUTING_SETTINGS_KEY,
                            Some(scope_id.to_string()),
                            serde_json::Value::Object(payload).to_string(),
                            NORMALIZE_SOURCE,
                            None,
                        )
                        .await?;
                }
            }
        }

        Ok(())
    }

    pub async fn update_subtitle_settings(
        &self,
        actor: &User,
        input: UpdateSubtitleSettings,
    ) -> AppResult<SubtitleSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if input.search_interval_hours < 1 {
            return Err(AppError::Validation(
                "subtitle search interval must be at least 1 hour".to_string(),
            ));
        }
        if input.minimum_score_series < 0 || input.minimum_score_movie < 0 {
            return Err(AppError::Validation(
                "subtitle minimum scores cannot be negative".to_string(),
            ));
        }
        if input.sync_threshold_series < 0
            || input.sync_threshold_movie < 0
            || input.sync_max_offset_seconds < 0
        {
            return Err(AppError::Validation(
                "subtitle sync settings cannot be negative".to_string(),
            ));
        }

        let languages = normalize_subtitle_languages(input.languages);
        self.upsert_system_setting_json(
            SUBTITLES_ENABLED_KEY,
            &input.enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_LANGUAGES_KEY,
            &languages,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY,
            &input.auto_download_on_import,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_MINIMUM_SCORE_SERIES_KEY,
            &input.minimum_score_series,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_MINIMUM_SCORE_MOVIE_KEY,
            &input.minimum_score_movie,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SEARCH_INTERVAL_HOURS_KEY,
            &input.search_interval_hours,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_INCLUDE_AI_TRANSLATED_KEY,
            &input.include_ai_translated,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY,
            &input.include_machine_translated,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_ENABLED_KEY,
            &input.sync_enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_THRESHOLD_SERIES_KEY,
            &input.sync_threshold_series,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY,
            &input.sync_threshold_movie,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY,
            &input.sync_max_offset_seconds,
            Some(actor.id.clone()),
        )
        .await?;

        let changed_keys = vec![
            SUBTITLES_ENABLED_KEY.to_string(),
            SUBTITLES_LANGUAGES_KEY.to_string(),
            SUBTITLES_AUTO_DOWNLOAD_ON_IMPORT_KEY.to_string(),
            SUBTITLES_MINIMUM_SCORE_SERIES_KEY.to_string(),
            SUBTITLES_MINIMUM_SCORE_MOVIE_KEY.to_string(),
            SUBTITLES_SEARCH_INTERVAL_HOURS_KEY.to_string(),
            SUBTITLES_INCLUDE_AI_TRANSLATED_KEY.to_string(),
            SUBTITLES_INCLUDE_MACHINE_TRANSLATED_KEY.to_string(),
            SUBTITLES_SYNC_ENABLED_KEY.to_string(),
            SUBTITLES_SYNC_THRESHOLD_SERIES_KEY.to_string(),
            SUBTITLES_SYNC_THRESHOLD_MOVIE_KEY.to_string(),
            SUBTITLES_SYNC_MAX_OFFSET_SECONDS_KEY.to_string(),
        ];

        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "subtitle_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self
            .runtime
            .events
            .settings_changed_broadcast
            .send(changed_keys);
        self.load_subtitle_settings().await
    }

    pub async fn update_acquisition_settings(
        &self,
        actor: &User,
        settings: AcquisitionSettings,
    ) -> AppResult<AcquisitionSettings> {
        self.require_app_permission(actor, scryer_domain::AppPermission::ManageCatalogSettings)
            .await?;

        if settings.upgrade_cooldown_hours < 0
            || settings.same_tier_min_delta < 0
            || settings.cross_tier_min_delta < 0
            || settings.forced_upgrade_delta_bypass < 0
        {
            return Err(AppError::Validation(
                "acquisition thresholds cannot be negative".to_string(),
            ));
        }
        if settings.poll_interval_seconds < 1 || settings.sync_interval_seconds < 1 {
            return Err(AppError::Validation(
                "acquisition intervals must be at least 1 second".to_string(),
            ));
        }
        if settings.batch_size < 1 {
            return Err(AppError::Validation(
                "acquisition batch size must be at least 1".to_string(),
            ));
        }

        self.upsert_system_setting_json(
            ACQUISITION_ENABLED_KEY,
            &settings.enabled,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY,
            &settings.upgrade_cooldown_hours,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY,
            &settings.same_tier_min_delta,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_CROSS_TIER_MIN_DELTA_KEY,
            &settings.cross_tier_min_delta,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY,
            &settings.forced_upgrade_delta_bypass,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY,
            &settings.poll_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_SYNC_INTERVAL_SECONDS_KEY,
            &settings.sync_interval_seconds,
            Some(actor.id.clone()),
        )
        .await?;
        self.upsert_system_setting_json(
            ACQUISITION_BATCH_SIZE_KEY,
            &settings.batch_size,
            Some(actor.id.clone()),
        )
        .await?;

        self.emit_configuration_changed_event(
            Some(actor.id.clone()),
            "acquisition_settings",
            None,
            scryer_domain::ConfigurationChangeAction::Updated,
        )
        .await;
        let _ = self.runtime.events.settings_changed_broadcast.send(vec![
            ACQUISITION_ENABLED_KEY.to_string(),
            ACQUISITION_UPGRADE_COOLDOWN_HOURS_KEY.to_string(),
            ACQUISITION_SAME_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_CROSS_TIER_MIN_DELTA_KEY.to_string(),
            ACQUISITION_FORCED_UPGRADE_DELTA_BYPASS_KEY.to_string(),
            ACQUISITION_POLL_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_SYNC_INTERVAL_SECONDS_KEY.to_string(),
            ACQUISITION_BATCH_SIZE_KEY.to_string(),
        ]);
        self.runtime.acquisition.acquisition_wake.notify_one();

        self.load_acquisition_settings().await
    }

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
            Some(actor.id.clone()),
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
            Some(actor.id.clone()),
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

    pub(crate) async fn acquisition_thresholds(
        &self,
        persona: &ScoringPersona,
    ) -> AcquisitionThresholds {
        match self.load_acquisition_settings().await {
            Ok(settings) => settings.thresholds(),
            Err(error) => {
                warn!(error = %error, "failed to load acquisition settings, using persona defaults");
                AcquisitionThresholds::for_persona(persona)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_auto_backup_daily_time_local_trims_and_zero_pads_values() {
        let normalized = normalize_auto_backup_daily_time_local(" 3:5 ").expect("normalized time");

        assert_eq!(normalized, "03:05");
    }

    #[test]
    fn normalize_auto_backup_daily_time_local_rejects_invalid_values() {
        assert!(normalize_auto_backup_daily_time_local("24:00").is_err());
        assert!(normalize_auto_backup_daily_time_local("10:60").is_err());
        assert!(normalize_auto_backup_daily_time_local("nope").is_err());
    }

    #[test]
    fn validate_auto_backup_key_update_rejects_replace_and_clear_together() {
        let error = validate_auto_backup_key_update(Some("secret"), true)
            .expect_err("set and clear should be rejected");

        assert!(
            error
                .to_string()
                .contains("automatic backup key cannot be replaced and cleared"),
        );
    }
}
