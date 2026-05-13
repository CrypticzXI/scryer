use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use scryer_application::{
    AppError, AppResult, BACKUP_TABLE_CATALOG, BLOB_MARKER_BASE64, BLOB_MARKER_TYPE,
    BackupBundleExportRequest, BackupBundleStaging, BackupRestorePreparedBundle,
    BackupTableClassification, EXPORT_BATCH_SIZE, LogicalBackupExporter,
    prepare_backup_restore_payload,
};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use sqlx::{Column, Row, TypeInfo, ValueRef};

#[derive(Clone, Debug)]
pub struct SqliteLogicalBackupExporter {
    db_path: String,
}

impl SqliteLogicalBackupExporter {
    pub fn new(db_path: impl Into<String>) -> Self {
        Self {
            db_path: db_path.into(),
        }
    }
}

#[async_trait]
impl LogicalBackupExporter for SqliteLogicalBackupExporter {
    async fn export_backup_bundle(
        &self,
        request: BackupBundleExportRequest,
    ) -> AppResult<scryer_application::BackupExportOutcome> {
        export_backup_bundle_from_sqlite(&self.db_path, request).await
    }
}

pub async fn export_backup_bundle_from_sqlite(
    db_path: &str,
    request: BackupBundleExportRequest,
) -> AppResult<scryer_application::BackupExportOutcome> {
    let mut staging = BackupBundleStaging::new()?;

    let mut connect_options = db_connect_options(db_path)?;
    connect_options = connect_options.read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to open source database for backup: {error}"
            ))
        })?;

    let export_result = export_backup_tables_from_pool(&pool, &mut staging).await;
    pool.close().await;
    export_result?;

    staging.finish(request)
}

async fn export_backup_tables_from_pool(
    pool: &sqlx::SqlitePool,
    staging: &mut BackupBundleStaging,
) -> AppResult<()> {
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let tables_dir = staging.tables_dir();

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to acquire source database connection: {error}"
        ))
    })?;
    sqlx::query("BEGIN")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to begin backup snapshot: {error}"))
        })?;

    let mut export_result = Ok(());
    for table in &export_tables {
        let table_result = async {
            let row_count = export_table_part(&mut conn, table, &tables_dir).await?;
            staging.record_table_part(table, row_count)
        }
        .await;

        if let Err(error) = table_result {
            export_result = Err(error);
            break;
        }
    }

    let rollback_result = sqlx::query("ROLLBACK")
        .execute(&mut *conn)
        .await
        .map(|_| ())
        .map_err(|error| AppError::Repository(format!("failed to close backup snapshot: {error}")));

    match (export_result, rollback_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

pub async fn restore_backup_bundle_into_sqlite_pool(
    pool: &sqlx::SqlitePool,
    bundle_path: &Path,
    passphrase: Option<&str>,
) -> AppResult<BackupRestorePreparedBundle> {
    let payload = prepare_backup_restore_payload(bundle_path, passphrase)?;
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let expected_tables = export_tables.iter().cloned().collect::<BTreeSet<_>>();
    let manifest_tables = payload
        .manifest()
        .row_counts
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    if manifest_tables != expected_tables {
        return Err(AppError::Validation(
            "backup bundle table set does not match the current restore catalog".into(),
        ));
    }

    let mut conn = pool.acquire().await.map_err(|error| {
        AppError::Repository(format!("failed to acquire restore connection: {error}"))
    })?;

    sqlx::query("PRAGMA foreign_keys = OFF")
        .execute(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to disable foreign keys for restore: {error}"
            ))
        })?;

    let tables_dir = payload.tables_dir();
    let restore_result = async {
        for table in export_tables.iter().rev() {
            let sql = format!("DELETE FROM {}", quote_identifier(table));
            sqlx::query(&sql)
                .execute(&mut *conn)
                .await
                .map_err(|error| {
                    AppError::Repository(format!("failed to clear restore table {table}: {error}"))
                })?;
        }

        for table in &export_tables {
            import_table_part(
                &mut conn,
                table,
                &tables_dir.join(format!("{table}.ndjson.zst")),
            )
            .await?;
        }

        AppResult::Ok(())
    }
    .await;

    let enable_foreign_keys_result = sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *conn)
        .await
        .map(|_| ())
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to re-enable foreign keys for restore: {error}"
            ))
        });

    if let Err(error) = restore_result {
        return Err(error);
    }
    enable_foreign_keys_result?;

    let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to validate restored foreign keys: {error}"))
        })?;
    if violations != 0 {
        return Err(AppError::Validation(
            "restored database failed foreign key validation".into(),
        ));
    }

    crate::queries::title_search::rebuild_title_search_projection(pool).await?;

    for (table, expected_rows) in &payload.manifest().row_counts {
        let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
        let actual_rows: i64 = sqlx::query_scalar(&sql)
            .fetch_one(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to validate restored table {table}: {error}"
                ))
            })?;
        if actual_rows as u64 != *expected_rows {
            return Err(AppError::Validation(format!(
                "restored table {table} row count mismatch: expected {expected_rows}, got {actual_rows}"
            )));
        }
    }

    Ok(
        BackupRestorePreparedBundle::from_summary_and_instance_secrets_env(
            payload.summary(),
            payload.instance_secrets_env()?,
        ),
    )
}

fn db_connect_options(db_path: &str) -> AppResult<SqliteConnectOptions> {
    db_path.parse::<SqliteConnectOptions>().map_err(|error| {
        AppError::Repository(format!("invalid sqlite database path {db_path}: {error}"))
    })
}

async fn validate_backup_catalog(pool: &sqlx::SqlitePool) -> AppResult<()> {
    let actual_tables = application_tables(pool).await?;
    let mut classified = BTreeSet::new();
    for entry in BACKUP_TABLE_CATALOG {
        classified.insert(entry.table.to_string());
    }

    let unclassified = actual_tables
        .into_iter()
        .filter(|table| !classified.contains(table))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        return Err(AppError::Repository(format!(
            "backup catalog is missing classifications for tables: {}",
            unclassified.join(", ")
        )));
    }

    Ok(())
}

async fn application_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
          ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| AppError::Repository(format!("failed to inspect sqlite schema: {error}")))?;

    let mut tables = Vec::new();
    for row in rows {
        let table: String = row.try_get("name").map_err(|error| {
            AppError::Repository(format!("failed to decode sqlite schema row: {error}"))
        })?;
        if is_engine_internal_table(&table) {
            continue;
        }
        tables.push(table);
    }
    Ok(tables)
}

fn is_engine_internal_table(table: &str) -> bool {
    table.starts_with("sqlite_") || table.starts_with("title_search_spellfix_")
}

async fn ordered_export_tables(pool: &sqlx::SqlitePool) -> AppResult<Vec<String>> {
    let export_tables = BACKUP_TABLE_CATALOG
        .iter()
        .filter(|entry| entry.classification == BackupTableClassification::Export)
        .map(|entry| entry.table.to_string())
        .collect::<BTreeSet<_>>();

    let mut incoming = BTreeMap::<String, usize>::new();
    let mut outgoing = BTreeMap::<String, BTreeSet<String>>::new();
    for table in &export_tables {
        incoming.insert(table.clone(), 0);
        outgoing.insert(table.clone(), BTreeSet::new());
    }

    for table in &export_tables {
        let pragma = format!("PRAGMA foreign_key_list({})", quote_identifier(table));
        let rows = sqlx::query(&pragma)
            .fetch_all(pool)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign keys for {table}: {error}"
                ))
            })?;
        for row in rows {
            let referenced: String = row.try_get("table").map_err(|error| {
                AppError::Repository(format!(
                    "failed to inspect foreign key for {table}: {error}"
                ))
            })?;
            if !export_tables.contains(&referenced) {
                continue;
            }
            if outgoing
                .get_mut(&referenced)
                .expect("known table")
                .insert(table.clone())
            {
                *incoming.get_mut(table).expect("known table") += 1;
            }
        }
    }

    let mut ready = incoming
        .iter()
        .filter_map(|(table, count)| (*count == 0).then_some(table.clone()))
        .collect::<VecDeque<_>>();
    let mut ordered = Vec::new();

    while let Some(table) = ready.pop_front() {
        ordered.push(table.clone());
        let dependents = outgoing.get(&table).cloned().unwrap_or_default();
        for dependent in dependents {
            let count = incoming.get_mut(&dependent).expect("known dependent");
            *count -= 1;
            if *count == 0 {
                let insert_at = ready
                    .iter()
                    .position(|candidate| candidate > &dependent)
                    .unwrap_or(ready.len());
                ready.insert(insert_at, dependent.clone());
            }
        }
    }

    if ordered.len() != export_tables.len() {
        return Err(AppError::Repository(
            "backup catalog dependencies contain a cycle".into(),
        ));
    }

    Ok(ordered)
}

async fn export_table_part(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    tables_dir: &Path,
) -> AppResult<u64> {
    let order_by = table_row_order_clause(conn, table).await?;
    let sql = if order_by.is_empty() {
        format!("SELECT * FROM {}", quote_identifier(table))
    } else {
        format!(
            "SELECT * FROM {} ORDER BY {}",
            quote_identifier(table),
            order_by
        )
    };

    let output_path = tables_dir.join(format!("{table}.ndjson.zst"));
    let file = File::create(&output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create table export {}: {error}",
            output_path.display()
        ))
    })?;
    let encoder = zstd::Encoder::new(file, 3).map_err(|error| {
        AppError::Repository(format!("failed to start zstd encoder for {table}: {error}"))
    })?;
    let mut writer = BufWriter::new(encoder.auto_finish());

    let mut count = 0_u64;
    let mut offset = 0_i64;
    let paged_sql = format!("{sql} LIMIT ? OFFSET ?");
    loop {
        let rows = sqlx::query(&paged_sql)
            .bind(EXPORT_BATCH_SIZE)
            .bind(offset)
            .fetch_all(&mut *conn)
            .await
            .map_err(|error| {
                AppError::Repository(format!("failed to export table {table}: {error}"))
            })?;

        if rows.is_empty() {
            break;
        }

        let row_count = rows.len() as i64;
        for row in rows {
            let value = encode_row(&row)?;
            serde_json::to_writer(&mut writer, &value).map_err(|error| {
                AppError::Repository(format!("failed to encode backup row for {table}: {error}"))
            })?;
            writer.write_all(b"\n").map_err(|error| {
                AppError::Repository(format!("failed to write backup row for {table}: {error}"))
            })?;
            count += 1;
        }
        offset += row_count;
    }

    writer.flush().map_err(|error| {
        AppError::Repository(format!("failed to flush table export for {table}: {error}"))
    })?;
    Ok(count)
}

async fn table_row_order_clause(
    executor: &mut sqlx::SqliteConnection,
    table: &str,
) -> AppResult<String> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(&pragma)
        .fetch_all(&mut *executor)
        .await
        .map_err(|error| {
            AppError::Repository(format!("failed to inspect table info for {table}: {error}"))
        })?;

    let mut pk_columns = rows
        .iter()
        .filter_map(|row| {
            let pk: i64 = row.try_get("pk").ok()?;
            let name: String = row.try_get("name").ok()?;
            (pk > 0).then_some((pk, name))
        })
        .collect::<Vec<_>>();
    pk_columns.sort_by_key(|(pk, _)| *pk);

    if !pk_columns.is_empty() {
        return Ok(pk_columns
            .into_iter()
            .map(|(_, column)| quote_identifier(&column))
            .collect::<Vec<_>>()
            .join(", "));
    }

    if rows
        .iter()
        .any(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("id"))
    {
        return Ok(quote_identifier("id"));
    }

    Ok("rowid".to_string())
}

fn encode_row(row: &SqliteRow) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index).map_err(|error| {
            AppError::Repository(format!(
                "failed to read backup column {} from row: {error}",
                column.name()
            ))
        })?;

        let value = if raw.is_null() {
            JsonValue::Null
        } else {
            match raw.type_info().name() {
                "INTEGER" => JsonValue::from(row.try_get::<i64, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode integer column {}: {error}",
                        column.name()
                    ))
                })?),
                "REAL" => {
                    let value = row.try_get::<f64, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode real column {}: {error}",
                            column.name()
                        ))
                    })?;
                    JsonValue::from(value)
                }
                "BLOB" => {
                    encode_blob_value(&row.try_get::<Vec<u8>, _>(index).map_err(|error| {
                        AppError::Repository(format!(
                            "failed to decode blob column {}: {error}",
                            column.name()
                        ))
                    })?)
                }
                _ => JsonValue::String(row.try_get::<String, _>(index).map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode text column {}: {error}",
                        column.name()
                    ))
                })?),
            }
        };

        object.insert(column.name().to_string(), value);
    }

    Ok(JsonValue::Object(object))
}

fn encode_blob_value(bytes: &[u8]) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert(
        BLOB_MARKER_TYPE.to_string(),
        JsonValue::String("blob".to_string()),
    );
    object.insert(
        BLOB_MARKER_BASE64.to_string(),
        JsonValue::String(STANDARD.encode(bytes)),
    );
    JsonValue::Object(object)
}

async fn import_table_part(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
    part_path: &Path,
) -> AppResult<()> {
    let target_columns = table_columns(conn, table).await?;

    let file = File::open(part_path).map_err(|error| {
        AppError::Validation(format!("backup table payload missing for {table}: {error}"))
    })?;
    let decoder = zstd::Decoder::new(BufReader::new(file)).map_err(|error| {
        AppError::Validation(format!(
            "backup table payload for {table} is invalid: {error}"
        ))
    })?;
    let reader = BufReader::new(decoder);

    for (line_number, line) in reader.lines().enumerate() {
        let line = line.map_err(|error| {
            AppError::Validation(format!(
                "failed to read backup row {table}:{line_number}: {error}"
            ))
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let value: JsonValue = serde_json::from_str(&line).map_err(|error| {
            AppError::Validation(format!(
                "invalid backup row for {table}:{line_number}: {error}"
            ))
        })?;
        let object = value.as_object().ok_or_else(|| {
            AppError::Validation(format!(
                "backup row for {table}:{line_number} is not an object"
            ))
        })?;

        let columns = target_columns
            .iter()
            .filter(|column| object.contains_key(*column))
            .cloned()
            .collect::<Vec<_>>();
        if columns.is_empty() {
            continue;
        }

        let insert_sql = format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote_identifier(table),
            columns
                .iter()
                .map(|column| quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", "),
            std::iter::repeat_n("?", columns.len())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let mut query = sqlx::query(&insert_sql);
        for column in &columns {
            let value = object.get(column).unwrap_or(&JsonValue::Null);
            query = bind_json_value(query, value)?;
        }
        query.execute(&mut **conn).await.map_err(|error| {
            AppError::Validation(format!(
                "failed to import backup row for {table}:{line_number}: {error}"
            ))
        })?;
    }

    Ok(())
}

async fn table_columns(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    table: &str,
) -> AppResult<Vec<String>> {
    let pragma = format!("PRAGMA table_info({})", quote_identifier(table));
    let rows = sqlx::query(&pragma)
        .fetch_all(&mut **conn)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect table columns for {table}: {error}"
            ))
        })?;
    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect())
}

fn bind_json_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>> {
    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(if *value { 1_i64 } else { 0_i64 }),
        JsonValue::Number(value) => {
            if let Some(value) = value.as_i64() {
                query.bind(value)
            } else if let Some(value) = value.as_u64() {
                let value = i64::try_from(value).map_err(|_| {
                    AppError::Validation(
                        "backup row contains an integer outside SQLite i64 range".into(),
                    )
                })?;
                query.bind(value)
            } else if let Some(value) = value.as_f64() {
                query.bind(value)
            } else {
                return Err(AppError::Validation(
                    "backup row contains an unsupported numeric value".into(),
                ));
            }
        }
        JsonValue::String(value) => query.bind(value.clone()),
        JsonValue::Object(object)
            if object.get(BLOB_MARKER_TYPE).and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get(BLOB_MARKER_BASE64)
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::Validation("backup blob payload is missing base64 bytes".into())
                })?;
            let bytes = STANDARD.decode(encoded).map_err(|error| {
                AppError::Validation(format!("backup blob payload is invalid base64: {error}"))
            })?;
            query.bind(bytes)
        }
        JsonValue::Array(_) | JsonValue::Object(_) => query.bind(value.to_string()),
    })
}

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}
