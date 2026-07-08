//! RFC 123 WP2 — real-artifact archive validation suite.
//!
//! These tests drive the checked-in `archive-extraction` plugin wasm through the
//! production host entry (`WasmArchiveExtractorClient::process`) and the
//! command-model describe path, exercising the frozen §5 crypto ABI, the COW
//! PAR2 staging path, and the WASI sandbox against REAL archive fixtures. W1's
//! unit tests already cover the host mechanics with inline WAT guests (stdio
//! spike, epoch/memory kills, crypto round-trip, error mapping, describe
//! classifier); this layer proves the real external artifact behaves.
//!
//! Fixture provenance: see `fixtures/archive-extraction/README.md` and
//! `fixtures/archive-corpus/README.md`. The plugin wasm is the archive-extraction
//! command binary from scryer-plugins @ 5b20a3f (baseline features); the corpus
//! is copied verbatim from the scryer-plugins harness and rarpar. Encrypted-RAR
//! password is `testpass123` (`-p` data-only encryption).
//!
//! Byte-correctness oracles: store/encrypted RAR members are compared against the
//! exact original plaintext; compressed members are cross-checked by recomputing
//! crc32 of the extracted bytes against the archive's own header CRC that the
//! plugin reports.

use std::path::{Path, PathBuf};
use std::time::Instant;

use scryer_application::{AppError, ArchiveExtractorClient};
use scryer_plugin_sdk::{
    ArchivePluginFormat, ArchivePluginOperation, ArchivePluginProcessRequest,
    ArchivePluginProcessResponse, ArchivePluginRepairState, ArchivePluginStatus, PluginDescriptor,
    PluginKind,
};
use wasmtime::{Engine, ExternType, Module, ValType};

use crate::archive_adapter::WasmArchiveExtractorClient;

const RAR_PASSWORD: &str = "testpass123";

// ── fixture plumbing ─────────────────────────────────────────────────────────

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn corpus_dir() -> PathBuf {
    fixtures_dir().join("archive-corpus")
}

/// Decompress the checked-in plugin fixture (`.wasm.zst`, matching the builtin
/// indexer convention) to raw wasm bytes.
fn plugin_wasm() -> Vec<u8> {
    let path = fixtures_dir()
        .join("archive-extraction")
        .join("plugin.wasm.zst");
    let compressed =
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    zstd::decode_all(compressed.as_slice()).expect("decompress plugin.wasm.zst")
}

/// Extract the descriptor from a real artifact via the loader's command-model
/// describe path (RFC §7.2.11 / §8.2). `None` = classified as an Extism/fleet
/// artifact (routes to the Extism describe path instead).
fn describe(wasm: &[u8]) -> Option<PluginDescriptor> {
    crate::wasmtime_host::command_model_describe(wasm)
        .map(|result| result.expect("command-model describe must succeed on the real artifact"))
}

/// Build the production host client from the real artifact.
fn client() -> WasmArchiveExtractorClient {
    let wasm = plugin_wasm();
    let descriptor = describe(&wasm).expect("real artifact must self-describe");
    assert_eq!(descriptor.kind(), PluginKind::ArchiveExtractor);
    WasmArchiveExtractorClient::new(wasm, descriptor).expect("construct archive client")
}

/// Run one request through the production async entry on a private runtime.
fn process(
    client: &WasmArchiveExtractorClient,
    request: ArchivePluginProcessRequest,
) -> Result<ArchivePluginProcessResponse, AppError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build test runtime");
    runtime.block_on(client.process(request))
}

fn extract_request(
    archive: &Path,
    output: &Path,
    format: ArchivePluginFormat,
    password: Option<&str>,
) -> ArchivePluginProcessRequest {
    ArchivePluginProcessRequest {
        operation: ArchivePluginOperation::ExtractArchive {
            archive_path: archive.to_string_lossy().into_owned(),
            output_dir: output.to_string_lossy().into_owned(),
            format,
            password: password.map(str::to_string),
        },
    }
}

/// Copy a set of fixture files into a fresh temp dir; returns the temp dir.
fn stage_files(files: &[PathBuf]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("create staging dir");
    for file in files {
        let name = file.file_name().expect("fixture has a file name");
        std::fs::copy(file, dir.path().join(name))
            .unwrap_or_else(|error| panic!("copy {}: {error}", file.display()));
    }
    dir
}

/// All corpus files under `par2-rar5` whose name starts with the plain
/// multi-volume RAR5 stem (the `.part*.rar` volumes) plus every `.par2` file.
fn par2_rar5_set() -> Vec<PathBuf> {
    let dir = corpus_dir().join("par2-rar5");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read par2-rar5 dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
}

fn crc32_hex(bytes: &[u8]) -> String {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    format!("{:08x}", hasher.finalize())
}

/// Assert every extracted RAR/other member on disk matches the header CRC the
/// plugin reported — a byte-correctness proof anchored on the archive's own
/// integrity data, without needing an external plaintext.
fn assert_files_crc_correct(response: &ArchivePluginProcessResponse, output: &Path) {
    assert!(
        !response.files.is_empty(),
        "expected at least one extracted file"
    );
    for file in &response.files {
        let disk = output.join(&file.relative_path);
        let bytes = std::fs::read(&disk)
            .unwrap_or_else(|error| panic!("read extracted {}: {error}", disk.display()));
        if let Some(size) = file.size {
            assert_eq!(
                bytes.len() as u64,
                size,
                "size mismatch for {}",
                file.relative_path
            );
        }
        if let Some(checksum) = &file.checksum {
            assert_eq!(
                &crc32_hex(&bytes),
                checksum,
                "crc32 of extracted {} must match the reported header CRC",
                file.relative_path
            );
        }
    }
}

// ── describe (RFC §7.2.11 / §8.2) ────────────────────────────────────────────

#[test]
fn describe_reports_expected_descriptor_from_real_artifact() {
    let wasm = plugin_wasm();
    let descriptor = describe(&wasm).expect("command-model describe returns a descriptor");
    assert_eq!(descriptor.id, "archive-extraction");
    assert_eq!(descriptor.kind(), PluginKind::ArchiveExtractor);
    let archive = descriptor
        .archive_extractor()
        .expect("descriptor carries archive-extractor capabilities");
    assert!(archive.capabilities.formats.contains(&ArchivePluginFormat::Rar));
    assert!(archive.capabilities.formats.contains(&ArchivePluginFormat::Zip));
    assert!(!archive.capabilities.repair_formats.is_empty());
}

#[test]
fn describe_classifier_routes_fleet_fixture_to_extism() {
    // The in-repo test-indexer is an Extism reactor plugin (exports
    // `scryer_describe`). The command-model classifier must decline it (returns
    // None) so the loader keeps routing fleet artifacts to the Extism describe
    // path — the deliberate contrast with the command-model archive artifact.
    let indexer = std::fs::read(fixtures_dir().join("test-indexer").join("plugin.wasm"))
        .expect("read test-indexer fixture");
    assert!(
        crate::wasmtime_host::command_model_describe(&indexer).is_none(),
        "Extism fleet fixture must not classify as command-model"
    );
}

// ── extraction: plain RAR ────────────────────────────────────────────────────

#[test]
fn extract_plain_rar4_multifile_is_byte_correct() {
    let client = client();
    let source = stage_files(&[corpus_dir().join("plain-rar4").join("rar4_multifile_lz.rar")]);
    let output = tempfile::tempdir().unwrap();
    let response = process(
        &client,
        extract_request(
            &source.path().join("rar4_multifile_lz.rar"),
            output.path(),
            ArchivePluginFormat::Rar,
            None,
        ),
    )
    .expect("plain RAR4 extraction");
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    // hello.txt, second.txt, zeros_64k.bin — LZ-compressed, host CRC verified.
    assert_eq!(response.files.len(), 3);
    assert_files_crc_correct(&response, output.path());
}

#[test]
fn extract_plain_rar5_multivolume_is_byte_correct() {
    let client = client();
    // The 6-volume plain RAR5 set (the par2-rar5 .part*.rar files) expands to a
    // single ~1.1 MiB mkv across volumes — proves multi-volume assembly + LZ.
    let source = stage_files(&par2_rar5_set());
    let output = tempfile::tempdir().unwrap();
    let response = process(
        &client,
        extract_request(
            &source.path().join("fixture_rar5_lz_plain.part1.rar"),
            output.path(),
            ArchivePluginFormat::Rar,
            None,
        ),
    )
    .expect("plain multi-volume RAR5 extraction");
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    assert_eq!(response.files.len(), 1);
    assert_eq!(response.expanded_bytes, Some(1_109_271));
    assert_files_crc_correct(&response, output.path());
}

// ── extraction: encrypted RAR (host AES + CRC ABI) ───────────────────────────

#[test]
fn extract_encrypted_rar4_password_states_and_bytes() {
    let client = client();
    let source = stage_files(&[corpus_dir().join("enc-rar").join("rar4_enc_store.rar")]);
    let archive = source.path().join("rar4_enc_store.rar");
    let expected = std::fs::read(corpus_dir().join("enc-rar").join("small.txt")).unwrap();

    // No password: data is encrypted -> PasswordRequired.
    let none = process(
        &client,
        extract_request(&archive, tempfile::tempdir().unwrap().path(), ArchivePluginFormat::Rar, None),
    )
    .expect("no-password run");
    assert_eq!(none.status, ArchivePluginStatus::PasswordRequired);

    // Wrong password: RAR4 has no password verifier, so it decrypts to garbage
    // and fails the member CRC -> Failed (documents the RAR4-vs-RAR5 difference).
    let wrong = process(
        &client,
        extract_request(
            &archive,
            tempfile::tempdir().unwrap().path(),
            ArchivePluginFormat::Rar,
            Some("not-the-password"),
        ),
    )
    .expect("wrong-password run");
    assert_eq!(wrong.status, ArchivePluginStatus::Failed);

    // Correct password: byte-correct against the original plaintext.
    let output = tempfile::tempdir().unwrap();
    let ok = process(
        &client,
        extract_request(&archive, output.path(), ArchivePluginFormat::Rar, Some(RAR_PASSWORD)),
    )
    .expect("correct-password run");
    assert_eq!(ok.status, ArchivePluginStatus::Ok);
    let disk = output.path().join(&ok.files[0].relative_path);
    assert_eq!(std::fs::read(&disk).unwrap(), expected, "encrypted RAR4 bytes");
    assert_files_crc_correct(&ok, output.path());
}

#[test]
fn extract_encrypted_rar5_lz_password_states_and_bytes() {
    let client = client();
    let source = stage_files(&[corpus_dir().join("enc-rar").join("rar5_enc_lz.rar")]);
    let archive = source.path().join("rar5_enc_lz.rar");
    let expected = std::fs::read(corpus_dir().join("enc-rar").join("compressible.txt")).unwrap();

    // No password -> PasswordRequired.
    let none = process(
        &client,
        extract_request(&archive, tempfile::tempdir().unwrap().path(), ArchivePluginFormat::Rar, None),
    )
    .expect("no-password run");
    assert_eq!(none.status, ArchivePluginStatus::PasswordRequired);

    // Wrong password: RAR5 carries a password check value -> PasswordInvalid.
    let wrong = process(
        &client,
        extract_request(
            &archive,
            tempfile::tempdir().unwrap().path(),
            ArchivePluginFormat::Rar,
            Some("not-the-password"),
        ),
    )
    .expect("wrong-password run");
    assert_eq!(wrong.status, ArchivePluginStatus::PasswordInvalid);

    // Correct password: LZ + host-AES decrypt, byte-correct against the original.
    let output = tempfile::tempdir().unwrap();
    let ok = process(
        &client,
        extract_request(&archive, output.path(), ArchivePluginFormat::Rar, Some(RAR_PASSWORD)),
    )
    .expect("correct-password run");
    assert_eq!(ok.status, ArchivePluginStatus::Ok);
    let disk = output.path().join(&ok.files[0].relative_path);
    assert_eq!(std::fs::read(&disk).unwrap(), expected, "encrypted RAR5 LZ bytes");
}

// ── extraction: ZIP ──────────────────────────────────────────────────────────

#[test]
fn extract_plain_zip_is_byte_correct() {
    let client = client();
    let payload = b"hello from a stored zip entry\n";
    let zip = build_stored_zip(&[("nested/hello.txt", payload)]);
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("sample.zip"), &zip).unwrap();
    let output = tempfile::tempdir().unwrap();

    let response = process(
        &client,
        extract_request(
            &source.path().join("sample.zip"),
            output.path(),
            ArchivePluginFormat::Zip,
            None,
        ),
    )
    .expect("zip extraction");
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    let disk = output.path().join("nested/hello.txt");
    assert_eq!(std::fs::read(&disk).unwrap(), payload, "zip bytes");
}

// ── PAR2 verify + repair-then-extract (COW staging) ──────────────────────────

#[test]
fn par2_verify_clean_set_reports_verified() {
    let client = client();
    let source = stage_files(&par2_rar5_set());
    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::VerifyRepairSet {
                source_dir: source.path().to_string_lossy().into_owned(),
                par2_path: None,
            },
        },
    )
    .expect("par2 verify");
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    let repair = response.repair.expect("verify carries a repair status");
    assert_eq!(repair.status, ArchivePluginRepairState::Verified);
}

#[test]
fn par2_verify_damaged_set_reports_repair_required() {
    let client = client();
    let source = stage_files(&par2_rar5_set());
    corrupt_part(&source.path().join("fixture_rar5_lz_plain.part3.rar"));

    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::VerifyRepairSet {
                source_dir: source.path().to_string_lossy().into_owned(),
                par2_path: None,
            },
        },
    )
    .expect("par2 verify (damaged)");
    // Recovery blocks are present, so damage is classified repairable.
    assert_eq!(response.status, ArchivePluginStatus::RepairRequired);
    assert_eq!(response.error_code.as_deref(), Some("par2_repair_required"));
}

#[test]
fn repair_then_extract_clean_set_through_cow_staging() {
    // The undamaged path exercises the full RepairThenExtract integration: the
    // adapter COW-clones the source into writable staging, the guest PAR2-gates
    // (Verified -> no reconstruction needed), then extracts the multi-volume
    // RAR5 from the staged copy. Proves the COW staging + repair-gate + extract
    // wiring end to end with a real Ok result.
    let client = client();
    let source = stage_files(&par2_rar5_set());
    let output = tempfile::tempdir().unwrap();
    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::RepairThenExtract {
                source_dir: source.path().to_string_lossy().into_owned(),
                output_dir: output.path().to_string_lossy().into_owned(),
                format: ArchivePluginFormat::Rar,
                par2_path: None,
                archive_path: Some("fixture_rar5_lz_plain.part1.rar".to_string()),
                password: None,
            },
        },
    )
    .expect("repair-then-extract (clean)");
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    assert_eq!(
        response.repair.as_ref().map(|r| r.status),
        Some(ArchivePluginRepairState::Verified)
    );
    assert_eq!(response.expanded_bytes, Some(1_109_271));
    assert_files_crc_correct(&response, output.path());
}

/// RFC 123 WP2.5 — damaged-member reconstruction now SUCCEEDS through the host
/// (this is the flipped §7.3 / §13.6-item-6 known-gap tripwire, formerly
/// `repair_then_extract_reconstruction_unsupported_under_wasip1`).
///
/// PAR2 reconstruction of a damaged member used to fail under single-threaded
/// `wasm32-wasip1`: weaver-par2 parallelizes the Reed–Solomon GF16 solve with
/// rayon, which cannot initialize a worker pool on wasip1, surfacing as an in-band
/// "operation not supported on this platform" error. WP2.5 moves the whole RS
/// solve to native host threads via the `scryer_par2_reconstruct` host fn: the
/// guest marshals the repair problem into the frozen PAR2-v2 descriptor and calls
/// the import; the host rebuilds the decode matrix with weaver-reed-solomon and
/// reconstructs on a large-stack worker; the guest re-verifies the reconstructed
/// on-disk set (fail-closed) before extraction proceeds. Same damage as the old
/// tripwire (one corrupted PAR2 slice in a volume) — opposite, now-passing outcome.
#[test]
fn repair_then_extract_reconstructs_damaged_member() {
    let client = client();
    let source = stage_files(&par2_rar5_set());
    // One 64 KiB PAR2 slice damaged (2 KiB flipped at offset 50k, within slice 0
    // of part3) — 1 missing block, well inside the 4-block recovery capacity.
    corrupt_part(&source.path().join("fixture_rar5_lz_plain.part3.rar"));
    let output = tempfile::tempdir().unwrap();

    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::RepairThenExtract {
                source_dir: source.path().to_string_lossy().into_owned(),
                output_dir: output.path().to_string_lossy().into_owned(),
                format: ArchivePluginFormat::Rar,
                par2_path: None,
                archive_path: Some("fixture_rar5_lz_plain.part1.rar".to_string()),
                password: None,
            },
        },
    )
    .expect("repair-then-extract (damaged) now reconstructs via the host, no host fault");

    // Reconstructed + re-verified: overall Ok, repair state Repaired (NOT
    // Verified — Verified would mean "no reconstruction was needed"). Repaired
    // proves the host actually ran the RS solve for the damaged slice.
    assert_eq!(response.status, ArchivePluginStatus::Ok);
    let repair = response.repair.as_ref().expect("repair status present");
    assert_eq!(
        repair.status,
        ArchivePluginRepairState::Repaired,
        "host must have reconstructed the damaged slice, not reported a clean verify"
    );
    // Exactly one 64 KiB slice was reconstructed.
    assert_eq!(repair.written_bytes, Some(65_536));
    // Extraction ran on the repaired staging and is byte-correct: the reconstructed
    // volume yields the exact expanded size and passes the RAR member CRC check.
    assert_eq!(response.expanded_bytes, Some(1_109_271));
    assert_files_crc_correct(&response, output.path());
}

/// RFC 123 WP2.5 — full damaged-repair end-to-end with byte-identity against a
/// known-good oracle. Damages three distinct PAR2 slices in one volume (3 missing
/// blocks, within the 4-block recovery cap), reconstructs through the host, then
/// asserts the repaired multi-volume extraction is byte-for-byte identical to a
/// clean extraction of the same set (captured in-test as the oracle).
#[test]
fn repair_then_extract_damaged_multivolume_is_byte_identical_to_original() {
    let client = client();

    // Oracle: extract the CLEAN set first and snapshot the exact output bytes.
    let clean_src = stage_files(&par2_rar5_set());
    let clean_out = tempfile::tempdir().unwrap();
    let clean = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::RepairThenExtract {
                source_dir: clean_src.path().to_string_lossy().into_owned(),
                output_dir: clean_out.path().to_string_lossy().into_owned(),
                format: ArchivePluginFormat::Rar,
                par2_path: None,
                archive_path: Some("fixture_rar5_lz_plain.part1.rar".to_string()),
                password: None,
            },
        },
    )
    .expect("clean repair-then-extract (oracle)");
    assert_eq!(clean.status, ArchivePluginStatus::Ok);
    assert_eq!(
        clean.repair.as_ref().map(|r| r.status),
        Some(ArchivePluginRepairState::Verified),
        "clean set needs no reconstruction"
    );
    let oracle: Vec<(String, Vec<u8>)> = clean
        .files
        .iter()
        .map(|file| {
            let bytes = std::fs::read(clean_out.path().join(&file.relative_path))
                .expect("read oracle output");
            (file.relative_path.clone(), bytes)
        })
        .collect();
    assert!(!oracle.is_empty(), "oracle produced output");

    // Damage three distinct 64 KiB slices in one volume (3 missing blocks <= 4 cap).
    let damaged_src = stage_files(&par2_rar5_set());
    corrupt_slices(&damaged_src.path().join("fixture_rar5_lz_plain.part3.rar"), 3);
    let damaged_out = tempfile::tempdir().unwrap();
    let repaired = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::RepairThenExtract {
                source_dir: damaged_src.path().to_string_lossy().into_owned(),
                output_dir: damaged_out.path().to_string_lossy().into_owned(),
                format: ArchivePluginFormat::Rar,
                par2_path: None,
                archive_path: Some("fixture_rar5_lz_plain.part1.rar".to_string()),
                password: None,
            },
        },
    )
    .expect("damaged repair-then-extract reconstructs via the host");

    // Host invoked reconstruction: Repaired, and exactly three slices (3 * 64 KiB)
    // were rebuilt — proves the missing data was reconstructed, not skipped.
    assert_eq!(repaired.status, ArchivePluginStatus::Ok);
    let repair = repaired.repair.as_ref().expect("repair status present");
    assert_eq!(repair.status, ArchivePluginRepairState::Repaired);
    assert_eq!(
        repair.written_bytes,
        Some(3 * 65_536),
        "three damaged slices must be reconstructed"
    );
    assert_eq!(repaired.expanded_bytes, Some(1_109_271));

    // Byte-identity: the repaired extraction matches the clean oracle exactly.
    assert_eq!(repaired.files.len(), oracle.len());
    for (path, want) in &oracle {
        let got = std::fs::read(damaged_out.path().join(path))
            .unwrap_or_else(|error| panic!("read repaired output {path}: {error}"));
        assert_eq!(
            &got, want,
            "repaired output {path} must be byte-identical to the original"
        );
    }
    assert_files_crc_correct(&repaired, damaged_out.path());
}

/// RFC 123 WP2.5 — fail-closed when damage exceeds recovery capacity. Corrupts
/// five distinct PAR2 slices (part3's three + two of part4's) against only four
/// recovery blocks, so the repairability gate classifies the set `Insufficient`
/// before any reconstruction runs. `repair_then_extract` gates extraction on the
/// repair status being `Ok`, so nothing is extracted and no output is written.
///
/// NOTE: "beyond capacity" maps to the (non-`Ok`) `RepairRequired` status via the
/// repairability gate, NOT `RepairFailed`. `RepairFailed` is reserved for a
/// reconstruct/plan/post-verify failure or a resource limit. Both are non-`Ok`
/// and both fail closed. The narrower "reconstruct silently produced wrong bytes
/// -> post-repair re-verify catches it" branch is covered-by-design by the
/// plugin's fail-closed re-verify after `execute_repair_with_solver`; inducing a
/// wrong-bytes reconstruct needs fault injection into the host solve and is not
/// simulated here.
#[test]
fn repair_then_extract_beyond_recovery_capacity_fails_closed() {
    let client = client();
    let source = stage_files(&par2_rar5_set());
    // 5 distinct damaged slices > 4 recovery blocks -> Insufficient.
    corrupt_slices(&source.path().join("fixture_rar5_lz_plain.part3.rar"), 3);
    corrupt_slices(&source.path().join("fixture_rar5_lz_plain.part4.rar"), 2);
    let output = tempfile::tempdir().unwrap();

    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::RepairThenExtract {
                source_dir: source.path().to_string_lossy().into_owned(),
                output_dir: output.path().to_string_lossy().into_owned(),
                format: ArchivePluginFormat::Rar,
                par2_path: None,
                archive_path: Some("fixture_rar5_lz_plain.part1.rar".to_string()),
                password: None,
            },
        },
    )
    .expect("beyond-capacity repair-then-extract returns an in-band failure, not a host fault");

    // Repairability gate tripped before any reconstruction: non-Ok status.
    assert_eq!(response.status, ArchivePluginStatus::RepairRequired);
    assert_eq!(
        response.error_code.as_deref(),
        Some("par2_insufficient_recovery_data")
    );
    assert_eq!(
        response.repair.as_ref().map(|r| r.status),
        Some(ArchivePluginRepairState::InsufficientRecoveryData)
    );
    // Fail-closed: extraction is gated on Ok, so nothing was extracted or written.
    assert!(
        response.files.is_empty(),
        "no files may be reported when repair fails closed"
    );
    assert_eq!(
        count_files(output.path()),
        0,
        "no bytes may be written to the output dir when repair fails closed"
    );
}

// ── inspect (documents current plugin behavior) ──────────────────────────────

#[test]
fn inspect_is_reported_unsupported() {
    let client = client();
    let source = stage_files(&[corpus_dir().join("plain-rar4").join("rar4_store.rar")]);
    let response = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::Inspect {
                source_dir: source.path().to_string_lossy().into_owned(),
                archive_path: None,
            },
        },
    )
    .expect("inspect");
    assert_eq!(response.status, ArchivePluginStatus::UnsupportedFormat);
}

// ── sandbox: path-escape rejection ───────────────────────────────────────────

#[test]
fn host_rejects_path_escape_in_request() {
    // The adapter's request-path guard (`map_child_path` / safe relative path)
    // rejects a `..` archive_path before the guest ever runs.
    let client = client();
    let source = stage_files(&[corpus_dir().join("plain-rar4").join("rar4_store.rar")]);
    let result = process(
        &client,
        ArchivePluginProcessRequest {
            operation: ArchivePluginOperation::Inspect {
                source_dir: source.path().to_string_lossy().into_owned(),
                archive_path: Some("../../../../etc/passwd".to_string()),
            },
        },
    );
    assert!(
        matches!(result, Err(AppError::Validation(_))),
        "path-escaping request must be rejected at the host boundary: {result:?}"
    );
}

#[test]
fn guest_rejects_path_escaping_zip_member() {
    // A crafted ZIP whose member name escapes the output root must be refused by
    // the guest's path guard, and the WASI preopen is the backstop — nothing may
    // be written outside the output root.
    let client = client();
    let zip = build_stored_zip(&[("../escape.txt", b"pwned")]);
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("evil.zip"), &zip).unwrap();
    let output = tempfile::tempdir().unwrap();

    let response = process(
        &client,
        extract_request(
            &source.path().join("evil.zip"),
            output.path(),
            ArchivePluginFormat::Zip,
            None,
        ),
    )
    .expect("malicious zip returns an in-band failure, not a host fault");
    assert_eq!(response.status, ArchivePluginStatus::Failed);
    assert_eq!(response.error_code.as_deref(), Some("unsafe_path"));
    // Nothing escaped: neither a sibling of the output root nor its parent holds
    // the smuggled file.
    let output_parent = output.path().parent().unwrap();
    assert!(!output_parent.join("escape.txt").exists());
    assert!(!output.path().join("escape.txt").exists());
}

// ── ABI drift tripwire (RFC §5 crypto pair + WP2.5 reconstruct dispatch) ──────

#[test]
fn abi_imports_match_frozen_contract() {
    let wasm = plugin_wasm();
    let engine = Engine::default();
    let module = Module::from_binary(&engine, &wasm).expect("compile real artifact");

    let mut host_user: Vec<(String, ExternType)> = Vec::new();
    for import in module.imports() {
        match import.module() {
            "extism:host/user" => host_user.push((import.name().to_string(), import.ty())),
            "wasi_snapshot_preview1" => {}
            other => panic!(
                "unexpected import module '{other}' for '{}': only extism:host/user + \
                 wasi_snapshot_preview1 are allowed (RFC §5)",
                import.name()
            ),
        }
    }
    host_user.sort_by(|a, b| a.0.cmp(&b.0));
    let names: Vec<&str> = host_user.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        names,
        vec!["scryer_aes_cbc_decrypt", "scryer_crc32", "scryer_par2_reconstruct"],
        "frozen host ABI drifted: extism:host/user imports must be exactly the two §5 crypto \
         functions plus the WP2.5 scryer_par2_reconstruct reconstruct dispatch (RFC §13.6 item 6)"
    );

    // Signatures: all params + result are i64 (the §5 crypto pair plus the WP2.5
    // reconstruct dispatch, `scryer_par2_reconstruct(desc_ptr, desc_len) -> i64`).
    for (name, arity) in [
        ("scryer_aes_cbc_decrypt", 5usize),
        ("scryer_crc32", 3usize),
        ("scryer_par2_reconstruct", 2usize),
    ] {
        let ty = &host_user.iter().find(|(n, _)| n == name).unwrap().1;
        let ExternType::Func(func) = ty else {
            panic!("{name} must be a function import");
        };
        let params: Vec<ValType> = func.params().collect();
        let results: Vec<ValType> = func.results().collect();
        assert_eq!(params.len(), arity, "{name} arity");
        assert!(params.iter().all(|p| matches!(p, ValType::I64)), "{name} params must be i64");
        assert_eq!(results.len(), 1, "{name} returns one value");
        assert!(matches!(results[0], ValType::I64), "{name} returns i64");
    }

    // Command exports: `_start` (func) + `memory` (linear memory).
    let mut has_start = false;
    let mut has_memory = false;
    for export in module.exports() {
        match export.name() {
            "_start" => has_start = matches!(export.ty(), ExternType::Func(_)),
            "memory" => has_memory = matches!(export.ty(), ExternType::Memory(_)),
            _ => {}
        }
    }
    assert!(has_start, "artifact must export _start (wasip1 command)");
    assert!(has_memory, "artifact must export a linear memory named 'memory'");
}

// ── benchmark (HI6, informal) ────────────────────────────────────────────────

/// Informal end-to-end throughput for encrypted-RAR extraction through the host,
/// feeding the memory-cap default (RFC §13.1) and the WP6 gate. Run with:
/// `cargo test -p scryer-plugins archive_real_artifact_tests::benchmark -- --ignored --nocapture`.
/// NOTE: the host compiles the ~2.2 MiB module per request (WP6 caches it later),
/// so wall-clock is dominated by instantiate/compile, not AES throughput. An
/// in-wasm-vs-host-AES comparator build is out of scope for WP2.
#[test]
#[ignore = "informal benchmark; run with --ignored --nocapture"]
fn benchmark_encrypted_rar_extraction() {
    let client = client();
    let source = stage_files(&[corpus_dir().join("enc-rar").join("rar5_enc_lz.rar")]);
    let archive = source.path().join("rar5_enc_lz.rar");
    let expected_len = std::fs::metadata(corpus_dir().join("enc-rar").join("compressible.txt"))
        .unwrap()
        .len();

    let runs = 5;
    let mut best = f64::MAX;
    let mut total = 0.0;
    for _ in 0..runs {
        let output = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let response = process(
            &client,
            extract_request(&archive, output.path(), ArchivePluginFormat::Rar, Some(RAR_PASSWORD)),
        )
        .expect("benchmark extraction");
        let elapsed = started.elapsed().as_secs_f64();
        assert_eq!(response.status, ArchivePluginStatus::Ok);
        best = best.min(elapsed);
        total += elapsed;
    }
    let avg = total / runs as f64;
    let mib = expected_len as f64 / (1024.0 * 1024.0);
    eprintln!(
        "BENCHMARK encrypted-RAR (rar5_enc_lz -> {expected_len} B / {mib:.3} MiB expanded, {runs} runs):"
    );
    eprintln!(
        "  wall-clock best={:.1} ms avg={:.1} ms  |  expanded throughput best={:.1} MiB/s (per-request module compile dominates)",
        best * 1000.0,
        avg * 1000.0,
        mib / best,
    );
}

// ── helpers: fixture damage + hand-built ZIP ─────────────────────────────────

/// Flip a 2 KiB run inside a PAR2-covered volume to damage ~1 recovery block.
fn corrupt_part(path: &Path) {
    let mut data = std::fs::read(path).expect("read volume to corrupt");
    for byte in data.iter_mut().skip(50_000).take(2_000) {
        *byte ^= 0xff;
    }
    std::fs::write(path, &data).expect("write corrupted volume");
}

/// Damage `count` distinct 64 KiB PAR2 slices of a volume by flipping a 2 KiB run
/// deep inside each (offset +40 KiB, which for slice 0 stays clear of the 16 KiB
/// file-id region so PAR2 still matches the file and flags the slice by its own
/// checksum). The 65536 slice size matches the corpus main packet; use `count`
/// against the set's 4-block recovery capacity to size damage above or below it.
fn corrupt_slices(path: &Path, count: usize) {
    const SLICE: usize = 65_536;
    let mut data = std::fs::read(path).expect("read volume to corrupt");
    for slice in 0..count {
        let start = slice * SLICE + 40_000;
        let end = start + 2_000;
        assert!(
            end <= data.len(),
            "slice {slice} out of range for {} ({} bytes)",
            path.display(),
            data.len()
        );
        for byte in &mut data[start..end] {
            *byte ^= 0xff;
        }
    }
    std::fs::write(path, &data).expect("write corrupted volume");
}

/// Count regular files anywhere under `dir` (recursive). Proves the fail-closed
/// path wrote nothing to the extraction output.
fn count_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            count += count_files(&path);
        } else if path.is_file() {
            count += 1;
        }
    }
    count
}

/// Build a minimal, valid STORED (uncompressed) ZIP in-test — the `zip` crate is
/// not in the scryer lock, so we hand-assemble local headers, the central
/// directory, and the end-of-central-directory record. Good enough for the
/// plugin's `zip::ZipArchive` reader and its `enclosed_name` path guard.
fn build_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(data);
        hasher.finalize()
    }

    let mut out = Vec::new();
    let mut offsets = Vec::new();
    for (name, data) in entries {
        let name = name.as_bytes();
        offsets.push(out.len() as u32);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // local file header sig
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date (1980-01-01)
        out.extend_from_slice(&crc32(data).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name);
        out.extend_from_slice(data);
    }

    let central_start = out.len() as u32;
    let mut central = Vec::new();
    for ((name, data), offset) in entries.iter().zip(&offsets) {
        let name = name.as_bytes();
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes()); // central dir sig
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc32(data).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed
        central.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra len
        central.extend_from_slice(&0u16.to_le_bytes()); // comment len
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes()); // local header offset
        central.extend_from_slice(name);
    }
    let central_len = central.len() as u32;
    out.extend_from_slice(&central);

    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes()); // EOCD sig
    out.extend_from_slice(&0u16.to_le_bytes()); // disk number
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with CD
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes()); // CD records here
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes()); // total CD records
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}
