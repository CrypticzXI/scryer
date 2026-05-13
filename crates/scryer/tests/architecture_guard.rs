use std::fs;
use std::path::{Path, PathBuf};

use scryer_application::{BACKUP_TABLE_CATALOG, BackupTableClassification};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives under crates/scryer")
        .to_path_buf()
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(path) = pending.pop() {
        let entries = fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files
}

fn production_rust_source(path: &Path) -> String {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    match source.find("#[cfg(test)]\nmod tests") {
        Some(index) => source[..index].to_string(),
        None => source,
    }
}

#[test]
fn scryer_runtime_does_not_import_engine_datastore_implementations() {
    let src = repo_root().join("crates/scryer/src");
    let forbidden = [
        "SqliteServices",
        "SqliteCatalogStore",
        "SqliteConfigStore",
        "SqliteSettingsStore",
        "SqliteWorkflowStore",
        "SqliteReleaseStore",
        "SqliteCustomizationStore",
        "SqliteNotificationStore",
        "SqliteLibraryStateStore",
        "PostgresServices",
        "PostgresCatalogStore",
        "PostgresConfigStore",
        "PostgresSettingsStore",
        "PostgresReleaseStore",
        "PostgresCustomizationStore",
        "PostgresLibraryStateStore",
        "PostgresNotificationStore",
        "PostgresWorkflowStore",
        "sqlx::PgPool",
        "sqlx::postgres::PgRow",
    ];

    for path in rust_files_under(&src) {
        let source = production_rust_source(&path);
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} must use the engine-neutral datastore assembly instead of importing {needle}",
                path.display()
            );
        }
    }
}

#[test]
fn application_boundary_stays_sqlite_agnostic() {
    let app_src = repo_root().join("crates/scryer-application/src");
    let forbidden = [
        "sqlx::SqlitePool",
        "sqlx::SqliteRow",
        "sqlx::PgPool",
        "sqlx::PgRow",
        "sqlx::postgres::PgRow",
        "SqlitePoolOptions",
        "SqliteConnectOptions",
        "PgPoolOptions",
        "PgConnectOptions",
        "SCRYER_DB_PATH",
        "SCRYER_DB_URL",
        "vacuum_into",
        "backup_dir_from_db_path",
        "crate::queries::",
    ];

    for path in rust_files_under(&app_src) {
        let source = production_rust_source(&path);
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} leaks SQLite-specific datastore detail `{needle}` across the application boundary",
                path.display()
            );
        }
    }
}

#[test]
fn datastore_assembly_does_not_wire_null_repositories_for_engines() {
    let datastore = repo_root().join("crates/scryer-infrastructure/src/datastore.rs");
    let source = production_rust_source(&datastore);
    let forbidden = [
        "NullAcquisitionStateRepository",
        "NullBlocklistRepository",
        "NullDomainEventRepository",
        "NullDownloadQueueCommandRepository",
        "NullDownloadSubmissionRepository",
        "NullExternalImportMonitorSnapshotRepository",
        "NullHousekeepingRepository",
        "NullImportArtifactRepository",
        "NullImportRepository",
        "NullJobRunRepository",
        "NullLibraryProbeRepository",
        "NullLibraryScanUnmatchedItemRepository",
        "NullMediaFileRepository",
        "NullPendingReleaseRepository",
        "NullSubtitleDownloadRepository",
        "NullTitleImageRepository",
        "NullWantedItemRepository",
        "NullWorkflowOperationRepository",
    ];

    for needle in forbidden {
        assert!(
            !source.contains(needle),
            "datastore assembly must not satisfy engine repository seams with `{needle}`"
        );
    }
}

#[test]
fn postgres_runtime_paths_do_not_ship_unsupported_markers() {
    let postgres_src = repo_root().join("crates/scryer-infrastructure/src/postgres");
    let forbidden = ["not implemented", "unsupported("];

    for path in rust_files_under(&postgres_src) {
        let source = production_rust_source(&path);
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "{} leaves a PostgreSQL runtime path marked as `{needle}`",
                path.display()
            );
        }
    }
}

#[test]
fn postgres_schema_declares_every_logical_backup_export_table() {
    let postgres_db = repo_root().join("crates/scryer/src/db/postgres");
    let mut pending = vec![postgres_db];
    let mut postgres_sql = String::new();
    while let Some(path) = pending.pop() {
        let entries = fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("sql") {
                postgres_sql.push_str(
                    &fs::read_to_string(&path).unwrap_or_else(|error| {
                        panic!("failed to read {}: {error}", path.display())
                    }),
                );
                postgres_sql.push('\n');
            }
        }
    }

    for entry in BACKUP_TABLE_CATALOG
        .iter()
        .filter(|entry| entry.classification == BackupTableClassification::Export)
    {
        let create_if_not_exists = format!("CREATE TABLE IF NOT EXISTS {}", entry.table);
        let create_table = format!("CREATE TABLE {}", entry.table);
        assert!(
            postgres_sql.contains(&create_if_not_exists) || postgres_sql.contains(&create_table),
            "PostgreSQL schema must declare logical backup table `{}`",
            entry.table
        );
    }
}

#[test]
fn settings_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_settings =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/settings_store.rs"));
    let postgres_settings = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/settings_store.rs"),
    );

    assert!(
        sqlite_settings.contains("pub struct SettingsStore<S>"),
        "settings should expose one shared repository kernel"
    );
    assert!(
        sqlite_settings
            .contains("pub type SqliteSettingsStore = SettingsStore<SqliteSettingsSql>;"),
        "SQLite settings should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_settings
            .contains("pub type PostgresSettingsStore = SettingsStore<PostgresSettingsSql>;"),
        "PostgreSQL settings should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteSettingsStore",
        "pub struct PostgresSettingsStore",
        "impl SettingsRepository for SqliteSettingsStore",
        "impl SettingsRepository for PostgresSettingsStore",
    ] {
        assert!(
            !sqlite_settings.contains(forbidden) && !postgres_settings.contains(forbidden),
            "settings repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn config_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_config =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/config_store.rs"));
    let postgres_config = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/config_store.rs"),
    );

    assert!(
        sqlite_config.contains("pub struct ConfigStore<S>"),
        "config should expose one shared repository kernel"
    );
    assert!(
        sqlite_config.contains("pub type SqliteConfigStore = ConfigStore<SqliteConfigSql>;"),
        "SQLite config should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_config.contains("pub type PostgresConfigStore = ConfigStore<PostgresConfigSql>;"),
        "PostgreSQL config should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteConfigStore",
        "pub struct PostgresConfigStore",
        "impl IndexerConfigRepository for SqliteConfigStore",
        "impl IndexerConfigRepository for PostgresConfigStore",
        "impl DownloadClientConfigRepository for SqliteConfigStore",
        "impl DownloadClientConfigRepository for PostgresConfigStore",
        "impl SubtitleProviderConfigRepository for SqliteConfigStore",
        "impl SubtitleProviderConfigRepository for PostgresConfigStore",
    ] {
        assert!(
            !sqlite_config.contains(forbidden) && !postgres_config.contains(forbidden),
            "config repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn notification_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_notification = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/notification_store.rs"),
    );
    let postgres_notification = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/notification_store.rs"),
    );

    assert!(
        sqlite_notification.contains("pub struct NotificationStore<S>"),
        "notifications should expose one shared repository kernel"
    );
    assert!(
        sqlite_notification.contains(
            "pub type SqliteNotificationStore = NotificationStore<SqliteNotificationSql>;"
        ),
        "SQLite notifications should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_notification.contains(
            "pub type PostgresNotificationStore = NotificationStore<PostgresNotificationSql>;"
        ),
        "PostgreSQL notifications should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteNotificationStore",
        "pub struct PostgresNotificationStore",
        "impl NotificationChannelRepository for SqliteNotificationStore",
        "impl NotificationChannelRepository for PostgresNotificationStore",
        "impl NotificationSubscriptionRepository for SqliteNotificationStore",
        "impl NotificationSubscriptionRepository for PostgresNotificationStore",
    ] {
        assert!(
            !sqlite_notification.contains(forbidden) && !postgres_notification.contains(forbidden),
            "notification repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn release_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_release =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/release_store.rs"));
    let postgres_release = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/release_store.rs"),
    );

    assert!(
        sqlite_release.contains("pub struct ReleaseStore<S>"),
        "release attempts should expose one shared repository kernel"
    );
    assert!(
        sqlite_release.contains("pub type SqliteReleaseStore = ReleaseStore<SqliteReleaseSql>;"),
        "SQLite release attempts should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_release
            .contains("pub type PostgresReleaseStore = ReleaseStore<PostgresReleaseSql>;"),
        "PostgreSQL release attempts should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteReleaseStore",
        "pub struct PostgresReleaseStore",
        "impl ReleaseAttemptRepository for SqliteReleaseStore",
        "impl ReleaseAttemptRepository for PostgresReleaseStore",
    ] {
        assert!(
            !sqlite_release.contains(forbidden) && !postgres_release.contains(forbidden),
            "release repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn customization_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_customization = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/customization_store.rs"),
    );
    let postgres_customization = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/customization_store.rs"),
    );

    assert!(
        sqlite_customization.contains("pub struct CustomizationStore<S>"),
        "customization should expose one shared repository kernel"
    );
    assert!(
        sqlite_customization.contains(
            "pub type SqliteCustomizationStore = CustomizationStore<SqliteCustomizationSql>;"
        ),
        "SQLite customization should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_customization.contains(
            "pub type PostgresCustomizationStore = CustomizationStore<PostgresCustomizationSql>;"
        ),
        "PostgreSQL customization should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteCustomizationStore",
        "pub struct PostgresCustomizationStore",
        "impl RuleSetRepository for SqliteCustomizationStore",
        "impl RuleSetRepository for PostgresCustomizationStore",
        "impl PostProcessingScriptRepository for SqliteCustomizationStore",
        "impl PostProcessingScriptRepository for PostgresCustomizationStore",
        "impl PluginInstallationRepository for SqliteCustomizationStore",
        "impl PluginInstallationRepository for PostgresCustomizationStore",
    ] {
        assert!(
            !sqlite_customization.contains(forbidden)
                && !postgres_customization.contains(forbidden),
            "customization repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn workflow_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_workflow =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/workflow_store.rs"));
    let postgres_workflow = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/workflow_store.rs"),
    );

    assert!(
        sqlite_workflow.contains("pub struct WorkflowStore<S>"),
        "workflow should expose one shared repository kernel"
    );
    assert!(
        sqlite_workflow
            .contains("pub type SqliteWorkflowStore = WorkflowStore<SqliteWorkflowSql>;"),
        "SQLite workflow should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_workflow
            .contains("pub type PostgresWorkflowStore = WorkflowStore<PostgresWorkflowSql>;"),
        "PostgreSQL workflow should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteWorkflowStore",
        "pub struct PostgresWorkflowStore",
        "impl AcquisitionStateRepository for SqliteWorkflowStore",
        "impl AcquisitionStateRepository for PostgresWorkflowStore",
        "impl DomainEventRepository for SqliteWorkflowStore",
        "impl DomainEventRepository for PostgresWorkflowStore",
        "impl DownloadSubmissionRepository for SqliteWorkflowStore",
        "impl DownloadSubmissionRepository for PostgresWorkflowStore",
        "impl ImportRepository for SqliteWorkflowStore",
        "impl ImportRepository for PostgresWorkflowStore",
        "impl WorkflowOperationRepository for SqliteWorkflowStore",
        "impl WorkflowOperationRepository for PostgresWorkflowStore",
    ] {
        assert!(
            !sqlite_workflow.contains(forbidden) && !postgres_workflow.contains(forbidden),
            "workflow repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn catalog_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_catalog =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/catalog_store.rs"));
    let postgres_catalog = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/catalog_store.rs"),
    );

    assert!(
        sqlite_catalog.contains("pub struct CatalogStore<S>"),
        "catalog should expose one shared repository kernel"
    );
    assert!(
        sqlite_catalog.contains("pub type SqliteCatalogStore = CatalogStore<SqliteCatalogSql>;"),
        "SQLite catalog should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_catalog
            .contains("pub type PostgresCatalogStore = CatalogStore<PostgresCatalogSql>;"),
        "PostgreSQL catalog should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteCatalogStore",
        "pub struct PostgresCatalogStore",
        "impl TitleRepository for SqliteCatalogStore",
        "impl TitleRepository for PostgresCatalogStore",
        "impl LibraryRepository for SqliteCatalogStore",
        "impl LibraryRepository for PostgresCatalogStore",
        "impl ShowRepository for SqliteCatalogStore",
        "impl ShowRepository for PostgresCatalogStore",
        "impl UserRepository for SqliteCatalogStore",
        "impl UserRepository for PostgresCatalogStore",
    ] {
        assert!(
            !sqlite_catalog.contains(forbidden) && !postgres_catalog.contains(forbidden),
            "catalog repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn library_state_repository_uses_shared_runtime_kernel() {
    let root = repo_root();
    let sqlite_library_state = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/library_state_store.rs"),
    );
    let postgres_library_state = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/library_state_store.rs"),
    );

    assert!(
        sqlite_library_state.contains("pub struct LibraryStateStore<S>"),
        "library state should expose one shared repository kernel"
    );
    assert!(
        sqlite_library_state.contains("pub trait LibraryStateSql:"),
        "library state should keep its engine adapter seam concern-local"
    );
    for forbidden in [
        "pub trait LibraryStateSql:\n    LibraryProbeRepository",
        "pub trait LibraryStateSql:\n    LibraryScanUnmatchedItemRepository",
        "pub trait LibraryStateSql:\n    MediaFileRepository",
        "pub trait LibraryStateSql:\n    WantedItemRepository",
        "pub trait LibraryStateSql:\n    HousekeepingRepository",
        "pub trait LibraryStateSql:\n    PendingReleaseRepository",
        "pub trait LibraryStateSql:\n    BlocklistRepository",
        "pub trait LibraryStateSql:\n    SubtitleDownloadRepository",
        "pub trait LibraryStateSql:\n    TitleImageRepository",
    ] {
        assert!(
            !sqlite_library_state.contains(forbidden),
            "library-state SQL seam must not compose application repository traits directly: `{forbidden}`"
        );
    }
    assert!(
        sqlite_library_state.contains(
            "pub type SqliteLibraryStateStore = LibraryStateStore<SqliteLibraryStateSql>;"
        ),
        "SQLite library state should be a primitive adapter over the shared kernel"
    );
    assert!(
        postgres_library_state.contains(
            "pub type PostgresLibraryStateStore = LibraryStateStore<PostgresLibraryStateSql>;"
        ),
        "PostgreSQL library state should be a primitive adapter over the shared kernel"
    );

    for forbidden in [
        "pub struct SqliteLibraryStateStore",
        "pub struct PostgresLibraryStateStore",
        "impl LibraryProbeRepository for SqliteLibraryStateStore",
        "impl LibraryProbeRepository for PostgresLibraryStateStore",
        "impl MediaFileRepository for SqliteLibraryStateStore",
        "impl MediaFileRepository for PostgresLibraryStateStore",
        "impl WantedItemRepository for SqliteLibraryStateStore",
        "impl WantedItemRepository for PostgresLibraryStateStore",
        "impl PendingReleaseRepository for SqliteLibraryStateStore",
        "impl PendingReleaseRepository for PostgresLibraryStateStore",
        "impl BlocklistRepository for SqliteLibraryStateStore",
        "impl BlocklistRepository for PostgresLibraryStateStore",
        "impl TitleImageRepository for SqliteLibraryStateStore",
        "impl TitleImageRepository for PostgresLibraryStateStore",
    ] {
        assert!(
            !sqlite_library_state.contains(forbidden)
                && !postgres_library_state.contains(forbidden),
            "library-state repository must not reintroduce paired full-store implementation `{forbidden}`"
        );
    }
}

#[test]
fn postgres_catalog_keeps_runtime_parity_for_backfills_and_transactions() {
    let root = repo_root();
    let shared_catalog =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/catalog_store.rs"));
    let postgres_catalog = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/postgres/catalog_store.rs"),
    );

    assert!(
        !shared_catalog.contains(
            "async fn list_anime_title_ids_missing_title_anidb_external_ids(\n        &self,\n        _limit: usize,\n    ) -> AppResult<Vec<String>> {\n        Ok(Vec::new())\n    }"
        ),
        "TitleSql must not provide a default no-op for title-level AniDB backfill"
    );
    assert!(
        postgres_catalog.contains("async fn list_anime_title_ids_missing_title_anidb_external_ids"),
        "PostgreSQL catalog must implement title-level AniDB backfill parity"
    );
    assert!(
        postgres_catalog.contains("LOWER(external_id ->> 'source') IN ('anidb', 'anidb_id')"),
        "PostgreSQL title-level AniDB backfill should match SQLite source normalization"
    );
    assert!(
        postgres_catalog.contains("let mut tx = self.pool.begin().await.map_err(repo_err)?;\n        sqlx::query(\n            \"INSERT INTO libraries"),
        "PostgreSQL library create with roots must run in one transaction"
    );
}

#[test]
fn datastore_bootstrap_wrappers_do_not_use_engine_forwarding_enums() {
    let root = repo_root();
    let datastore =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/datastore.rs"));

    for forbidden in [
        "pub enum DatastoreSettingsStore",
        "pub enum DatastoreCustomizationStore",
        "Self::Sqlite(store) => store.get_setting_json",
        "Self::Postgres(store) => store.get_plugin_installation",
    ] {
        assert!(
            !datastore.contains(forbidden),
            "datastore bootstrap wrappers must not reintroduce engine forwarding enum branch `{forbidden}`"
        );
    }
}

#[test]
fn runtime_sql_sharing_stays_concern_local() {
    let root = repo_root();
    let forbidden_shared_queries = [
        root.join("crates/scryer-infrastructure/src/shared/queries"),
        root.join("crates/scryer-infrastructure/src/queries/shared"),
        root.join("crates/scryer-infrastructure/src/portable_sql.rs"),
    ];

    for path in forbidden_shared_queries {
        assert!(
            !path.exists(),
            "runtime SQL sharing should stay concern-local; do not add global portable SQL catalog `{}`",
            path.display()
        );
    }
}

#[test]
fn engine_query_modules_do_not_leak_other_engine_json_sql() {
    let root = repo_root();
    let postgres_src = root.join("crates/scryer-infrastructure/src/postgres");
    let sqlite_src = root.join("crates/scryer-infrastructure/src");
    let postgres_forbidden = ["json_extract", "json_each", "json_valid"];
    let sqlite_forbidden = ["jsonb_", "jsonb_array_elements", "::jsonb", "->>", "->'"];

    for path in rust_files_under(&postgres_src) {
        let source = production_rust_source(&path);
        for needle in postgres_forbidden {
            assert!(
                !source.contains(needle),
                "{} leaks SQLite JSON SQL `{needle}` into PostgreSQL infrastructure",
                path.display()
            );
        }
    }

    for path in rust_files_under(&sqlite_src) {
        if path.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("postgres")
        }) {
            continue;
        }
        let source = production_rust_source(&path);
        for needle in sqlite_forbidden {
            assert!(
                !source.contains(needle),
                "{} leaks PostgreSQL JSONB SQL `{needle}` into SQLite infrastructure",
                path.display()
            );
        }
    }
}
