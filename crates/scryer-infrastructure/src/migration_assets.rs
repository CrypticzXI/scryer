#![allow(dead_code)]

use crate::migration_hook_ids;
use blake3::Hasher as Blake3Hasher;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha384};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Sha384,
    Blake3,
}

impl ChecksumAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha384 => "sha384",
            Self::Blake3 => "blake3",
        }
    }

    pub fn digest(self, bytes: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha384 => Sha384::digest(bytes).to_vec(),
            Self::Blake3 => {
                let mut hasher = Blake3Hasher::new();
                hasher.update(bytes);
                hasher.finalize().as_bytes().to_vec()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationInstallKind {
    FreshInstall,
    Upgrade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StepScope {
    #[default]
    All,
    UpgradeOnly,
    NewInstallOnly,
}

impl StepScope {
    pub fn applies_to(self, install_kind: MigrationInstallKind) -> bool {
        matches!(
            (self, install_kind),
            (Self::All, _)
                | (Self::UpgradeOnly, MigrationInstallKind::Upgrade)
                | (Self::NewInstallOnly, MigrationInstallKind::FreshInstall)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineScope {
    All,
    Sqlite,
    Postgres,
}

impl EngineScope {
    pub fn applies_to(self, engine: EngineScope) -> bool {
        matches!(self, Self::All) || self == engine
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMigrationManifest {
    #[serde(default = "default_manifest_version")]
    pub format_version: u32,
    pub legacy_sql: LegacySqlBlock,
    #[serde(default, rename = "migration")]
    pub migrations: Vec<SourceExplicitMigration>,
    #[serde(default, rename = "baseline")]
    pub baselines: Vec<SourceBaselineEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacySqlBlock {
    pub path: String,
    pub through_version: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceExplicitMigration {
    pub version: i64,
    pub description: String,
    #[serde(default = "default_explicit_checksum_algorithm")]
    pub checksum_algo: ChecksumAlgorithm,
    #[serde(default)]
    pub steps: Vec<SourceMigrationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceMigrationStep {
    Sql {
        file: String,
        engine: EngineScope,
        #[serde(default)]
        scope: StepScope,
    },
    Rust {
        hook_id: String,
        engine: EngineScope,
        #[serde(default)]
        scope: StepScope,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceBaselineEntry {
    pub through_version: i64,
    pub file: String,
    pub engine: EngineScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigrationBundle {
    pub catalog: CompiledMigrationCatalog,
    pub payload_bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigrationCatalog {
    pub format_version: u32,
    pub migrations: Vec<CompiledMigration>,
    pub baselines: Vec<CompiledBaseline>,
}

impl CompiledMigrationCatalog {
    pub fn max_version(&self) -> i64 {
        self.migrations
            .last()
            .map(|migration| migration.version)
            .unwrap_or(0)
    }

    pub fn find_migration(&self, version: i64) -> Option<&CompiledMigration> {
        self.migrations
            .iter()
            .find(|migration| migration.version == version)
    }

    pub fn latest_baseline_at_or_below(
        &self,
        version: i64,
        engine: EngineScope,
    ) -> Option<&CompiledBaseline> {
        self.baselines
            .iter()
            .filter(|baseline| baseline.through_version <= version)
            .filter(|baseline| baseline.engine.applies_to(engine))
            .max_by_key(|baseline| baseline.through_version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledMigration {
    pub version: i64,
    pub description: String,
    pub key: String,
    pub filename: String,
    pub checksum_algo: ChecksumAlgorithm,
    pub checksum: Vec<u8>,
    pub steps: Vec<CompiledMigrationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompiledMigrationStep {
    Sql {
        file: String,
        engine: EngineScope,
        scope: StepScope,
        payload: PayloadSlice,
    },
    Rust {
        hook_id: String,
        engine: EngineScope,
        scope: StepScope,
    },
}

impl CompiledMigrationStep {
    pub fn scope(&self) -> StepScope {
        match self {
            Self::Sql { scope, .. } | Self::Rust { scope, .. } => *scope,
        }
    }

    pub fn engine(&self) -> EngineScope {
        match self {
            Self::Sql { engine, .. } | Self::Rust { engine, .. } => *engine,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledBaseline {
    pub through_version: i64,
    pub file: String,
    pub engine: EngineScope,
    pub payload: PayloadSlice,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PayloadSlice {
    pub start: u64,
    pub len: u64,
}

impl PayloadSlice {
    pub fn bytes<'a>(&self, payload_bytes: &'a [u8]) -> Result<&'a [u8], String> {
        let start = usize::try_from(self.start).map_err(|_| "payload start out of range")?;
        let len = usize::try_from(self.len).map_err(|_| "payload length out of range")?;
        let end = start
            .checked_add(len)
            .ok_or_else(|| "payload slice overflow".to_string())?;
        payload_bytes
            .get(start..end)
            .ok_or_else(|| "payload slice outside bundle".to_string())
    }

    pub fn text<'a>(&self, payload_bytes: &'a [u8]) -> Result<&'a str, String> {
        std::str::from_utf8(self.bytes(payload_bytes)?)
            .map_err(|error| format!("payload is not valid UTF-8: {error}"))
    }
}

#[derive(Debug, Serialize)]
struct CanonicalMigration {
    version: i64,
    description: String,
    steps: Vec<CanonicalStep>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CanonicalStep {
    Sql {
        engine: EngineScope,
        scope: StepScope,
        sql: String,
    },
    Rust {
        engine: EngineScope,
        scope: StepScope,
        hook_id: String,
    },
}

fn default_manifest_version() -> u32 {
    DEFAULT_MANIFEST_VERSION
}

fn default_explicit_checksum_algorithm() -> ChecksumAlgorithm {
    ChecksumAlgorithm::Blake3
}

pub fn source_manifest_path(db_root: &Path) -> PathBuf {
    db_root.join("migration_manifest.toml")
}

pub fn load_source_manifest(db_root: &Path) -> Result<SourceMigrationManifest, String> {
    let path = source_manifest_path(db_root);
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    toml::from_str(&contents)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

pub fn write_source_manifest(
    db_root: &Path,
    manifest: &SourceMigrationManifest,
) -> Result<(), String> {
    let path = source_manifest_path(db_root);
    let contents = toml::to_string_pretty(manifest)
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(&path, format!("{contents}\n"))
        .map_err(|error| format!("failed to write {}: {error}", path.display()))
}

pub fn compile_source_bundle(db_root: &Path) -> Result<CompiledMigrationBundle, String> {
    let manifest = load_source_manifest(db_root)?;
    if manifest.format_version != DEFAULT_MANIFEST_VERSION {
        return Err(format!(
            "unsupported migration manifest version {}",
            manifest.format_version
        ));
    }

    let mut payload_bytes = Vec::new();
    let mut migrations =
        compile_legacy_migrations(db_root, &manifest.legacy_sql, &mut payload_bytes)?;
    let legacy_through_version = manifest.legacy_sql.through_version;

    let mut explicit = manifest.migrations.clone();
    explicit.sort_by_key(|migration| migration.version);

    for (expected_version, migration) in (legacy_through_version + 1..).zip(explicit) {
        if migration.version != expected_version {
            return Err(format!(
                "explicit migration versions must be contiguous starting at {expected_version:04}; found {:04}",
                migration.version
            ));
        }

        migrations.push(compile_explicit_migration(
            db_root,
            &migration,
            &mut payload_bytes,
        )?);
    }

    validate_contiguous_versions(&migrations)?;

    let mut baselines = Vec::new();
    let mut baseline_versions = std::collections::HashSet::new();
    for baseline in manifest.baselines {
        if !baseline_versions.insert((baseline.through_version, baseline.engine)) {
            return Err(format!(
                "duplicate baseline entry for version {:04} and engine {:?}",
                baseline.through_version, baseline.engine
            ));
        }
        if migrations
            .iter()
            .all(|migration| migration.version != baseline.through_version)
        {
            return Err(format!(
                "baseline {:04} does not match any known migration version",
                baseline.through_version
            ));
        }
        let path = db_root.join(&baseline.file);
        let sql = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let payload = push_payload(sql.as_bytes(), &mut payload_bytes);
        baselines.push(CompiledBaseline {
            through_version: baseline.through_version,
            file: baseline.file,
            engine: baseline.engine,
            payload,
        });
    }
    baselines.sort_by_key(|baseline| (baseline.through_version, baseline.file.clone()));

    Ok(CompiledMigrationBundle {
        catalog: CompiledMigrationCatalog {
            format_version: manifest.format_version,
            migrations,
            baselines,
        },
        payload_bytes,
    })
}

pub fn encode_catalog(catalog: &CompiledMigrationCatalog) -> Result<Vec<u8>, String> {
    serde_json::to_vec(catalog)
        .map_err(|error| format!("failed to serialize migration catalog: {error}"))
}

pub fn decode_catalog(bytes: &[u8]) -> Result<CompiledMigrationCatalog, String> {
    serde_json::from_slice(bytes)
        .map_err(|error| format!("failed to decode migration catalog: {error}"))
}

pub fn checksum_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|value| format!("{value:02x}")).collect()
}

pub fn migration_key_from_version_and_desc(version: i64, description: &str) -> String {
    format!("{version:04}_{}", description.replace(' ', "_"))
}

fn compile_legacy_migrations(
    db_root: &Path,
    legacy: &LegacySqlBlock,
    payload_bytes: &mut Vec<u8>,
) -> Result<Vec<CompiledMigration>, String> {
    let migrations_dir = db_root.join(&legacy.path);
    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&migrations_dir)
        .map_err(|error| format!("failed to read {}: {error}", migrations_dir.display()))?;

    for entry in read_dir {
        let entry = entry
            .map_err(|error| format!("failed to read {}: {error}", migrations_dir.display()))?;
        let path = entry.path();
        if path.extension().is_none_or(|value| value != "sql") {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy().to_string();
        let (version, description, key) = parse_legacy_filename(&file_name)?;
        if version > legacy.through_version {
            continue;
        }

        let sql = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let payload = push_payload(sql.as_bytes(), payload_bytes);
        entries.push(CompiledMigration {
            version,
            description,
            key,
            filename: file_name,
            checksum_algo: ChecksumAlgorithm::Sha384,
            checksum: ChecksumAlgorithm::Sha384.digest(sql.as_bytes()),
            steps: vec![CompiledMigrationStep::Sql {
                file: normalize_relative_path(db_root, &path),
                engine: EngineScope::Sqlite,
                scope: StepScope::All,
                payload,
            }],
        });
    }

    entries.sort_by_key(|migration| migration.version);
    if entries.len() != legacy.through_version as usize {
        return Err(format!(
            "legacy migration directory {} does not contain a contiguous 0001..{:04} prefix",
            migrations_dir.display(),
            legacy.through_version
        ));
    }

    validate_contiguous_versions(&entries)?;
    Ok(entries)
}

fn compile_explicit_migration(
    db_root: &Path,
    migration: &SourceExplicitMigration,
    payload_bytes: &mut Vec<u8>,
) -> Result<CompiledMigration, String> {
    if migration.steps.is_empty() {
        return Err(format!("migration {:04} has no steps", migration.version));
    }

    let mut compiled_steps = Vec::with_capacity(migration.steps.len());
    let mut canonical_steps = Vec::with_capacity(migration.steps.len());

    for step in &migration.steps {
        match step {
            SourceMigrationStep::Sql {
                file,
                engine,
                scope,
            } => {
                let path = db_root.join(file);
                let sql = fs::read_to_string(&path)
                    .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
                let payload = push_payload(sql.as_bytes(), payload_bytes);
                compiled_steps.push(CompiledMigrationStep::Sql {
                    file: file.clone(),
                    engine: *engine,
                    scope: *scope,
                    payload,
                });
                canonical_steps.push(CanonicalStep::Sql {
                    engine: *engine,
                    scope: *scope,
                    sql,
                });
            }
            SourceMigrationStep::Rust {
                hook_id,
                engine,
                scope,
            } => {
                migration_hook_ids::validate_migration_hook_id(hook_id)?;
                compiled_steps.push(CompiledMigrationStep::Rust {
                    hook_id: hook_id.clone(),
                    engine: *engine,
                    scope: *scope,
                });
                canonical_steps.push(CanonicalStep::Rust {
                    engine: *engine,
                    scope: *scope,
                    hook_id: hook_id.clone(),
                });
            }
        }
    }

    let canonical = CanonicalMigration {
        version: migration.version,
        description: migration.description.clone(),
        steps: canonical_steps,
    };
    let canonical_bytes = serde_json::to_vec(&canonical).map_err(|error| {
        format!(
            "failed to serialize canonical migration {:04}: {error}",
            migration.version
        )
    })?;

    let key = migration_key_from_version_and_desc(migration.version, &migration.description);
    let filename = infer_explicit_filename(migration, &key);

    Ok(CompiledMigration {
        version: migration.version,
        description: migration.description.clone(),
        key,
        filename,
        checksum_algo: migration.checksum_algo,
        checksum: migration.checksum_algo.digest(&canonical_bytes),
        steps: compiled_steps,
    })
}

fn infer_explicit_filename(migration: &SourceExplicitMigration, key: &str) -> String {
    if migration.steps.len() == 1
        && let SourceMigrationStep::Sql { file, .. } = &migration.steps[0]
        && let Some(name) = Path::new(file).file_name().and_then(|value| value.to_str())
    {
        return name.to_string();
    }

    format!("{key}.migration")
}

fn parse_legacy_filename(file_name: &str) -> Result<(i64, String, String), String> {
    let stem = file_name
        .strip_suffix(".sql")
        .ok_or_else(|| format!("legacy migration {file_name} must end with .sql"))?;
    let (version, rest) = stem.split_once('_').ok_or_else(|| {
        format!("legacy migration {file_name} must be named NNNN_description.sql")
    })?;
    let version = version
        .parse::<i64>()
        .map_err(|error| format!("invalid migration version in {file_name}: {error}"))?;
    let description = rest.replace('_', " ");
    Ok((version, description, stem.to_string()))
}

fn normalize_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn push_payload(bytes: &[u8], payload_bytes: &mut Vec<u8>) -> PayloadSlice {
    let start = payload_bytes.len() as u64;
    payload_bytes.extend_from_slice(bytes);
    PayloadSlice {
        start,
        len: bytes.len() as u64,
    }
}

fn validate_contiguous_versions(migrations: &[CompiledMigration]) -> Result<(), String> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected = index as i64 + 1;
        if migration.version != expected {
            return Err(format!(
                "migration versions must be contiguous from 0001; expected {expected:04}, found {:04}",
                migration.version
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Sha256;

    const BASELINE_0100_SHA256: &str =
        "61042ad74ec32e3d1f16fc49b548d1fd1e29dbcf64680f4ece78dc86141c577d";

    fn source_db_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scryer/src/db")
    }

    #[test]
    fn baseline_0100_snapshot_is_immutable() {
        let digest = Sha256::digest(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scryer/src/db/baselines/0100_baseline.sql"
        )));
        assert_eq!(
            checksum_hex(&digest),
            BASELINE_0100_SHA256,
            "baseline 0100 is immutable; add a new baseline instead of editing this file"
        );
    }

    #[test]
    fn source_bundle_registers_latest_migration_and_engine_baselines() {
        let bundle =
            compile_source_bundle(&source_db_root()).expect("compile source migration bundle");
        assert!(
            bundle.catalog.find_migration(113).is_some(),
            "migration 0113 must be registered in migration_manifest.toml"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(100, EngineScope::Sqlite)
                .is_some_and(|baseline| baseline.file == "baselines/0100_baseline.sql"),
            "SQLite baseline must remain manifest-owned"
        );
        assert!(
            bundle
                .catalog
                .latest_baseline_at_or_below(100, EngineScope::Postgres)
                .is_some_and(|baseline| baseline.file == "postgres/baselines/0100_baseline.sql"),
            "PostgreSQL baseline must be manifest-owned"
        );
    }

    #[test]
    fn postgres_parity_secondary_index_migration_keeps_expected_coverage() {
        let sql = fs::read_to_string(
            source_db_root().join("postgres/migrations/0109_parity_secondary_indexes.sql"),
        )
        .expect("read PostgreSQL parity secondary-index migration");
        let index_statement_count = sql
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("CREATE INDEX IF NOT EXISTS ")
                    || trimmed.starts_with("CREATE UNIQUE INDEX IF NOT EXISTS ")
            })
            .count();
        assert_eq!(
            index_statement_count, 88,
            "PostgreSQL parity secondary-index migration should preserve the audited index set"
        );

        for index_name in [
            "idx_titles_facet_normalized_slug",
            "idx_pending_releases_wanted",
            "idx_domain_events_stream_sequence",
            "idx_download_queue_commands_source",
            "idx_external_subtitle_probe_cache_file_path",
            "idx_history_events_title_time",
            "idx_notification_subscriptions_channel_scope",
            "idx_release_download_attempts_outcome_attempted",
            "idx_subtitle_provider_configs_provider_type",
            "idx_wanted_items_next_search",
            "idx_workflow_operations_job_key_status",
        ] {
            assert!(
                sql.contains(index_name),
                "expected PostgreSQL parity secondary-index migration to include {index_name}"
            );
        }
    }

    #[test]
    fn postgres_baseline_0100_keeps_library_scan_unmatched_title_binding() {
        let sql = fs::read_to_string(source_db_root().join("postgres/baselines/0100_baseline.sql"))
            .expect("read PostgreSQL baseline 0100");
        assert!(
            sql.contains("title_id TEXT"),
            "PostgreSQL baseline 0100 must include library_scan_unmatched_items.title_id"
        );
    }

    #[test]
    fn postgres_library_scan_unmatched_title_binding_migration_keeps_index() {
        let sql = fs::read_to_string(
            source_db_root()
                .join("postgres/migrations/0110_library_scan_unmatched_title_binding.sql"),
        )
        .expect("read PostgreSQL library-scan unmatched title binding migration");
        assert!(
            sql.contains("ADD COLUMN IF NOT EXISTS title_id TEXT"),
            "PostgreSQL migration 0110 must add library_scan_unmatched_items.title_id"
        );
        assert!(
            sql.contains("idx_library_scan_unmatched_items_facet_title_status_updated"),
            "PostgreSQL migration 0110 must restore the title-aware unmatched-items index"
        );
    }

    #[test]
    fn postgres_title_image_schema_alignment_migration_keeps_runtime_columns() {
        let sql = fs::read_to_string(
            source_db_root().join("postgres/migrations/0112_title_image_schema_alignment.sql"),
        )
        .expect("read PostgreSQL title image schema alignment migration");
        for expected in [
            "ADD COLUMN IF NOT EXISTS poster_local_path TEXT",
            "ADD COLUMN IF NOT EXISTS banner_local_path TEXT",
            "ADD COLUMN IF NOT EXISTS background_local_path TEXT",
            "CREATE TABLE IF NOT EXISTS title_images",
            "CREATE TABLE IF NOT EXISTS title_image_variants",
        ] {
            assert!(
                sql.contains(expected),
                "expected PostgreSQL migration 0112 to include {expected}"
            );
        }
    }

    #[test]
    fn postgres_title_read_indexes_migration_keeps_title_listing_indexes() {
        let sql = fs::read_to_string(
            source_db_root().join("postgres/migrations/0113_title_read_indexes.sql"),
        )
        .expect("read PostgreSQL title read indexes migration");
        for expected in [
            "idx_titles_facet_library_name_id",
            "idx_titles_facet_slug_library_id",
        ] {
            assert!(
                sql.contains(expected),
                "expected PostgreSQL migration 0113 to include {expected}"
            );
        }
    }

    #[test]
    fn explicit_migrations_after_postgres_baseline_treat_both_engines() {
        let bundle =
            compile_source_bundle(&source_db_root()).expect("compile source migration bundle");
        for migration in bundle
            .catalog
            .migrations
            .iter()
            .filter(|migration| migration.version >= 101)
        {
            let treats_sqlite = migration
                .steps
                .iter()
                .any(|step| step.engine().applies_to(EngineScope::Sqlite));
            let treats_postgres = migration
                .steps
                .iter()
                .any(|step| step.engine().applies_to(EngineScope::Postgres));
            assert!(
                treats_sqlite && treats_postgres,
                "migration {} must explicitly treat both sqlite and postgres",
                migration.key
            );
        }
    }

    #[test]
    fn source_manifest_requires_step_engine_scope() {
        let manifest = r#"
format_version = 1

[legacy_sql]
path = "migrations"
through_version = 0

[[migration]]
version = 1
description = "missing engine"

[[migration.steps]]
kind = "sql"
file = "0001_missing_engine.sql"
"#;
        let error = toml::from_str::<SourceMigrationManifest>(manifest)
            .expect_err("manifest without step engine must fail");
        assert!(
            error.to_string().contains("engine"),
            "expected missing engine field error, got {error}"
        );
    }
}
