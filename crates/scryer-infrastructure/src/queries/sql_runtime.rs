use chrono::{DateTime, Utc};
use scryer_application::{AppError, AppResult};
use serde_json::Value as JsonValue;
use sqlx::postgres::{PgArguments, PgPool, PgRow};
use sqlx::query::Query;
use sqlx::sqlite::{SqliteArguments, SqlitePool, SqliteRow};
use sqlx::types::Json;
use sqlx::{Postgres, Row, Sqlite, Transaction};

use super::common::parse_utc_datetime;

pub(crate) enum SqlTarget<'a> {
    Sqlite(&'a SqlitePool),
    Postgres(&'a PgPool),
}

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

pub(crate) enum SqlTx<'a> {
    Sqlite(Transaction<'a, Sqlite>),
    Postgres(Transaction<'a, Postgres>),
}

pub(crate) struct SqlRuntime;

impl SqlRuntime {
    pub(crate) async fn execute(
        target: SqlTarget<'_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<u64> {
        match target {
            SqlTarget::Sqlite(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
            SqlTarget::Postgres(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                let result = query.execute(pool).await.map_err(repo_err)?;
                Ok(result.rows_affected())
            }
        }
    }

    pub(crate) async fn fetch_optional(
        target: SqlTarget<'_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Option<SqlRow>> {
        match target {
            SqlTarget::Sqlite(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Sqlite))
                    .map_err(repo_err)
            }
            SqlTarget::Postgres(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                query
                    .fetch_optional(pool)
                    .await
                    .map(|row| row.map(SqlRow::Postgres))
                    .map_err(repo_err)
            }
        }
    }

    pub(crate) async fn fetch_all(
        target: SqlTarget<'_>,
        template: &str,
        args: &[SqlArg],
    ) -> AppResult<Vec<SqlRow>> {
        match target {
            SqlTarget::Sqlite(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Sqlite, args.len())?;
                let query = bind_sqlite(sqlx::query(&sql), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect())
                    .map_err(repo_err)
            }
            SqlTarget::Postgres(pool) => {
                let sql = render_sql(template, PlaceholderDialect::Postgres, args.len())?;
                let query = bind_postgres(sqlx::query(&sql), args);
                query
                    .fetch_all(pool)
                    .await
                    .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect())
                    .map_err(repo_err)
            }
        }
    }

    pub(crate) async fn begin(target: SqlTarget<'_>) -> AppResult<SqlTx<'_>> {
        match target {
            SqlTarget::Sqlite(pool) => pool.begin().await.map(SqlTx::Sqlite).map_err(repo_err),
            SqlTarget::Postgres(pool) => pool.begin().await.map(SqlTx::Postgres).map_err(repo_err),
        }
    }
}

impl SqlTx<'_> {
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
            SqlRow::Postgres(row) => row.try_get(column).map_err(repo_err),
        }
    }
}

type SqliteQuery<'q> = Query<'q, Sqlite, SqliteArguments<'q>>;
type PostgresQuery<'q> = Query<'q, Postgres, PgArguments>;

fn bind_sqlite<'q>(mut query: SqliteQuery<'q>, args: &[SqlArg]) -> SqliteQuery<'q> {
    for arg in args {
        query = match arg {
            SqlArg::Text(value) => query.bind(value.clone()),
            SqlArg::OptText(value) => query.bind(value.clone()),
            SqlArg::I64(value) => query.bind(*value),
            SqlArg::OptI64(value) => query.bind(*value),
            SqlArg::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
            SqlArg::OptBool(value) => {
                query.bind(value.map(|value| if value { 1_i64 } else { 0_i64 }))
            }
            SqlArg::Timestamp(value) => query.bind(value.to_rfc3339()),
            SqlArg::OptTimestamp(value) => query.bind(value.map(|value| value.to_rfc3339())),
            SqlArg::Json(value) => query.bind(value.to_string()),
            SqlArg::OptJson(value) => query.bind(value.as_ref().map(ToString::to_string)),
        };
    }
    query
}

fn bind_postgres<'q>(mut query: PostgresQuery<'q>, args: &[SqlArg]) -> PostgresQuery<'q> {
    for arg in args {
        query = match arg {
            SqlArg::Text(value) => query.bind(value.clone()),
            SqlArg::OptText(value) => query.bind(value.clone()),
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
            "placeholder count mismatch: expected {placeholder_count} binds for `{template}`, got {bind_count}"
        )));
    }

    let mut rendered = String::with_capacity(template.len() + bind_count * 3);
    let mut index = 1usize;
    let mut parts = template.split("{}").peekable();

    while let Some(part) = parts.next() {
        rendered.push_str(part);
        if parts.peek().is_none() {
            continue;
        }

        match dialect {
            PlaceholderDialect::Sqlite => rendered.push('?'),
            PlaceholderDialect::Postgres => {
                rendered.push('$');
                rendered.push_str(&index.to_string());
                index += 1;
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
    match row.try_get::<Option<i64>, _>(column) {
        Ok(value) => Ok(value),
        Err(integer_error) => match row.try_get::<Option<String>, _>(column) {
            Ok(value) => value
                .map(|value| {
                    value.parse::<i64>().map_err(|parse_error| {
                        AppError::Repository(format!("failed parse {column} as i64: {parse_error}"))
                    })
                })
                .transpose(),
            Err(string_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional i64: {integer_error}; {string_error}"
            ))),
        },
    }
}

fn opt_i64_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<i64>> {
    match row.try_get::<Option<i64>, _>(column) {
        Ok(value) => Ok(value),
        Err(integer_error) => match row.try_get::<Option<String>, _>(column) {
            Ok(value) => value
                .map(|value| {
                    value.parse::<i64>().map_err(|parse_error| {
                        AppError::Repository(format!("failed parse {column} as i64: {parse_error}"))
                    })
                })
                .transpose(),
            Err(string_error) => Err(AppError::Repository(format!(
                "failed decode {column} as optional i64: {integer_error}; {string_error}"
            ))),
        },
    }
}

fn bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<bool> {
    match row.try_get::<i64, _>(column) {
        Ok(value) => Ok(value != 0),
        Err(integer_error) => row.try_get::<bool, _>(column).map_err(|bool_error| {
            AppError::Repository(format!(
                "failed decode {column} as bool: {integer_error}; {bool_error}"
            ))
        }),
    }
}

fn bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<bool> {
    match row.try_get::<bool, _>(column) {
        Ok(value) => Ok(value),
        Err(bool_error) => row
            .try_get::<i64, _>(column)
            .map(|value| value != 0)
            .map_err(|integer_error| {
                AppError::Repository(format!(
                    "failed decode {column} as bool: {bool_error}; {integer_error}"
                ))
            }),
    }
}

fn opt_bool_from_sqlite_row(row: &SqliteRow, column: &str) -> AppResult<Option<bool>> {
    match row.try_get::<Option<i64>, _>(column) {
        Ok(value) => Ok(value.map(|value| value != 0)),
        Err(integer_error) => row
            .try_get::<Option<bool>, _>(column)
            .map_err(|bool_error| {
                AppError::Repository(format!(
                    "failed decode {column} as optional bool: {integer_error}; {bool_error}"
                ))
            }),
    }
}

fn opt_bool_from_pg_row(row: &PgRow, column: &str) -> AppResult<Option<bool>> {
    match row.try_get::<Option<bool>, _>(column) {
        Ok(value) => Ok(value),
        Err(bool_error) => row
            .try_get::<Option<i64>, _>(column)
            .map(|value| value.map(|value| value != 0))
            .map_err(|integer_error| {
                AppError::Repository(format!(
                    "failed decode {column} as optional bool: {bool_error}; {integer_error}"
                ))
            }),
    }
}

pub(crate) fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}
