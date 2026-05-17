use std::collections::{BTreeMap, BTreeSet};
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

fn postgres_0115_baseline_columns(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let baseline = root.join("crates/scryer/src/db/postgres/baselines/0115_baseline.sql");
    let sql = fs::read_to_string(&baseline)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", baseline.display()));
    parse_create_table_columns(&sql)
}

fn postgres_0115_baseline_foreign_keys(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let baseline = root.join("crates/scryer/src/db/postgres/baselines/0115_baseline.sql");
    let sql = fs::read_to_string(&baseline)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", baseline.display()));
    parse_create_table_foreign_keys(&sql)
}

fn parse_create_table_columns(sql: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut schema = BTreeMap::<String, BTreeSet<String>>::new();
    let mut current_table: Option<String> = None;
    for line in sql.lines() {
        let trimmed = line.trim();
        if let Some(table) = current_table.as_ref() {
            if trimmed == ");" {
                current_table = None;
                continue;
            }
            if trimmed.is_empty()
                || trimmed.starts_with("CONSTRAINT ")
                || trimmed.starts_with("PRIMARY ")
                || trimmed.starts_with("UNIQUE ")
                || trimmed.starts_with("FOREIGN ")
                || trimmed.starts_with("CHECK ")
            {
                continue;
            }
            if let Some(column) = trimmed
                .trim_end_matches(',')
                .split_whitespace()
                .next()
                .map(|value| value.trim_matches('"').to_string())
            {
                schema.entry(table.clone()).or_default().insert(column);
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("CREATE TABLE ") {
            let table = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('(')
                .trim_start_matches("public.")
                .to_string();
            schema.entry(table.clone()).or_default();
            current_table = Some(table);
        }
    }
    schema
}

fn parse_create_table_foreign_keys(sql: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut foreign_keys = BTreeMap::<String, BTreeSet<String>>::new();
    let mut current_table: Option<String> = None;
    for line in sql.lines() {
        let trimmed = line.trim();
        if let Some(table) = current_table.as_ref() {
            if trimmed == ");" {
                current_table = None;
                continue;
            }

            if let Some(index) = trimmed.find("REFERENCES ") {
                let reference = &trimmed[index + "REFERENCES ".len()..];
                if let Some(parent) = reference
                    .split_whitespace()
                    .next()
                    .map(|value| value.split('(').next().unwrap_or_default())
                    .map(|value| value.trim_matches('"').trim_start_matches("public."))
                    .filter(|value| !value.is_empty())
                {
                    foreign_keys
                        .entry(table.clone())
                        .or_default()
                        .insert(parent.to_string());
                }
            }
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("CREATE TABLE ") {
            let table = rest
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches('(')
                .trim_start_matches("public.")
                .to_string();
            current_table = Some(table);
        }
    }
    foreign_keys
}

fn line_number(source: &str, index: usize) -> usize {
    source[..index]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1
}

fn skip_ws(source: &str, mut index: usize) -> usize {
    while let Some(ch) = source[index..].chars().next() {
        if !ch.is_whitespace() {
            break;
        }
        index += ch.len_utf8();
    }
    index
}

fn identifier_at(source: &str, mut index: usize) -> Option<(String, usize)> {
    index = skip_ws(source, index);
    let start = index;
    while let Some(ch) = source[index..].chars().next() {
        if !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.') {
            break;
        }
        index += ch.len_utf8();
    }
    (index > start).then(|| {
        (
            source[start..index]
                .trim_start_matches("public.")
                .trim_matches('"')
                .to_string(),
            index,
        )
    })
}

fn parenthesized_at(source: &str, mut index: usize) -> Option<(String, usize)> {
    index = skip_ws(source, index);
    if source[index..].chars().next()? != '(' {
        return None;
    }
    let mut depth = 0usize;
    let content_start = index + 1;
    for (offset, ch) in source[index..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = index + offset;
                    return Some((source[content_start..end].to_string(), end + 1));
                }
            }
            _ => {}
        }
    }
    None
}

fn split_sql_columns(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter_map(|part| {
            part.split_whitespace()
                .next()
                .map(|value| value.trim_matches('"').to_string())
                .filter(|value| {
                    value
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                })
        })
        .collect()
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn extract_sql_table_references(source: &str) -> Vec<(usize, &'static str, String)> {
    let patterns = [
        ("INSERT INTO", "insert"),
        ("DELETE FROM", "delete"),
        ("UPDATE", "update"),
        ("FROM", "from"),
        ("JOIN", "join"),
        ("TRUNCATE", "truncate"),
    ];
    let mut out = Vec::new();

    for (keyword, operation) in patterns {
        let mut offset = 0usize;
        while let Some(relative) = source[offset..].find(keyword) {
            let start = offset + relative;
            let before_is_identifier = source[..start]
                .chars()
                .next_back()
                .is_some_and(is_identifier_char);
            let after = start + keyword.len();
            let after_is_identifier = source[after..]
                .chars()
                .next()
                .is_some_and(is_identifier_char);
            if before_is_identifier || after_is_identifier {
                offset = start + 1;
                continue;
            }

            if let Some((table, _)) = identifier_at(source, after) {
                out.push((line_number(source, start), operation, table));
            }
            offset = start + 1;
        }
    }

    out
}

fn extract_insert_column_lists(source: &str) -> Vec<(usize, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find("INSERT INTO ") {
        let start = offset + relative;
        let Some((table, table_end)) = identifier_at(source, start + "INSERT INTO ".len()) else {
            offset = start + 1;
            continue;
        };
        if let Some((raw_columns, _)) = parenthesized_at(source, table_end) {
            out.push((
                line_number(source, start),
                table,
                split_sql_columns(&raw_columns),
            ));
        }
        offset = start + 1;
    }
    out
}

fn extract_update_columns(source: &str) -> Vec<(usize, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find("UPDATE ") {
        let start = offset + relative;
        let Some((table, table_end)) = identifier_at(source, start + "UPDATE ".len()) else {
            offset = start + 1;
            continue;
        };
        if table == "SET" {
            offset = start + 1;
            continue;
        }
        let rest = &source[table_end..];
        let Some(set_relative) = rest.find("SET ") else {
            offset = start + 1;
            continue;
        };
        let set_start = table_end + set_relative + "SET ".len();
        let segment = &source[set_start..source.len().min(set_start + 2_000)];
        let end = [" WHERE ", "\nWHERE ", " RETURNING ", "\""]
            .iter()
            .filter_map(|needle| segment.find(needle))
            .min()
            .unwrap_or(segment.len());
        let columns = segment[..end]
            .split(',')
            .filter_map(|assignment| assignment.split_once('=').map(|(column, _)| column))
            .filter_map(|column| {
                column
                    .split_whitespace()
                    .last()
                    .map(|value| value.trim_matches('"').to_string())
            })
            .filter(|value| {
                value
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            })
            .collect::<Vec<_>>();
        if !columns.is_empty() {
            out.push((line_number(source, start), table, columns));
        }
        offset = start + 1;
    }
    out
}

fn extract_row_getter_columns(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    while let Some(relative) = source[offset..].find("try_get") {
        let start = offset + relative;
        let call_end = source[start..]
            .find(')')
            .map(|relative_end| start + relative_end)
            .unwrap_or_else(|| source.len());
        if let Some(quote_start) = source[start..call_end].find('"') {
            let quote_start = start + quote_start + 1;
            if let Some(quote_end) = source[quote_start..call_end].find('"') {
                let quote_end = quote_start + quote_end;
                let column = &source[quote_start..quote_end];
                if column
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
                {
                    out.push((line_number(source, start), column.to_string()));
                }
                offset = quote_end + 1;
                continue;
            }
        }
        offset = start + 1;
    }
    out
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
fn postgres_runtime_sql_references_match_0115_baseline_columns() {
    let root = repo_root();
    let schema = postgres_0115_baseline_columns(&root);
    let all_columns = schema
        .values()
        .flat_map(|columns| columns.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let allowed_row_aliases = BTreeSet::from([
        "base_path",
        "candidate_distance",
        "checksum",
        "checksum_algo",
        "cnt",
        "column_default",
        "column_name",
        "count",
        "definition_id",
        "effective_value_json",
        "episode_count",
        "episode_set_ids",
        "episode_ids",
        "episode_title",
        "exists",
        "fts_rank",
        "has_more",
        "image_count",
        "installed_on",
        "is_nullable",
        "latest_created_at",
        "latest_decision_created_at",
        "latest_decision_decision_code",
        "latest_decision_explanation_json",
        "latest_decision_id",
        "latest_decision_release_size_bytes",
        "latest_decision_release_title",
        "latest_decision_release_url",
        "latest_decision_title_id",
        "latest_decision_wanted_item_id",
        "latest_message",
        "latest_status",
        "library_facet",
        "library_name",
        "library_path",
        "library_slug",
        "match_rank",
        "max_sequence",
        "metadata",
        "movie_count",
        "monitored_episodes",
        "normalized_score",
        "owned_episodes",
        "preview_path",
        "rank",
        "referenced_table",
        "release_count",
        "score",
        "sequence_name",
        "sequence_schema",
        "source_rank",
        "success",
        "table_name",
        "title_facet",
        "title_name",
        "title_slug",
        "total",
        "total_episodes",
        "total_size_bytes",
        "udt_name",
        "value",
        "variant_count",
        "version",
    ]);
    let removed_runtime_identifiers = [
        "quality_profiles_json",
        "mediarr_schema_migrations",
        "interstitial_movie_json",
        "specials_movies_json",
        "event_json",
    ];
    let removed_runtime_tables = BTreeSet::from([
        "import_artifacts",
        "job_runs",
        "mediarr_schema_migrations",
        "quality_profiles_json",
        "subtitle_providers",
    ]);
    let ignored_tables = BTreeSet::from(["_sqlx_migrations"]);
    let mut failures = Vec::new();

    for path in rust_files_under(&root.join("crates/scryer-infrastructure/src/postgres")) {
        let source = production_rust_source(&path);

        for identifier in removed_runtime_identifiers {
            if source.contains(identifier) {
                failures.push(format!(
                    "{} references removed PostgreSQL schema identifier `{identifier}`",
                    path.display()
                ));
            }
        }

        for (line, operation, table) in extract_sql_table_references(&source) {
            if removed_runtime_tables.contains(table.as_str()) {
                failures.push(format!(
                    "{}:{line} has stale PostgreSQL {operation} reference to removed table `{table}`",
                    path.display()
                ));
            }
        }

        for (line, table, columns) in extract_insert_column_lists(&source) {
            let Some(valid_columns) = schema.get(&table) else {
                if !ignored_tables.contains(table.as_str()) {
                    failures.push(format!(
                        "{}:{line} inserts into table `{table}` that is not in the 0115 baseline",
                        path.display()
                    ));
                }
                continue;
            };
            for column in columns {
                if !valid_columns.contains(&column) {
                    failures.push(format!(
                        "{}:{line} inserts unknown column `{table}.{column}`",
                        path.display()
                    ));
                }
            }
        }

        for (line, table, columns) in extract_update_columns(&source) {
            let Some(valid_columns) = schema.get(&table) else {
                if !ignored_tables.contains(table.as_str()) {
                    failures.push(format!(
                        "{}:{line} updates table `{table}` that is not in the 0115 baseline",
                        path.display()
                    ));
                }
                continue;
            };
            for column in columns {
                if !valid_columns.contains(&column) {
                    failures.push(format!(
                        "{}:{line} updates unknown column `{table}.{column}`",
                        path.display()
                    ));
                }
            }
        }

        for (line, column) in extract_row_getter_columns(&source) {
            if !all_columns.contains(column.as_str())
                && !allowed_row_aliases.contains(column.as_str())
            {
                failures.push(format!(
                    "{}:{line} reads unknown PostgreSQL row column or alias `{column}`",
                    path.display()
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "PostgreSQL runtime SQL drifted from the 0115 baseline:\n{}",
        failures.join("\n")
    );
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
fn backup_catalog_exports_foreign_key_parents() {
    let root = repo_root();
    let foreign_keys = postgres_0115_baseline_foreign_keys(&root);
    let classifications = BACKUP_TABLE_CATALOG
        .iter()
        .map(|entry| (entry.table, entry.classification))
        .collect::<BTreeMap<_, _>>();

    for (table, parents) in foreign_keys {
        if classifications.get(table.as_str()) != Some(&BackupTableClassification::Export) {
            continue;
        }

        for parent in parents {
            let Some(classification) = classifications.get(parent.as_str()) else {
                continue;
            };
            assert_eq!(
                *classification,
                BackupTableClassification::Export,
                "backup export table `{table}` references `{parent}`, so the parent must also export"
            );
        }
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
    let rule_sets =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/rule_set_store.rs"));
    let scripts = production_rust_source(
        &root.join("crates/scryer-infrastructure/src/post_processing_script_store.rs"),
    );
    let plugins =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/plugin_store.rs"));

    for (name, source, trait_impl) in [
        (
            "rule sets",
            &rule_sets,
            "impl RuleSetRepository for RuleSetStore",
        ),
        (
            "post-processing scripts",
            &scripts,
            "impl PostProcessingScriptRepository for PostProcessingScriptStore",
        ),
        (
            "plugins",
            &plugins,
            "impl PluginInstallationRepository for PluginStore",
        ),
    ] {
        assert!(
            source.contains("datastore: StoreDatastore"),
            "{name} should own StoreDatastore directly"
        );
        assert!(
            source.contains("SqlRuntime::"),
            "{name} should use the shared SQL runtime"
        );
        assert!(
            source.contains(trait_impl),
            "{name} should implement its repository trait directly"
        );
    }

    for forbidden in [
        "pub trait CustomizationSql",
        "pub struct CustomizationStore<S>",
        "SqliteCustomizationStore",
        "PostgresCustomizationStore",
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
            !rule_sets.contains(forbidden)
                && !scripts.contains(forbidden)
                && !plugins.contains(forbidden),
            "customization repositories must not reintroduce monolithic or paired implementation `{forbidden}`"
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
fn catalog_repository_has_been_evacuated_to_domain_stores() {
    let root = repo_root();
    assert!(
        !root
            .join("crates/scryer-infrastructure/src/catalog_store.rs")
            .exists(),
        "legacy SQLite catalog file should stay deleted"
    );
    assert!(
        !root
            .join("crates/scryer-infrastructure/src/postgres/catalog_store.rs")
            .exists(),
        "legacy PostgreSQL catalog file should stay deleted"
    );

    let stores = [
        (
            "title_store.rs",
            "pub struct TitleStore",
            "impl TitleRepository for TitleStore",
        ),
        (
            "show_store.rs",
            "pub struct ShowStore",
            "impl ShowRepository for ShowStore",
        ),
        (
            "library_store.rs",
            "pub struct LibraryStore",
            "impl LibraryRepository for LibraryStore",
        ),
        (
            "user_store.rs",
            "pub struct UserStore",
            "impl UserRepository for UserStore",
        ),
    ];

    for (file, struct_name, impl_name) in stores {
        let source =
            production_rust_source(&root.join("crates/scryer-infrastructure/src").join(file));
        assert!(
            source.contains(struct_name),
            "{file} should own its canonical store type"
        );
        assert!(
            source.contains(impl_name),
            "{file} should own its application repository implementation"
        );
        assert!(
            source.contains("StoreDatastore"),
            "{file} should route through the shared datastore runtime"
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
fn title_store_keeps_runtime_parity_for_backfills_and_transactions() {
    let root = repo_root();
    let title_store =
        production_rust_source(&root.join("crates/scryer-infrastructure/src/title_store.rs"));

    assert!(
        !title_store.contains(
            "async fn list_anime_title_ids_missing_title_anidb_external_ids(\n        &self,\n        _limit: usize,\n    ) -> AppResult<Vec<String>> {\n        Ok(Vec::new())\n    }"
        ),
        "TitleStore must not provide a default no-op for title-level AniDB backfill"
    );
    assert!(
        title_store.contains("async fn list_anime_title_ids_missing_title_anidb_external_ids"),
        "TitleStore must implement title-level AniDB backfill parity"
    );
    assert!(
        title_store.contains("LOWER(title_external_ids.source) IN ('anidb', 'anidb_id')"),
        "title-level AniDB backfill should keep explicit source normalization"
    );
    assert!(
        title_store.contains("SqlRuntime::run_in_transaction"),
        "TitleStore mutations must run through the shared transaction helper"
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
