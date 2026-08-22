use std::sync::Arc;

use scryer_application::{AppUseCase, SETTINGS_SCOPE_SYSTEM};
use scryer_infrastructure_configuration::settings::settings_store::SettingsStore;

pub(crate) const PENDING_ZERO_SEEDERS_EXPIRED_0018_STATE_KEY: &str =
    "acquisition.pending_zero_seeders_expired_0018_state";
const STATE_NONE: &str = "none";
const STATE_PENDING: &str = "pending";
const STATE_COMPLETED: &str = "completed";

async fn read_state(settings_store: Arc<SettingsStore>) -> String {
    settings_store
        .get_setting_with_defaults(
            SETTINGS_SCOPE_SYSTEM,
            PENDING_ZERO_SEEDERS_EXPIRED_0018_STATE_KEY,
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
            PENDING_ZERO_SEEDERS_EXPIRED_0018_STATE_KEY,
            None,
            value_json,
            "system",
            None,
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// Expire delayed releases that recorded an empty swarm when they were parked.
///
/// This is deliberately a run-once startup job. `None` means the indexer did
/// not report seeders, so those rows remain eligible.
pub(crate) async fn expire_zero_seeder_pending_releases(
    app_use_case: &AppUseCase,
    settings_store: Arc<SettingsStore>,
) {
    if read_state(settings_store.clone()).await == STATE_COMPLETED {
        return;
    }

    if let Err(error) = set_state(settings_store.clone(), STATE_PENDING).await {
        tracing::warn!(
            error = %error,
            "failed to mark zero-seeder pending-release migration pending"
        );
        return;
    }

    match app_use_case.expire_zero_seeder_pending_releases().await {
        Ok(expired) => {
            if let Err(error) = set_state(settings_store, STATE_COMPLETED).await {
                tracing::warn!(
                    error = %error,
                    expired,
                    "zero-seeder pending-release migration completed but failed to mark migration completed"
                );
                return;
            }
            tracing::info!(
                expired,
                "expired delayed pending releases with zero seeders"
            );
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "zero-seeder pending-release migration failed; it will retry on next startup"
            );
        }
    }
}
