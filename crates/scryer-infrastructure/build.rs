use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[path = "src/migration_assets.rs"]
mod migration_assets;
#[path = "src/migration_hook_ids.rs"]
mod migration_hook_ids;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=src/graphql");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    compile_spellfix_extension(&manifest_dir);
    validate_graphql_documents(&manifest_dir);
    compile_migration_bundle(&manifest_dir);
}

fn validate_graphql_documents(manifest_dir: &Path) {
    let graphql_dir = manifest_dir.join("src/graphql");

    if !graphql_dir.exists() {
        return;
    }

    let mut stack = vec![graphql_dir];

    while let Some(dir) = stack.pop() {
        println!("cargo:rerun-if-changed={}", dir.display());

        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) => {
                panic!(
                    "failed to read GraphQL directory {}: {error}",
                    dir.display()
                );
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            if !path.extension().is_some_and(|ext| ext == "graphql") {
                continue;
            }

            println!("cargo:rerun-if-changed={}", path.display());

            let document = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            graphql_parser::query::parse_query::<String>(&document).unwrap_or_else(|error| {
                panic!("invalid GraphQL document {}: {error}", path.display())
            });
        }
    }
}

fn compile_spellfix_extension(manifest_dir: &Path) {
    let spellfix_source = manifest_dir.join("vendor/sqlite/ext/misc/spellfix.c");
    println!("cargo:rerun-if-changed={}", spellfix_source.display());

    let sqlite_include =
        env::var("DEP_SQLITE3_INCLUDE").expect("DEP_SQLITE3_INCLUDE must be set by libsqlite3-sys");

    cc::Build::new()
        .file(&spellfix_source)
        .include(sqlite_include)
        .warnings(false)
        .compile("scryer_spellfix1");
}

fn compile_migration_bundle(manifest_dir: &Path) {
    let db_root = manifest_dir.join("../scryer/src/db");
    watch_tree(&db_root);

    let bundle = migration_assets::compile_source_bundle(&db_root)
        .unwrap_or_else(|error| panic!("failed to compile migration bundle: {error}"));
    let bundle_bytes = migration_assets::encode_bundle(&bundle)
        .unwrap_or_else(|error| panic!("failed to encode migration bundle: {error}"));
    let compressed =
        zstd::stream::encode_all(bundle_bytes.as_slice(), zstd::zstd_safe::max_c_level())
            .unwrap_or_else(|error| panic!("failed to compress migration bundle: {error}"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("migration_bundle.bin.zst");
    fs::write(&out_path, compressed)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", out_path.display()));
}

fn watch_tree(root: &Path) {
    if !root.exists() {
        return;
    }

    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        println!("cargo:rerun-if-changed={}", dir.display());

        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
