use scryer_application::{AppError, AppResult};
use sqlx::Row;

pub(crate) async fn converge_post_0_16_6_prerelease_schema_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    sqlite_add_column_if_missing(
        tx,
        "discovery_sync_state",
        "inflight_context_snapshot_run_id",
        "TEXT",
    )
    .await?;
    sqlite_add_column_if_missing(tx, "discovery_sync_state", "lease_owner_id", "TEXT").await?;
    sqlite_add_column_if_missing(tx, "discovery_sync_state", "lease_expires_at", "TEXT").await?;
    sqlite_add_column_if_missing(
        tx,
        "discovery_sync_state",
        "transient_failure_count",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    sqlite_add_column_if_missing(tx, "titles", "catalog_sort_key", "TEXT NOT NULL DEFAULT ''")
        .await?;
    sqlite_add_column_if_missing(tx, "titles", "popularity", "REAL").await?;

    sqlite_converge_external_import_monitor_snapshot_chunks(tx).await?;
    sqlite_drop_column_if_exists(tx, "discovery_titles", "rating").await?;
    sqlite_drop_column_if_exists(tx, "discovery_titles", "canonical_subject_id").await?;
    sqlite_drop_table_if_exists(tx, "discovery_title_ratings").await?;
    sqlite_drop_table_if_exists(tx, "title_external_ratings").await?;
    sqlite_drop_table_if_exists(tx, "title_rating_sources").await?;
    sqlite_drop_table_if_exists(tx, "title_rating_summaries").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_external_ratings").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_rating_sources").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_rating_summaries").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_tag_source_keys").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_tag_sources").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_tags").await?;
    sqlite_drop_table_if_exists(tx, "canonical_media_subjects").await?;
    Ok(())
}

pub(crate) async fn converge_post_0_16_6_prerelease_schema_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    for sql in [
        "ALTER TABLE IF EXISTS discovery_sync_state ADD COLUMN IF NOT EXISTS inflight_context_snapshot_run_id TEXT",
        "ALTER TABLE IF EXISTS discovery_sync_state ADD COLUMN IF NOT EXISTS lease_owner_id TEXT",
        "ALTER TABLE IF EXISTS discovery_sync_state ADD COLUMN IF NOT EXISTS lease_expires_at TIMESTAMPTZ",
        "ALTER TABLE IF EXISTS discovery_sync_state ADD COLUMN IF NOT EXISTS transient_failure_count BIGINT NOT NULL DEFAULT 0",
        "ALTER TABLE IF EXISTS titles ADD COLUMN IF NOT EXISTS catalog_sort_key TEXT NOT NULL DEFAULT ''",
        "ALTER TABLE IF EXISTS titles ADD COLUMN IF NOT EXISTS popularity DOUBLE PRECISION",
        "ALTER TABLE IF EXISTS discovery_titles DROP COLUMN IF EXISTS rating",
        "ALTER TABLE IF EXISTS discovery_titles DROP COLUMN IF EXISTS canonical_subject_id",
        "DROP TABLE IF EXISTS discovery_title_ratings",
        "DROP TABLE IF EXISTS title_external_ratings",
        "DROP TABLE IF EXISTS title_rating_sources",
        "DROP TABLE IF EXISTS title_rating_summaries",
        "DROP TABLE IF EXISTS canonical_media_external_ratings",
        "DROP TABLE IF EXISTS canonical_media_rating_sources",
        "DROP TABLE IF EXISTS canonical_media_rating_summaries",
        "DROP TABLE IF EXISTS canonical_media_tag_source_keys",
        "DROP TABLE IF EXISTS canonical_media_tag_sources",
        "DROP TABLE IF EXISTS canonical_media_tags",
        "DROP TABLE IF EXISTS canonical_media_subjects",
    ] {
        sqlx::raw_sql(sql)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }

    sqlx::raw_sql(
        r#"
DO $$
DECLARE
    existing_primary_key TEXT;
BEGIN
    IF to_regclass('external_import_monitor_snapshot_chunks') IS NOT NULL THEN
        ALTER TABLE external_import_monitor_snapshot_chunks
            ADD COLUMN IF NOT EXISTS session_id TEXT NOT NULL DEFAULT 'external-import-monitor-apply';
        ALTER TABLE external_import_monitor_snapshot_chunks
            ALTER COLUMN session_id DROP DEFAULT;
        ALTER TABLE external_import_monitor_snapshot_chunks
            ALTER COLUMN created_at DROP DEFAULT;
        SELECT conname
          INTO existing_primary_key
          FROM pg_constraint
         WHERE conrelid = 'external_import_monitor_snapshot_chunks'::regclass
           AND contype = 'p'
         LIMIT 1;
        IF existing_primary_key IS NOT NULL THEN
            EXECUTE format(
                'ALTER TABLE external_import_monitor_snapshot_chunks DROP CONSTRAINT %I',
                existing_primary_key
            );
        END IF;
        ALTER TABLE external_import_monitor_snapshot_chunks
            ADD PRIMARY KEY (session_id, facet, entry_kind, chunk_index);
    END IF;
END $$;
        "#,
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;

    Ok(())
}

async fn sqlite_add_column_if_missing(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    column: &str,
    definition: &str,
) -> AppResult<()> {
    if !sqlite_table_exists(tx, table).await? || sqlite_column_exists(tx, table, column).await? {
        return Ok(());
    }
    let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
    sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_drop_column_if_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    column: &str,
) -> AppResult<()> {
    if !sqlite_table_exists(tx, table).await? || !sqlite_column_exists(tx, table, column).await? {
        return Ok(());
    }
    let sql = format!("ALTER TABLE {table} DROP COLUMN {column}");
    sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_drop_table_if_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
) -> AppResult<()> {
    let sql = format!("DROP TABLE IF EXISTS {table}");
    sqlx::query(sqlx::AssertSqlSafe(&*sql))
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_converge_external_import_monitor_snapshot_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    if !sqlite_table_exists(tx, "external_import_monitor_snapshot_chunks").await?
        || sqlite_column_exists(tx, "external_import_monitor_snapshot_chunks", "session_id").await?
    {
        return Ok(());
    }

    sqlx::query("DROP TABLE IF EXISTS external_import_monitor_snapshot_chunks_old_0154")
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    sqlx::query(
        "ALTER TABLE external_import_monitor_snapshot_chunks
         RENAME TO external_import_monitor_snapshot_chunks_old_0154",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    sqlx::query(
        "CREATE TABLE external_import_monitor_snapshot_chunks (
            session_id TEXT NOT NULL,
            facet TEXT NOT NULL CHECK (facet IN ('movie', 'series', 'anime')),
            entry_kind TEXT NOT NULL CHECK (entry_kind IN ('movie', 'series')),
            chunk_index INTEGER NOT NULL,
            payload_ndjson TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (session_id, facet, entry_kind, chunk_index)
        )",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    sqlx::query(
        "INSERT INTO external_import_monitor_snapshot_chunks (
            session_id, facet, entry_kind, chunk_index, payload_ndjson, created_at
         )
         SELECT
            'external-import-monitor-apply',
            facet,
            entry_kind,
            chunk_index,
            payload_ndjson,
            created_at
           FROM external_import_monitor_snapshot_chunks_old_0154
          WHERE facet IN ('movie', 'series', 'anime')",
    )
    .execute(&mut **tx)
    .await
    .map_err(repo_err)?;
    sqlx::query("DROP TABLE external_import_monitor_snapshot_chunks_old_0154")
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(())
}

async fn sqlite_table_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
) -> AppResult<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM sqlite_master
          WHERE type = 'table'
            AND name = ?1",
    )
    .bind(table)
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    Ok(count > 0)
}

async fn sqlite_column_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    column: &str,
) -> AppResult<bool> {
    let sql = format!(
        "SELECT COUNT(*)
           FROM pragma_table_info('{}')
          WHERE name = ?1",
        table.replace('\'', "''")
    );
    let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(&*sql))
        .bind(column)
        .fetch_one(&mut **tx)
        .await
        .map_err(repo_err)?;
    Ok(count > 0)
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
