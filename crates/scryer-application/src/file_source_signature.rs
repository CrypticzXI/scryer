use crate::AppResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSourceSignature {
    pub scheme: String,
    pub value: String,
}

#[cfg(windows)]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "windows_last_write_100ns_v1";
#[cfg(unix)]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "unix_mtime_nsec_v1";
#[cfg(all(not(unix), not(windows)))]
pub const MEDIA_FILE_SOURCE_SIGNATURE_SCHEME: &str = "system_time_nsec_v1";

pub fn file_source_signature_from_metadata(
    metadata: &std::fs::Metadata,
) -> AppResult<FileSourceSignature> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value: metadata.last_write_time().to_string(),
        })
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value: format!("{}:{}", metadata.mtime(), metadata.mtime_nsec()),
        })
    }

    #[cfg(all(not(unix), not(windows)))]
    {
        let modified = metadata.modified().map_err(|error| {
            crate::AppError::Repository(format!("failed to read media file modified time: {error}"))
        })?;
        let value = match modified.duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
            Err(error) => {
                let duration = error.duration();
                format!("-{}:{}", duration.as_secs(), duration.subsec_nanos())
            }
        };

        Ok(FileSourceSignature {
            scheme: MEDIA_FILE_SOURCE_SIGNATURE_SCHEME.to_string(),
            value,
        })
    }
}
