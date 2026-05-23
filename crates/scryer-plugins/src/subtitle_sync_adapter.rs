use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{AppError, AppResult, SubtitleSyncClient, SubtitleSyncJob};

use crate::loader::build_plugin;
use crate::types::{
    AudioStreamSelector, EXPORT_SUBSYNC_ALIGN, PluginDescriptor, SubtitleSyncAlignInputRef,
    SubtitleSyncAlignRequest, SubtitleSyncAlignResponse, decode_plugin_result,
};

const GUEST_INPUT_ROOT: &str = "/input";
const GUEST_SCRATCH_ROOT: &str = "/scratch";
const SUBTITLE_SYNC_TIMEOUT_SECONDS: u64 = 60 * 60;

pub struct WasmSubtitleSyncClient {
    wasm_bytes: Arc<Vec<u8>>,
    descriptor: PluginDescriptor,
}

impl WasmSubtitleSyncClient {
    pub fn new(wasm_bytes: Vec<u8>, descriptor: PluginDescriptor) -> Self {
        Self {
            wasm_bytes: Arc::new(wasm_bytes),
            descriptor,
        }
    }
}

#[async_trait]
impl SubtitleSyncClient for WasmSubtitleSyncClient {
    async fn align_subtitle(&self, job: SubtitleSyncJob) -> AppResult<SubtitleSyncAlignResponse> {
        let scratch_dir = tempfile::tempdir().map_err(|error| {
            AppError::Repository(format!(
                "failed to create subtitle sync scratch directory: {error}"
            ))
        })?;
        let input_path = job.input_path.clone();
        let guest_input_path = guest_file_path(GUEST_INPUT_ROOT, &input_path)?;
        let request = SubtitleSyncAlignRequest {
            input: SubtitleSyncAlignInputRef {
                path: guest_input_path,
            },
            subtitle_spans: job.subtitle_spans,
            max_offset_seconds: job.max_offset_seconds,
            selector: Some(AudioStreamSelector::Default),
            expected_codec: job.expected_codec,
        };
        let input = serde_json::to_string(&request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize subtitle sync request: {error}"
            ))
        })?;

        let manifest = build_subtitle_sync_manifest(
            self.wasm_bytes.as_slice(),
            &input_path,
            scratch_dir.path(),
        );
        let output = tokio::task::spawn_blocking(move || {
            let mut plugin = build_plugin(manifest).map_err(|error| {
                AppError::Repository(format!("failed to compile subtitle sync plugin: {error}"))
            })?;
            if !plugin.function_exists(EXPORT_SUBSYNC_ALIGN) {
                return Err(AppError::Repository(format!(
                    "plugin does not export {EXPORT_SUBSYNC_ALIGN}"
                )));
            }
            plugin
                .call::<&str, String>(EXPORT_SUBSYNC_ALIGN, &input)
                .map_err(|error| plugin_call_error(EXPORT_SUBSYNC_ALIGN, error))
        })
        .await
        .map_err(|error| AppError::Repository(format!("plugin task panicked: {error}")))??;

        decode_plugin_result(&output, EXPORT_SUBSYNC_ALIGN)
    }
}

impl std::fmt::Debug for WasmSubtitleSyncClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmSubtitleSyncClient")
            .field("plugin_id", &self.descriptor.id)
            .field("plugin_name", &self.descriptor.name)
            .finish()
    }
}

pub(crate) fn build_subtitle_sync_manifest(
    wasm_bytes: &[u8],
    input_path: &Path,
    scratch_dir: &Path,
) -> extism::Manifest {
    extism::Manifest::new([extism::Wasm::data(wasm_bytes.to_vec())])
        .with_timeout(std::time::Duration::from_secs(
            SUBTITLE_SYNC_TIMEOUT_SECONDS,
        ))
        .with_allowed_path(format!("ro:{}", input_path.display()), GUEST_INPUT_ROOT)
        .with_allowed_path(scratch_dir.display().to_string(), GUEST_SCRATCH_ROOT)
}

fn guest_file_path(root: &str, host_path: &Path) -> AppResult<PathBuf> {
    let file_name = host_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "subtitle sync path '{}' has no file name",
            host_path.display()
        ))
    })?;
    Ok(Path::new(root).join(file_name))
}

fn plugin_call_error(export: &str, error: extism::Error) -> AppError {
    AppError::Repository(format!("{export}() failed: {error}"))
}
