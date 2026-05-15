use anyhow::{Context, Result, anyhow, bail};
use sqlx::{Row, TypeInfo, ValueRef, postgres::PgPoolOptions, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use xtask_support::{TaskContext, require_command, run_capture, run_status};

use crate::RebaselineArgs;

const CANONICAL_ADMIN_USER_ID: &str = "00000000000000000000000000000001";
const CANONICAL_TIMESTAMP: &str = "1970-01-01T00:00:00Z";

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
    let sqlite_baseline_relative = baseline_relative(args.through, BaselineEngine::Sqlite);
    let sqlite_baseline_path = db_root.join(&sqlite_baseline_relative);
    let postgres_baseline_relative = baseline_relative(args.through, BaselineEngine::Postgres);
    let postgres_baseline_path = db_root.join(&postgres_baseline_relative);

    let mut manifest = scryer_infrastructure::migration_assets::load_source_manifest(&db_root)
        .map_err(|error| anyhow!(error))?;
    let sqlite_entry_present =
        manifest_has_baseline_entry(&manifest, args.through, BaselineEngine::Sqlite);
    let postgres_entry_present =
        manifest_has_baseline_entry(&manifest, args.through, BaselineEngine::Postgres);

    let should_write_sqlite = !sqlite_baseline_path.exists();
    let should_write_postgres = !postgres_baseline_path.exists();
    let sqlite_changed = should_write_sqlite || !sqlite_entry_present;
    let postgres_changed = should_write_postgres || !postgres_entry_present;
    if !sqlite_changed && !postgres_changed {
        bail!(
            "SQLite and PostgreSQL baselines through {:04} already exist and are already registered",
            args.through
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
    if postgres_changed
        && source_bundle
            .catalog
            .latest_baseline_at_or_below(
                args.through,
                scryer_infrastructure::migration_assets::EngineScope::Postgres,
            )
            .is_none()
    {
        bail!(
            "cannot generate PostgreSQL baseline through {:04}: no PostgreSQL baseline exists at or below that version",
            args.through
        );
    }

    let mut generated_paths = Vec::new();
    if should_write_sqlite {
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
        write_baseline_file(&sqlite_baseline_path, &reference_dump)?;
        generated_paths.push(sqlite_baseline_path.clone());
    }

    let docker = if postgres_changed {
        Some(DockerPostgresContainer::start(ctx, args.through)?)
    } else {
        None
    };

    if should_write_postgres {
        let container = docker
            .as_ref()
            .expect("PostgreSQL container is present when PostgreSQL work is required");
        let target_db = format!("rebaseline_target_{}", unique_token(args.through));
        let target_pool = container.create_database_pool(&target_db).await?;
        scryer_infrastructure::postgres::replay_source_catalog_for_fresh_install(
            &target_pool,
            Some(args.through),
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let postgres_dump = container.schema_dump(&target_db)?;
        write_baseline_file(&postgres_baseline_path, &postgres_dump)?;
        generated_paths.push(postgres_baseline_path.clone());
    }

    let mut manifest_changed = false;
    manifest_changed |= upsert_baseline_entry(
        &mut manifest,
        args.through,
        &sqlite_baseline_relative,
        BaselineEngine::Sqlite,
    );
    manifest_changed |= upsert_baseline_entry(
        &mut manifest,
        args.through,
        &postgres_baseline_relative,
        BaselineEngine::Postgres,
    );
    if manifest_changed {
        scryer_infrastructure::migration_assets::write_source_manifest(&db_root, &manifest)
            .map_err(|error| anyhow!(error))?;
    }

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

    if postgres_changed {
        let container = docker
            .as_ref()
            .expect("PostgreSQL container is present when PostgreSQL work is required");
        let reference_db = format!("rebaseline_reference_{}", unique_token(args.through));
        let reference_pool = container.create_database_pool(&reference_db).await?;
        scryer_infrastructure::postgres::replay_catalog_into_fresh_db(
            &reference_pool,
            &source_bundle.catalog,
            &source_bundle.payload_bytes,
            None,
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let reference_dump = container.schema_dump(&reference_db)?;

        let verification_db = format!("rebaseline_verification_{}", unique_token(args.through));
        let verification_pool = container.create_database_pool(&verification_db).await?;
        scryer_infrastructure::postgres::replay_catalog_into_fresh_db(
            &verification_pool,
            &updated_bundle.catalog,
            &updated_bundle.payload_bytes,
            None,
        )
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        let verification_dump = container.schema_dump(&verification_db)?;

        if reference_dump != verification_dump {
            let debug_dir = ctx.path("tmp/rebaseline-debug");
            std::fs::create_dir_all(&debug_dir)
                .with_context(|| format!("failed to create {}", debug_dir.display()))?;
            let reference_path =
                debug_dir.join(format!("{:04}_postgres_reference_head.sql", args.through));
            let verification_path = debug_dir.join(format!(
                "{:04}_postgres_verification_head.sql",
                args.through
            ));
            std::fs::write(&reference_path, reference_dump.as_bytes()).with_context(|| {
                format!(
                    "failed to write debug PostgreSQL reference dump {}",
                    reference_path.display()
                )
            })?;
            std::fs::write(&verification_path, verification_dump.as_bytes()).with_context(
                || {
                    format!(
                        "failed to write debug PostgreSQL verification dump {}",
                        verification_path.display()
                    )
                },
            )?;
            bail!(
                "PostgreSQL baseline replay verification failed for version {:04}; wrote {} and {}",
                args.through,
                reference_path.display(),
                verification_path.display()
            );
        }
    }

    println!(
        "updated baseline sources through {:04}: {}",
        args.through,
        generated_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaselineEngine {
    Sqlite,
    Postgres,
}

impl BaselineEngine {
    fn scope(self) -> scryer_infrastructure::migration_assets::EngineScope {
        match self {
            Self::Sqlite => scryer_infrastructure::migration_assets::EngineScope::Sqlite,
            Self::Postgres => scryer_infrastructure::migration_assets::EngineScope::Postgres,
        }
    }

    fn relative_path(self, through_version: i64) -> String {
        match self {
            Self::Sqlite => format!("baselines/{through_version:04}_baseline.sql"),
            Self::Postgres => {
                format!("postgres/baselines/{through_version:04}_baseline.sql")
            }
        }
    }
}

fn baseline_relative(through_version: i64, engine: BaselineEngine) -> String {
    engine.relative_path(through_version)
}

fn manifest_has_baseline_entry(
    manifest: &scryer_infrastructure::migration_assets::SourceMigrationManifest,
    through_version: i64,
    engine: BaselineEngine,
) -> bool {
    manifest
        .baselines
        .iter()
        .any(|entry| entry.through_version == through_version && entry.engine == engine.scope())
}

fn upsert_baseline_entry(
    manifest: &mut scryer_infrastructure::migration_assets::SourceMigrationManifest,
    through_version: i64,
    file: &str,
    engine: BaselineEngine,
) -> bool {
    let desired_engine = engine.scope();
    let desired_file = file.to_string();
    if let Some(entry) = manifest
        .baselines
        .iter_mut()
        .find(|entry| entry.through_version == through_version && entry.engine == desired_engine)
    {
        if entry.file == desired_file {
            return false;
        }
        entry.file = desired_file;
    } else {
        manifest.baselines.push(
            scryer_infrastructure::migration_assets::SourceBaselineEntry {
                through_version,
                file: desired_file,
                engine: desired_engine,
            },
        );
    }

    manifest.baselines.sort_by(|left, right| {
        baseline_sort_key(left.through_version, left.engine, &left.file).cmp(&baseline_sort_key(
            right.through_version,
            right.engine,
            &right.file,
        ))
    });
    true
}

fn baseline_sort_key(
    through_version: i64,
    engine: scryer_infrastructure::migration_assets::EngineScope,
    file: &str,
) -> (i64, u8, &str) {
    (through_version, engine_sort_key(engine), file)
}

fn engine_sort_key(engine: scryer_infrastructure::migration_assets::EngineScope) -> u8 {
    match engine {
        scryer_infrastructure::migration_assets::EngineScope::All => 0,
        scryer_infrastructure::migration_assets::EngineScope::Sqlite => 1,
        scryer_infrastructure::migration_assets::EngineScope::Postgres => 2,
    }
}

fn write_baseline_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

struct DockerPostgresContainer {
    name: String,
    port: u16,
}

impl DockerPostgresContainer {
    fn start(ctx: &TaskContext, through_version: i64) -> Result<Self> {
        require_command("docker")?;

        let port = reserve_local_port()?;
        let name = format!("scryer-rebaseline-pg-{}", unique_token(through_version));
        let mut command = ctx.command("docker");
        command.args([
            "run",
            "-d",
            "--rm",
            "--name",
            &name,
            "-e",
            "POSTGRES_PASSWORD=postgres",
            "-p",
            &format!("127.0.0.1:{port}:5432"),
            "postgres:17-alpine",
        ]);
        run_capture(&mut command).with_context(|| {
            format!(
                "failed to start Docker PostgreSQL container for {:04}",
                through_version
            )
        })?;

        let container = Self { name, port };
        container.wait_until_ready(ctx)?;
        Ok(container)
    }

    fn database_url(&self, database: &str) -> String {
        format!(
            "postgres://postgres:postgres@127.0.0.1:{}/{}",
            self.port, database
        )
    }

    fn admin_database_url(&self) -> String {
        self.database_url("postgres")
    }

    fn wait_until_ready(&self, ctx: &TaskContext) -> Result<()> {
        for _ in 0..40 {
            let mut command = ctx.command("docker");
            command.args(["exec", &self.name, "pg_isready", "-U", "postgres"]);
            match run_status(&mut command) {
                Ok(status) if status.success() => return Ok(()),
                Ok(_) | Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }

        bail!(
            "Docker PostgreSQL container {} did not become ready on port {}",
            self.name,
            self.port
        );
    }

    async fn create_database_pool(&self, database: &str) -> Result<sqlx::PgPool> {
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.admin_database_url())
            .await
            .with_context(|| {
                format!(
                    "failed to connect to Docker PostgreSQL admin database at {}",
                    self.admin_database_url()
                )
            })?;
        sqlx::query(&format!("CREATE DATABASE {}", quote_pg_ident(database)))
            .execute(&admin_pool)
            .await
            .with_context(|| format!("failed to create Docker PostgreSQL database {database}"))?;
        admin_pool.close().await;

        PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.database_url(database))
            .await
            .with_context(|| {
                format!(
                    "failed to connect to Docker PostgreSQL database {} at {}",
                    database,
                    self.database_url(database)
                )
            })
    }

    fn schema_dump(&self, database: &str) -> Result<String> {
        let mut command = Command::new("docker");
        command.args([
            "exec",
            &self.name,
            "pg_dump",
            "--schema-only",
            "--no-owner",
            "--no-privileges",
            "--schema=public",
            "--exclude-table=_sqlx_migrations",
            "-U",
            "postgres",
            "-d",
            database,
        ]);
        let dump = run_capture(&mut command)
            .with_context(|| format!("failed to dump PostgreSQL schema for database {database}"))?;
        Ok(normalize_postgres_schema_dump(&dump))
    }
}

impl Drop for DockerPostgresContainer {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .output();
    }
}

fn reserve_local_port() -> Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .context("failed to reserve a local port for Docker PostgreSQL")?;
    let port = listener
        .local_addr()
        .context("failed to read reserved Docker PostgreSQL port")?
        .port();
    drop(listener);
    Ok(port)
}

fn unique_token(through_version: i64) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{through_version:04}-{}-{millis:x}", std::process::id())
}

fn normalize_postgres_schema_dump(raw: &str) -> String {
    let mut out = String::new();
    let mut previous_blank = true;

    for line in raw.lines() {
        let trimmed = line.trim();
        if should_skip_postgres_dump_line(trimmed) {
            continue;
        }

        let normalized = line.replace("public.", "");
        let normalized = normalized.trim_end();
        if normalized.trim().is_empty() {
            if !previous_blank {
                out.push('\n');
                previous_blank = true;
            }
            continue;
        }

        out.push_str(normalized);
        out.push('\n');
        previous_blank = false;
    }

    out
}

fn should_skip_postgres_dump_line(line: &str) -> bool {
    line.is_empty()
        || line.starts_with("--")
        || line.starts_with("\\restrict ")
        || line.starts_with("\\unrestrict ")
        || line.starts_with("SET ")
        || line.starts_with("SELECT pg_catalog.set_config(")
        || line == "CREATE SCHEMA public;"
        || line.starts_with("ALTER SCHEMA public ")
        || line.starts_with("COMMENT ON SCHEMA public ")
}

fn quote_pg_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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
            build_order_clause(&table, &columns)
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

fn build_order_clause(table: &str, columns: &[TableColumn]) -> String {
    if table == "library_roots" {
        return format!(
            " ORDER BY {}, {}",
            quote_ident("library_id"),
            quote_ident("normalized_path")
        );
    }

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
                row,
                table,
                column,
                &value,
                normalization,
            )?))
        }
    }
}

fn normalize_dump_text_value(
    row: &sqlx::sqlite::SqliteRow,
    table: &str,
    column: &str,
    value: &str,
    normalization: &DumpNormalization,
) -> Result<String> {
    if table == "library_roots" && column == "id" {
        let library_id = row
            .try_get::<String, _>("library_id")
            .context("failed to load library_roots.library_id during dump normalization")?;
        return Ok(format!("canonical_root_for_{library_id}"));
    }

    if column.ends_with("_at") && looks_like_utc_timestamp(value) {
        return Ok(CANONICAL_TIMESTAMP.to_string());
    }

    let Some(admin_user_id) = normalization.admin_user_id.as_deref() else {
        return Ok(value.to_string());
    };

    if value == admin_user_id
        && ((table == "users" && column == "id") || column.ends_with("user_id"))
    {
        Ok(CANONICAL_ADMIN_USER_ID.to_string())
    } else {
        Ok(value.to_string())
    }
}

fn looks_like_utc_timestamp(value: &str) -> bool {
    value.len() == 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(13) == Some(&b':')
        && value.as_bytes().get(16) == Some(&b':')
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_infrastructure::migration_assets::{
        EngineScope, LegacySqlBlock, SourceMigrationManifest,
    };
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn normalize_postgres_schema_dump_strips_runtime_noise() {
        let dump = r#"
--
-- PostgreSQL database dump
--

\restrict abc123
SET statement_timeout = 0;
SET search_path = public, pg_catalog;
SELECT pg_catalog.set_config('search_path', '', false);

CREATE SCHEMA public;

CREATE TABLE public.download_jobs (
    id uuid NOT NULL
);

ALTER TABLE ONLY public.download_jobs
    ADD CONSTRAINT download_jobs_pkey PRIMARY KEY (id);

\unrestrict abc123
"#;

        assert_eq!(
            normalize_postgres_schema_dump(dump),
            "CREATE TABLE download_jobs (\n    id uuid NOT NULL\n);\nALTER TABLE ONLY download_jobs\n    ADD CONSTRAINT download_jobs_pkey PRIMARY KEY (id);\n"
        );
    }

    #[test]
    fn upsert_baseline_entry_preserves_other_engine_entries() {
        let mut manifest = SourceMigrationManifest {
            format_version: 1,
            legacy_sql: LegacySqlBlock {
                path: "migrations".to_string(),
                through_version: 100,
            },
            migrations: Vec::new(),
            baselines: vec![
                scryer_infrastructure::migration_assets::SourceBaselineEntry {
                    through_version: 114,
                    file: "baselines/0114_baseline.sql".to_string(),
                    engine: EngineScope::Sqlite,
                },
            ],
        };

        assert!(upsert_baseline_entry(
            &mut manifest,
            114,
            "postgres/baselines/0114_baseline.sql",
            BaselineEngine::Postgres,
        ));
        assert!(manifest_has_baseline_entry(
            &manifest,
            114,
            BaselineEngine::Sqlite
        ));
        assert!(manifest_has_baseline_entry(
            &manifest,
            114,
            BaselineEngine::Postgres
        ));
        assert_eq!(manifest.baselines.len(), 2);
    }

    #[test]
    fn saved_0115_baselines_have_table_column_constraint_and_index_parity() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask-migrations has a repository parent");
        let sqlite = parse_baseline_shape(
            &std::fs::read_to_string(
                repo_root.join("crates/scryer/src/db/baselines/0115_baseline.sql"),
            )
            .expect("read SQLite 0115 baseline"),
        );
        let postgres = parse_baseline_shape(
            &std::fs::read_to_string(
                repo_root.join("crates/scryer/src/db/postgres/baselines/0115_baseline.sql"),
            )
            .expect("read PostgreSQL 0115 baseline"),
        );

        assert_eq!(
            sqlite.tables.keys().collect::<Vec<_>>(),
            postgres.tables.keys().collect::<Vec<_>>()
        );
        assert_eq!(sqlite.indexes, postgres.indexes);

        for (table, sqlite_table) in &sqlite.tables {
            let postgres_table = postgres
                .tables
                .get(table)
                .unwrap_or_else(|| panic!("PostgreSQL missing table {table}"));
            assert_eq!(
                sqlite_table.columns, postgres_table.columns,
                "column set mismatch for table {table}"
            );
            assert_eq!(
                sqlite_table.primary_keys, postgres_table.primary_keys,
                "primary-key mismatch for table {table}"
            );
            assert_eq!(
                sqlite_table.unique_constraints, postgres_table.unique_constraints,
                "unique-constraint mismatch for table {table}"
            );
            assert_eq!(
                sqlite_table.foreign_keys, postgres_table.foreign_keys,
                "foreign-key mismatch for table {table}"
            );
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    struct BaselineShape {
        tables: BTreeMap<String, TableShape>,
        indexes: BTreeMap<String, IndexShape>,
    }

    #[derive(Debug, Default, Eq, PartialEq)]
    struct TableShape {
        columns: BTreeSet<String>,
        primary_keys: BTreeSet<Vec<String>>,
        unique_constraints: BTreeSet<Vec<String>>,
        foreign_keys: BTreeSet<ForeignKeyShape>,
    }

    #[derive(Debug, Eq, PartialEq, Ord, PartialOrd)]
    struct ForeignKeyShape {
        columns: Vec<String>,
        referenced_table: String,
        referenced_columns: Vec<String>,
        on_delete: Option<String>,
    }

    #[derive(Debug, Eq, PartialEq)]
    struct IndexShape {
        unique: bool,
        table: String,
        expressions: Vec<String>,
        predicate: Option<String>,
    }

    fn parse_baseline_shape(sql: &str) -> BaselineShape {
        let mut tables = BTreeMap::new();
        let mut indexes = BTreeMap::new();
        for statement in split_sql_statements(sql) {
            let statement = statement.trim();
            if let Some((table, shape)) = parse_create_table_statement(statement) {
                tables.insert(table, shape);
            } else if let Some((table, constraint)) = parse_alter_table_constraint(statement) {
                let shape = tables.entry(table).or_insert_with(TableShape::default);
                apply_table_constraint(shape, constraint);
            } else if let Some((name, index)) = parse_create_index_statement(statement) {
                indexes.insert(name, index);
            }
        }

        BaselineShape { tables, indexes }
    }

    enum TableConstraint {
        PrimaryKey(Vec<String>),
        Unique(Vec<String>),
        ForeignKey(ForeignKeyShape),
    }

    fn parse_create_table_statement(statement: &str) -> Option<(String, TableShape)> {
        let rest = statement.trim().strip_prefix("CREATE TABLE")?.trim_start();
        let (table, after_name) = parse_table_name(rest);
        let open = after_name.find('(')?;
        let close = matching_close_paren(after_name, open)?;
        let mut shape = TableShape::default();
        parse_table_body(&after_name[open + 1..close], &table, &mut shape);
        Some((table, shape))
    }

    fn parse_table_body(body: &str, table: &str, shape: &mut TableShape) {
        for item in split_table_items(&strip_line_comments(body)) {
            let item = normalize_sql_whitespace(&item);
            if item.is_empty() {
                continue;
            }
            let item = strip_constraint_name(&item);
            let upper = item.to_ascii_uppercase();
            if upper.starts_with("PRIMARY KEY") {
                if let Some(columns) = first_parenthesized_list(&item) {
                    shape.primary_keys.insert(columns);
                }
                continue;
            }
            if upper.starts_with("UNIQUE") {
                if let Some(columns) = first_parenthesized_list(&item) {
                    shape.unique_constraints.insert(columns);
                }
                continue;
            }
            if upper.starts_with("FOREIGN KEY") {
                if let Some(foreign_key) = parse_foreign_key(table, &item, None) {
                    shape.foreign_keys.insert(foreign_key);
                }
                continue;
            }
            if upper.starts_with("CHECK") {
                continue;
            }

            let (column, _) = parse_identifier(&item);
            shape.columns.insert(column.clone());
            if upper.contains("PRIMARY KEY") {
                shape.primary_keys.insert(vec![column.clone()]);
            }
            if upper.contains("UNIQUE") {
                shape.unique_constraints.insert(vec![column.clone()]);
            }
            if upper.contains(" REFERENCES ")
                && let Some(foreign_key) = parse_foreign_key(table, &item, Some(vec![column]))
            {
                shape.foreign_keys.insert(foreign_key);
            }
        }
    }

    fn parse_alter_table_constraint(statement: &str) -> Option<(String, TableConstraint)> {
        let statement = normalize_sql_whitespace(statement);
        let rest = statement.strip_prefix("ALTER TABLE ONLY ")?;
        let (table, rest) = parse_table_name(rest);
        let rest = rest.trim_start().strip_prefix("ADD CONSTRAINT ")?;
        let (_, constraint) = parse_identifier(rest);
        let constraint = strip_constraint_name(&format!("CONSTRAINT ignored {constraint}"));
        let upper = constraint.to_ascii_uppercase();
        if upper.starts_with("PRIMARY KEY") {
            return first_parenthesized_list(&constraint)
                .map(TableConstraint::PrimaryKey)
                .map(|constraint| (table, constraint));
        }
        if upper.starts_with("UNIQUE") {
            return first_parenthesized_list(&constraint)
                .map(TableConstraint::Unique)
                .map(|constraint| (table, constraint));
        }
        if upper.starts_with("FOREIGN KEY") {
            return parse_foreign_key(&table, &constraint, None)
                .map(TableConstraint::ForeignKey)
                .map(|constraint| (table, constraint));
        }
        None
    }

    fn apply_table_constraint(shape: &mut TableShape, constraint: TableConstraint) {
        match constraint {
            TableConstraint::PrimaryKey(columns) => {
                shape.primary_keys.insert(columns);
            }
            TableConstraint::Unique(columns) => {
                shape.unique_constraints.insert(columns);
            }
            TableConstraint::ForeignKey(foreign_key) => {
                shape.foreign_keys.insert(foreign_key);
            }
        }
    }

    fn parse_create_index_statement(statement: &str) -> Option<(String, IndexShape)> {
        let statement = normalize_sql_whitespace(statement);
        let mut rest = statement.strip_prefix("CREATE ")?;
        let unique = rest.starts_with("UNIQUE ");
        if unique {
            rest = rest.strip_prefix("UNIQUE ")?;
        }
        rest = rest.strip_prefix("INDEX ")?;
        let (name, after_name) = parse_identifier(rest);
        let after_on = after_name.trim_start().strip_prefix("ON ")?;
        let (table, mut rest) = parse_identifier(after_on);
        rest = rest.trim_start();
        if let Some(after_using) = rest.strip_prefix("USING ") {
            let (_, after_method) = parse_identifier(after_using);
            rest = after_method.trim_start();
        }
        let open = rest.find('(')?;
        let close = matching_close_paren(rest, open)?;
        let expressions = split_table_items(&rest[open + 1..close])
            .into_iter()
            .map(|expression| normalize_index_expression(&expression))
            .collect::<Vec<_>>();
        let predicate = rest[close + 1..]
            .trim_start()
            .strip_prefix("WHERE ")
            .map(normalize_index_predicate);
        Some((
            name,
            IndexShape {
                unique,
                table,
                expressions,
                predicate,
            },
        ))
    }

    fn parse_table_name(input: &str) -> (String, &str) {
        let input = input.trim_start();
        let input = input.strip_prefix("IF NOT EXISTS ").unwrap_or(input);
        let input = input.strip_prefix("public.").unwrap_or(input);
        let input = input.trim_start_matches('"');
        let name_end = input
            .find(|ch: char| ch == '"' || ch.is_whitespace() || ch == '(')
            .expect("CREATE TABLE contains a table name");
        (
            input[..name_end].to_string(),
            input[name_end..].trim_start_matches('"'),
        )
    }

    fn parse_identifier(input: &str) -> (String, &str) {
        let input = input.trim_start();
        if let Some(input) = input.strip_prefix('"') {
            let end = input.find('"').expect("quoted identifier is closed");
            return (input[..end].to_string(), &input[end + 1..]);
        }
        let input = input.strip_prefix("public.").unwrap_or(input);
        let end = input
            .find(|ch: char| ch.is_whitespace() || ch == '(' || ch == ',')
            .unwrap_or(input.len());
        (input[..end].to_string(), &input[end..])
    }

    fn strip_constraint_name(item: &str) -> String {
        let item = item.trim();
        if !item.to_ascii_uppercase().starts_with("CONSTRAINT ") {
            return item.to_string();
        }
        let rest = item["CONSTRAINT ".len()..].trim_start();
        let (_, rest) = parse_identifier(rest);
        rest.trim_start().to_string()
    }

    fn first_parenthesized_list(input: &str) -> Option<Vec<String>> {
        let open = input.find('(')?;
        let close = matching_close_paren(input, open)?;
        Some(
            split_table_items(&input[open + 1..close])
                .into_iter()
                .map(|column| normalize_identifier_expression(&column))
                .collect(),
        )
    }

    fn parse_foreign_key(
        table: &str,
        item: &str,
        inline_columns: Option<Vec<String>>,
    ) -> Option<ForeignKeyShape> {
        let columns = if let Some(columns) = inline_columns {
            columns
        } else {
            first_parenthesized_list(item)?
        };
        let references_at = item.to_ascii_uppercase().find("REFERENCES ")?;
        let after_references = &item[references_at + "REFERENCES ".len()..];
        let (referenced_table, rest) = parse_identifier(after_references);
        let referenced_columns = first_parenthesized_list(rest)?;
        Some(ForeignKeyShape {
            columns,
            referenced_table,
            referenced_columns,
            on_delete: parse_on_delete(item),
        })
        .filter(|_| !table.is_empty())
    }

    fn parse_on_delete(input: &str) -> Option<String> {
        let upper = input.to_ascii_uppercase();
        let after = upper.split_once("ON DELETE")?.1.trim_start();
        for action in [
            "SET NULL",
            "SET DEFAULT",
            "NO ACTION",
            "CASCADE",
            "RESTRICT",
        ] {
            if after.starts_with(action) {
                return Some(action.to_string());
            }
        }
        None
    }

    fn normalize_identifier_expression(input: &str) -> String {
        input
            .trim()
            .trim_matches('"')
            .strip_prefix("public.")
            .unwrap_or(input.trim().trim_matches('"'))
            .to_string()
    }

    fn normalize_index_expression(input: &str) -> String {
        normalize_expression_text(input)
    }

    fn normalize_index_predicate(input: &str) -> String {
        normalize_expression_text(input)
            .replace("=anyarray[", "in")
            .replace(']', "")
    }

    fn normalize_expression_text(input: &str) -> String {
        let normalized: String = input
            .trim()
            .replace('"', "")
            .replace("public.", "")
            .replace("::text", "")
            .replace("::bigint", "")
            .replace("::integer", "")
            .to_ascii_lowercase()
            .replace("trim(both from ", "trim(")
            .chars()
            .filter(|ch| !ch.is_whitespace() && *ch != '(' && *ch != ')')
            .collect();
        normalized
            .strip_suffix("asc")
            .unwrap_or(&normalized)
            .to_string()
    }

    fn matching_close_paren(input: &str, open: usize) -> Option<usize> {
        let mut depth = 0_i32;
        let mut quote = None;
        for (idx, ch) in input.char_indices().skip_while(|(idx, _)| *idx < open) {
            if let Some(active_quote) = quote {
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(idx);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn normalize_sql_whitespace(input: &str) -> String {
        input.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn split_sql_statements(sql: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut quote = None;
        for (idx, ch) in sql.char_indices() {
            if let Some(active_quote) = quote {
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                ';' => {
                    let statement = sql[start..idx].trim();
                    if !statement.is_empty() {
                        out.push(statement.to_string());
                    }
                    start = idx + 1;
                }
                _ => {}
            }
        }
        let statement = sql[start..].trim();
        if !statement.is_empty() {
            out.push(statement.to_string());
        }
        out
    }

    fn strip_line_comments(input: &str) -> String {
        input
            .lines()
            .map(|line| line.split_once("--").map_or(line, |(prefix, _)| prefix))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn split_table_items(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut depth = 0_i32;
        let mut quote = None;
        for (idx, ch) in body.char_indices() {
            if let Some(active_quote) = quote {
                if ch == active_quote {
                    quote = None;
                }
                continue;
            }
            match ch {
                '\'' | '"' => quote = Some(ch),
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(body[start..idx].to_string());
                    start = idx + 1;
                }
                _ => {}
            }
        }
        out.push(body[start..].to_string());
        out
    }
}
