use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=src/graphql");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    compile_spellfix_extension(&manifest_dir);
    validate_graphql_documents(&manifest_dir);
    let migrations_dir = manifest_dir.join("../scryer/src/db/migrations");

    if migrations_dir.exists() {
        let mut stack = vec![migrations_dir];

        while let Some(dir) = stack.pop() {
            println!("cargo:rerun-if-changed={}", dir.display());

            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(_) => continue,
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }

                if path.extension().is_some_and(|ext| ext == "sql") {
                    println!("cargo:rerun-if-changed={}", path.display());
                }
            }
        }
    }
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
