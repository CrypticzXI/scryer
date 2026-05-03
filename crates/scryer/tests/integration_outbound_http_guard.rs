use std::fs;
use std::path::{Path, PathBuf};

const ALLOWED_DIRECT_SEND_FILES: &[&str] = &["crates/scryer-outbound-http/src/lib.rs"];

const ALLOWED_CLIENT_CONSTRUCTION_FILES: &[&str] = &[
    "crates/scryer-outbound-http/src/lib.rs",
    "crates/scryer-application/src/plugins/plugins.rs",
    "crates/scryer-application/src/subtitles/provider.rs",
    "crates/scryer-infrastructure/src/download_clients/nzbget.rs",
    "crates/scryer-infrastructure/src/download_clients/router.rs",
    "crates/scryer-infrastructure/src/download_clients/sabnzbd.rs",
    "crates/scryer-infrastructure/src/download_clients/weaver.rs",
    "crates/scryer-infrastructure/src/external_import.rs",
    "crates/scryer-infrastructure/src/metadata_gateway.rs",
    "crates/scryer-infrastructure/src/smg_enrollment.rs",
    "crates/scryer-infrastructure/src/title_images.rs",
];

#[test]
fn native_outbound_http_uses_canonical_transport() {
    let repo_root = repo_root();
    let crates_root = repo_root.join("crates");
    let mut violations = Vec::new();
    collect_violations(&crates_root, &repo_root, &mut violations);

    assert!(
        violations.is_empty(),
        "native outbound HTTP guard failed:\n{}",
        violations.join("\n")
    );
}

fn collect_violations(path: &Path, repo_root: &Path, violations: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path, repo_root) {
                continue;
            }
            collect_violations(&path, repo_root, violations);
            continue;
        }

        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }

        let relative_path = repo_relative_path(&path, repo_root);
        if should_skip_file(&relative_path) {
            continue;
        }

        let Ok(content) = fs::read_to_string(&path) else {
            continue;
        };

        if !ALLOWED_DIRECT_SEND_FILES.contains(&relative_path.as_str()) {
            record_token_violations(&relative_path, &content, "reqwest::get(", false, violations);
            record_token_violations(&relative_path, &content, ".send()", false, violations);
        }

        if !ALLOWED_CLIENT_CONSTRUCTION_FILES.contains(&relative_path.as_str()) {
            record_token_violations(&relative_path, &content, "Client::new(", true, violations);
            record_token_violations(
                &relative_path,
                &content,
                "Client::builder(",
                true,
                violations,
            );
            record_token_violations(
                &relative_path,
                &content,
                "reqwest::Client::builder(",
                false,
                violations,
            );
        }
    }
}

fn record_token_violations(
    relative_path: &str,
    content: &str,
    token: &str,
    require_identifier_boundary: bool,
    violations: &mut Vec<String>,
) {
    for offset in find_token_offsets(content, token, require_identifier_boundary) {
        let line = content[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        violations.push(format!("{relative_path}:{line}: forbidden token `{token}`"));
    }
}

fn find_token_offsets(content: &str, token: &str, require_identifier_boundary: bool) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut start = 0usize;

    while let Some(found) = content[start..].find(token) {
        let offset = start + found;
        let boundary_ok = !require_identifier_boundary
            || offset == 0
            || !is_identifier_byte(content.as_bytes()[offset - 1]);
        if boundary_ok {
            offsets.push(offset);
        }
        start = offset + token.len();
    }

    offsets
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn should_skip_dir(path: &Path, repo_root: &Path) -> bool {
    let relative_path = repo_relative_path(path, repo_root);
    relative_path == "crates/scryer-plugins"
        || relative_path.starts_with("crates/scryer-plugins/")
        || relative_path.ends_with("/vendor")
        || relative_path.contains("/vendor/")
}

fn should_skip_file(relative_path: &str) -> bool {
    relative_path.starts_with("crates/scryer/tests/")
        || relative_path.starts_with("crates/scryer-plugins/")
        || relative_path.contains("/tests/")
        || relative_path.ends_with("/tests.rs")
        || relative_path.ends_with("_tests.rs")
}

fn repo_relative_path(path: &Path, repo_root: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}
