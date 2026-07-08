use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, ArchiveExtractorClient};
use scryer_plugin_sdk::{
    ArchivePluginProcessRequest, ArchivePluginProcessResponse, EXPORT_ARCHIVE_PROCESS,
    PluginDescriptor,
};

use crate::loader::build_archive_plugin;
use crate::types::decode_plugin_result;

const GUEST_SOURCE_ROOT: &str = "/scryer/source";
const GUEST_OUTPUT_ROOT: &str = "/scryer/output";
const ARCHIVE_PROCESS_TIMEOUT_SECONDS: u64 = 60 * 60;

pub struct WasmArchiveExtractorClient {
    wasm_bytes: Arc<Vec<u8>>,
}

impl WasmArchiveExtractorClient {
    pub fn new(wasm_bytes: Vec<u8>, descriptor: PluginDescriptor) -> AppResult<Self> {
        let _ = descriptor;

        Ok(Self {
            wasm_bytes: Arc::new(wasm_bytes),
        })
    }
}

#[async_trait]
impl ArchiveExtractorClient for WasmArchiveExtractorClient {
    async fn process(
        &self,
        request: ArchivePluginProcessRequest,
    ) -> AppResult<ArchivePluginProcessResponse> {
        let prepared = PreparedArchiveRequest::new(request)?;
        let input = serde_json::to_string(&prepared.request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize archive process request: {error}"
            ))
        })?;

        let wasm_bytes = Arc::clone(&self.wasm_bytes);
        let output = tokio::task::spawn_blocking(move || {
            let manifest = prepared.manifest((*wasm_bytes).clone());
            let mut plugin = build_archive_plugin(manifest).map_err(|error| {
                AppError::Repository(format!(
                    "failed to instantiate WASM archive extractor plugin: {error}"
                ))
            })?;
            plugin
                .call::<&str, String>(EXPORT_ARCHIVE_PROCESS, &input)
                .map_err(|error| plugin_call_error(&format!("{EXPORT_ARCHIVE_PROCESS}()"), error))
        })
        .await
        .map_err(|error| AppError::Repository(format!("plugin task panicked: {error}")))??;

        decode_plugin_result(&output, EXPORT_ARCHIVE_PROCESS)
    }
}

struct PreparedArchiveRequest {
    request: ArchivePluginProcessRequest,
    source_root: Option<PathBuf>,
    source_writable: bool,
    output_root: Option<PathBuf>,
    _staging_root: Option<tempfile::TempDir>,
}

impl PreparedArchiveRequest {
    fn new(request: ArchivePluginProcessRequest) -> AppResult<Self> {
        use scryer_plugin_sdk::ArchivePluginOperation;

        match request.operation {
            ArchivePluginOperation::Inspect {
                source_dir,
                archive_path,
            } => {
                let source_root = PathBuf::from(source_dir);
                let archive_path = archive_path
                    .map(|path| map_child_path(Path::new(&source_root), Path::new(&path)))
                    .transpose()?;
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::Inspect {
                            source_dir: GUEST_SOURCE_ROOT.to_string(),
                            archive_path,
                        },
                    },
                    source_root: Some(source_root),
                    source_writable: false,
                    output_root: None,
                    _staging_root: None,
                })
            }
            ArchivePluginOperation::ExtractArchive {
                archive_path,
                output_dir,
                format,
                password,
            } => {
                let archive_path = PathBuf::from(archive_path);
                let source_root = archive_path.parent().unwrap_or_else(|| Path::new("."));
                let source_root = source_root.to_path_buf();
                let guest_archive_path = map_child_path(&source_root, &archive_path)?;
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::ExtractArchive {
                            archive_path: guest_archive_path,
                            output_dir: GUEST_OUTPUT_ROOT.to_string(),
                            format,
                            password,
                        },
                    },
                    source_root: Some(source_root),
                    source_writable: false,
                    output_root: Some(PathBuf::from(output_dir)),
                    _staging_root: None,
                })
            }
            ArchivePluginOperation::VerifyRepairSet {
                source_dir,
                par2_path,
            } => {
                let source_root = PathBuf::from(source_dir);
                let par2_path = par2_path
                    .map(|path| map_child_path(Path::new(&source_root), Path::new(&path)))
                    .transpose()?;
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::VerifyRepairSet {
                            source_dir: GUEST_SOURCE_ROOT.to_string(),
                            par2_path,
                        },
                    },
                    source_root: Some(source_root),
                    source_writable: false,
                    output_root: None,
                    _staging_root: None,
                })
            }
            ArchivePluginOperation::RepairThenExtract {
                source_dir,
                output_dir,
                format,
                par2_path,
                archive_path,
                password,
            } => {
                let source_root = PathBuf::from(source_dir);
                let par2_path = par2_path
                    .map(|path| map_child_path(Path::new(&source_root), Path::new(&path)))
                    .transpose()?;
                let archive_path = archive_path
                    .map(|path| map_child_path(Path::new(&source_root), Path::new(&path)))
                    .transpose()?;
                let staging_root = prepare_repair_staging(&source_root)?;
                let staged_source_root = staging_root.path().to_path_buf();
                Ok(Self {
                    request: ArchivePluginProcessRequest {
                        operation: ArchivePluginOperation::RepairThenExtract {
                            source_dir: GUEST_SOURCE_ROOT.to_string(),
                            output_dir: GUEST_OUTPUT_ROOT.to_string(),
                            format,
                            par2_path,
                            archive_path,
                            password,
                        },
                    },
                    source_root: Some(staged_source_root),
                    source_writable: true,
                    output_root: Some(PathBuf::from(output_dir)),
                    _staging_root: Some(staging_root),
                })
            }
        }
    }

    fn manifest(&self, wasm_bytes: Vec<u8>) -> extism::Manifest {
        let mut manifest = extism::Manifest::new([extism::Wasm::data(wasm_bytes)])
            .with_timeout(Duration::from_secs(ARCHIVE_PROCESS_TIMEOUT_SECONDS));
        if let Some(source_root) = &self.source_root {
            let source_path = if self.source_writable {
                source_root.display().to_string()
            } else {
                format!("ro:{}", source_root.display())
            };
            manifest = manifest.with_allowed_path(source_path, GUEST_SOURCE_ROOT);
        }
        if let Some(output_root) = &self.output_root {
            manifest =
                manifest.with_allowed_path(output_root.display().to_string(), GUEST_OUTPUT_ROOT);
        }
        manifest
    }
}

fn prepare_repair_staging(source_root: &Path) -> AppResult<tempfile::TempDir> {
    let parent = source_root.parent().unwrap_or_else(|| Path::new("."));
    let staging_root = tempfile::Builder::new()
        .prefix(".scryer-par2-stage-")
        .tempdir_in(parent)
        .map_err(|error| {
            AppError::Repository(format!(
                "failed to create archive repair staging directory near '{}': {error}",
                source_root.display()
            ))
        })?;

    clone_directory_contents_cow(source_root, staging_root.path())?;
    Ok(staging_root)
}

fn clone_directory_contents_cow(source: &Path, destination: &Path) -> AppResult<()> {
    for entry in fs::read_dir(source).map_err(|error| {
        AppError::Repository(format!(
            "failed to read archive repair source '{}': {error}",
            source.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            AppError::Repository(format!(
                "failed to read archive repair source '{}': {error}",
                source.display()
            ))
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(|error| {
            AppError::Repository(format!(
                "failed to inspect archive repair source '{}': {error}",
                source_path.display()
            ))
        })?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            return Err(AppError::Validation(format!(
                "archive repair staging refuses symbolic link '{}'",
                source_path.display()
            )));
        }
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path).map_err(|error| {
                AppError::Repository(format!(
                    "failed to create archive repair staging directory '{}': {error}",
                    destination_path.display()
                ))
            })?;
            clone_directory_contents_cow(&source_path, &destination_path)?;
            continue;
        }
        if file_type.is_file() {
            clone_file_cow(&source_path, &destination_path).map_err(|error| {
                AppError::Repository(format!(
                    "archive PAR2 repair requires copy-on-write staging; failed to reflink '{}' to '{}': {error}",
                    source_path.display(),
                    destination_path.display()
                ))
            })?;
            continue;
        }

        return Err(AppError::Validation(format!(
            "archive repair staging refuses special file '{}'",
            source_path.display()
        )));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn clone_file_cow(source: &Path, destination: &Path) -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;

    const FICLONE: libc::c_ulong = 0x4004_9409;

    let source_file = fs::File::open(source)?;
    let destination_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    let result = unsafe {
        libc::ioctl(
            destination_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        let _ = fs::remove_file(destination);
        Err(error)
    }
}

#[cfg(target_os = "macos")]
fn clone_file_cow(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source path contains NUL"))?;
    let destination = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "destination path contains NUL")
    })?;
    let result = unsafe { libc::clonefile(source.as_ptr(), destination.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn clone_file_cow(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "copy-on-write file cloning is not implemented for this platform",
    ))
}

fn map_child_path(root: &Path, path: &Path) -> AppResult<String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let relative = path.strip_prefix(root).map_err(|_| {
        AppError::Validation(format!(
            "archive plugin path '{}' is outside allowed root '{}'",
            path.display(),
            root.display()
        ))
    })?;
    if !is_safe_relative_plugin_path(relative) {
        return Err(AppError::Validation(format!(
            "archive plugin path '{}' is not a safe relative path",
            path.display()
        )));
    }
    let guest_path = Path::new(GUEST_SOURCE_ROOT).join(relative);
    Ok(guest_path.to_string_lossy().into_owned())
}

fn is_safe_relative_plugin_path(path: &Path) -> bool {
    path.components().all(|component| {
        matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )
    })
}

fn plugin_call_error(operation: &str, error: extism::Error) -> AppError {
    let root_cause = error.root_cause().to_string();
    let detail = if root_cause.trim().is_empty() || root_cause == error.to_string() {
        error.to_string()
    } else {
        root_cause
    };

    AppError::Repository(format!(
        "archive extractor plugin {operation} failed: {detail}"
    ))
}
