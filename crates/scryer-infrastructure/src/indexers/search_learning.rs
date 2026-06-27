use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use scryer_application::{
    AppError, AppResult, IndexerSearchLearningKey, IndexerSearchLearningRecord,
    IndexerSearchLearningRepository,
};

use crate::queries::sql_runtime::{SqlArg, SqlRow, SqlRuntime, StoreDatastore, repo_err};

fn sqlite_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[derive(Clone)]
pub struct IndexerSearchLearningStore {
    datastore: StoreDatastore,
}

impl IndexerSearchLearningStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn get_record(
        &self,
        key: &IndexerSearchLearningKey,
    ) -> AppResult<Option<IndexerSearchLearningRecord>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
             FROM indexer_search_learning
             WHERE indexer_id = {} AND title_id = {} AND facet = {} AND strategy_key = {}",
            &[
                SqlArg::Text(key.indexer_id.clone()),
                SqlArg::Text(key.title_id.clone()),
                SqlArg::Text(key.facet.clone()),
                SqlArg::Text(key.strategy_key.clone()),
            ],
        )
        .await?;

        row.as_ref().map(row_to_learning_record).transpose()
    }
}

#[async_trait]
impl IndexerSearchLearningRepository for IndexerSearchLearningStore {
    async fn list_for_title(
        &self,
        indexer_id: &str,
        title_id: &str,
        facet: &str,
    ) -> AppResult<Vec<IndexerSearchLearningRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT indexer_id, title_id, facet, strategy_key, attempts, empty_successes,
                    usable_successes, last_attempt_at, last_usable_at, suppressed, updated_at
             FROM indexer_search_learning
             WHERE indexer_id = {} AND title_id = {} AND facet = {}",
            &[
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(facet.to_string()),
            ],
        )
        .await?;

        rows.iter().map(row_to_learning_record).collect()
    }

    async fn record_outcome(
        &self,
        key: &IndexerSearchLearningKey,
        usable_hits: u32,
    ) -> AppResult<IndexerSearchLearningRecord> {
        match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                let key = key.clone();
                SqlRuntime::run_serialized_sqlite(
                    &self.datastore,
                    "record_indexer_search_learning_outcome",
                    move |pool| {
                        let key = key.clone();
                        async move {
                            sqlx::query(
                                "INSERT INTO indexer_search_learning (
                                    indexer_id, title_id, facet, strategy_key, attempts,
                                    empty_successes, usable_successes, last_attempt_at,
                                    last_usable_at, suppressed, updated_at
                                 )
                                 VALUES (
                                    ?, ?, ?, ?, 1,
                                    CASE WHEN ? = 0 THEN 1 ELSE 0 END,
                                    CASE WHEN ? > 0 THEN 1 ELSE 0 END,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    CASE WHEN ? > 0 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') ELSE NULL END,
                                    0,
                                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                 )
                                 ON CONFLICT(indexer_id, title_id, facet, strategy_key)
                                 DO UPDATE SET
                                    attempts = indexer_search_learning.attempts + 1,
                                    empty_successes = indexer_search_learning.empty_successes
                                        + CASE WHEN excluded.usable_successes = 0 THEN 1 ELSE 0 END,
                                    usable_successes = indexer_search_learning.usable_successes
                                        + CASE WHEN excluded.usable_successes > 0 THEN 1 ELSE 0 END,
                                    last_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                                    last_usable_at = CASE
                                        WHEN excluded.usable_successes > 0 THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                                        ELSE indexer_search_learning.last_usable_at
                                    END,
                                    suppressed = CASE
                                        WHEN excluded.usable_successes > 0 THEN 0
                                        ELSE indexer_search_learning.suppressed
                                    END,
                                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
                            )
                            .bind(&key.indexer_id)
                            .bind(&key.title_id)
                            .bind(&key.facet)
                            .bind(&key.strategy_key)
                            .bind(usable_hits as i64)
                            .bind(usable_hits as i64)
                            .bind(usable_hits as i64)
                            .execute(&pool)
                            .await
                            .map_err(repo_err)?;
                            Ok(())
                        }
                    },
                )
                .await?;
            }
            StoreDatastore::Postgres { pool } => {
                sqlx::query(
                    "INSERT INTO indexer_search_learning (
                        indexer_id, title_id, facet, strategy_key, attempts,
                        empty_successes, usable_successes, last_attempt_at,
                        last_usable_at, suppressed, updated_at
                     )
                     VALUES (
                        $1, $2, $3, $4, 1,
                        CASE WHEN $5 = 0 THEN 1 ELSE 0 END,
                        CASE WHEN $5 > 0 THEN 1 ELSE 0 END,
                        NOW(),
                        CASE WHEN $5 > 0 THEN NOW() ELSE NULL END,
                        FALSE,
                        NOW()
                     )
                     ON CONFLICT(indexer_id, title_id, facet, strategy_key)
                     DO UPDATE SET
                        attempts = indexer_search_learning.attempts + 1,
                        empty_successes = indexer_search_learning.empty_successes
                            + CASE WHEN EXCLUDED.usable_successes = 0 THEN 1 ELSE 0 END,
                        usable_successes = indexer_search_learning.usable_successes
                            + CASE WHEN EXCLUDED.usable_successes > 0 THEN 1 ELSE 0 END,
                        last_attempt_at = NOW(),
                        last_usable_at = CASE
                            WHEN EXCLUDED.usable_successes > 0 THEN NOW()
                            ELSE indexer_search_learning.last_usable_at
                        END,
                        suppressed = CASE
                            WHEN EXCLUDED.usable_successes > 0 THEN FALSE
                            ELSE indexer_search_learning.suppressed
                        END,
                        updated_at = NOW()",
                )
                .bind(&key.indexer_id)
                .bind(&key.title_id)
                .bind(&key.facet)
                .bind(&key.strategy_key)
                .bind(usable_hits as i64)
                .execute(pool)
                .await
                .map_err(repo_err)?;
            }
        }

        self.get_record(key).await?.ok_or_else(|| {
            AppError::Repository("indexer search learning outcome was not persisted".to_string())
        })
    }

    async fn set_suppressed(
        &self,
        key: &IndexerSearchLearningKey,
        suppressed: bool,
    ) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE indexer_search_learning
             SET suppressed = {}, updated_at = {}
             WHERE indexer_id = {} AND title_id = {} AND facet = {} AND strategy_key = {}",
            &[
                SqlArg::Bool(suppressed),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(key.indexer_id.clone()),
                SqlArg::Text(key.title_id.clone()),
                SqlArg::Text(key.facet.clone()),
                SqlArg::Text(key.strategy_key.clone()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn try_claim_suppressed_reprobe(
        &self,
        key: &IndexerSearchLearningKey,
        stale_before: DateTime<Utc>,
    ) -> AppResult<bool> {
        let now = Utc::now();
        let rows = match &self.datastore {
            StoreDatastore::Sqlite { .. } => {
                SqlRuntime::execute(
                    self.datastore.read_exec(),
                    "UPDATE indexer_search_learning
                     SET updated_at = {}
                     WHERE indexer_id = {}
                       AND title_id = {}
                       AND facet = {}
                       AND strategy_key = {}
                       AND suppressed = {}
                       AND (updated_at IS NULL OR updated_at < {})",
                    &[
                        SqlArg::Text(sqlite_timestamp(now)),
                        SqlArg::Text(key.indexer_id.clone()),
                        SqlArg::Text(key.title_id.clone()),
                        SqlArg::Text(key.facet.clone()),
                        SqlArg::Text(key.strategy_key.clone()),
                        SqlArg::Bool(true),
                        SqlArg::Text(sqlite_timestamp(stale_before)),
                    ],
                )
                .await?
            }
            StoreDatastore::Postgres { .. } => {
                SqlRuntime::execute(
                    self.datastore.read_exec(),
                    "UPDATE indexer_search_learning
                     SET updated_at = {}
                     WHERE indexer_id = {}
                       AND title_id = {}
                       AND facet = {}
                       AND strategy_key = {}
                       AND suppressed = {}
                       AND (updated_at IS NULL OR updated_at < {})",
                    &[
                        SqlArg::Timestamp(now),
                        SqlArg::Text(key.indexer_id.clone()),
                        SqlArg::Text(key.title_id.clone()),
                        SqlArg::Text(key.facet.clone()),
                        SqlArg::Text(key.strategy_key.clone()),
                        SqlArg::Bool(true),
                        SqlArg::Timestamp(stale_before),
                    ],
                )
                .await?
            }
        };

        Ok(rows > 0)
    }
}

fn row_to_learning_record(row: &SqlRow) -> AppResult<IndexerSearchLearningRecord> {
    Ok(IndexerSearchLearningRecord {
        key: IndexerSearchLearningKey {
            indexer_id: row.text("indexer_id")?,
            title_id: row.text("title_id")?,
            facet: row.text("facet")?,
            strategy_key: row.text("strategy_key")?,
        },
        attempts: i64_to_u32(row.i64("attempts")?, "attempts")?,
        empty_successes: i64_to_u32(row.i64("empty_successes")?, "empty_successes")?,
        usable_successes: i64_to_u32(row.i64("usable_successes")?, "usable_successes")?,
        last_attempt_at: row
            .opt_timestamp("last_attempt_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
        last_usable_at: row
            .opt_timestamp("last_usable_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
        suppressed: row.bool("suppressed")?,
        updated_at: row
            .opt_timestamp("updated_at")?
            .map(|timestamp| timestamp.to_rfc3339()),
    })
}

fn i64_to_u32(value: i64, column: &str) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| {
        AppError::Repository(format!(
            "indexer_search_learning.{column} is outside u32 range"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use sqlx::sqlite::SqlitePoolOptions;

    async fn sqlite_store() -> (IndexerSearchLearningStore, sqlx::SqlitePool) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");

        sqlx::query(
            "CREATE TABLE indexer_search_learning (
                indexer_id TEXT NOT NULL,
                title_id TEXT NOT NULL,
                facet TEXT NOT NULL,
                strategy_key TEXT NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0,
                empty_successes INTEGER NOT NULL DEFAULT 0,
                usable_successes INTEGER NOT NULL DEFAULT 0,
                last_attempt_at TEXT,
                last_usable_at TEXT,
                suppressed INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                PRIMARY KEY (indexer_id, title_id, facet, strategy_key)
            )",
        )
        .execute(&pool)
        .await
        .expect("learning table should be created");

        let store = IndexerSearchLearningStore::new(StoreDatastore::Sqlite {
            pool: pool.clone(),
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        });

        (store, pool)
    }

    #[tokio::test]
    async fn sqlite_store_records_lists_and_updates_learning_records() {
        let (store, pool) = sqlite_store().await;
        let key = IndexerSearchLearningKey {
            indexer_id: "idx-1".into(),
            title_id: "title-1".into(),
            facet: "anime".into(),
            strategy_key: "ids_abs".into(),
        };

        let empty_record = store
            .record_outcome(&key, 0)
            .await
            .expect("empty outcome should persist");
        assert_eq!(empty_record.attempts, 1);
        assert_eq!(empty_record.empty_successes, 1);
        assert_eq!(empty_record.usable_successes, 0);
        assert!(!empty_record.suppressed);

        store
            .set_suppressed(&key, true)
            .await
            .expect("suppression flag should update");
        let suppressed_records = store
            .list_for_title("idx-1", "title-1", "anime")
            .await
            .expect("records should list");
        assert_eq!(suppressed_records.len(), 1);
        assert!(suppressed_records[0].suppressed);
        assert!(
            !store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("recent suppression should not be claimed")
        );

        sqlx::query(
            "UPDATE indexer_search_learning
             SET updated_at = ?
             WHERE indexer_id = ? AND title_id = ? AND facet = ? AND strategy_key = ?",
        )
        .bind(sqlite_timestamp(Utc::now() - chrono::Duration::days(8)))
        .bind(&key.indexer_id)
        .bind(&key.title_id)
        .bind(&key.facet)
        .bind(&key.strategy_key)
        .execute(&pool)
        .await
        .expect("learning row should be aged");
        assert!(
            store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("stale suppression should be claimed")
        );
        assert!(
            !store
                .try_claim_suppressed_reprobe(&key, Utc::now() - chrono::Duration::days(7))
                .await
                .expect("claimed suppression should not be claimed twice")
        );

        let usable_record = store
            .record_outcome(&key, 2)
            .await
            .expect("usable outcome should persist and reload");
        assert_eq!(usable_record.attempts, 2);
        assert_eq!(usable_record.empty_successes, 1);
        assert_eq!(usable_record.usable_successes, 1);
        assert!(usable_record.last_attempt_at.is_some());
        assert!(usable_record.last_usable_at.is_some());
        assert!(!usable_record.suppressed);
    }
}
