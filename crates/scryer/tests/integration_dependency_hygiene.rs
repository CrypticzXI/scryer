// `zip` is deliberately absent from this list: scryer-application depends on
// it directly (introduced with the in-app upgrade engine, 2488ee601) solely to
// validate and extract the SIGNED APPLICATION UPGRADE ARTIFACT in
// application_upgrade/engine.rs (`validate_zip_members` / `extract_zip`).
// That is not media-release archive extraction — RAR/ZIP/7z handling for
// downloaded releases must still live behind the optional archive plugin,
// which is what this guard exists to enforce. If `zip` ever gains a second
// consumer in core, move that consumer behind the plugin instead of relying
// on this exemption (`cargo tree -i zip` should list scryer-application's
// upgrade engine and build tooling only).
const FORBIDDEN_ARCHIVE_CORE_DEPENDENCIES: &[&str] = &["sevenz-rust2", "unrar-rs"];
const FORBIDDEN_BUILTIN_ARCHIVE_PLUGIN_NAMES: &[&str] =
    &["archive-extraction", "archive_extraction"];

#[test]
fn archive_restricted_crates_stay_out_of_scryer_dependency_graph() {
    let cargo_lock = include_str!("../../../Cargo.lock");
    let mut present = Vec::new();

    for package_name in FORBIDDEN_ARCHIVE_CORE_DEPENDENCIES {
        let needle = format!("name = \"{package_name}\"");
        if cargo_lock.lines().any(|line| line.trim() == needle) {
            present.push(*package_name);
        }
    }

    assert!(
        present.is_empty(),
        "RAR/ZIP/7z archive extraction must stay behind the optional archive plugin; \
         remove these exact packages from Scryer core before merging: {present:?}"
    );
}

#[test]
fn archive_extraction_plugin_is_not_bundled_as_builtin() {
    let builtins_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scryer-plugins/builtins");
    let entries = std::fs::read_dir(&builtins_dir).unwrap_or_else(|error| {
        panic!(
            "failed to read builtin plugin directory {}: {error}",
            builtins_dir.display()
        )
    });
    let mut forbidden = Vec::new();

    for entry in entries {
        let entry = entry.expect("builtin plugin directory entry should be readable");
        let file_name = entry.file_name().to_string_lossy().to_lowercase();
        if FORBIDDEN_BUILTIN_ARCHIVE_PLUGIN_NAMES
            .iter()
            .any(|name| file_name.contains(name))
        {
            forbidden.push(file_name);
        }
    }

    assert!(
        forbidden.is_empty(),
        "archive extraction must remain an optional user-installed plugin, not a Scryer builtin: \
         {forbidden:?}"
    );
}
