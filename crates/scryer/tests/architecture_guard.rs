use std::fs;
use std::path::{Path, PathBuf};

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
fn scryer_runtime_does_not_import_sqlite_datastore_implementations() {
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
        "SqlitePoolOptions",
        "SqliteConnectOptions",
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
