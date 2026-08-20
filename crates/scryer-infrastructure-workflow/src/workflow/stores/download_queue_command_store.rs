use std::collections::HashMap;

use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadQueueCommandRecord, DownloadQueueCommandRepository,
};
use scryer_domain::{DownloadQueueCommandAction, DownloadQueueDeleteStatus, Id};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct DownloadQueueCommandStore {
    datastore: StoreDatastore,
}

impl DownloadQueueCommandStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DownloadQueueCommandRepository for DownloadQueueCommandStore {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let client_id = client_id.map(str::to_string);
        let client_type = client_type.to_string();
        let download_client_item_id = download_client_item_id.to_string();
        let requested_by_user_id = requested_by_user_id.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "queue_delete_download_command",
            move |tx| {
                let client_id = client_id.clone();
                let client_type = client_type.clone();
                let download_client_item_id = download_client_item_id.clone();
                let requested_by_user_id = requested_by_user_id.clone();
                Box::pin(async move {
                    let id = Id::new().0;
                    let now = Utc::now();
                    let normalized_client_id = normalize_download_client_id(client_id.as_deref());
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_queue_commands
                         (id, action, client_id, client_type, download_client_item_id, is_history, status, error_text, requested_by_user_id, started_at, finished_at, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, NULL, {}, NULL, NULL, {}, {})
                         ON CONFLICT DO NOTHING",
                        &[
                            SqlArg::Text(id),
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id.clone()),
                            SqlArg::Text(client_type.clone()),
                            SqlArg::Text(download_client_item_id.clone()),
                            SqlArg::Bool(is_history),
                            SqlArg::Text(DownloadQueueDeleteStatus::Queued.as_str().to_string()),
                            SqlArg::OptText(requested_by_user_id),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    fetch_optional_delete_command(
                        SqlExec::Tx(tx),
                        "WHERE action = {}
                           AND COALESCE(client_id, '') = {}
                           AND client_type = {}
                           AND download_client_item_id = {}
                           AND is_history = {}
                           AND status IN ('queued', 'running')
                         ORDER BY created_at DESC, id DESC
                         LIMIT 1",
                        &[
                            SqlArg::Text(DownloadQueueCommandAction::Delete.as_str().to_string()),
                            SqlArg::Text(normalized_client_id),
                            SqlArg::Text(client_type),
                            SqlArg::Text(download_client_item_id),
                            SqlArg::Bool(is_history),
                        ],
                    )
                    .await?
                    .ok_or_else(|| AppError::Repository("failed to load queued delete command".into()))
                })
            },
        )
        .await
    }

    async fn recover_stale_running_delete_commands(&self, stale_seconds: i64) -> AppResult<u64> {
        let now = Utc::now();
        let cutoff = now - chrono::Duration::seconds(stale_seconds);
        let rows = execute_write(
            &self.datastore,
            "recover_stale_running_delete_download_commands",
            "UPDATE download_queue_commands
             SET status = 'queued',
                 error_text = NULL,
                 started_at = NULL,
                 finished_at = NULL,
                 updated_at = {}
             WHERE action = 'delete'
               AND status = 'running'
               AND updated_at <= {}"
                .to_string(),
            vec![SqlArg::Timestamp(now), SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        fetch_delete_commands(
            self.datastore.read_exec(),
            "WHERE action = 'delete' AND status = 'queued' ORDER BY created_at ASC, id ASC",
            &[],
        )
        .await
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Running,
            None,
        )
        .await
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Completed,
            None,
        )
        .await
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        update_delete_command_status(
            &self.datastore,
            id,
            DownloadQueueDeleteStatus::Failed,
            error_text,
        )
        .await
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        let mut clauses = Vec::with_capacity(sources.len());
        for (client_id, client_type, download_client_item_id, is_history) in sources {
            let normalized_client_id = normalize_download_client_id(client_id.as_deref());
            let client_clause = if normalized_client_id.is_empty() {
                "COALESCE(client_id, '') = ''".to_string()
            } else {
                args.push(SqlArg::Text(normalized_client_id));
                "(COALESCE(client_id, '') = {} OR COALESCE(client_id, '') = '')".to_string()
            };
            args.push(SqlArg::Text(client_type.clone()));
            args.push(SqlArg::Text(download_client_item_id.clone()));
            args.push(SqlArg::Bool(*is_history));
            clauses.push(format!(
                "({client_clause} AND client_type = {{}} AND download_client_item_id = {{}} AND is_history = {{}})"
            ));
        }
        let rows = fetch_delete_commands(
            self.datastore.read_exec(),
            &format!(
                "WHERE action = 'delete' AND ({}) ORDER BY created_at DESC, id DESC",
                clauses.join(" OR ")
            ),
            &args,
        )
        .await?;
        let mut latest = HashMap::new();
        for record in rows {
            let key = (
                record.client_id.clone().unwrap_or_default(),
                record.client_type.clone(),
                record.download_client_item_id.clone(),
                record.is_history,
            );
            latest.entry(key).or_insert(record);
        }
        Ok(latest.into_values().collect())
    }

    async fn prune_terminal_delete_commands_older_than(&self, days: i64) -> AppResult<u32> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let rows = execute_write(
            &self.datastore,
            "prune_terminal_delete_download_commands_older_than",
            "DELETE FROM download_queue_commands
             WHERE action = 'delete'
               AND status IN ('completed', 'failed')
               AND updated_at < {}"
                .to_string(),
            vec![SqlArg::Timestamp(cutoff)],
        )
        .await?;
        Ok(rows as u32)
    }
}
