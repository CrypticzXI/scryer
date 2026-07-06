use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppResult, ScopeIndexerCoverageRepository};

use crate::queries::sql_runtime::{SqlArg, SqlRuntime, StoreDatastore};

/// RFC 119 convergence ledger store (SQLite + Postgres via the shared runtime).
///
/// Upsert uses portable `INSERT … ON CONFLICT (pk) DO UPDATE SET … = excluded.…`
/// which both dialects support; a re-search under a new fingerprint overwrites
/// the prior row's fingerprint + timestamp.
#[derive(Clone)]
pub struct ScopeIndexerCoverageStore {
    datastore: StoreDatastore,
}

impl ScopeIndexerCoverageStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl ScopeIndexerCoverageRepository for ScopeIndexerCoverageStore {
    async fn record_coverage(
        &self,
        scope_key: &str,
        facet: &str,
        indexer_id: &str,
        fingerprint: &str,
    ) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "INSERT INTO scope_indexer_coverage (scope_key, facet, indexer_id, fingerprint, searched_at)
             VALUES ({}, {}, {}, {}, {})
             ON CONFLICT (scope_key, facet, indexer_id)
             DO UPDATE SET fingerprint = excluded.fingerprint, searched_at = excluded.searched_at",
            &[
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(facet.to_string()),
                SqlArg::Text(indexer_id.to_string()),
                SqlArg::Text(fingerprint.to_string()),
                SqlArg::Timestamp(Utc::now()),
            ],
        )
        .await?;
        Ok(())
    }

    async fn covered_indexers(
        &self,
        scope_key: &str,
        facet: &str,
        fingerprint: &str,
        stale_before: Option<DateTime<Utc>>,
    ) -> AppResult<Vec<String>> {
        let mut sql = String::from(
            "SELECT indexer_id FROM scope_indexer_coverage
             WHERE scope_key = {} AND facet = {} AND fingerprint = {}",
        );
        let mut args = vec![
            SqlArg::Text(scope_key.to_string()),
            SqlArg::Text(facet.to_string()),
            SqlArg::Text(fingerprint.to_string()),
        ];
        if let Some(stale_before) = stale_before {
            sql.push_str(" AND searched_at >= {}");
            args.push(SqlArg::Timestamp(stale_before));
        }

        SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args)
            .await?
            .into_iter()
            .map(|row| row.text("indexer_id"))
            .collect()
    }

    async fn prune_scope(&self, scope_key: &str, facet: &str) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM scope_indexer_coverage WHERE scope_key = {} AND facet = {}",
            &[
                SqlArg::Text(scope_key.to_string()),
                SqlArg::Text(facet.to_string()),
            ],
        )
        .await?;
        Ok(())
    }
}
