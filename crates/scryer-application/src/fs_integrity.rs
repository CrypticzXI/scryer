//! Filesystem integrity helpers shared across file move/copy/delete surfaces.
//!
//! Whenever a file is copied (e.g. a cross-device move, or moving into/out of the
//! recycle bin) and the source is subsequently removed, we must prove the
//! destination is a faithful copy *before* deleting the source. A size-only check
//! is not sufficient to guarantee "it is the same file", so we require a
//! whole-file blake3 content match. This is the single shared implementation used
//! by the renamer (`scryer-infrastructure`) and the recycle bin
//! (`scryer-application`).

use std::io::Read;
use std::path::Path;

use crate::{AppError, AppResult};

const VERIFY_READ_BUFFER_BYTES: usize = 1 << 20; // 1 MiB

/// Compute `(size_bytes, blake3-hex)` over the entire contents of `path`.
pub fn file_content_digest(path: &Path) -> AppResult<(u64, String)> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::Repository(format!(
            "failed to open {} for content verification: {error}",
            path.display()
        ))
    })?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; VERIFY_READ_BUFFER_BYTES];
    let mut size_bytes = 0u64;
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            AppError::Repository(format!(
                "failed to read {} during content verification: {error}",
                path.display()
            ))
        })?;
        if read == 0 {
            break;
        }
        size_bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((size_bytes, hasher.finalize().to_hex().to_string()))
}

/// Verify that `dest` is a byte-for-byte copy of `source` (size + whole-file
/// blake3). Returns an error if the contents differ or either file cannot be read.
pub fn verify_same_file(source: &Path, dest: &Path) -> AppResult<()> {
    let (source_size, source_hash) = file_content_digest(source)?;
    let (dest_size, dest_hash) = file_content_digest(dest)?;
    if source_size != dest_size || source_hash != dest_hash {
        return Err(AppError::Repository(format!(
            "copy verification failed: {} (size={source_size} blake3={source_hash}) is not identical to {} (size={dest_size} blake3={dest_hash})",
            source.display(),
            dest.display()
        )));
    }
    Ok(())
}

/// Async wrapper that runs the (blocking) whole-file verification on a blocking
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
    fn different_contents_same_size_fail() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        std::fs::write(&a, b"aaaaaaaa").unwrap();
        std::fs::write(&b, b"aaaaAaaa").unwrap();
        assert!(
            verify_same_file(&a, &b).is_err(),
            "same-size but different content must fail verification"
        );
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
