use super::*;

#[tokio::test]
async fn migration_validate_mode_rejects_pending_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    assert!(
        result.is_err(),
        "validate mode should reject unapplied migrations"
    );
    let err = match result {
        Ok(_) => panic!("validate mode should reject unapplied migrations"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("pending migration"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_validate_mode_does_not_mutate_legacy_sqlx_ledger() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_legacy_ledger_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    sqlx::query("ALTER TABLE _sqlx_migrations RENAME TO _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("legacy ledger rename should succeed");
    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration ledger should be created");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         SELECT version, description, installed_on, success, checksum, execution_time
           FROM _sqlx_migrations_current
          WHERE version <= 102",
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration rows should be copied");
    sqlx::query("DROP TABLE _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("temporary migration ledger should be dropped");

    drop(services);

    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    let err = match result {
        Ok(_) => panic!("validate mode should reject missing migration 0103"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("0103_custom_migrator_runtime_cutover"),
        "validate mode should report the pending custom migration, got {err:?}"
    );

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    let applied_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration row count should load");
    assert_eq!(applied_rows, 102);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_rejects_unknown_or_newer_schema_history() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_compat_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let too_new_key = "999999_too_new";
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(Vec::<u8>::new())
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("tamper first migration checksum");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
        (version, description, installed_on, success, checksum, execution_time)
        VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(999999i64)
    .bind(too_new_key)
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await
    .expect("insert new migration");

    let result = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::Apply).await;
    assert!(result.is_err());
    let err = match result {
        Ok(_) => panic!("bad migration history should fail compatibility check"),
        Err(err) => err,
    };

    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(message.contains("migrations newer than supported"));
    assert!(message.contains("Please update scryer"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_status_listing_reads_legacy_ledger_without_mutating_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_status_legacy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&pool)
    .await
    .expect("legacy migration ledger should be created");

    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         VALUES (1, 'init', CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(vec![1u8, 2, 3])
    .execute(&pool)
    .await
    .expect("legacy migration row should be inserted");

    let statuses = crate::migrations::list_applied_migrations(&pool)
        .await
        .expect("status listing should succeed");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].migration_checksum_algo, "inferred");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[test]
fn compile_source_bundle_rejects_unknown_rust_hook_ids() {
    let db_root = std::env::temp_dir().join(format!(
        "scryer_migration_hook_fixture_{}",
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(db_root.join("migrations")).expect("fixture migrations dir");
    std::fs::write(
        db_root.join("migrations/0001_initial.sql"),
        "CREATE TABLE example (id INTEGER PRIMARY KEY);\n",
    )
    .expect("write legacy migration");
    std::fs::write(
        db_root.join("migration_manifest.toml"),
        r#"
format_version = 1

[legacy_sql]
path = "migrations"
through_version = 1

[[migration]]
version = 2
description = "bad hook"
checksum_algo = "blake3"
steps = [
  { kind = "rust", hook_id = "missing_hook", engine = "all", scope = "all" },
]
"#,
    )
    .expect("write manifest");

    let error = crate::migration_assets::compile_source_bundle(&db_root)
        .expect_err("unknown hook id should fail manifest compilation");
    assert!(error.contains("unknown migration hook id 'missing_hook'"));

    let _ = std::fs::remove_dir_all(db_root);
}

#[tokio::test]
async fn specials_convergence_migration_repoints_legacy_season_zero_references() {
    let db = std::env::temp_dir().join(format!(
        "scryer_specials_convergence_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS title_history (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            episode_id TEXT,
            collection_id TEXT,
            event_type TEXT NOT NULL,
            source_title TEXT,
            quality TEXT,
            download_id TEXT,
            data_json TEXT,
            occurred_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
        )",
    )
    .execute(&pool)
    .await
    .expect("create legacy title_history compatibility table");

    for statement in [
        "CREATE TABLE IF NOT EXISTS releases (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS workflow_operations (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS download_submissions (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("create legacy compatibility table");
    }

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO titles (
            id, name, name_normalized, library_id, facet, monitored, status,
            tags, external_ids, root_folder_id, created_at
         )
         VALUES (?, ?, ?, ?, ?, 1, 'active', '[]', '[]', ?, ?)",
    )
    .bind("title-series")
    .bind("Legacy Series")
    .bind("legacy series")
    .bind(scryer_domain::default_library_id_for_facet(
        &scryer_domain::MediaFacet::Series,
    ))
    .bind("series")
    .bind(scryer_domain::root_folder_id_for_path("/data/series"))
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert title");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-specials")
    .bind("title-series")
    .bind("season")
    .bind("0")
    .bind("Season 0")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert legacy specials");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("canonical-specials")
    .bind("title-series")
    .bind("specials")
    .bind("0")
    .bind("Specials")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert canonical specials");

    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("episode-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("special")
    .bind("1")
    .bind("0")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy episode");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-legacy")
    .bind("title-series")
    .bind("episode")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("legacy-specials")
    .execute(&pool)
    .await
    .expect("insert legacy wanted item");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-canonical")
    .bind("title-series")
    .bind("episode")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("canonical-specials")
    .execute(&pool)
    .await
    .expect("insert canonical wanted item");

    sqlx::query(
        "INSERT INTO title_history
         (id, title_id, collection_id, event_type, occurred_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("history-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("imported")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy title history row");

    let migration_sql =
        include_str!("../../../scryer/src/db/migrations/0070_specials_collection_convergence.sql");
    for statement in migration_sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("run migration statement");
    }

    let collections: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, collection_type FROM collections WHERE title_id = ? ORDER BY id",
    )
    .bind("title-series")
    .fetch_all(&pool)
    .await
    .expect("load collections");
    assert_eq!(
        collections,
        vec![("canonical-specials".to_string(), "specials".to_string())]
    );

    let episode_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM episodes WHERE id = ?")
            .bind("episode-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated episode collection");
    assert_eq!(episode_collection, "canonical-specials");

    let wanted_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM wanted_items WHERE collection_id = ? ORDER BY id")
            .bind("canonical-specials")
            .fetch_all(&pool)
            .await
            .expect("load wanted items");
    assert_eq!(wanted_ids, vec!["wanted-canonical".to_string()]);

    let history_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM title_history WHERE id = ?")
            .bind("history-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated title history collection");
    assert_eq!(history_collection, "canonical-specials");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_apply_then_validate_is_idempotent() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_then_apply_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    drop(services);

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("applied DB should pass validate mode");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0140_rollup_creates_scheduler_tables_and_rss_gap_columns() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0140_scheduler_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    for table in [
        "upstream_scheduler_states",
        "upstream_destination_cooldowns",
        "upstream_scheduler_rss_cadence",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name = ?",
        )
        .bind(table)
        .fetch_one(&services.pool)
        .await
        .expect("sqlite_master query should succeed");
        assert_eq!(exists, 1, "{table} should exist after migrations apply");
    }

    let rss_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('upstream_scheduler_rss_cadence')")
            .fetch_all(&services.pool)
            .await
            .expect("rss cadence columns should load");
    for column in [
        "host_key",
        "account_quota_key",
        "destination_key",
        "rss_request_key",
        "target_interval_seconds",
        "latest_safe_poll_at",
        "last_seen_release_identity",
        "last_seen_release_published_at",
        "last_feed_gap_start_at",
        "last_feed_gap_end_at",
    ] {
        assert!(
            rss_columns.iter().any(|name| name == column),
            "upstream_scheduler_rss_cadence should include {column}; columns were {rss_columns:?}"
        );
    }

    let destination_columns: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('upstream_destination_cooldowns')")
            .fetch_all(&services.pool)
            .await
            .expect("destination cooldown columns should load");
    for column in [
        "destination_key",
        "cooldown_until",
        "retry_after_seconds",
        "source",
        "observed_at",
    ] {
        assert!(
            destination_columns.iter().any(|name| name == column),
            "upstream_destination_cooldowns should include {column}; columns were {destination_columns:?}"
        );
    }

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0140_uses_canonical_rating_storage_only() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0140_canonical_ratings_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    for table in [
        "canonical_media_rating_summaries",
        "canonical_media_rating_sources",
        "canonical_media_external_ratings",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name = ?",
        )
        .bind(table)
        .fetch_one(&services.pool)
        .await
        .expect("canonical rating table lookup should succeed");
        assert_eq!(exists, 1, "{table} should exist after migrations apply");
    }

    for table in [
        "discovery_title_ratings",
        "title_rating_summaries",
        "title_rating_sources",
        "title_external_ratings",
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
               FROM sqlite_master
              WHERE type = 'table'
                AND name = ?",
        )
        .bind(table)
        .fetch_one(&services.pool)
        .await
        .expect("legacy rating table lookup should succeed");
        assert_eq!(exists, 0, "{table} should not exist after migrations apply");
    }

    let discovery_rating_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('discovery_titles')
          WHERE name = 'rating'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("discovery title columns should load");
    assert_eq!(discovery_rating_columns, 0);

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[test]
fn migration_0140_sqlite_and_postgres_rollup_sources_include_scheduler_columns() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("crate should live under repo/crates");
    let sqlite = std::fs::read_to_string(
        repo_root.join("crates/scryer/src/db/migrations/0140_0_17_release_rollup.sql"),
    )
    .expect("sqlite 0140 rollup migration should load");
    let postgres =
        std::fs::read_to_string(repo_root.join(
            "crates/scryer/src/db/postgres/migrations/0140_0_17_release_rollup.sql",
        ))
        .expect("postgres 0140 rollup migration should load");

    for sql in [&sqlite, &postgres] {
        for required in [
            "CREATE TABLE IF NOT EXISTS upstream_scheduler_states",
            "CREATE TABLE IF NOT EXISTS upstream_destination_cooldowns",
            "CREATE TABLE IF NOT EXISTS upstream_scheduler_rss_cadence",
            "CREATE TABLE IF NOT EXISTS user_ui_settings",
            "CREATE TABLE IF NOT EXISTS user_ui_table_columns",
            "quota_observed_at",
            "quota_probe_after",
            "quota_reset_at",
            "retry_after_seconds",
            "rss_request_key",
            "host_key",
            "last_seen_release_identity",
            "last_seen_release_published_at",
            "last_feed_gap_start_at",
            "last_feed_gap_end_at",
        ] {
            assert!(
                sql.contains(required),
                "0140 rollup migration source should include {required}"
            );
        }
    }
}

#[tokio::test]
async fn migration_0079_faceted_projection_allows_cross_facet_duplicates_and_seeds_only_tvdb_titles()
 {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-1")
    .bind("Series")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-1")
    .bind("Movie")
    .bind("movie")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-imdb")
    .bind("IMDb Only")
    .bind("movie")
    .bind(r#"[{"source":"imdb","value":"tt1234567"}]"#)
    .execute(&pool)
    .await
    .expect("insert legacy titles");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"),
    )
    .await;

    let faceted_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT title_id, facet, external_id
         FROM title_external_ids
         WHERE source = 'tvdb'
         ORDER BY facet, title_id",
    )
    .fetch_all(&pool)
    .await
    .expect("load projected faceted tvdb ids");
    assert_eq!(
        faceted_rows,
        vec![
            (
                "movie-1".to_string(),
                "movie".to_string(),
                "123".to_string()
            ),
            (
                "series-1".to_string(),
                "series".to_string(),
                "123".to_string()
            ),
        ]
    );

    let due_now: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("load hydration due markers");
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-imdb")
            .expect("imdb title marker")
            .1
            .is_none()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-1")
            .expect("movie tvdb marker")
            .1
            .is_some()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "series-1")
            .expect("series tvdb marker")
            .1
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_same_facet_duplicate_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_duplicate_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on same-facet duplicate")
    };
    assert!(
        err.to_string().contains("UNIQUE"),
        "expected uniqueness failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let legacy_external_id: String =
        sqlx::query_scalar("SELECT external_id FROM title_external_ids WHERE id = 'legacy-row'")
            .fetch_one(&pool)
            .await
            .expect("legacy row should remain");
    assert_eq!(legacy_external_id, "legacy");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_conflict_hint_lists_colliding_title_ids() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_conflict_hint_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let hint = crate::migrations::title_external_id_projection_conflict_hint(&pool)
        .await
        .expect("conflict hint should be present");
    assert!(hint.contains("series/tvdb/999"));
    assert!(hint.contains("series-a"));
    assert!(hint.contains("series-b"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_invalid_projection_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_invalid_json_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind("series-bad")
    .bind("Broken Series")
    .bind("series")
    .bind("{not-valid-json")
    .execute(&pool)
    .await
    .expect("insert malformed legacy title");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on malformed external_ids json")
    };
    assert!(
        err.to_string().contains("malformed"),
        "expected malformed json failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0104_accepts_plain_path_settings_without_choking_on_unrelated_invalid_json() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE settings_definitions (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            scope TEXT NOT NULL,
            key_name TEXT NOT NULL,
            data_type TEXT NOT NULL,
            default_value_json TEXT,
            is_sensitive INTEGER NOT NULL DEFAULT 0,
            validation_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_definitions should create");

    sqlx::query(
        "CREATE TABLE settings_values (
            id TEXT PRIMARY KEY,
            setting_definition_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_id TEXT,
            value_json TEXT NOT NULL,
            source TEXT NOT NULL,
            updated_by_user_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_values should create");

    sqlx::query(
        "CREATE TEMP TABLE _default_library_roots (
            library_id TEXT NOT NULL,
            path TEXT NOT NULL,
            is_default INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("_default_library_roots should create");

    for (id, key_name) in [
        ("def-movies-path", "movies.path"),
        ("def-series-path", "series.path"),
        ("def-unrelated", "service:system:smg.client_key"),
    ] {
        sqlx::query(
            "INSERT INTO settings_definitions (
                id, category, scope, key_name, data_type, default_value_json,
                is_sensitive, validation_json, created_at, updated_at
            ) VALUES (?, 'test', 'system', ?, 'string', '\"\"', 0, NULL, 'now', 'now')",
        )
        .bind(id)
        .bind(key_name)
        .execute(&pool)
        .await
        .expect("setting definition should insert");
    }

    sqlx::query(
        "INSERT INTO settings_values (
            id, setting_definition_id, scope, scope_id, value_json, source,
            updated_by_user_id, created_at, updated_at
        ) VALUES
            ('row-movies', 'def-movies-path', 'media', NULL, '\"/Volumes/Media/Movies\"', 'test', NULL, 'now', 'now'),
            ('row-series', 'def-series-path', 'media', NULL, '/Volumes/Media/TV', 'test', NULL, 'now', 'now'),
            ('row-unrelated', 'def-unrelated', 'system', NULL, 'enc:v1:not-json', 'test', NULL, 'now', 'now')",
    )
    .execute(&pool)
    .await
    .expect("setting values should insert");

    let migration_sql = include_str!(
        "../../../scryer/src/db/migrations/0104_first_class_libraries_and_permissions.sql"
    );
    let statement = migration_sql
        .split(';')
        .map(str::trim)
        .find(|statement| statement.starts_with("INSERT INTO _default_library_roots (library_id, path, is_default)\nSELECT\n    CASE sd.key_name\n        WHEN 'movies.path'"))
        .expect("0104 path backfill statement should exist");

    sqlx::query(statement)
        .execute(&pool)
        .await
        .expect("legacy plain path values should backfill without malformed json errors");

    let roots: Vec<(String, String)> =
        sqlx::query_as("SELECT library_id, path FROM _default_library_roots ORDER BY library_id")
            .fetch_all(&pool)
            .await
            .expect("backfilled roots should load");
    assert_eq!(
        roots,
        vec![
            (
                "movie_default_library".to_string(),
                "/Volumes/Media/Movies".to_string()
            ),
            (
                "series_default_library".to_string(),
                "/Volumes/Media/TV".to_string()
            ),
        ]
    );
}

async fn create_0136_test_pool() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE titles (
            id TEXT PRIMARY KEY,
            library_id TEXT,
            facet TEXT NOT NULL,
            tags TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("titles should create");
    sqlx::query(
        "CREATE TABLE libraries (
            id TEXT PRIMARY KEY,
            facet TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("libraries should create");
    sqlx::query(
        "CREATE TABLE library_roots (
            id TEXT PRIMARY KEY,
            library_id TEXT NOT NULL,
            path TEXT NOT NULL,
            normalized_path TEXT,
            is_default INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00Z',
            updated_at TEXT NOT NULL DEFAULT '2026-01-01T00:00:00Z'
        )",
    )
    .execute(&pool)
    .await
    .expect("library_roots should create");
    pool
}

async fn run_0136_sqlite(pool: &sqlx::SqlitePool) -> Result<(), AppError> {
    run_embedded_migration(
        pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;

    let mut tx = pool
        .begin()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
    crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx).await?;
    tx.commit()
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;

    sqlx::raw_sql(include_str!(
        "../../../scryer/src/db/migrations/0136_title_root_folder_id_post.sql"
    ))
    .execute(pool)
    .await
    .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(())
}

#[tokio::test]
async fn migration_0136_rekeys_roots_and_backfills_concrete_title_root_ids() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");

    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('random-default-id', 'anime-library', '/Library/Default', '/library/default', 1),
            ('random-custom-id', 'anime-library', '/Library/Custom', '/library/custom', 0)",
    )
    .execute(&pool)
    .await
    .expect("library roots should insert");
    sqlx::query(
        "INSERT INTO titles (id, library_id, facet, tags)
         VALUES
            ('title-default', 'anime-library', 'anime', '[\"keep-default\"]'),
            ('title-custom', 'anime-library', 'anime', '[\"scryer:root-folder:/Library/Custom/\",\"keep-custom\"]'),
            ('title-unmatched', 'anime-library', 'anime', '[\"scryer:root-folder:/Library/Missing\",\"keep-unmatched\"]')",
    )
    .execute(&pool)
    .await
    .expect("titles should insert");

    run_0136_sqlite(&pool)
        .await
        .expect("0136 migration should run");

    let root_rows: Vec<(String, String, String, i64)> = sqlx::query_as(
        "SELECT id, path, normalized_path, is_default
           FROM library_roots
          ORDER BY path",
    )
    .fetch_all(&pool)
    .await
    .expect("migrated roots should query");
    let root_ids_by_path = root_rows
        .iter()
        .map(|(id, path, _, _)| (path.clone(), id.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(
        root_ids_by_path["/Library/Default"],
        scryer_domain::root_folder_id_for_path("/Library/Default")
    );
    assert_eq!(
        root_ids_by_path["/Library/Custom"],
        scryer_domain::root_folder_id_for_path("/Library/Custom")
    );
    assert_eq!(
        root_ids_by_path["/Library/Missing"],
        scryer_domain::root_folder_id_for_path("/Library/Missing")
    );
    assert!(
        root_rows
            .iter()
            .all(|(id, _, _, _)| id != "random-default-id" && id != "random-custom-id")
    );

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT id, root_folder_id, tags
           FROM titles
          ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("migrated titles should query");

    assert_eq!(rows[0].0, "title-custom");
    assert_eq!(rows[0].1, root_ids_by_path["/Library/Custom"]);
    let custom_tags: Vec<String> =
        serde_json::from_str(&rows[0].2).expect("custom tags should decode");
    assert_eq!(custom_tags, vec!["keep-custom".to_string()]);

    assert_eq!(rows[1].0, "title-default");
    assert_eq!(rows[1].1, root_ids_by_path["/Library/Default"]);
    let default_tags: Vec<String> =
        serde_json::from_str(&rows[1].2).expect("default tags should decode");
    assert_eq!(default_tags, vec!["keep-default".to_string()]);

    assert_eq!(rows[2].0, "title-unmatched");
    assert_eq!(rows[2].1, root_ids_by_path["/Library/Missing"]);
    let unmatched_tags: Vec<String> =
        serde_json::from_str(&rows[2].2).expect("unmatched tags should decode");
    assert_eq!(unmatched_tags, vec!["keep-unmatched".to_string()]);

    let orphan_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE root_folder_id IS NULL
             OR NOT EXISTS (
                SELECT 1 FROM library_roots
                 WHERE library_roots.id = titles.root_folder_id
                   AND library_roots.library_id = titles.library_id
             )",
    )
    .fetch_one(&pool)
    .await
    .expect("orphan count should query");
    assert_eq!(orphan_count, 0);
}

#[tokio::test]
async fn migration_0136_rejects_legacy_root_path_from_another_library() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('movie-root', 'movie-library', '/shared/root', '/shared/root', 1)",
    )
    .execute(&pool)
    .await
    .expect("movie root should insert");
    sqlx::query(
        "INSERT INTO titles (id, library_id, facet, tags)
         VALUES
            ('title-cross-root', 'anime-library', 'anime', '[\"scryer:root-folder:/shared/root\"]')",
    )
    .execute(&pool)
    .await
    .expect("title should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;
    let mut tx = pool.begin().await.expect("transaction should begin");
    let err =
        crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx)
            .await
            .expect_err("cross-library legacy root should fail");
    assert!(
        err.to_string()
            .contains("configured on library movie-library"),
        "unexpected migration error: {err}"
    );
}

#[tokio::test]
async fn migration_0136_rejects_duplicate_existing_root_paths_before_rekey() {
    let pool = create_0136_test_pool().await;

    sqlx::query(
        "INSERT INTO libraries (id, facet)
         VALUES
            ('anime-library', 'anime'),
            ('movie-library', 'movie')",
    )
    .execute(&pool)
    .await
    .expect("libraries should insert");
    sqlx::query(
        "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default)
         VALUES
            ('anime-root', 'anime-library', '/shared/root', '/shared/root', 1),
            ('movie-root', 'movie-library', '/shared/root/', '/shared/root', 1)",
    )
    .execute(&pool)
    .await
    .expect("roots should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../../scryer/src/db/migrations/0136_title_root_folder_id_pre.sql"),
    )
    .await;
    let mut tx = pool.begin().await.expect("transaction should begin");
    let err =
        crate::migrations::title_root_folder_ids::migrate_title_root_folder_ids_sqlite(&mut tx)
            .await
            .expect_err("duplicate root paths should fail before rekey");
    assert!(
        err.to_string()
            .contains("duplicate root paths must be merged before migration"),
        "unexpected migration error: {err}"
    );
}
