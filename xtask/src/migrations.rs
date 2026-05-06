use anyhow::{Context, Result, anyhow, bail};
use sqlx::{Row, TypeInfo, ValueRef, sqlite::SqlitePoolOptions};
use std::collections::HashSet;

use crate::{RebaselineArgs, TaskContext};

const CANONICAL_ADMIN_USER_ID: &str = "00000000000000000000000000000001";

pub(crate) fn run_rebaseline(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("failed to create tokio runtime")?;
    runtime.block_on(async move { run_rebaseline_inner(ctx, args).await })
}

async fn run_rebaseline_inner(ctx: &TaskContext, args: RebaselineArgs) -> Result<()> {
    if args.through <= 0 {
        bail!("--through must be a positive migration version");
    }

    scryer_infrastructure::register_spellfix_auto_extension()
        .map_err(|error| anyhow!(error.to_string()))?;

    let db_root = ctx.path("crates/scryer/src/db");
    let baseline_relative = format!("baselines/{:04}_baseline.sql", args.through);
    let baseline_path = db_root.join(&baseline_relative);
    if baseline_path.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            baseline_path.display()
        );
    }

    let source_bundle = scryer_infrastructure::migrations::load_source_migration_catalog()
        .map_err(|error| anyhow!(error.to_string()))?;
    if source_bundle.catalog.find_migration(args.through).is_none() {
        bail!(
            "migration {:04} does not exist in the source catalog",
            args.through
        );
    }

    let reference_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open reference in-memory sqlite database")?;
    scryer_infrastructure::migrations::replay_catalog_into_fresh_db(
        &reference_pool,
        &source_bundle.catalog,
        &source_bundle.payload_bytes,
        Some(args.through),
        false,
    )
    .await
    .map_err(|error| anyhow!(error.to_string()))?;
    let reference_dump = canonical_database_dump(&reference_pool).await?;

    if let Some(parent) = baseline_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(&baseline_path, reference_dump.as_bytes())
        .with_context(|| format!("failed to write {}", baseline_path.display()))?;

    let mut manifest = scryer_infrastructure::migration_assets::load_source_manifest(&db_root)
        .map_err(|error| anyhow!(error))?;
    manifest
        .baselines
        .retain(|entry| entry.through_version != args.through);
    manifest.baselines.push(
        scryer_infrastructure::migration_assets::SourceBaselineEntry {
            through_version: args.through,
            file: baseline_relative,
        },
    );
    manifest
        .baselines
        .sort_by_key(|entry| entry.through_version);
    scryer_infrastructure::migration_assets::write_source_manifest(&db_root, &manifest)
        .map_err(|error| anyhow!(error))?;

    let updated_bundle = scryer_infrastructure::migrations::load_source_migration_catalog()
        .map_err(|error| anyhow!(error.to_string()))?;

    let reference_head_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open reference full-replay in-memory sqlite database")?;
    scryer_infrastructure::migrations::replay_catalog_into_fresh_db(
        &reference_head_pool,
        &source_bundle.catalog,
        &source_bundle.payload_bytes,
        None,
        false,
    )
    .await
    .map_err(|error| anyhow!(error.to_string()))?;
    let reference_head_dump = canonical_database_dump(&reference_head_pool).await?;

    let verification_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("failed to open verification in-memory sqlite database")?;
    scryer_infrastructure::migrations::replay_catalog_into_fresh_db(
        &verification_pool,
        &updated_bundle.catalog,
        &updated_bundle.payload_bytes,
        None,
        true,
    )
    .await
    .map_err(|error| anyhow!(error.to_string()))?;
    let verification_dump = canonical_database_dump(&verification_pool).await?;

    if reference_head_dump != verification_dump {
        let debug_dir = ctx.path("tmp/rebaseline-debug");
        std::fs::create_dir_all(&debug_dir)
            .with_context(|| format!("failed to create {}", debug_dir.display()))?;
        let reference_path = debug_dir.join(format!("{:04}_reference_head.sql", args.through));
        let verification_path =
            debug_dir.join(format!("{:04}_verification_head.sql", args.through));
        std::fs::write(&reference_path, reference_head_dump.as_bytes()).with_context(|| {
            format!(
                "failed to write debug reference dump {}",
                reference_path.display()
            )
        })?;
        std::fs::write(&verification_path, verification_dump.as_bytes()).with_context(|| {
            format!(
                "failed to write debug verification dump {}",
                verification_path.display()
            )
        })?;
        bail!(
            "baseline replay verification failed for version {:04}; wrote {} and {}",
            args.through,
            reference_path.display(),
            verification_path.display()
        );
    }

    println!(
        "wrote {} and updated migration manifest through {:04}",
        baseline_path.display(),
        args.through
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct TableColumn {
    cid: i64,
    name: String,
    pk: i64,
}

#[derive(Debug, Default, Clone)]
struct DumpNormalization {
    admin_user_id: Option<String>,
}

async fn canonical_database_dump(pool: &sqlx::SqlitePool) -> Result<String> {
    let mut out = String::new();
    let virtual_tables = virtual_table_names(pool).await?;
    let normalization = build_dump_normalization(pool).await?;
    let schema_rows = sqlx::query(
        "SELECT type, name, sql
           FROM sqlite_master
          WHERE sql IS NOT NULL
            AND name NOT LIKE 'sqlite_%'
            AND name NOT LIKE '_sqlx_%'
          ORDER BY CASE type
              WHEN 'table' THEN 1
              WHEN 'index' THEN 2
              WHEN 'trigger' THEN 3
              WHEN 'view' THEN 4
              ELSE 5
          END, name",
    )
    .fetch_all(pool)
    .await
    .context("failed to query sqlite_master")?;

    for row in schema_rows {
        let name: String = row.try_get("name")?;
        if is_virtual_shadow_table(&virtual_tables, &name) {
            continue;
        }
        let sql: String = row.try_get("sql")?;
        out.push_str(sql.trim());
        out.push_str(";\n");
    }

    let tables = sqlx::query_scalar::<_, String>(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
            AND sql IS NOT NULL
            AND name NOT LIKE 'sqlite_%'
            AND name NOT LIKE '_sqlx_%'
          ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate tables for baseline dump")?;

    for table in tables {
        if is_virtual_shadow_table(&virtual_tables, &table) {
            continue;
        }
        let columns = table_columns(pool, &table).await?;
        if columns.is_empty() {
            continue;
        }

        let select_sql = format!(
            "SELECT * FROM {}{}",
            quote_ident(&table),
            build_order_clause(&columns)
        );
        let rows = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .with_context(|| format!("failed to dump rows from {table}"))?;

        if rows.is_empty() {
            continue;
        }

        let column_sql = columns
            .iter()
            .map(|column| quote_ident(&column.name))
            .collect::<Vec<_>>()
            .join(", ");

        for row in rows {
            let mut values = Vec::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                values.push(sql_literal(
                    &row,
                    index,
                    &table,
                    &column.name,
                    &normalization,
                )?);
            }
            out.push_str(&format!(
                "INSERT INTO {} ({column_sql}) VALUES ({});\n",
                quote_ident(&table),
                values.join(", ")
            ));
        }
    }

    Ok(out)
}

async fn build_dump_normalization(pool: &sqlx::SqlitePool) -> Result<DumpNormalization> {
    if !sqlite_table_exists(pool, "users").await? {
        return Ok(DumpNormalization::default());
    }

    let admin_user_id = sqlx::query_scalar::<_, String>(
        "SELECT id
           FROM users
          WHERE username = 'admin'
          ORDER BY id
          LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("failed to load admin user id for baseline normalization")?;

    Ok(DumpNormalization { admin_user_id })
}

async fn sqlite_table_exists(pool: &sqlx::SqlitePool, table_name: &str) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE type = 'table'
            AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .with_context(|| format!("failed to probe sqlite table {table_name}"))?;

    Ok(count > 0)
}

async fn virtual_table_names(pool: &sqlx::SqlitePool) -> Result<HashSet<String>> {
    let names = sqlx::query_scalar::<_, String>(
        "SELECT name
           FROM sqlite_master
          WHERE type = 'table'
            AND sql LIKE 'CREATE VIRTUAL TABLE %'
          ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("failed to enumerate sqlite virtual tables")?;

    Ok(names.into_iter().collect())
}

fn is_virtual_shadow_table(virtual_tables: &HashSet<String>, table_name: &str) -> bool {
    virtual_tables.iter().any(|virtual_table| {
        table_name != virtual_table
            && table_name
                .strip_prefix(virtual_table)
                .is_some_and(|suffix| suffix.starts_with('_'))
    })
}

async fn table_columns(pool: &sqlx::SqlitePool, table: &str) -> Result<Vec<TableColumn>> {
    let pragma_sql = format!("PRAGMA table_info({})", quote_sql_string(table));
    let rows = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to load table info for {table}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(TableColumn {
            cid: row.try_get("cid")?,
            name: row.try_get("name")?,
            pk: row.try_get("pk")?,
        });
    }

    out.sort_by_key(|column| column.cid);
    Ok(out)
}

fn build_order_clause(columns: &[TableColumn]) -> String {
    let mut ordered = columns
        .iter()
        .filter(|column| column.pk > 0)
        .collect::<Vec<_>>();
    if !ordered.is_empty() {
        ordered.sort_by_key(|column| column.pk);
    } else {
        ordered = columns.iter().collect();
    }

    let clause = ordered
        .into_iter()
        .map(|column| quote_ident(&column.name))
        .collect::<Vec<_>>()
        .join(", ");
    if clause.is_empty() {
        String::new()
    } else {
        format!(" ORDER BY {clause}")
    }
}

fn sql_literal(
    row: &sqlx::sqlite::SqliteRow,
    index: usize,
    table: &str,
    column: &str,
    normalization: &DumpNormalization,
) -> Result<String> {
    let raw = row.try_get_raw(index)?;
    if raw.is_null() {
        return Ok("NULL".to_string());
    }

    match raw.type_info().name() {
        "INTEGER" | "BOOLEAN" => Ok(row.try_get::<i64, _>(index)?.to_string()),
        "REAL" => {
            let value = row.try_get::<f64, _>(index)?;
            if !value.is_finite() {
                bail!("non-finite REAL values are not supported in baseline dumps");
            }
            Ok(value.to_string())
        }
        "BLOB" => {
            let value = row.try_get::<Vec<u8>, _>(index)?;
            Ok(format!(
                "X'{}'",
                value
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            ))
        }
        _ => {
            let value = row.try_get::<String, _>(index)?;
            Ok(quote_sql_string(&normalize_dump_text_value(
                table,
                column,
                &value,
                normalization,
            )))
        }
    }
}

fn normalize_dump_text_value(
    table: &str,
    column: &str,
    value: &str,
    normalization: &DumpNormalization,
) -> String {
    let Some(admin_user_id) = normalization.admin_user_id.as_deref() else {
        return value.to_string();
    };

    if value == admin_user_id
        && ((table == "users" && column == "id") || column.ends_with("user_id"))
    {
        CANONICAL_ADMIN_USER_ID.to_string()
    } else {
        value.to_string()
    }
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
