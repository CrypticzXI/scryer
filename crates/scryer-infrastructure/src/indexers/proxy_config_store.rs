use async_trait::async_trait;
use scryer_application::{AppError, AppResult, IndexerProxyConfigRepository};
use scryer_domain::{
    ChallengeSolverProtocol, IndexerProxyConfig, IndexerProxyHealthStatus, IndexerProxyProviderType,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

const INDEXER_PROXY_COLUMNS: &str = "id, name, provider_type, protocol, base_url,
    request_timeout_seconds, is_enabled, last_health_status, last_error_message,
    last_error_at, created_at, updated_at";

const INDEXER_PROXY_INSERT_SQL: &str = "INSERT INTO indexer_proxy_configs (
    id, name, provider_type, protocol, base_url, request_timeout_seconds, is_enabled,
    last_health_status, last_error_message, last_error_at, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

#[derive(Clone)]
pub struct IndexerProxyConfigStore {
    datastore: StoreDatastore,
}

impl IndexerProxyConfigStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl IndexerProxyConfigRepository for IndexerProxyConfigStore {
    async fn list(
        &self,
        provider_type: Option<IndexerProxyProviderType>,
    ) -> AppResult<Vec<IndexerProxyConfig>> {
        let (sql, args) = match provider_type {
            Some(provider_type) => (
                format!(
                    "SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs WHERE provider_type = {{}} ORDER BY created_at DESC"
                ),
                vec![SqlArg::Text(provider_type.as_str().to_string())],
            ),
            None => (
                format!(
                    "SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs ORDER BY created_at DESC"
                ),
                Vec::new(),
            ),
        };
        fetch_proxy_configs(self.datastore.read_exec(), &sql, &args).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerProxyConfig>> {
        fetch_optional_proxy_config(
            self.datastore.read_exec(),
            &format!("SELECT {INDEXER_PROXY_COLUMNS} FROM indexer_proxy_configs WHERE id = {{}}"),
            &[SqlArg::Text(id.to_string())],
        )
        .await
    }

    async fn create(&self, config: IndexerProxyConfig) -> AppResult<IndexerProxyConfig> {
        let args = proxy_insert_args(&config);
        SqlRuntime::run_in_transaction(&self.datastore, "create_indexer_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                SqlRuntime::execute(SqlExec::Tx(tx), INDEXER_PROXY_INSERT_SQL, &args).await?;
                Ok(config)
            })
        })
        .await
    }

    async fn update(&self, config: IndexerProxyConfig) -> AppResult<IndexerProxyConfig> {
        let args = vec![
            SqlArg::Text(config.name.clone()),
            SqlArg::Text(config.provider_type.as_str().to_string()),
            SqlArg::Text(config.protocol.as_str().to_string()),
            SqlArg::Text(config.base_url.clone()),
            SqlArg::I64(i64::from(config.request_timeout_seconds)),
            SqlArg::Bool(config.is_enabled),
            SqlArg::OptText(
                config
                    .last_health_status
                    .map(|status| status.as_str().to_string()),
            ),
            SqlArg::OptText(config.last_error_message.clone()),
            SqlArg::OptTimestamp(config.last_error_at),
            SqlArg::Timestamp(config.updated_at),
            SqlArg::Text(config.id.clone()),
        ];
        SqlRuntime::run_in_transaction(&self.datastore, "update_indexer_proxy_config", move |tx| {
            let config = config.clone();
            let args = args.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE indexer_proxy_configs SET
                            name = {}, provider_type = {}, protocol = {}, base_url = {},
                            request_timeout_seconds = {}, is_enabled = {},
                            last_health_status = {}, last_error_message = {},
                            last_error_at = {}, updated_at = {}
                         WHERE id = {}",
                    &args,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!(
                        "indexer proxy config {}",
                        config.id
                    )));
                }
                Ok(config)
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_indexer_proxy_config", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM indexer_proxy_configs WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("indexer proxy config {id}")));
                }
                Ok(())
            })
        })
        .await
    }
}

fn proxy_insert_args(config: &IndexerProxyConfig) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(config.id.clone()),
        SqlArg::Text(config.name.clone()),
        SqlArg::Text(config.provider_type.as_str().to_string()),
        SqlArg::Text(config.protocol.as_str().to_string()),
        SqlArg::Text(config.base_url.clone()),
        SqlArg::I64(i64::from(config.request_timeout_seconds)),
        SqlArg::Bool(config.is_enabled),
        SqlArg::OptText(
            config
                .last_health_status
                .map(|status| status.as_str().to_string()),
        ),
        SqlArg::OptText(config.last_error_message.clone()),
        SqlArg::OptTimestamp(config.last_error_at),
        SqlArg::Timestamp(config.created_at),
        SqlArg::Timestamp(config.updated_at),
    ]
}

async fn fetch_proxy_configs(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Vec<IndexerProxyConfig>> {
    SqlRuntime::fetch_all(exec, sql, args)
        .await?
        .into_iter()
        .map(|row| row_to_proxy_config(&row))
        .collect()
}

async fn fetch_optional_proxy_config(
    exec: SqlExec<'_, '_>,
    sql: &str,
    args: &[SqlArg],
) -> AppResult<Option<IndexerProxyConfig>> {
    SqlRuntime::fetch_optional(exec, sql, args)
        .await?
        .map(|row| row_to_proxy_config(&row))
        .transpose()
}

fn row_to_proxy_config(row: &SqlRow) -> AppResult<IndexerProxyConfig> {
    let provider_type = row.text("provider_type")?;
    let protocol = row.text("protocol")?;
    let last_health_status = row
        .opt_text("last_health_status")?
        .as_deref()
        .and_then(IndexerProxyHealthStatus::parse);

    Ok(IndexerProxyConfig {
        id: row.text("id")?,
        name: row.text("name")?,
        provider_type: IndexerProxyProviderType::parse(&provider_type).ok_or_else(|| {
            AppError::Repository(format!(
                "unknown indexer proxy provider type '{provider_type}'"
            ))
        })?,
        protocol: ChallengeSolverProtocol::parse(&protocol).ok_or_else(|| {
            AppError::Repository(format!("unknown indexer proxy protocol '{protocol}'"))
        })?,
        base_url: row.text("base_url")?,
        request_timeout_seconds: row.i64("request_timeout_seconds")?.max(0) as u32,
        is_enabled: row.bool("is_enabled")?,
        last_health_status,
        last_error_message: row.opt_text("last_error_message")?,
        last_error_at: row.opt_timestamp("last_error_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}
