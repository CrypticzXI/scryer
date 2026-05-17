use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::Row;
use sqlx::types::Json;

use scryer_application::AppResult;

use super::runtime::{SqlArg, SqlRow, repo_err};

pub(crate) fn canonical_json_text<T: Serialize>(value: &T) -> AppResult<String> {
    serde_json::to_string(value).map_err(repo_err)
}

pub(crate) fn canonical_json_arg<T: Serialize>(value: &T) -> AppResult<SqlArg> {
    canonical_json_text(value).map(SqlArg::Text)
}

pub(crate) fn opt_json_text(row: &SqlRow, column: &str) -> AppResult<Option<String>> {
    match row {
        SqlRow::Sqlite(row) => {
            let raw: Option<String> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.filter(|value| !value.trim().is_empty()))
        }
        SqlRow::Postgres(row) => {
            if let Ok(raw) = row.try_get::<Option<String>, _>(column) {
                return Ok(raw.filter(|value| !value.trim().is_empty()));
            }
            let raw: Option<Json<JsonValue>> = row.try_get(column).map_err(repo_err)?;
            Ok(raw.map(|value| value.0.to_string()))
        }
    }
}

pub(crate) fn json_text_or(row: &SqlRow, column: &str, default: &str) -> AppResult<String> {
    Ok(opt_json_text(row, column)?.unwrap_or_else(|| default.to_string()))
}
