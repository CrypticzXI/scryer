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

    async fn prune_scope(&self, scope_key: &str) -> AppResult<()> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM scope_indexer_coverage WHERE scope_key = {}",
            &[SqlArg::Text(scope_key.to_string())],
        )
        .await?;
        Ok(())
    }

    async fn prune_orphaned_coverage(&self) -> AppResult<()> {
        // Delete coverage whose id-based scope no longer exists (episode / series_movie
        // / collection / title) and coverage whose indexer no longer exists. Compare
        // the full `scope_key` to a reconstructed `'<prefix>:' || id` (portable, no
        // brittle offsets). `episode_set:` keys are content hashes with no single
        // entity, so they are left alone (harmless — UUID member ids never
        // re-associate). Every arm is guarded by `EXISTS (...)` so a transiently-empty
        // entity/indexer table can never wipe live coverage. All literals, no binds.
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "DELETE FROM scope_indexer_coverage
             WHERE (scope_key LIKE 'episode:%'
                    AND EXISTS (SELECT 1 FROM episodes)
                    AND scope_key NOT IN (SELECT 'episode:' || id FROM episodes))
                OR (scope_key LIKE 'series_movie:%'
                    AND EXISTS (SELECT 1 FROM series_movie_links)
                    AND scope_key NOT IN (SELECT 'series_movie:' || id FROM series_movie_links))
                OR (scope_key LIKE 'collection:%'
                    AND EXISTS (SELECT 1 FROM collections)
                    AND scope_key NOT IN (SELECT 'collection:' || id FROM collections))
                OR (scope_key LIKE 'title:%'
                    AND EXISTS (SELECT 1 FROM titles)
                    AND scope_key NOT IN (SELECT 'title:' || id FROM titles))
                OR (EXISTS (SELECT 1 FROM indexers)
                    AND indexer_id NOT IN (SELECT id FROM indexers))",
            &[],
        )
        .await?;
        Ok(())
    }
}
