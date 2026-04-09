use async_trait::async_trait;
use scryer_application::{
    AppResult, ReleaseAttemptRepository, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureSignature, TitleReleaseBlocklistEntry,
};

use crate::SqliteServices;

#[derive(Clone)]
pub struct SqliteReleaseStore {
    pool: sqlx::SqlitePool,
}

impl SqliteReleaseStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl ReleaseAttemptRepository for SqliteReleaseStore {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        crate::queries::workflow::create_release_download_attempt_query(
            &self.pool,
            title_id,
            source_hint,
            source_title,
            outcome,
            error_message,
            source_password,
        )
        .await
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        Ok(
            crate::queries::workflow::list_failed_release_download_attempts_query(
                &self.pool,
                limit as i64,
            )
            .await?
            .into_iter()
            .map(|record| ReleaseDownloadFailureSignature {
                source_hint: record.source_hint,
                source_title: record.source_title,
            })
            .collect(),
        )
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        Ok(
            crate::queries::workflow::list_failed_release_download_attempts_for_title_query(
                &self.pool,
                title_id,
                limit as i64,
            )
            .await?
            .into_iter()
            .map(|record| TitleReleaseBlocklistEntry {
                source_hint: record.source_hint,
                source_title: record.source_title,
                error_message: record.error_message,
                attempted_at: record.attempted_at,
            })
            .collect(),
        )
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        crate::queries::workflow::get_latest_source_password_query(
            &self.pool,
            title_id,
            source_hint,
            source_title,
        )
        .await
    }
}
