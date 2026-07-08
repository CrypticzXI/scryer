use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scryer_application::{AppError, AppResult, SubtitleSyncClient, SubtitleSyncJob};

use crate::legacy_runtime::{LegacyPlugin, LegacyPluginSpec};
use crate::runtime_backing::PreopenSpec;
use crate::types::{
    AudioStreamSelector, EXPORT_SUBSYNC_ALIGN, PluginDescriptor, SubtitleSyncAlignInputRef,
    SubtitleSyncAlignRequest, SubtitleSyncAlignResponse, SubtitleSyncInputSubtitle,
    SubtitleSyncReferenceSubtitle, decode_plugin_result,
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
            subtitle: SubtitleSyncInputSubtitle {
                content_base64: BASE64.encode(&job.subtitle_content),
                format: job.subtitle_format,
                file_name: job.subtitle_file_name,
                encoding_hint: job.subtitle_encoding_hint,
            },
            reference_subtitle: job.reference_subtitle.map(|subtitle| {
                SubtitleSyncReferenceSubtitle {
                    content_base64: BASE64.encode(&subtitle.content),
                    format: subtitle.format,
                    file_name: subtitle.file_name,
                    encoding_hint: subtitle.encoding_hint,
                }
            }),
            subtitle_spans: Vec::new(),
            max_offset_seconds: job.max_offset_seconds,
            sync_options: Some(job.sync_options),
            selector: Some(AudioStreamSelector::Default),
            expected_codec: job.expected_codec,
        };
        let input = serde_json::to_string(&request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize subtitle sync request: {error}"
            ))
        })?;

        let spec = build_subtitle_sync_spec(
            self.wasm_bytes.as_slice(),
            &self.descriptor,
            &input_path,
            scratch_dir.path(),
        );
        let output = tokio::task::spawn_blocking(move || {
            let mut plugin = LegacyPlugin::instantiate(spec).map_err(|error| {
                AppError::Repository(format!("failed to compile subtitle sync plugin: {error}"))
            })?;
            if !plugin.function_exists(EXPORT_SUBSYNC_ALIGN) {
                return Err(AppError::Repository(format!(
                    "plugin does not export {EXPORT_SUBSYNC_ALIGN}"
                )));
            }
            plugin
                .call_string(EXPORT_SUBSYNC_ALIGN, &input)
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

pub(crate) fn build_subtitle_sync_spec(
    wasm_bytes: &[u8],
    descriptor: &PluginDescriptor,
    input_path: &Path,
    scratch_dir: &Path,
) -> LegacyPluginSpec {
    let input_root = input_path.parent().unwrap_or_else(|| Path::new("."));

    let mut spec = LegacyPluginSpec::new(wasm_bytes.to_vec(), descriptor.id.clone());
    spec.timeout = std::time::Duration::from_secs(SUBTITLE_SYNC_TIMEOUT_SECONDS);
    spec.preopens
        .push(PreopenSpec::read_only(input_root, GUEST_INPUT_ROOT));
    spec.preopens
        .push(PreopenSpec::writable(scratch_dir, GUEST_SCRATCH_ROOT));
    spec
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

fn plugin_call_error(export: &str, error: AppError) -> AppError {
    AppError::Repository(format!("{export}() failed: {error}"))
}
