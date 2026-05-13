use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Map as JsonMap, Value as JsonValue};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Column, Row, TypeInfo, ValueRef};

use scryer_application::{
    AppError, AppResult, BACKUP_TABLE_CATALOG, BackupBundleExportRequest, BackupBundleStaging,
    BackupExportOutcome, BackupRestorePreparedBundle, BackupTableClassification,
    LogicalBackupExporter, prepare_backup_restore_payload,
};
use scryer_domain::Title;

use crate::postgres::PostgresServices;
use crate::queries::title_search::{self, TitleSearchProjectionSource};

const EXPORT_BATCH_SIZE: i64 = 500;

#[derive(Clone)]
pub struct PostgresLogicalBackupExporter {
    pool: PgPool,
}

#[derive(Clone, Debug)]
struct PgColumnInfo {
    name: String,
    udt_name: String,
}

impl PostgresLogicalBackupExporter {
    pub fn new(db: &PostgresServices) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl LogicalBackupExporter for PostgresLogicalBackupExporter {
    async fn export_backup_bundle(
        &self,
        request: BackupBundleExportRequest,
    ) -> AppResult<BackupExportOutcome> {
        export_backup_bundle_from_postgres_pool(&self.pool, request).await
    }
}

pub async fn export_backup_bundle_from_postgres_pool(
    pool: &PgPool,
    request: BackupBundleExportRequest,
) -> AppResult<BackupExportOutcome> {
    let mut staging = BackupBundleStaging::new()?;
    export_backup_tables_from_pool(pool, &mut staging).await?;
    staging.finish(request)
}

async fn export_backup_tables_from_pool(
    pool: &PgPool,
    staging: &mut BackupBundleStaging,
) -> AppResult<()> {
    validate_backup_catalog(pool).await?;
    let export_tables = ordered_export_tables(pool).await?;
    let tables_dir = staging.tables_dir();

    let mut tx = pool.begin().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to begin PostgreSQL backup snapshot: {error}"
        ))
    })?;

    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to configure PostgreSQL backup snapshot: {error}"
            ))
        })?;

    for table in &export_tables {
        let row_count = export_table_part(&mut tx, table, &tables_dir).await?;
        staging.record_table_part(table, row_count)?;
    }

    tx.rollback().await.map_err(|error| {
        AppError::Repository(format!(
            "failed to close PostgreSQL backup snapshot: {error}"
        ))
    })?;
    Ok(())
}

pub async fn restore_backup_bundle_into_postgres_pool(
    pool: &PgPool,
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

    let mut tx = pool.begin().await.map_err(|error| {
        AppError::Repository(format!("failed to begin PostgreSQL restore: {error}"))
    })?;

    for table in export_tables.iter().rev() {
        let sql = format!("DELETE FROM {}", quote_identifier(table));
        sqlx::query(&sql).execute(&mut *tx).await.map_err(|error| {
            AppError::Repository(format!("failed to clear restore table {table}: {error}"))
        })?;
    }

    let tables_dir = payload.tables_dir();
    for table in &export_tables {
        import_table_part(
            &mut tx,
            table,
            &tables_dir.join(format!("{table}.ndjson.zst")),
        )
        .await?;
    }

    rebuild_title_search_projection(&mut tx).await?;
    repair_sequences(&mut tx).await?;

    for (table, expected_rows) in &payload.manifest().row_counts {
        let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
        let actual_rows: i64 =
            sqlx::query_scalar(&sql)
                .fetch_one(&mut *tx)
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

    tx.commit().await.map_err(|error| {
        AppError::Repository(format!("failed to commit PostgreSQL restore: {error}"))
    })?;

    Ok(
        BackupRestorePreparedBundle::from_summary_and_instance_secrets_env(
            payload.summary(),
            payload.instance_secrets_env()?,
        ),
    )
}

async fn validate_backup_catalog(pool: &PgPool) -> AppResult<()> {
    let actual_tables = application_tables(pool).await?;
    let classified = BACKUP_TABLE_CATALOG
        .iter()
        .map(|entry| entry.table.to_string())
        .collect::<BTreeSet<_>>();
    let unclassified = actual_tables
        .into_iter()
        .filter(|table| !classified.contains(table))
        .collect::<Vec<_>>();
    if !unclassified.is_empty() {
        return Err(AppError::Repository(format!(
            "backup catalog is missing classifications for PostgreSQL tables: {}",
            unclassified.join(", ")
        )));
    }
    Ok(())
}

async fn application_tables(pool: &PgPool) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT table_name
           FROM information_schema.tables
          WHERE table_schema = current_schema()
            AND table_type = 'BASE TABLE'
          ORDER BY table_name ASC",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect PostgreSQL schema: {error}"))
    })?;

    Ok(rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("table_name").ok())
        .filter(|table| !is_engine_internal_table(table))
        .collect())
}

fn is_engine_internal_table(table: &str) -> bool {
    table == "_sqlx_migrations" || table == "mediarr_schema_migrations"
}

async fn ordered_export_tables(pool: &PgPool) -> AppResult<Vec<String>> {
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

    let rows = sqlx::query(
        "SELECT tc.table_name AS table_name,
                ccu.table_name AS referenced_table
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
             ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
           JOIN information_schema.constraint_column_usage ccu
             ON ccu.constraint_name = tc.constraint_name
            AND ccu.table_schema = tc.table_schema
          WHERE tc.constraint_type = 'FOREIGN KEY'
            AND tc.table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PostgreSQL foreign keys: {error}"
        ))
    })?;

    for row in rows {
        let table: String = row.try_get("table_name").map_err(repo_err)?;
        let referenced: String = row.try_get("referenced_table").map_err(repo_err)?;
        if !export_tables.contains(&table) || !export_tables.contains(&referenced) {
            continue;
        }
        if outgoing
            .get_mut(&referenced)
            .expect("known table")
            .insert(table.clone())
        {
            *incoming.get_mut(&table).expect("known table") += 1;
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
                ready.insert(insert_at, dependent);
            }
        }
    }

    if ordered.len() != export_tables.len() {
        return Err(AppError::Repository(
            "backup catalog dependencies contain a PostgreSQL cycle".into(),
        ));
    }

    Ok(ordered)
}

async fn export_table_part(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    tables_dir: &Path,
) -> AppResult<u64> {
    let order_by = table_row_order_clause(&mut **tx, table).await?;
    let sql = format!(
        "SELECT * FROM {} ORDER BY {} LIMIT $1 OFFSET $2",
        quote_identifier(table),
        order_by
    );

    let output_path = tables_dir.join(format!("{table}.ndjson.zst"));
    let file = File::create(&output_path).map_err(|error| {
        AppError::Repository(format!(
            "failed to create PostgreSQL table export {}: {error}",
            output_path.display()
        ))
    })?;
    let encoder = zstd::Encoder::new(file, 3).map_err(|error| {
        AppError::Repository(format!("failed to start zstd encoder for {table}: {error}"))
    })?;
    let mut writer = BufWriter::new(encoder.auto_finish());

    let mut count = 0_u64;
    let mut offset = 0_i64;
    loop {
        let rows = sqlx::query(&sql)
            .bind(EXPORT_BATCH_SIZE)
            .bind(offset)
            .fetch_all(&mut **tx)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to export PostgreSQL table {table}: {error}"
                ))
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
    executor: &mut sqlx::PgConnection,
    table: &str,
) -> AppResult<String> {
    let pk_rows = sqlx::query(
        "SELECT kcu.column_name
           FROM information_schema.table_constraints tc
           JOIN information_schema.key_column_usage kcu
             ON tc.constraint_name = kcu.constraint_name
            AND tc.table_schema = kcu.table_schema
          WHERE tc.constraint_type = 'PRIMARY KEY'
            AND tc.table_schema = current_schema()
            AND tc.table_name = $1
          ORDER BY kcu.ordinal_position ASC",
    )
    .bind(table)
    .fetch_all(&mut *executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect primary key for {table}: {error}"
        ))
    })?;

    let pk_columns = pk_rows
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("column_name").ok())
        .collect::<Vec<_>>();
    if !pk_columns.is_empty() {
        return Ok(pk_columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", "));
    }

    let has_id: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
               FROM information_schema.columns
              WHERE table_schema = current_schema()
                AND table_name = $1
                AND column_name = 'id'
         )",
    )
    .bind(table)
    .fetch_one(&mut *executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect id column for {table}: {error}"))
    })?;
    if has_id {
        return Ok(quote_identifier("id"));
    }

    Ok("ctid".to_string())
}

fn encode_row(row: &PgRow) -> AppResult<JsonValue> {
    let mut object = JsonMap::new();
    for (index, column) in row.columns().iter().enumerate() {
        let raw = row.try_get_raw(index).map_err(|error| {
            AppError::Repository(format!(
                "failed to read backup column {} from PostgreSQL row: {error}",
                column.name()
            ))
        })?;
        let value = if raw.is_null() {
            JsonValue::Null
        } else {
            encode_pg_value(row, index, column.name(), raw.type_info().name())?
        };
        object.insert(column.name().to_string(), value);
    }
    Ok(JsonValue::Object(object))
}

fn encode_pg_value(
    row: &PgRow,
    index: usize,
    column: &str,
    type_name: &str,
) -> AppResult<JsonValue> {
    let normalized = type_name.to_ascii_uppercase();
    match normalized.as_str() {
        "BOOL" | "BOOLEAN" => Ok(JsonValue::Bool(row.try_get::<bool, _>(index).map_err(
            |error| AppError::Repository(format!("failed to decode bool column {column}: {error}")),
        )?)),
        "INT2" | "SMALLINT" => Ok(JsonValue::from(row.try_get::<i16, _>(index).map_err(
            |error| {
                AppError::Repository(format!(
                    "failed to decode smallint column {column}: {error}"
                ))
            },
        )?)),
        "INT4" | "INTEGER" => Ok(JsonValue::from(row.try_get::<i32, _>(index).map_err(
            |error| {
                AppError::Repository(format!("failed to decode integer column {column}: {error}"))
            },
        )?)),
        "INT8" | "BIGINT" => Ok(JsonValue::from(row.try_get::<i64, _>(index).map_err(
            |error| {
                AppError::Repository(format!("failed to decode bigint column {column}: {error}"))
            },
        )?)),
        "FLOAT4" | "FLOAT8" | "REAL" | "DOUBLE PRECISION" => Ok(JsonValue::from(
            row.try_get::<f64, _>(index).map_err(|error| {
                AppError::Repository(format!("failed to decode float column {column}: {error}"))
            })?,
        )),
        "BYTEA" => Ok(encode_blob_value(
            &row.try_get::<Vec<u8>, _>(index).map_err(|error| {
                AppError::Repository(format!("failed to decode bytea column {column}: {error}"))
            })?,
        )),
        "JSON" | "JSONB" => row.try_get::<JsonValue, _>(index).map_err(|error| {
            AppError::Repository(format!("failed to decode JSON column {column}: {error}"))
        }),
        "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => Ok(JsonValue::String(
            row.try_get::<chrono::DateTime<chrono::Utc>, _>(index)
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode timestamptz column {column}: {error}"
                    ))
                })?
                .to_rfc3339(),
        )),
        "TIMESTAMP" | "TIMESTAMP WITHOUT TIME ZONE" => Ok(JsonValue::String(
            row.try_get::<chrono::NaiveDateTime, _>(index)
                .map_err(|error| {
                    AppError::Repository(format!(
                        "failed to decode timestamp column {column}: {error}"
                    ))
                })?
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        )),
        _ => row
            .try_get::<String, _>(index)
            .map(JsonValue::String)
            .map_err(|error| {
                AppError::Repository(format!("failed to decode text column {column}: {error}"))
            }),
    }
}

fn encode_blob_value(bytes: &[u8]) -> JsonValue {
    let mut object = JsonMap::new();
    object.insert(
        "__scryer_type".to_string(),
        JsonValue::String("blob".to_string()),
    );
    object.insert(
        "base64".to_string(),
        JsonValue::String(STANDARD.encode(bytes)),
    );
    JsonValue::Object(object)
}

async fn import_table_part(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    part_path: &Path,
) -> AppResult<()> {
    let target_columns = table_columns(&mut **tx, table).await?;
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
        let mut object = value.as_object().cloned().ok_or_else(|| {
            AppError::Validation(format!(
                "backup row for {table}:{line_number} is not an object"
            ))
        })?;
        normalize_import_object(table, &mut object)?;
        let columns = target_columns
            .iter()
            .filter(|column| object.contains_key(&column.name))
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
                .map(|column| quote_identifier(&column.name))
                .collect::<Vec<_>>()
                .join(", "),
            columns
                .iter()
                .enumerate()
                .map(|(index, column)| format!("${}{}", index + 1, pg_cast(column)))
                .collect::<Vec<_>>()
                .join(", ")
        );

        let mut query = sqlx::query(&insert_sql);
        for column in &columns {
            let value = object.get(&column.name).unwrap_or(&JsonValue::Null);
            query = bind_pg_value(query, column, value)?;
        }
        query.execute(&mut **tx).await.map_err(|error| {
            AppError::Validation(format!(
                "failed to import backup row for {table}:{line_number}: {error}"
            ))
        })?;
    }

    Ok(())
}

fn normalize_import_object(table: &str, object: &mut JsonMap<String, JsonValue>) -> AppResult<()> {
    if table == "titles" {
        ensure_title_record_json(object);
    }
    Ok(())
}

fn ensure_title_record_json(object: &mut JsonMap<String, JsonValue>) {
    let has_complete_record = object
        .get("record_json")
        .and_then(JsonValue::as_object)
        .is_some_and(|record| record.contains_key("id"));
    if has_complete_record {
        return;
    }

    let mut record = JsonMap::new();
    for field in [
        "id",
        "name",
        "created_by",
        "created_at",
        "year",
        "overview",
        "poster_url",
        "banner_url",
        "background_url",
        "sort_title",
        "slug",
        "imdb_id",
        "runtime_minutes",
        "content_status",
        "language",
        "first_aired",
        "network",
        "studio",
        "country",
        "metadata_language",
        "metadata_fetched_at",
        "min_availability",
        "digital_release_date",
        "folder_path",
    ] {
        record.insert(
            field.to_string(),
            object.get(field).cloned().unwrap_or(JsonValue::Null),
        );
    }

    record.insert(
        "library_id".to_string(),
        object
            .get("library_id")
            .cloned()
            .unwrap_or_else(|| JsonValue::String(String::new())),
    );
    record.insert(
        "facet".to_string(),
        object
            .get("facet")
            .cloned()
            .unwrap_or_else(|| JsonValue::String("movie".to_string())),
    );
    record.insert(
        "monitored".to_string(),
        sqlite_bool_value(object.get("monitored")).unwrap_or(JsonValue::Bool(true)),
    );
    for (record_field, source_field) in [
        ("tags", "tags"),
        ("external_ids", "external_ids"),
        ("genres", "genres"),
        ("aliases", "aliases"),
        ("tagged_aliases", "tagged_aliases_json"),
    ] {
        record.insert(
            record_field.to_string(),
            object
                .get(source_field)
                .and_then(logical_json_value)
                .unwrap_or_else(|| JsonValue::Array(Vec::new())),
        );
    }
    for field in [
        "poster_source_url",
        "banner_source_url",
        "background_source_url",
    ] {
        record.insert(field.to_string(), JsonValue::Null);
    }

    object.insert("record_json".to_string(), JsonValue::Object(record));
}

fn sqlite_bool_value(value: Option<&JsonValue>) -> Option<JsonValue> {
    match value {
        Some(JsonValue::Bool(value)) => Some(JsonValue::Bool(*value)),
        Some(JsonValue::Number(value)) => value.as_i64().map(|value| JsonValue::Bool(value != 0)),
        Some(JsonValue::String(value)) => match value.as_str() {
            "1" | "true" | "TRUE" => Some(JsonValue::Bool(true)),
            "0" | "false" | "FALSE" => Some(JsonValue::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

async fn table_columns(
    executor: &mut sqlx::PgConnection,
    table: &str,
) -> AppResult<Vec<PgColumnInfo>> {
    let rows = sqlx::query(
        "SELECT column_name, data_type, udt_name
           FROM information_schema.columns
          WHERE table_schema = current_schema()
            AND table_name = $1
          ORDER BY ordinal_position ASC",
    )
    .bind(table)
    .fetch_all(executor)
    .await
    .map_err(|error| {
        AppError::Repository(format!(
            "failed to inspect PostgreSQL table columns for {table}: {error}"
        ))
    })?;
    Ok(rows
        .into_iter()
        .map(|row| {
            Ok(PgColumnInfo {
                name: row.try_get("column_name").map_err(repo_err)?,
                udt_name: row.try_get("udt_name").map_err(repo_err)?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?)
}

fn bind_pg_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    column: &PgColumnInfo,
    value: &JsonValue,
) -> AppResult<sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>> {
    if is_json_column(column) {
        return match logical_json_value(value) {
            Some(value) => Ok(query.bind(sqlx::types::Json(value))),
            None => Ok(query.bind(None::<sqlx::types::Json<JsonValue>>)),
        };
    }
    if is_bytea_column(column) {
        return match blob_bytes(value)? {
            Some(bytes) => Ok(query.bind(bytes)),
            None => Ok(query.bind(None::<Vec<u8>>)),
        };
    }

    Ok(match value {
        JsonValue::Null => query.bind(None::<String>),
        JsonValue::Bool(value) => query.bind(value.to_string()),
        JsonValue::Number(value) => query.bind(value.to_string()),
        JsonValue::String(value) if value.is_empty() && is_timestamp_column(column) => {
            query.bind(None::<String>)
        }
        JsonValue::String(value) => query.bind(value.clone()),
        JsonValue::Array(_) | JsonValue::Object(_) => query.bind(value.to_string()),
    })
}

fn logical_json_value(value: &JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(value) => {
            Some(serde_json::from_str(value).unwrap_or_else(|_| JsonValue::String(value.clone())))
        }
        value => Some(value.clone()),
    }
}

fn blob_bytes(value: &JsonValue) -> AppResult<Option<Vec<u8>>> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Object(object)
            if object.get("__scryer_type").and_then(JsonValue::as_str) == Some("blob") =>
        {
            let encoded = object
                .get("base64")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    AppError::Validation("backup blob payload is missing base64 bytes".into())
                })?;
            STANDARD.decode(encoded).map(Some).map_err(|error| {
                AppError::Validation(format!("backup blob payload is invalid base64: {error}"))
            })
        }
        JsonValue::String(value) => Ok(Some(value.as_bytes().to_vec())),
        _ => Err(AppError::Validation(
            "backup row contains an unsupported PostgreSQL bytea value".into(),
        )),
    }
}

fn pg_cast(column: &PgColumnInfo) -> &'static str {
    match column.udt_name.as_str() {
        "bool" => "::boolean",
        "int2" => "::smallint",
        "int4" => "::integer",
        "int8" => "::bigint",
        "float4" => "::real",
        "float8" => "::double precision",
        "numeric" => "::numeric",
        "timestamp" => "::timestamp",
        "timestamptz" => "::timestamptz",
        "date" => "::date",
        "json" => "::json",
        "jsonb" => "::jsonb",
        _ => "",
    }
}

fn is_json_column(column: &PgColumnInfo) -> bool {
    matches!(column.udt_name.as_str(), "json" | "jsonb")
}

fn is_bytea_column(column: &PgColumnInfo) -> bool {
    column.udt_name == "bytea"
}

fn is_timestamp_column(column: &PgColumnInfo) -> bool {
    matches!(
        column.udt_name.as_str(),
        "timestamp" | "timestamptz" | "date"
    )
}

async fn repair_sequences(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<()> {
    let rows = sqlx::query(
        "SELECT ns.nspname AS sequence_schema,
                seq.relname AS sequence_name,
                tbl.relname AS table_name,
                att.attname AS column_name
           FROM pg_class seq
           JOIN pg_namespace ns ON ns.oid = seq.relnamespace
           JOIN pg_depend dep ON dep.objid = seq.oid AND dep.deptype = 'a'
           JOIN pg_class tbl ON tbl.oid = dep.refobjid
           JOIN pg_attribute att ON att.attrelid = tbl.oid AND att.attnum = dep.refobjsubid
          WHERE seq.relkind = 'S'
            AND ns.nspname = current_schema()",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| {
        AppError::Repository(format!("failed to inspect PostgreSQL sequences: {error}"))
    })?;

    for row in rows {
        let schema: String = row.try_get("sequence_schema").map_err(repo_err)?;
        let sequence: String = row.try_get("sequence_name").map_err(repo_err)?;
        let table: String = row.try_get("table_name").map_err(repo_err)?;
        let column: String = row.try_get("column_name").map_err(repo_err)?;
        let qualified = format!(
            "{}.{}",
            quote_identifier(&schema),
            quote_identifier(&sequence)
        );
        let table_name = quote_identifier(&table);
        let column_name = quote_identifier(&column);
        let sql = format!(
            "SELECT setval(
                $1::regclass,
                GREATEST(COALESCE((SELECT MAX({column_name}) FROM {table_name}), 0), 1),
                COALESCE((SELECT MAX({column_name}) FROM {table_name}), 0) > 0
             )"
        );
        sqlx::query(&sql)
            .bind(qualified)
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to repair PostgreSQL sequence {sequence}: {error}"
                ))
            })?;
    }
    Ok(())
}

async fn rebuild_title_search_projection(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let rows = sqlx::query("SELECT record_json FROM titles ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to read restored titles for PostgreSQL search rebuild: {error}"
            ))
        })?;

    sqlx::query("DELETE FROM title_search_terms")
        .execute(&mut **tx)
        .await
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to clear PostgreSQL title search projection: {error}"
            ))
        })?;

    for row in rows {
        let record_json: JsonValue = row.try_get("record_json").map_err(repo_err)?;
        let title: Title = serde_json::from_value(record_json).map_err(repo_err)?;
        let source = TitleSearchProjectionSource::from(&title);
        for term in title_search::build_title_search_terms(&source) {
            sqlx::query(
                "INSERT INTO title_search_terms
                 (title_id, facet, term_kind, raw_term, normalized_term, weight, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, NOW())
                 ON CONFLICT (title_id, term_kind, normalized_term) DO UPDATE SET
                    raw_term = EXCLUDED.raw_term,
                    weight = EXCLUDED.weight,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(&title.id)
            .bind(title.facet.as_str())
            .bind(term.term_kind)
            .bind(&term.raw_term)
            .bind(&term.normalized_term)
            .bind(term.weight)
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                AppError::Repository(format!(
                    "failed to rebuild PostgreSQL title search projection for {}: {error}",
                    title.id
                ))
            })?;
        }
    }

    Ok(())
}

fn quote_identifier(value: &str) -> String {
    let escaped = value.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
