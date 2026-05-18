use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, DownloadClientConfigRepository, DownloadClientConfigUpdate,
};
use scryer_domain::{DownloadClientConfig, DownloadClientStatus};

use crate::config_store::{current_encryption_key, decrypt_value, maybe_encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const DOWNLOAD_CLIENT_COLUMNS: &str = "id, name, client_type, config_json, is_enabled, status,
    client_priority, last_error, last_seen_at, created_at, updated_at";

const DOWNLOAD_CLIENT_INSERT_SQL: &str = "INSERT INTO download_clients (
    id, name, client_type, config_json, is_enabled, status,
    client_priority, last_error, last_seen_at, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct DownloadClientConfigStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl DownloadClientConfigStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl DownloadClientConfigRepository for DownloadClientConfigStore {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        let encryption_key = self.encryption_key()?;
        let (sql, args) = match client_type {
            Some(client_type) => (
                format!(
                    "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE client_type = {{}} ORDER BY client_priority ASC"
                ),
                vec![SqlArg::Text(client_type)],
            ),
            None => (
                format!(
                    "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients ORDER BY client_priority ASC"
                ),
                Vec::new(),
            ),
        };
        fetch_download_clients(
            self.datastore.read_exec(),
            &sql,
            &args,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let encryption_key = self.encryption_key()?;
        fetch_optional_download_client(
            self.datastore.read_exec(),
            &format!("SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
            encryption_key.as_ref(),
        )
        .await
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key()?;
        let args = download_client_insert_args(&config, encryption_key.as_ref())?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_download_client_config",
            move |tx| {
                let config = config.clone();
                let args = args.clone();
                Box::pin(async move {
                    SqlRuntime::execute(SqlExec::Tx(tx), DOWNLOAD_CLIENT_INSERT_SQL, &args).await?;
                    Ok(config)
                })
            },
        )
        .await
    }

    async fn update(&self, update: DownloadClientConfigUpdate) -> AppResult<DownloadClientConfig> {
        let encryption_key = self.encryption_key()?;
        let mut assignments = vec!["updated_at = {}".to_string()];
        let mut args = vec![SqlArg::Timestamp(Utc::now())];

        if let Some(name) = update.name.as_ref() {
            assignments.push("name = {}".to_string());
            args.push(SqlArg::Text(name.clone()));
        }
        if let Some(client_type) = update.client_type.as_ref() {
            assignments.push("client_type = {}".to_string());
            args.push(SqlArg::Text(client_type.clone()));
        }
        if let Some(config_json) = update.config_json.as_ref() {
            assignments.push("config_json = {}".to_string());
            args.push(SqlArg::Text(maybe_encrypt_value(
                encryption_key.as_ref(),
                config_json,
            )?));
        }
        if let Some(is_enabled) = update.is_enabled {
            assignments.push("is_enabled = {}".to_string());
            args.push(SqlArg::Bool(is_enabled));
        }

        if assignments.len() == 1 {
            return Err(AppError::Validation(
                "at least one download client config field must be provided".into(),
            ));
        }

        let id = update.id.clone();
        args.push(SqlArg::Text(id.clone()));
        let sql = format!(
            "UPDATE download_clients SET {} WHERE id = {{}}",
            assignments.join(", ")
        );
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_download_client_config",
            move |tx| {
                let sql = sql.clone();
                let args = args.clone();
                let id = id.clone();
                let encryption_key = encryption_key.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(SqlExec::Tx(tx), &sql, &args).await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!("download client config {id}")));
                    }
                    fetch_optional_download_client(
                        SqlExec::Tx(tx),
                        &format!(
                            "SELECT {DOWNLOAD_CLIENT_COLUMNS} FROM download_clients WHERE id = {{}}"
                        ),
                        &[SqlArg::Text(id.clone())],
                        encryption_key.as_ref(),
                    )
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("download client config {id}")))
                })
            },
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_client_config",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_clients WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                    if rows == 0 {
                        return Err(AppError::NotFound(format!("download client config {id}")));
                    }
                    Ok(())
                })
            },
        )
        .await
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "reorder_download_client_configs",
            move |tx| {
                let ordered_ids = ordered_ids.clone();
                Box::pin(async move {
                    for (index, id) in ordered_ids.iter().enumerate() {
                        SqlRuntime::execute(
                            SqlExec::Tx(tx),
                            "UPDATE download_clients
                             SET client_priority = {}, updated_at = {}
                             WHERE id = {}",
                            &[
                                SqlArg::I64(index as i64),
                                SqlArg::Timestamp(Utc::now()),
                                SqlArg::Text(id.clone()),
                            ],
                        )
                        .await?;
                    }
                    Ok(())
                })
            },
        )
        .await
    }
}

fn download_client_insert_args(
    config: &DownloadClientConfig,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<SqlArg>> {
    Ok(vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.client_type.clone()),
        SqlArg::Text(maybe_encrypt_value(encryption_key, &config.config_json)?),
        SqlArg::Bool(config.is_enabled),
        SqlArg::Text(config.status.as_str().to_string()),
        SqlArg::I64(config.client_priority),
        SqlArg::OptText(config.last_error.clone()),
        SqlArg::OptTimestamp(config.last_seen_at),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ])
}

async fn fetch_download_clients(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Vec<DownloadClientConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_download_client_config(&row, encryption_key))
        .collect()
}

async fn fetch_optional_download_client(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<DownloadClientConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_download_client_config(&row, encryption_key))
        .transpose()
}

fn row_to_download_client_config(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<DownloadClientConfig> {
    let status_raw = row.text("status")?;
    Ok(DownloadClientConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        client_type: row.text("client_type")?,
        config_json: decrypt_value(
            encryption_key,
            row.text("config_json")?,
            "config_json",
            false,
        )?,
        client_priority: row.i64("client_priority")?,
        is_enabled: row.bool("is_enabled")?,
        status: DownloadClientStatus::parse(&status_raw).unwrap_or_default(),
        last_error: row.opt_text("last_error")?,
        last_seen_at: row.opt_timestamp("last_seen_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}
