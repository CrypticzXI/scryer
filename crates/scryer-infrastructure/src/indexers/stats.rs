use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, Utc};
use scryer_application::{IndexerQueryStats, IndexerStatsTracker};

use crate::queries::sql_runtime::StoreDatastore;

const QUOTA_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

struct IndexerEntry {
    indexer_name: String,
    queries: Vec<(DateTime<Utc>, bool)>,
    /// Timestamps of releases Scryer grabbed through this indexer. Shares the
    /// rolling 24-hour window and in-memory-only lifetime of `queries`; unlike
    /// `grab_current` below, this is Scryer's own count rather than a
    /// provider-reported quota reading, so it is never persisted.
    grabs: Vec<DateTime<Utc>>,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
}

impl IndexerEntry {
    fn new(indexer_name: &str) -> Self {
        Self {
            indexer_name: indexer_name.to_string(),
            queries: Vec::new(),
            grabs: Vec::new(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PendingQuotaUpdate {
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
    query_delta: u32,
}

impl PendingQuotaUpdate {
    fn merge(
        &mut self,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    ) {
        if api_current.is_some() {
            self.api_current = api_current;
        }
        if api_max.is_some() {
            self.api_max = api_max;
        }
        if grab_current.is_some() {
            self.grab_current = grab_current;
        }
        if grab_max.is_some() {
            self.grab_max = grab_max;
        }
        self.query_delta = self.query_delta.saturating_add(1);
    }

    fn merge_requeued(&mut self, older: Self) {
        self.api_current = self.api_current.or(older.api_current);
        self.api_max = self.api_max.or(older.api_max);
        self.grab_current = self.grab_current.or(older.grab_current);
        self.grab_max = self.grab_max.or(older.grab_max);
        self.query_delta = self.query_delta.saturating_add(older.query_delta);
    }
}

/// Thread-safe indexer stats tracker with in-memory 24-hour rolling window
/// and optional SQLite persistence for API quota enforcement.
#[derive(Clone)]
pub struct InMemoryIndexerStatsTracker {
    entries: Arc<Mutex<HashMap<String, IndexerEntry>>>,
    datastore: Option<StoreDatastore>,
    pending_quota_updates: Arc<Mutex<HashMap<String, PendingQuotaUpdate>>>,
    quota_flush_scheduled: Arc<AtomicBool>,
}

impl Default for InMemoryIndexerStatsTracker {
    fn default() -> Self {
        Self::new(None)
    }
}

impl InMemoryIndexerStatsTracker {
    pub fn new(datastore: Option<StoreDatastore>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            datastore,
            pending_quota_updates: Arc::new(Mutex::new(HashMap::new())),
            quota_flush_scheduled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn prune_old(entry: &mut IndexerEntry) {
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entry.queries.retain(|(ts, _)| *ts > cutoff);
        entry.grabs.retain(|ts| *ts > cutoff);
    }

    fn schedule_quota_flush(&self) {
        if self.datastore.is_none() || self.quota_flush_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }

        let tracker = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUOTA_FLUSH_INTERVAL).await;
            tracker.flush_pending_quota_updates().await;
        });
    }

    pub async fn flush_pending_quota_updates(&self) {
        let Some(datastore) = &self.datastore else {
            self.quota_flush_scheduled.store(false, Ordering::Release);
            return;
        };
        let datastore = datastore.clone();
        let updates = {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            if pending.is_empty() {
                self.quota_flush_scheduled.store(false, Ordering::Release);
                return;
            }
            pending.drain().collect::<Vec<_>>()
        };

        let mut failed_updates = Vec::new();
        for (indexer_id, update) in updates {
            if update.query_delta == 0 {
                continue;
            }
            if let Err(error) = crate::queries::indexer::upsert_indexer_quota(
                &datastore,
                &indexer_id,
                update.api_current,
                update.api_max,
                update.grab_current,
                update.grab_max,
                update.query_delta,
            )
            .await
            {
                tracing::warn!(
                    indexer_id,
                    error = %error,
                    "failed to flush coalesced indexer quota update; requeueing"
                );
                failed_updates.push((indexer_id, update));
            }
        }

        if !failed_updates.is_empty() {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            for (indexer_id, update) in failed_updates {
                pending
                    .entry(indexer_id)
                    .or_default()
                    .merge_requeued(update);
            }
        }

        self.quota_flush_scheduled.store(false, Ordering::Release);
        if !self.pending_quota_updates.lock().unwrap().is_empty() {
            self.schedule_quota_flush();
        }
    }
}

impl IndexerStatsTracker for InMemoryIndexerStatsTracker {
    fn record_query(&self, indexer_id: &str, indexer_name: &str, success: bool) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(indexer_id.to_string())
            .or_insert_with(|| IndexerEntry::new(indexer_name));
        entry.indexer_name = indexer_name.to_string();
        entry.queries.push((Utc::now(), success));
        Self::prune_old(entry);
    }

    fn record_grab(&self, indexer_id: &str, indexer_name: &str) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries
            .entry(indexer_id.to_string())
            .or_insert_with(|| IndexerEntry::new(indexer_name));
        entry.indexer_name = indexer_name.to_string();
        entry.grabs.push(Utc::now());
        Self::prune_old(entry);
    }

    fn record_api_limits(
        &self,
        indexer_id: &str,
        api_current: Option<u32>,
        api_max: Option<u32>,
        grab_current: Option<u32>,
        grab_max: Option<u32>,
    ) {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.get_mut(indexer_id) {
            if api_current.is_some() {
                entry.api_current = api_current;
            }
            if api_max.is_some() {
                entry.api_max = api_max;
            }
            if grab_current.is_some() {
                entry.grab_current = grab_current;
            }
            if grab_max.is_some() {
                entry.grab_max = grab_max;
            }
        }
        drop(entries);

        if self.datastore.is_some() {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            pending.entry(indexer_id.to_string()).or_default().merge(
                api_current,
                api_max,
                grab_current,
                grab_max,
            );
            drop(pending);
            self.schedule_quota_flush();
        }
    }

    fn all_stats(&self) -> Vec<IndexerQueryStats> {
        let mut entries = self.entries.lock().unwrap();
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entries
            .iter_mut()
            .map(|(id, entry)| {
                entry.queries.retain(|(ts, _)| *ts > cutoff);
                entry.grabs.retain(|ts| *ts > cutoff);
                let successful = entry.queries.iter().filter(|(_, s)| *s).count() as u32;
                let total = entry.queries.len() as u32;
                let last_query_at = entry.queries.last().map(|(ts, _)| ts.to_rfc3339());
                IndexerQueryStats {
                    indexer_id: id.clone(),
                    indexer_name: entry.indexer_name.clone(),
                    queries_last_24h: total,
                    successful_last_24h: successful,
                    failed_last_24h: total - successful,
                    grabs_last_24h: entry.grabs.len() as u32,
                    last_query_at,
                    api_current: entry.api_current,
                    api_max: entry.api_max,
                    grab_current: entry.grab_current,
                    grab_max: entry.grab_max,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Row, SqlitePool, sqlite::SqlitePoolOptions};
    use tokio::time::{Duration, timeout};

    fn tracker_with_gate(
        pool: &SqlitePool,
        writer_gate: Arc<tokio::sync::Mutex<()>>,
    ) -> InMemoryIndexerStatsTracker {
        InMemoryIndexerStatsTracker::new(Some(StoreDatastore::sqlite(pool.clone(), writer_gate)))
    }

    /// Backdate every recorded grab for one indexer, standing in for time
    /// passing without making the test sleep.
    fn backdate_grabs(
        tracker: &InMemoryIndexerStatsTracker,
        indexer_id: &str,
        age: chrono::Duration,
    ) {
        let mut entries = tracker.entries.lock().unwrap();
        let entry = entries.get_mut(indexer_id).expect("indexer entry");
        for grab in entry.grabs.iter_mut() {
            *grab -= age;
        }
    }

    fn grabs_for(tracker: &InMemoryIndexerStatsTracker, indexer_id: &str) -> u32 {
        tracker
            .all_stats()
            .into_iter()
            .find(|stats| stats.indexer_id == indexer_id)
            .map(|stats| stats.grabs_last_24h)
            .unwrap_or(0)
    }

    #[test]
    fn record_grab_increments_the_rolling_24h_count() {
        let tracker = InMemoryIndexerStatsTracker::default();
        assert_eq!(grabs_for(&tracker, "idx-grab"), 0);

        tracker.record_grab("idx-grab", "Grabby Indexer");
        tracker.record_grab("idx-grab", "Grabby Indexer");

        assert_eq!(grabs_for(&tracker, "idx-grab"), 2);
        let stats = tracker.all_stats();
        let entry = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-grab")
            .expect("grab-only indexer should appear in stats");
        assert_eq!(entry.indexer_name, "Grabby Indexer");
        // Grabs must not be confused with queries or with provider quota counters.
        assert_eq!(entry.queries_last_24h, 0);
        assert_eq!(entry.grab_current, None);
        assert_eq!(entry.grab_max, None);
    }

    #[test]
    fn grabs_older_than_the_window_are_pruned() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-prune", "Pruned Indexer");
        assert_eq!(grabs_for(&tracker, "idx-prune"), 1);

        backdate_grabs(&tracker, "idx-prune", chrono::Duration::hours(25));
        assert_eq!(
            grabs_for(&tracker, "idx-prune"),
            0,
            "a grab older than 24 hours must leave the window"
        );

        // A fresh grab after pruning still counts, and recording prunes too.
        tracker.record_grab("idx-prune", "Pruned Indexer");
        assert_eq!(grabs_for(&tracker, "idx-prune"), 1);
    }

    #[test]
    fn recording_a_grab_prunes_expired_grabs_for_that_indexer() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-prune-on-write", "Indexer");
        backdate_grabs(&tracker, "idx-prune-on-write", chrono::Duration::hours(25));

        tracker.record_grab("idx-prune-on-write", "Indexer");

        let entries = tracker.entries.lock().unwrap();
        assert_eq!(
            entries
                .get("idx-prune-on-write")
                .expect("indexer entry")
                .grabs
                .len(),
            1,
            "record_grab should drop expired grabs like record_query does"
        );
    }

    #[test]
    fn grabs_do_not_bleed_between_indexers() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_grab("idx-a", "Indexer A");
        tracker.record_grab("idx-a", "Indexer A");
        tracker.record_grab("idx-b", "Indexer B");
        tracker.record_query("idx-c", "Indexer C", true);

        assert_eq!(grabs_for(&tracker, "idx-a"), 2);
        assert_eq!(grabs_for(&tracker, "idx-b"), 1);
        assert_eq!(
            grabs_for(&tracker, "idx-c"),
            0,
            "an indexer that was only queried has no grabs"
        );
    }

    #[test]
    fn grabs_and_queries_share_an_entry_without_overwriting_each_other() {
        let tracker = InMemoryIndexerStatsTracker::default();
        tracker.record_query("idx-mixed", "Mixed Indexer", true);
        tracker.record_grab("idx-mixed", "Mixed Indexer");
        tracker.record_query("idx-mixed", "Mixed Indexer", false);

        let stats = tracker.all_stats();
        let entry = stats
            .iter()
            .find(|stats| stats.indexer_id == "idx-mixed")
            .expect("mixed indexer stats");
        assert_eq!(entry.queries_last_24h, 2);
        assert_eq!(entry.successful_last_24h, 1);
        assert_eq!(entry.failed_last_24h, 1);
        assert_eq!(entry.grabs_last_24h, 1);
    }

    #[test]
    fn requeued_updates_preserve_newer_limits_and_sum_deltas() {
        let mut older = PendingQuotaUpdate::default();
        older.merge(Some(1), Some(100), Some(2), Some(20));

        let mut newer = PendingQuotaUpdate::default();
        newer.merge(Some(5), None, None, Some(50));
        newer.merge_requeued(older);

        assert_eq!(newer.api_current, Some(5));
        assert_eq!(newer.api_max, Some(100));
        assert_eq!(newer.grab_current, Some(2));
        assert_eq!(newer.grab_max, Some(50));
        assert_eq!(newer.query_delta, 2);
    }

    async fn create_quota_table(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE indexer_api_quotas (
                indexer_id TEXT PRIMARY KEY NOT NULL,
                api_current INTEGER,
                api_max INTEGER,
                grab_current INTEGER,
                grab_max INTEGER,
                queries_today INTEGER NOT NULL DEFAULT 0,
                last_query_at TEXT,
                last_reset_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(pool)
        .await
        .expect("quota table should be created");
    }

    #[tokio::test]
    async fn quota_flush_coalesces_updates_and_preserves_query_delta() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_query("idx-1", "Indexer One", true);
        tracker.record_api_limits("idx-1", Some(1), Some(100), None, None);
        tracker.record_api_limits("idx-1", Some(2), None, Some(3), Some(50));
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT api_current, api_max, grab_current, grab_max, queries_today
             FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-1")
        .fetch_one(&pool)
        .await
        .expect("quota row should exist");

        assert_eq!(row.get::<i64, _>("api_current"), 2);
        assert_eq!(row.get::<i64, _>("api_max"), 100);
        assert_eq!(row.get::<i64, _>("grab_current"), 3);
        assert_eq!(row.get::<i64, _>("grab_max"), 50);
        assert_eq!(row.get::<i64, _>("queries_today"), 2);

        tracker.record_api_limits("idx-1", None, Some(101), None, None);
        tracker.flush_pending_quota_updates().await;

        let row = sqlx::query(
            "SELECT api_current, api_max, queries_today
             FROM indexer_api_quotas WHERE indexer_id = ?",
        )
        .bind("idx-1")
        .fetch_one(&pool)
        .await
        .expect("quota row should still exist");

        assert_eq!(row.get::<i64, _>("api_current"), 2);
        assert_eq!(row.get::<i64, _>("api_max"), 101);
        assert_eq!(row.get::<i64, _>("queries_today"), 3);
    }

    #[tokio::test]
    async fn quota_flush_waits_for_the_shared_sqlite_writer_gate() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        create_quota_table(&pool).await;

        let writer_gate = Arc::new(tokio::sync::Mutex::new(()));
        let tracker = tracker_with_gate(&pool, Arc::clone(&writer_gate));
        let guard = writer_gate.lock().await;
        tracker.record_api_limits("idx-gated", Some(7), Some(100), None, None);

        let flushing_tracker = tracker.clone();
        let mut flush_task = tokio::spawn(async move {
            flushing_tracker.flush_pending_quota_updates().await;
        });
        assert!(
            timeout(Duration::from_millis(100), &mut flush_task)
                .await
                .is_err(),
            "quota persistence must wait for the shared writer gate"
        );

        let count_while_held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM indexer_api_quotas")
            .fetch_one(&pool)
            .await
            .expect("quota table should remain readable");
        assert_eq!(count_while_held, 0);

        drop(guard);
        timeout(Duration::from_secs(5), flush_task)
            .await
            .expect("quota flush should finish after releasing the writer gate")
            .expect("quota flush task should not panic");

        let queries_today: i64 =
            sqlx::query_scalar("SELECT queries_today FROM indexer_api_quotas WHERE indexer_id = ?")
                .bind("idx-gated")
                .fetch_one(&pool)
                .await
                .expect("gated quota update should persist");
        assert_eq!(queries_today, 1);
    }

    #[tokio::test]
    async fn quota_flush_requeues_failed_updates() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        let tracker = tracker_with_gate(&pool, Arc::new(tokio::sync::Mutex::new(())));
        tracker.record_api_limits("idx-retry", Some(9), Some(200), None, None);

        tracker.flush_pending_quota_updates().await;
        assert_eq!(tracker.pending_quota_updates.lock().unwrap().len(), 1);

        create_quota_table(&pool).await;
        let row = timeout(Duration::from_secs(5), async {
            loop {
                if let Some(row) = sqlx::query(
                    "SELECT api_current, api_max, queries_today
                     FROM indexer_api_quotas WHERE indexer_id = ?",
                )
                .bind("idx-retry")
                .fetch_optional(&pool)
                .await
                .expect("query requeued quota update")
                {
                    break row;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("requeued quota update should flush without new indexer activity");
        assert_eq!(row.get::<i64, _>("api_current"), 9);
        assert_eq!(row.get::<i64, _>("api_max"), 200);
        assert_eq!(row.get::<i64, _>("queries_today"), 1);
        assert!(tracker.pending_quota_updates.lock().unwrap().is_empty());
    }
}
