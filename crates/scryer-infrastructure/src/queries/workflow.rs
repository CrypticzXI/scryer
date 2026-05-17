use chrono::Utc;
use scryer_application::{AppError, AppResult};
use sqlx::SqlitePool;

pub(crate) async fn upsert_library_probe_signature_query(
    pool: &SqlitePool,
    title_id: &str,
    path: &str,
    probe_signature_scheme: Option<String>,
    probe_signature_value: Option<String>,
    last_probed_at: Option<String>,
    last_changed_at: Option<String>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO library_probe_signatures
         (title_id, path, probe_signature_scheme, probe_signature_value, last_probed_at, last_changed_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(title_id) DO UPDATE SET
            path = excluded.path,
            probe_signature_scheme = excluded.probe_signature_scheme,
            probe_signature_value = excluded.probe_signature_value,
            last_probed_at = excluded.last_probed_at,
            last_changed_at = excluded.last_changed_at,
            updated_at = excluded.updated_at",
    )
    .bind(title_id)
    .bind(path)
    .bind(probe_signature_scheme)
    .bind(probe_signature_value)
    .bind(last_probed_at)
    .bind(last_changed_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}
