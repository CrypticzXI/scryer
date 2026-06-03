use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure::SettingsStore;

use super::versioning::{MajorMinor, is_upgrade_from_before_to_at_least};

pub(crate) const TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY: &str =
    "catalog.title_image_url_rehydrate_016_state";
const STATE_NONE: &str = "none";
const STATE_PENDING: &str = "pending";
const STATE_COMPLETED: &str = "completed";

fn is_upgrade_to_016_or_later(previous_version: Option<&str>, current_version: &str) -> bool {
    is_upgrade_from_before_to_at_least(previous_version, current_version, MajorMinor::new(0, 16))
}

fn should_attempt_title_image_artwork_url_refresh(
    previous_version: Option<&str>,
    current_version: &str,
    migration_state: &str,
) -> bool {
    migration_state != STATE_COMPLETED
        && (is_upgrade_to_016_or_later(previous_version, current_version)
            || migration_state == STATE_PENDING)
}

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY,
            None,
        )
        .await
        .ok()
        .flatten()
        .and_then(|record| serde_json::from_str::<String>(&record.effective_value_json).ok())
        .unwrap_or_else(|| STATE_NONE.to_string())
}

async fn set_state(settings_store: Arc<SettingsStore>, state: &str) -> Result<(), String> {
    let value_json = serde_json::to_string(state).map_err(|error| error.to_string())?;
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            TITLE_IMAGE_ARTWORK_URL_REFRESH_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) async fn refresh_title_image_artwork_urls_for_upgrade(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
    previous_version: Option<&str>,
    current_version: &str,
) {
    let state = read_state(settings_store.clone()).await;
    if !should_attempt_title_image_artwork_url_refresh(previous_version, current_version, &state) {
        return;
    }

    if let Err(error) = set_state(settings_store.clone(), STATE_PENDING).await {
        tracing::warn!(
            error = %error,
            "failed to mark title image artwork URL refresh migration pending"
        );
        return;
    }

    match app_use_case.run_title_image_cache_refresh().await {
        Ok(summary) => {
            if let Err(error) = set_state(settings_store, STATE_COMPLETED).await {
                tracing::warn!(
                    error = %error,
                    "title image artwork URL refresh completed but failed to mark migration completed"
                );
                return;
            }
            tracing::info!(
                titles_scanned = summary.titles_scanned,
                title_urls_updated = summary.title_urls_updated,
                episode_urls_updated = summary.episode_urls_updated,
                missing_artwork_results = summary.missing_artwork_results,
                "title image artwork URL refresh migration completed"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "title image artwork URL refresh migration failed; it will retry on next startup"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_image_artwork_url_refresh_only_targets_upgrades_to_016_or_later() {
        assert!(is_upgrade_to_016_or_later(Some("0.15.9"), "0.16.0"));
        assert!(is_upgrade_to_016_or_later(Some("v0.15.0"), "0.16.1"));
        assert!(is_upgrade_to_016_or_later(Some("0.14.9"), "0.16.0"));
        assert!(is_upgrade_to_016_or_later(Some("0.15.9"), "1.0.0"));
        assert!(!is_upgrade_to_016_or_later(Some("0.16.0"), "0.16.1"));
        assert!(!is_upgrade_to_016_or_later(Some("0.15.9"), "0.15.10"));
        assert!(!is_upgrade_to_016_or_later(None, "0.16.0"));
    }

    #[test]
    fn title_image_artwork_url_refresh_retries_pending_state() {
        assert!(should_attempt_title_image_artwork_url_refresh(
            Some("0.15.9"),
            "0.16.0",
            STATE_NONE,
        ));
        assert!(should_attempt_title_image_artwork_url_refresh(
            None,
            "0.16.0",
            STATE_PENDING,
        ));
        assert!(!should_attempt_title_image_artwork_url_refresh(
            None,
            "0.16.0",
            STATE_COMPLETED,
        ));
        assert!(!should_attempt_title_image_artwork_url_refresh(
            None, "0.16.0", STATE_NONE,
        ));
    }
}
