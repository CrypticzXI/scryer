use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppResult, ReleaseAttemptRepository, ReleaseDownloadAttemptOutcome,
    ReleaseDownloadFailureSignature, TitleReleaseBlocklistEntry,
};
use scryer_domain::Id;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};
use crate::sqlite_services::SqliteServices;

const FAILED_SIGNATURE_LIMIT_MAX: i64 = 20_000;
const TITLE_FAILED_SIGNATURE_LIMIT_MAX: i64 = 1_000;

const INSERT_RELEASE_ATTEMPT_SQL: &str = "INSERT INTO release_download_attempts (
    id, title_id, source_hint, source_title, outcome, error_message, source_password,
    attempted_at, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const LIST_FAILED_SIGNATURES_SQL: &str = "SELECT source_hint, source_title
    FROM (
        SELECT source_hint,
               source_title,
               attempted_at AS last_attempted_at,
               ROW_NUMBER() OVER (
                   PARTITION BY LOWER(TRIM(source_title))
                   ORDER BY attempted_at DESC
               ) AS row_number
          FROM release_download_attempts
         WHERE outcome = 'failed'
           AND COALESCE(TRIM(source_title), '') <> ''
    )
    WHERE row_number = 1
    ORDER BY last_attempted_at DESC
    LIMIT {}";

const LIST_FAILED_SIGNATURES_FOR_TITLE_SQL: &str =
    "SELECT source_hint, source_title, error_message, attempted_at
    FROM (
        SELECT source_hint,
               source_title,
               error_message,
               attempted_at,
               ROW_NUMBER() OVER (
                   PARTITION BY LOWER(TRIM(source_title))
                   ORDER BY attempted_at DESC
               ) AS row_number
          FROM release_download_attempts
         WHERE outcome = 'failed'
           AND title_id = {}
           AND COALESCE(TRIM(source_title), '') <> ''
    )
    WHERE row_number = 1
    ORDER BY attempted_at DESC
    LIMIT {}";

const GET_LATEST_SOURCE_PASSWORD_SQL: &str = "SELECT source_password
    FROM release_download_attempts
    WHERE source_password IS NOT NULL
      AND (CAST({} AS TEXT) IS NULL OR title_id = {})
      AND (CAST({} AS TEXT) IS NULL OR source_hint = {})
      AND (CAST({} AS TEXT) IS NULL OR source_title = {})
    ORDER BY attempted_at DESC
    LIMIT 1";

#[derive(Clone)]
pub struct ReleaseStore {
    datastore: StoreDatastore,
}

impl ReleaseStore {
    pub(crate) fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub fn from_sqlite_services(db: &SqliteServices) -> Self {
        Self::new(StoreDatastore::Sqlite {
            pool: db.pool().clone(),
            writer_gate: db.writer_gate(),
        })
    }

    pub fn from_postgres_services(db: &crate::postgres::PostgresServices) -> Self {
        Self::new(StoreDatastore::Postgres {
            pool: db.pool().clone(),
        })
    }
}

#[async_trait]
impl ReleaseAttemptRepository for ReleaseStore {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now();
        let args = vec![
            SqlArg::Text(Id::new().0),
            SqlArg::OptText(title_id),
            SqlArg::OptText(source_hint),
            SqlArg::OptText(source_title),
            SqlArg::Text(outcome.as_str().to_string()),
            SqlArg::OptText(error_message),
            SqlArg::OptText(source_password),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ];

        SqlRuntime::run_in_transaction(&self.datastore, "record_release_attempt", move |tx| {
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INSERT_RELEASE_ATTEMPT_SQL, &args).await?;
                Ok(())
            })
        })
        .await
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            LIST_FAILED_SIGNATURES_SQL,
            &[SqlArg::I64(clamp_limit(limit, FAILED_SIGNATURE_LIMIT_MAX))],
        )
        .await?;

        rows.into_iter().map(decode_failed_signature).collect()
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            LIST_FAILED_SIGNATURES_FOR_TITLE_SQL,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::I64(clamp_limit(limit, TITLE_FAILED_SIGNATURE_LIMIT_MAX)),
            ],
        )
        .await?;

        rows.into_iter()
            .map(decode_title_release_blocklist)
            .collect()
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        let title_id = title_id.map(str::to_string);
        let source_hint = source_hint.map(str::to_string);
        let source_title = source_title.map(str::to_string);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            GET_LATEST_SOURCE_PASSWORD_SQL,
            &[
                SqlArg::OptText(title_id.clone()),
                SqlArg::OptText(title_id),
                SqlArg::OptText(source_hint.clone()),
                SqlArg::OptText(source_hint),
                SqlArg::OptText(source_title.clone()),
                SqlArg::OptText(source_title),
            ],
        )
        .await?;

        match row {
            Some(row) => row.opt_text("source_password"),
            None => Ok(None),
        }
    }
}

fn clamp_limit(limit: usize, max: i64) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX).clamp(1, max)
}

fn decode_failed_signature(row: SqlRow) -> AppResult<ReleaseDownloadFailureSignature> {
    Ok(ReleaseDownloadFailureSignature {
        source_hint: row.opt_text("source_hint")?,
        source_title: row.opt_text("source_title")?,
    })
}

fn decode_title_release_blocklist(row: SqlRow) -> AppResult<TitleReleaseBlocklistEntry> {
    let source_hint = row.opt_text("source_hint")?;
    let source_title = row.opt_text("source_title")?;
    let attempted_at = row.timestamp("attempted_at")?.to_rfc3339();

    Ok(TitleReleaseBlocklistEntry {
        id: format!(
            "failed-attempt:{}:{}:{}",
            attempted_at,
            source_title.as_deref().unwrap_or_default(),
            source_hint.as_deref().unwrap_or_default(),
        ),
        source_hint,
        source_title,
        error_message: row.opt_text("error_message")?,
        attempted_at,
        episode_ids: Vec::new(),
    })
}
