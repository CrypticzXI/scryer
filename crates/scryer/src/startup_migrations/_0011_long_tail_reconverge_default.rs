use std::sync::Arc;

use scryer_application::SETTINGS_SCOPE_SYSTEM;
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

const RECONVERGE_DAYS_KEY: &str = "acquisition.long_tail_reconverge_days";
pub(crate) const MIGRATION_STATE_KEY: &str =
    "acquisition.long_tail_reconverge_default_30_migration_state";
const STATE_COMPLETED: &str = "completed";

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, MIGRATION_STATE_KEY, None)
        .await
        .ok()
        .flatten()
        .and_then(|record| serde_json::from_str::<String>(&record.effective_value_json).ok())
        .unwrap_or_default()
}

async fn mark_completed(settings_store: Arc<SettingsStore>) -> Result<(), String> {
    settings_store
        .upsert_setting_value(
            SETTINGS_SCOPE_SYSTEM,
            MIGRATION_STATE_KEY,
            None,
            serde_json::to_string(STATE_COMPLETED).map_err(|error| error.to_string())?,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Clears only the legacy system-written explicit zero so the new 30-day
/// definition default applies. A user-attributed zero remains an explicit
/// opt-out.
pub(crate) async fn migrate(settings_store: Arc<SettingsStore>) {
    if read_state(settings_store.clone()).await == STATE_COMPLETED {
        return;
    }

    let record = match settings_store
        .get_setting_with_defaults(SETTINGS_SCOPE_SYSTEM, RECONVERGE_DAYS_KEY, None)
        .await
    {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to read long-tail reconvergence setting; retrying migration next start"
            );
            return;
        }
    };

    if let Some(record) = record {
        let system_written = record
            .updated_by_user_id
            .as_deref()
            .is_none_or(|user_id| user_id.trim().is_empty());
        let explicit_zero = record
            .value_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<i64>(value).ok())
            == Some(0);
        if system_written
            && explicit_zero
            && let Err(error) = settings_store
                .delete_setting_value(SETTINGS_SCOPE_SYSTEM, RECONVERGE_DAYS_KEY, None)
                .await
        {
            tracing::warn!(
                error = %error,
                "failed to clear the legacy system-written reconvergence default; retrying next start"
            );
            return;
        }
    }

    if let Err(error) = mark_completed(settings_store).await {
        tracing::warn!(
            error = %error,
            "long-tail reconvergence default migration completed but its marker was not saved"
        );
    }
}
