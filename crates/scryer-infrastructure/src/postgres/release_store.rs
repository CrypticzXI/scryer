use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, ReleaseDownloadAttemptOutcome, ReleaseDownloadFailureSignature,
    TitleReleaseBlocklistEntry,
};
use scryer_domain::Id;
use sqlx::Row;

use crate::release_store::{ReleaseSql, ReleaseStore};

pub type PostgresReleaseStore = ReleaseStore<PostgresReleaseSql>;

#[derive(Clone)]
pub struct PostgresReleaseSql {
    pool: sqlx::PgPool,
}

impl PostgresReleaseStore {
    pub fn new(db: &super::PostgresServices) -> Self {
        Self::from_sql(PostgresReleaseSql::new(db.pool().clone()))
    }
}

impl PostgresReleaseSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReleaseSql for PostgresReleaseSql {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO release_download_attempts
             (id, title_id, source_hint, source_title, outcome, error_message, source_password,
              attempted_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW(), NOW())",
        )
        .bind(Id::new().0)
        .bind(title_id)
        .bind(source_hint)
        .bind(source_title)
        .bind(outcome.as_str())
        .bind(error_message)
        .bind(source_password)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        let rows = sqlx::query(
            "SELECT DISTINCT ON (LOWER(COALESCE(source_hint, '')), LOWER(COALESCE(source_title, '')))
                    source_hint, source_title
               FROM release_download_attempts
              WHERE outcome = 'failed'
              ORDER BY LOWER(COALESCE(source_hint, '')), LOWER(COALESCE(source_title, '')), attempted_at DESC
              LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(ReleaseDownloadFailureSignature {
                    source_hint: row.try_get("source_hint").unwrap_or(None),
                    source_title: row.try_get("source_title").unwrap_or(None),
                })
            })
            .collect()
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let rows = sqlx::query(
            "SELECT source_hint, source_title, error_message, attempted_at
               FROM release_download_attempts
              WHERE outcome = 'failed' AND title_id = $1
              ORDER BY attempted_at DESC
              LIMIT $2",
        )
        .bind(title_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                let attempted_at = row
                    .try_get::<chrono::DateTime<chrono::Utc>, _>("attempted_at")
                    .map_err(repo_err)?
                    .to_rfc3339();
                let source_title = row.try_get("source_title").unwrap_or(None);
                let source_hint = row.try_get("source_hint").unwrap_or(None);
                Ok(TitleReleaseBlocklistEntry {
                    id: format!(
                        "failed-attempt:{}:{}:{}",
                        attempted_at,
                        source_title.as_deref().unwrap_or_default(),
                        source_hint.as_deref().unwrap_or_default(),
                    ),
                    source_hint,
                    source_title,
                    error_message: row.try_get("error_message").unwrap_or(None),
                    attempted_at,
                    episode_ids: Vec::new(),
                })
            })
            .collect()
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        sqlx::query_scalar(
            "SELECT source_password
               FROM release_download_attempts
              WHERE source_password IS NOT NULL
                AND ($1::TEXT IS NULL OR title_id = $1)
                AND ($2::TEXT IS NULL OR source_hint = $2)
                AND ($3::TEXT IS NULL OR source_title = $3)
              ORDER BY attempted_at DESC
              LIMIT 1",
        )
        .bind(title_id)
        .bind(source_hint)
        .bind(source_title)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)
    }
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
