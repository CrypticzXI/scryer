use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    compile_spellfix_extension(&manifest_dir);
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

fn compile_spellfix_extension(manifest_dir: &PathBuf) {
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
