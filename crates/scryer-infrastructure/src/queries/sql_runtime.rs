use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use scryer_application::{AppError, AppResult};
use serde_json::Value as JsonValue;
use sqlx::postgres::{PgArguments, PgPool, PgRow};
use sqlx::query::Query;
use sqlx::sqlite::{SqliteArguments, SqlitePool, SqliteRow};
use sqlx::types::Json;
use sqlx::{Postgres, Row, Sqlite, Transaction};
use tokio::sync::Mutex;

use super::common::parse_utc_datetime;

const SQLITE_BUSY_RETRY_DELAYS: [Duration; 5] = [
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1000),
];
const SQLITE_BUSY_RETRY_HARD_CAP: Duration = Duration::from_secs(120);

#[derive(Clone)]
pub(crate) enum StoreDatastore {
    Sqlite {
        pool: SqlitePool,
        writer_gate: Arc<Mutex<()>>,
    },
    Postgres {
        pool: PgPool,
    },
}

impl StoreDatastore {
    pub(crate) fn read_exec(&self) -> SqlExec<'_, '_> {
        match self {
            Self::Sqlite { pool, .. } => SqlExec::Target(SqlTarget::Sqlite(pool)),
            Self::Postgres { pool } => SqlExec::Target(SqlTarget::Postgres(pool)),
        }
    }
}

pub(crate) enum SqlTarget<'a> {
    Sqlite(&'a SqlitePool),
    Postgres(&'a PgPool),
}

pub(crate) enum SqlExec<'tx, 'db> {
    Target(SqlTarget<'db>),
    Tx(&'tx mut SqlTx<'db>),
}

pub(crate) type TxFuture<'a, T> = Pin<Box<dyn Future<Output = AppResult<T>> + Send + 'a>>;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub(crate) enum SqlArg {
    Text(String),
    OptText(Option<String>),
    I64(i64),
    OptI64(Option<i64>),
    Bool(bool),
    OptBool(Option<bool>),
    Timestamp(DateTime<Utc>),
    OptTimestamp(Option<DateTime<Utc>>),
    Json(JsonValue),
    OptJson(Option<JsonValue>),
}

#[derive(Clone, Copy, Debug)]
enum PlaceholderDialect {
    Sqlite,
    Postgres,
}

pub(crate) enum SqlRow {
    Sqlite(SqliteRow),
    Postgres(PgRow),
}

pub(crate) enum SqlTx<'db> {
    Sqlite(Transaction<'db, Sqlite>),
    Postgres(Transaction<'db, Postgres>),
}

pub(crate) struct SqlRuntime;

impl SqlRuntime {
    pub(crate) async fn execute(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<u64> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlExec::Tx(tx) => tx.execute(template, args).await,
        }
    }

    pub(crate) async fn fetch_optional(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Option<SqlRow>> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Sqlite))
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Postgres))
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(&sql), args);
                    query
                        .fetch_optional(&mut **tx)
                        .await
                        .map(|row| row.map(SqlRow::Sqlite))
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(&sql), args);
                    query
                        .fetch_optional(&mut **tx)
                        .await
                        .map(|row| row.map(SqlRow::Postgres))
                        .map_err(repo_err)
                }
            },
        }
    }

    pub(crate) async fn fetch_all(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Vec<SqlRow>> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(&sql), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(&sql), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                        .map_err(repo_err)
                }
            },
        }
    }

    pub(crate) async fn begin(target: SqlTarget<'_>) -> AppResult<SqlTx<'_>> {
        match target {
            SqlTarget::Sqlite(pool) => pool.begin().await.map(SqlTx::Sqlite).map_err(repo_err),
            SqlTarget::Postgres(pool) => pool.begin().await.map(SqlTx::Postgres).map_err(repo_err),
        }
    }

    pub(crate) async fn run_in_transaction<T, F>(
        datastore: &StoreDatastore,
        op_name: &'static str,
        op: F,
    ) -> AppResult<T>
    where
        T: Send,
        F: for<'tx, 'db> Fn(&'tx mut SqlTx<'db>) -> TxFuture<'tx, T> + Send + Sync,
    {
        match datastore {
            StoreDatastore::Sqlite { pool, writer_gate } => {
                let _guard = writer_gate.lock().await;
                run_with_sqlite_busy_retries(op_name, || {
                    let pool = pool.clone();
                    let op = &op;
                    async move {
                        let mut tx = SqlTx::Sqlite(pool.begin().await.map_err(repo_err)?);
                        let result = {
                            let future = op(&mut tx);
                            future.await?
                        };
                        tx.commit().await?;
                        Ok(result)
                    }
                })
                .await
            }
            StoreDatastore::Postgres { pool } => {
                let mut tx = SqlTx::Postgres(pool.begin().await.map_err(repo_err)?);
                let result = {
                    let future = op(&mut tx);
                    future.await?
                };
                tx.commit().await?;
                Ok(result)
            }
        }
    }
}

impl<'db> SqlTx<'db> {
    pub(crate) async fn execute(&mut self, template: &str, args: &[SqlArg]) -> AppResult<u64> {
        match self {
            SqlTx::Sqlite(tx) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                let result = query.execute(&mut **tx).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlTx::Postgres(tx) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                let result = query.execute(&mut **tx).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
        }
    }

    pub(crate) fn sqlite<'tx>(&'tx mut self) -> Option<&'tx mut Transaction<'db, Sqlite>> {
        match self {
            Self::Sqlite(tx) => Some(tx),
            Self::Postgres(_) => None,
        }
    }

    pub(crate) fn postgres<'tx>(&'tx mut self) -> Option<&'tx mut Transaction<'db, Postgres>> {
        match self {
            Self::Sqlite(_) => None,
            Self::Postgres(tx) => Some(tx),
        }
    }

    pub(crate) async fn commit(self) -> AppResult<()> {
        match self {
            SqlTx::Sqlite(tx) => tx.commit().await.map_err(repo_err),
            SqlTx::Postgres(tx) => tx.commit().await.map_err(repo_err),
        }
    }
}

#[allow(dead_code)]
impl SqlRow {
    pub(crate) fn text(&self, column: &str) -> AppResult<String> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub(crate) fn opt_text(&self, column: &str) -> AppResult<Option<String>> {
        match self {
            SqlRow::Sqlite(row) => opt_text_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_text_from_pg_row(row, column),
        }
    }

    pub(crate) fn i64(&self, column: &str) -> AppResult<i64> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub(crate) fn opt_i64(&self, column: &str) -> AppResult<Option<i64>> {
        match self {
            SqlRow::Sqlite(row) => opt_i64_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_i64_from_pg_row(row, column),
        }
    }

    pub(crate) fn bool(&self, column: &str) -> AppResult<bool> {
        match self {
            SqlRow::Sqlite(row) => bool_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => bool_from_pg_row(row, column),
        }
    }

    pub(crate) fn opt_bool(&self, column: &str) -> AppResult<Option<bool>> {
        match self {
            SqlRow::Sqlite(row) => opt_bool_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_bool_from_pg_row(row, column),
        }
    }

    pub(crate) fn timestamp(&self, column: &str) -> AppResult<DateTime<Utc>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: String = row.try_get(column).map_err(repo_err)?;
                parse_utc_datetime(&raw)
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub(crate) fn opt_timestamp(&self, column: &str) -> AppResult<Option<DateTime<Utc>>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                match raw {
                    Some(raw) if !raw.trim().is_empty() => parse_utc_datetime(&raw).map(Some),
                    Some(_) | None => Ok(None),
                }
            }
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }

    pub(crate) fn opt_json(&self, column: &str) -> AppResult<Option<JsonValue>> {
        match self {
            SqlRow::Sqlite(row) => {
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                match raw {
                    Some(raw) if !raw.trim().is_empty() => {
                        serde_json::from_str(&raw).map(Some).map_err(repo_err)
                    }
                    Some(_) | None => Ok(None),
                }
            }
            SqlRow::Postgres(row) => {
                let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
                Ok(raw.map(|value| value.0))
            }
        }
    }
}

type SqliteQuery<'q> = Query<'q, Sqlite, SqliteArguments<'q>>;
type PostgresQuery<'q> = Query<'q, Postgres, PgArguments>;

fn bind_sqlite<'q>(mut query: SqliteQuery<'q>, values: &'q [SqlArg]) -> SqliteQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
            SqlArg::OptBool(value) => {
                query.bind(value.map(|value| if value { 1_i64 } else { 0_i64 }))
            }
            SqlArg::Timestamp(value) => {
                query.bind(value.to_rfc3339_opts(SecondsFormat::Secs, true))
            }
            SqlArg::OptTimestamp(value) => {
                query.bind(value.map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true)))
            }
            SqlArg::Json(value) => query.bind(value.to_string()),
            SqlArg::OptJson(value) => query.bind(value.as_ref().map(JsonValue::to_string)),
        };
    }
    query
}

fn bind_postgres<'q>(mut query: PostgresQuery<'q>, values: &'q [SqlArg]) -> PostgresQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(*value),
            SqlArg::OptBool(value) => query.bind(*value),
            SqlArg::Timestamp(value) => query.bind(*value),
            SqlArg::OptTimestamp(value) => query.bind(*value),
            SqlArg::Json(value) => query.bind(Json(value.clone())),
            SqlArg::OptJson(value) => query.bind(value.clone().map(Json)),
        };
    }
    query
}

fn render_sql(template: &str, dialect: PlaceholderDialect, bind_count: usize) -> AppResult<String> {
    let placeholder_count = template.matches("{}").count();
    if placeholder_count != bind_count {
        return Err(AppError::Repository(format!(
            "sql placeholder mismatch: expected {placeholder_count} bind(s), received {bind_count}"
        )));
    }

    let mut next_index = 1usize;
    let mut rendered = String::with_capacity(template.len() + bind_count * 2);
    let mut parts = template.split("{}").peekable();
    while let Some(part) = parts.next() {
        rendered.push_str(part);
        if parts.peek().is_some() {
            match dialect {
                PlaceholderDialect::Sqlite => rendered.push('?'),
                PlaceholderDialect::Postgres => {
                    rendered.push('$');
                    rendered.push_str(&next_index.to_string());
                    next_index += 1;
                }
            }
        }
    }
    Ok(rendered)
}

fn opt_text_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<String>> {
    let value: Option<String> = row.try_get(column).map_err(repo_err)?;
    match value {
        Some(value) if value.trim().is_empty() => Ok(None),
        other => Ok(other),
    }
}

fn opt_text_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<String>> {
    let value: Option<String> = row.try_get(column).map_err(repo_err)?;
    match value {
        Some(value) if value.trim().is_empty() => Ok(None),
        other => Ok(other),
    }
}

fn opt_i64_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get(column).map_err(repo_err)
}

fn opt_i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get(column).map_err(repo_err)
}

fn bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<bool> {
    let value: i64 = row.try_get(column).map_err(repo_err)?;
    Ok(value != 0)
}

fn bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<bool> {
    row.try_get(column).map_err(repo_err)
}

fn opt_bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<bool>> {
    let value: Option<i64> = row.try_get(column).map_err(repo_err)?;
    Ok(value.map(|value| value != 0))
}

fn opt_bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<bool>> {
    row.try_get(column).map_err(repo_err)
}

pub(crate) fn is_transient_sqlite_busy(error: &AppError) -> bool {
    let AppError::Repository(message) = error else {
        return false;
    };

    let normalized = message.to_ascii_lowercase();
    normalized.contains("sqlite_code=5")
        || normalized.contains("sqlite_code=517")
        || normalized.contains("database is locked")
        || normalized.contains("database table is locked")
        || normalized.contains("database schema is locked")
        || normalized.contains("sqlite_busy")
        || normalized.contains("busy_snapshot")
        || normalized.contains("code: 5")
        || normalized.contains("code: 517")
}

pub(crate) async fn run_with_sqlite_busy_retries<T, Op, Fut>(
    operation_name: &str,
    mut operation: Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    run_with_sqlite_busy_retries_with_deadline(
        operation_name,
        SQLITE_BUSY_RETRY_HARD_CAP,
        &mut operation,
    )
    .await
}

pub(crate) async fn run_with_sqlite_busy_retries_with_deadline<T, Op, Fut>(
    operation_name: &str,
    hard_cap: Duration,
    operation: &mut Op,
) -> AppResult<T>
where
    Op: FnMut() -> Fut,
    Fut: Future<Output = AppResult<T>>,
{
    let started_at = tokio::time::Instant::now();
    let mut attempt = 0usize;

    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if is_transient_sqlite_busy(&error) => {
                let elapsed = started_at.elapsed();
                if elapsed >= hard_cap {
                    tracing::warn!(
                        attempts = attempt,
                        elapsed_ms = elapsed.as_millis(),
                        error = %error,
                        operation = operation_name,
                        "serialized sqlite writer: retry deadline exhausted"
                    );
                    return Err(AppError::Repository(format!(
                        "serialized sqlite writer: retry deadline exceeded for operation `{operation_name}` after {attempt} attempts over {}ms: {error}",
                        elapsed.as_millis()
                    )));
                }

                let scheduled_delay = SQLITE_BUSY_RETRY_DELAYS
                    [attempt.min(SQLITE_BUSY_RETRY_DELAYS.len().saturating_sub(1))];
                let remaining = hard_cap.saturating_sub(elapsed);
                let delay = scheduled_delay.min(remaining);
                tracing::debug!(
                    attempt = attempt + 1,
                    retry_after_ms = delay.as_millis(),
                    elapsed_ms = elapsed.as_millis(),
                    error = %error,
                    operation = operation_name,
                    "serialized sqlite writer: retrying transient sqlite busy"
                );
                attempt = attempt.saturating_add(1);
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
