use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use chrono::{DateTime, Utc};
use scryer_application::{IndexerQueryStats, IndexerStatsTracker};
use sqlx::SqlitePool;

const QUOTA_FLUSH_INTERVAL: Duration = Duration::from_millis(500);

struct IndexerEntry {
    indexer_name: String,
    queries: Vec<(DateTime<Utc>, bool)>,
    api_current: Option<u32>,
    api_max: Option<u32>,
    grab_current: Option<u32>,
    grab_max: Option<u32>,
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
}

/// Thread-safe indexer stats tracker with in-memory 24-hour rolling window
/// and optional SQLite persistence for API quota enforcement.
#[derive(Clone)]
pub struct InMemoryIndexerStatsTracker {
    entries: Arc<Mutex<HashMap<String, IndexerEntry>>>,
    pool: Option<SqlitePool>,
    pending_quota_updates: Arc<Mutex<HashMap<String, PendingQuotaUpdate>>>,
    quota_flush_scheduled: Arc<AtomicBool>,
}

impl Default for InMemoryIndexerStatsTracker {
    fn default() -> Self {
        Self::new(None)
    }
}

impl InMemoryIndexerStatsTracker {
    pub fn new(pool: Option<SqlitePool>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            pool,
            pending_quota_updates: Arc::new(Mutex::new(HashMap::new())),
            quota_flush_scheduled: Arc::new(AtomicBool::new(false)),
        }
    }

    fn prune_old(entry: &mut IndexerEntry) {
        let cutoff = Utc::now() - chrono::Duration::hours(24);
        entry.queries.retain(|(ts, _)| *ts > cutoff);
    }

    fn schedule_quota_flush(&self) {
        if self.pool.is_none() || self.quota_flush_scheduled.swap(true, Ordering::AcqRel) {
            return;
        }

        let tracker = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(QUOTA_FLUSH_INTERVAL).await;
            tracker.flush_pending_quota_updates().await;
        });
    }

    pub async fn flush_pending_quota_updates(&self) {
        let Some(pool) = &self.pool else {
            self.quota_flush_scheduled.store(false, Ordering::Release);
            return;
        };
        let pool = pool.clone();
        let updates = {
            let mut pending = self.pending_quota_updates.lock().unwrap();
            if pending.is_empty() {
                self.quota_flush_scheduled.store(false, Ordering::Release);
                return;
            }
            pending.drain().collect::<Vec<_>>()
        };

        for (indexer_id, update) in updates {
            if update.query_delta == 0 {
                continue;
            }
            if let Err(error) = crate::queries::indexer::upsert_indexer_quota(
                &pool,
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
                    "failed to flush coalesced indexer quota update"
                );
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
            .or_insert_with(|| IndexerEntry {
                indexer_name: indexer_name.to_string(),
                queries: Vec::new(),
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            });
        entry.indexer_name = indexer_name.to_string();
        entry.queries.push((Utc::now(), success));
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

        if self.pool.is_some() {
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
                let successful = entry.queries.iter().filter(|(_, s)| *s).count() as u32;
                let total = entry.queries.len() as u32;
                let last_query_at = entry.queries.last().map(|(ts, _)| ts.to_rfc3339());
                IndexerQueryStats {
                    indexer_id: id.clone(),
                    indexer_name: entry.indexer_name.clone(),
                    queries_last_24h: total,
                    successful_last_24h: successful,
                    failed_last_24h: total - successful,
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
    use sqlx::{Row, sqlite::SqlitePoolOptions};

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

        let tracker = InMemoryIndexerStatsTracker::new(Some(pool.clone()));
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
}
