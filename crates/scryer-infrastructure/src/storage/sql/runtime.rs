use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
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
pub enum StoreDatastore {
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
    I32(i32),
    I64(i64),
    OptI32(Option<i32>),
    OptI64(Option<i64>),
    F64(f64),
    OptF64(Option<f64>),
    Bool(bool),
    OptBool(Option<bool>),
    Timestamp(DateTime<Utc>),
    OptTimestamp(Option<DateTime<Utc>>),
    Json(JsonValue),
    OptJson(Option<JsonValue>),
    OptBytes(Option<Vec<u8>>),
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
    pub(crate) async fn run_serialized_sqlite<T, Op, Fut>(
        datastore: &StoreDatastore,
        op_name: &'static str,
        mut op: Op,
    ) -> AppResult<T>
    where
        T: Send,
        Op: FnMut(SqlitePool) -> Fut + Send,
        Fut: Future<Output = AppResult<T>> + Send,
    {
        let StoreDatastore::Sqlite { pool, writer_gate } = datastore else {
            return Err(AppError::Repository(format!(
                "operation `{op_name}` requires sqlite datastore"
            )));
        };

        let _guard = writer_gate.lock().await;
        run_with_sqlite_busy_retries(op_name, || op(pool.clone())).await
    }

    pub(crate) async fn execute(
        exec: SqlExec<'_, '_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<u64> {
        match exec {
            SqlExec::Target(SqlTarget::Sqlite(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
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
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Sqlite))
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Postgres))
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_optional(&mut **tx)
                        .await
                        .map(|row| row.map(SqlRow::Sqlite))
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
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
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                    .map_err(repo_err)
            }
            SqlExec::Target(SqlTarget::Postgres(pool)) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                    .map_err(repo_err)
            }
            SqlExec::Tx(tx) => match tx {
                SqlTx::Sqlite(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                    let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                        .map_err(repo_err)
                }
                SqlTx::Postgres(tx) => {
                    let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                    let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                    query
                        .fetch_all(&mut **tx)
                        .await
                        .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                        .map_err(repo_err)
                }
            },
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
                let query = bind_sqlite(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
                let result = query.execute(&mut **tx).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlTx::Postgres(tx) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(sqlx::AssertSqlSafe(&*sql)), args);
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
            SqlRow::Postgres(row) => i64_from_pg_row(row, column),
        }
    }

    pub(crate) fn i32(&self, column: &str) -> AppResult<i32> {
        match self {
            SqlRow::Sqlite(row) => i32_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => i32_from_pg_row(row, column),
        }
    }

    pub(crate) fn opt_i64(&self, column: &str) -> AppResult<Option<i64>> {
        match self {
            SqlRow::Sqlite(row) => opt_i64_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_i64_from_pg_row(row, column),
        }
    }

    pub(crate) fn opt_i32(&self, column: &str) -> AppResult<Option<i32>> {
        match self {
            SqlRow::Sqlite(row) => opt_i32_from_sqlite_row(row, column),
            SqlRow::Postgres(row) => opt_i32_from_pg_row(row, column),
        }
    }

    pub(crate) fn opt_f64(&self, column: &str) -> AppResult<Option<f64>> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
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
                if let Ok(raw) = row.try_get::<Option<Json<JsonValue>>, _>(column) {
                    return Ok(raw.map(|value| value.0));
                }
                let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
                raw.filter(|value| !value.trim().is_empty())
                    .map(|value| serde_json::from_str(&value).map_err(repo_err))
                    .transpose()
            }
        }
    }

    pub(crate) fn opt_bytes(&self, column: &str) -> AppResult<Option<Vec<u8>>> {
        match self {
            SqlRow::Sqlite(row) => row.try_get(column).map_err(repo_err),
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }
}

type SqliteQuery<'q> = Query<'q, Sqlite, SqliteArguments>;
type PostgresQuery<'q> = Query<'q, Postgres, PgArguments>;

fn bind_sqlite<'q>(mut query: SqliteQuery<'q>, values: &'q [SqlArg]) -> SqliteQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I32(value) => query.bind(i64::from(*value)),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI32(value) => query.bind(value.map(i64::from)),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::F64(value) => query.bind(*value),
            SqlArg::OptF64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
            SqlArg::OptBool(value) => {
                query.bind(value.map(|value| if value { 1_i64 } else { 0_i64 }))
            }
            SqlArg::Timestamp(value) => query.bind(value.to_rfc3339()),
            SqlArg::OptTimestamp(value) => query.bind(value.map(|value| value.to_rfc3339())),
            SqlArg::Json(value) => query.bind(value.to_string()),
            SqlArg::OptJson(value) => query.bind(value.as_ref().map(JsonValue::to_string)),
            SqlArg::OptBytes(value) => query.bind(value),
        };
    }
    query
}

fn bind_postgres<'q>(mut query: PostgresQuery<'q>, values: &'q [SqlArg]) -> PostgresQuery<'q> {
    for value in values {
        query = match value {
            SqlArg::Text(value) => query.bind(value),
            SqlArg::OptText(value) => query.bind(value),
            SqlArg::I32(value) => query.bind(*value),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI32(value) => query.bind(*value),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::F64(value) => query.bind(*value),
            SqlArg::OptF64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(*value),
            SqlArg::OptBool(value) => query.bind(*value),
            SqlArg::Timestamp(value) => query.bind(*value),
            SqlArg::OptTimestamp(value) => query.bind(*value),
            SqlArg::Json(value) => query.bind(Json(value.clone())),
            SqlArg::OptJson(value) => query.bind(value.clone().map(Json)),
            SqlArg::OptBytes(value) => query.bind(value),
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
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        Err(string_error) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_error}; {integer_error}"
            ))),
        },
    }
}

fn opt_text_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<String>> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        Err(string_error) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_error}; {integer_error}"
            ))),
        },
    }
}

fn opt_i64_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get(column).map_err(repo_err)
}

fn opt_i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i64>> {
    row.try_get::<Option<i64>, _>(column).or_else(|_| {
        row.try_get::<Option<i32>, _>(column)
            .map(|value| value.map(i64::from))
            .or_else(|_| {
                row.try_get::<Option<i16>, _>(column)
                    .map(|value| value.map(i64::from))
            })
            .map_err(repo_err)
    })
}

fn i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<i64> {
    row.try_get::<i64, _>(column).or_else(|_| {
        row.try_get::<i32, _>(column)
            .map(i64::from)
            .or_else(|_| row.try_get::<i16, _>(column).map(i64::from))
            .map_err(repo_err)
    })
}

fn i32_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<i32> {
    let value: i64 = row.try_get(column).map_err(repo_err)?;
    i32_from_i64(column, value)
}

fn i32_from_pg_row(row: &PgRow, column: &str) -> AppResult<i32> {
    row.try_get::<i32, _>(column).or_else(|_| {
        row.try_get::<i64, _>(column)
            .map_err(repo_err)
            .and_then(|value| i32_from_i64(column, value))
    })
}

fn opt_i32_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<i32>> {
    row.try_get::<Option<i64>, _>(column)
        .map_err(repo_err)?
        .map(|value| {
            i32::try_from(value).map_err(|_| {
                AppError::Repository(format!(
                    "value out of range for i32 column {column}: {value}"
                ))
            })
        })
        .transpose()
}

fn opt_i32_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i32>> {
    row.try_get::<Option<i32>, _>(column).or_else(|_| {
        row.try_get::<Option<i64>, _>(column)
            .map_err(repo_err)
            .and_then(|value| value.map(|value| i32_from_i64(column, value)).transpose())
    })
}

fn i32_from_i64(column: &str, value: i64) -> AppResult<i32> {
    i32::try_from(value).map_err(|_| {
        AppError::Repository(format!(
            "value out of range for i32 column {column}: {value}"
        ))
    })
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

// SQLITE_BUSY (5) and SQLITE_LOCKED (6) families only; anything else (IOERR,
// CONSTRAINT, ABORT, ...) must fail immediately instead of burning the retry
// deadline while holding the writer gate.
const TRANSIENT_SQLITE_ERROR_CODES: [u64; 6] = [5, 261, 517, 773, 6, 262];

pub(crate) fn is_transient_sqlite_busy(error: &AppError) -> bool {
    let AppError::Repository(message) = error else {
        return false;
    };

    let normalized = message.to_ascii_lowercase();
    normalized.contains("database is locked")
        || normalized.contains("database table is locked")
        || normalized.contains("database schema is locked")
        || normalized.contains("sqlite_busy")
        || normalized.contains("busy_snapshot")
        || sqlite_error_codes(&normalized).any(|code| TRANSIENT_SQLITE_ERROR_CODES.contains(&code))
}

// sqlx renders sqlite errors as "(code: N) message"; match N exactly so codes
// that merely start with a transient digit (516, 522, 526, ...) never retry.
fn sqlite_error_codes(message: &str) -> impl Iterator<Item = u64> + '_ {
    const CODE_PREFIX: &str = "code: ";
    message.match_indices(CODE_PREFIX).filter_map(|(index, _)| {
        let digits: String = message[index + CODE_PREFIX.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_error(message: &str) -> AppError {
        AppError::Repository(message.to_string())
    }

    #[test]
    fn busy_matcher_accepts_busy_and_locked_codes() {
        for message in [
            "error returned from database: (code: 5) database is locked",
            "error returned from database: (code: 261) recovery in progress",
            "error returned from database: (code: 517) busy snapshot",
            "error returned from database: (code: 773) busy timeout",
            "error returned from database: (code: 6) database table is locked",
            "error returned from database: (code: 262) shared cache locked",
        ] {
            assert!(
                is_transient_sqlite_busy(&repo_error(message)),
                "expected transient: {message}"
            );
        }
    }

    #[test]
    fn busy_matcher_accepts_stable_message_fallbacks() {
        assert!(is_transient_sqlite_busy(&repo_error(
            "SQLITE_BUSY: unable to acquire lock"
        )));
        assert!(is_transient_sqlite_busy(&repo_error(
            "transaction failed: BUSY_SNAPSHOT"
        )));
    }

    #[test]
    fn busy_matcher_rejects_non_transient_codes() {
        for message in [
            "error returned from database: (code: 516) abort due to rollback",
            "error returned from database: (code: 522) disk I/O error (short read)",
            "error returned from database: (code: 526) unable to open database file",
            "error returned from database: (code: 531) constraint failed in commit hook",
            "error returned from database: (code: 1555) UNIQUE constraint failed: titles.id",
            "error returned from database: (code: 1) SQL logic error",
        ] {
            assert!(
                !is_transient_sqlite_busy(&repo_error(message)),
                "expected non-transient: {message}"
            );
        }
    }

    #[test]
    fn busy_matcher_ignores_non_repository_errors() {
        assert!(!is_transient_sqlite_busy(&AppError::Validation(
            "(code: 5) database is locked".to_string()
        )));
    }
}
