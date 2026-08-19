use super::*;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult, DashboardActivityStats, DomainEventRepository};
use scryer_domain::{
    DomainEvent, DomainEventFilter, DomainEventType, NewDomainEvent, TitleHistoryEventType,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct DomainEventStore {
    datastore: StoreDatastore,
}

impl DomainEventStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DomainEventRepository for DomainEventStore {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        append_domain_events(&self.datastore, vec![event])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Repository("failed to append domain event".into()))
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        append_domain_events(&self.datastore, events).await
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let (sql, args) = build_domain_event_list_sql(filter);
        fetch_domain_events(self.datastore.read_exec(), &sql, &args).await
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let (where_sql, args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!("SELECT COUNT(*) AS count FROM domain_events{where_sql}"),
            &args,
        )
        .await?
        .ok_or_else(|| AppError::Repository("missing domain event count".into()))?;
        row.i64("count")
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { 50 } else { limit.min(500) };
        let (where_sql, mut args) =
            build_title_history_filter_sql(&self.datastore, event_types, title_ids, download_id);
        args.push(SqlArg::I64(page_size as i64));
        args.push(SqlArg::I64(offset as i64));
        fetch_domain_events(
            self.datastore.read_exec(),
            &format!(
                "SELECT {DOMAIN_EVENT_COLUMNS} FROM domain_events{where_sql} ORDER BY sequence DESC LIMIT {{}} OFFSET {{}}"
            ),
            &args,
        )
        .await
    }

    async fn count_dashboard_activity_events(
        &self,
        library_ids: &[String],
        previous_start: DateTime<Utc>,
        current_start: DateTime<Utc>,
        current_end: DateTime<Utc>,
    ) -> AppResult<DashboardActivityStats> {
        if library_ids.is_empty() {
            return Ok(DashboardActivityStats::default());
        }

        let (sql, args) = build_dashboard_activity_stats_sql(
            &self.datastore,
            library_ids,
            previous_start,
            current_start,
            current_end,
        );
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;

        let mut stats = DashboardActivityStats::default();
        for row in rows {
            let window_key = row.text("window_key")?;
            let event_type = row.text("event_type")?;
            let count = row.i64("event_count")?;
            let window = if window_key == "current" {
                &mut stats.current
            } else {
                &mut stats.previous
            };
            // `import_rejected` reaches this point only with a failed status:
            // the aggregate already filtered skipped rejections out.
            match DomainEventType::parse(&event_type) {
                Some(DomainEventType::ReleaseGrabbed) => window.grabbed += count,
                Some(DomainEventType::MediaFileUpgraded) => window.upgraded += count,
                Some(DomainEventType::ImportCompleted) => window.imported += count,
                Some(DomainEventType::ImportRejected) => window.import_failed += count,
                Some(DomainEventType::DownloadFailed) => window.download_failed += count,
                _ => {}
            }
        }
        Ok(stats)
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let filter = DomainEventFilter {
            after_sequence: Some(after_sequence),
            limit,
            ..DomainEventFilter::default()
        };
        self.list(&filter).await
    }

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if title_ids.is_empty() {
            return Ok(0);
        }
        let mut args = Vec::with_capacity(title_ids.len() + 1);
        args.extend(title_ids.iter().cloned().map(SqlArg::Text));
        args.push(SqlArg::Text(DomainEventType::TitleDeleted.as_str().into()));
        let rows = execute_write(
            &self.datastore,
            "delete_domain_events_for_title_ids",
            format!(
                "DELETE FROM domain_events WHERE title_id IN ({}) AND event_type <> {{}}",
                placeholders(title_ids.len())
            ),
            args,
        )
        .await?;
        Ok(rows as u32)
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT sequence FROM event_subscriber_offsets WHERE subscriber_name = {}",
            &[SqlArg::Text(subscriber.to_string())],
        )
        .await?;
        Ok(row.map(|row| row.i64("sequence")).transpose()?.unwrap_or(0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let subscriber = subscriber.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_event_subscriber_offset", move |tx| {
            let subscriber = subscriber.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO event_subscriber_offsets (subscriber_name, sequence, updated_at)
                         VALUES ({}, {}, {})
                         ON CONFLICT(subscriber_name) DO UPDATE SET
                            sequence = excluded.sequence,
                            updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(subscriber),
                        SqlArg::I64(sequence),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}
