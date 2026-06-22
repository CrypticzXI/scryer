//! Filesystem integrity helpers shared across file move/copy/delete surfaces.
//!
//! Whenever a file is copied (e.g. a cross-device move, or moving into/out of the
//! recycle bin) and the source is subsequently removed, we compare the same
//! sampled content proof used by import cleanup: file size plus a BLAKE3 digest of
//! the first and last sample windows. This is intentionally not a full-file hash.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{AppError, AppResult};
use scryer_domain::ImportContentProof;

pub const IMPORT_CONTENT_PROOF_SAMPLE_BYTES: usize = 1024 * 1024;

/// Compute the import content proof for `path`: file size plus a BLAKE3 digest
/// over the first and last `IMPORT_CONTENT_PROOF_SAMPLE_BYTES` bytes.
pub fn import_content_proof(path: &Path) -> AppResult<ImportContentProof> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open import content proof path: {}: {error}",
            path.display()
        ))
    })?;
    let size_bytes = file
        .metadata()
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to stat import content proof path: {}: {error}",
                path.display()
            ))
        })?
        .len();

    let first_len = size_bytes.min(IMPORT_CONTENT_PROOF_SAMPLE_BYTES as u64) as usize;
    let mut sample = Vec::with_capacity(first_len.saturating_mul(2));
    read_import_content_sample(&mut file, path, 0, first_len, &mut sample)?;

    let remaining_after_first = size_bytes.saturating_sub(first_len as u64);
    let last_len = remaining_after_first.min(IMPORT_CONTENT_PROOF_SAMPLE_BYTES as u64) as usize;
    if last_len > 0 {
        let last_offset = size_bytes - last_len as u64;
        read_import_content_sample(&mut file, path, last_offset, last_len, &mut sample)?;
    }

    Ok(ImportContentProof {
        size_bytes,
        sample_bytes: sample.len() as u64,
        sample_blake3: blake3::hash(&sample).to_hex().to_string(),
    })
}

fn read_import_content_sample(
    file: &mut std::fs::File,
    path: &Path,
    offset: u64,
    len: usize,
    sample: &mut Vec<u8>,
) -> AppResult<()> {
    file.seek(SeekFrom::Start(offset)).map_err(|error| {
        AppError::Repository(format!(
            "failed to seek import content proof path: {}: {error}",
            path.display()
        ))
    })?;
    let start = sample.len();
    sample.resize(start + len, 0);
    file.read_exact(&mut sample[start..]).map_err(|error| {
        AppError::Repository(format!(
            "failed to read import content proof path: {}: {error}",
            path.display()
        ))
    })
}

pub fn verify_content_proofs(
    source_label: &str,
    source: &ImportContentProof,
    dest_label: &str,
    dest: &ImportContentProof,
) -> AppResult<()> {
    if source != dest {
        return Err(AppError::Repository(format!(
            "copy verification failed: {source_label} (size={} sample_bytes={} sample_blake3={}) is not identical to {dest_label} (size={} sample_bytes={} sample_blake3={})",
            source.size_bytes,
            source.sample_bytes,
            source.sample_blake3,
            dest.size_bytes,
            dest.sample_bytes,
            dest.sample_blake3
        )));
    }
    Ok(())
}

/// Verify that `dest` has the same sampled import content proof as `source`.
pub fn verify_same_file(source: &Path, dest: &Path) -> AppResult<()> {
    let source_proof = import_content_proof(source)?;
    let dest_proof = import_content_proof(dest)?;
    verify_content_proofs(
        &source.display().to_string(),
        &source_proof,
        &dest.display().to_string(),
        &dest_proof,
    )
}

/// Async wrapper that runs the (blocking) sampled verification on a blocking
/// thread so large media files do not stall the async runtime.
pub async fn verify_same_file_async(source: &Path, dest: &Path) -> AppResult<()> {
    let source = source.to_path_buf();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || verify_same_file(&source, &dest))
        .await
        .map_err(|error| {
            AppError::Repository(format!("content verification task failed to join: {error}"))
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_files_verify_ok() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"the quick brown fox").unwrap();
        std::fs::write(&b, b"the quick brown fox").unwrap();
        verify_same_file(&a, &b).expect("identical files should verify");
    }

    #[test]
    fn changed_tail_sample_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 4096;
        let mut left = vec![b'a'; size];
        let mut right = left.clone();
        left[size - 1] = b'b';
        right[size - 1] = b'c';
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();
        assert!(
            verify_same_file(&a, &b).is_err(),
            "same-size content with changed tail sample must fail verification"
        );
    }

    #[test]
    fn same_sample_blake3_different_size_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let first_sample = vec![b'f'; IMPORT_CONTENT_PROOF_SAMPLE_BYTES];
        let last_sample = vec![b'l'; IMPORT_CONTENT_PROOF_SAMPLE_BYTES];
        let mut left = Vec::new();
        left.extend_from_slice(&first_sample);
        left.extend_from_slice(&vec![b'a'; 128]);
        left.extend_from_slice(&last_sample);
        let mut right = Vec::new();
        right.extend_from_slice(&first_sample);
        right.extend_from_slice(&vec![b'b'; 256]);
        right.extend_from_slice(&last_sample);
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();

        let left_proof = import_content_proof(&a).unwrap();
        let right_proof = import_content_proof(&b).unwrap();
        assert_eq!(left_proof.sample_blake3, right_proof.sample_blake3);
        assert_eq!(left_proof.sample_bytes, right_proof.sample_bytes);
        assert_ne!(left_proof.size_bytes, right_proof.size_bytes);
        assert!(
            verify_same_file(&a, &b).is_err(),
            "size_bytes must be part of sampled proof verification"
        );
    }

    #[test]
    fn same_size_unsampled_middle_change_matches_proof() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let size = IMPORT_CONTENT_PROOF_SAMPLE_BYTES * 3;
        let mut left = vec![b'a'; size];
        let mut right = left.clone();
        left[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 17] = b'b';
        right[IMPORT_CONTENT_PROOF_SAMPLE_BYTES + 17] = b'c';
        std::fs::write(&a, left).unwrap();
        std::fs::write(&b, right).unwrap();

        verify_same_file(&a, &b)
            .expect("the sampled proof intentionally ignores unsampled middle bytes");
    }

    #[test]
    fn truncated_copy_fails() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, vec![7u8; 4096]).unwrap();
        std::fs::write(&b, vec![7u8; 2048]).unwrap();
        assert!(
            verify_same_file(&a, &b).is_err(),
            "truncated copy must fail verification"
        );
    }
}
