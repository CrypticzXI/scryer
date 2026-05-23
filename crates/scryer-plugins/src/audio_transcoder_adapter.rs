use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, AudioTranscodeArtifact, AudioTranscodeJob, AudioTranscoderClient,
};

use crate::loader::build_plugin;
use crate::types::{
    AudioStreamSelector, AudioTranscodeInputRef, AudioTranscodeOutputRef, AudioTranscodeProfile,
    AudioTranscodeRequest, AudioTranscodeResponse, AudioTranscodeStatus, EXPORT_AUDIO_TRANSCODE,
    PluginDescriptor, PluginResult, decode_plugin_result,
};

const GUEST_INPUT_ROOT: &str = "/input";
const GUEST_OUTPUT_ROOT: &str = "/output";
const AUDIO_TRANSCODE_TIMEOUT_SECONDS: u64 = 60 * 60;

pub struct WasmAudioTranscoderClient {
    wasm_bytes: Arc<Vec<u8>>,
    descriptor: PluginDescriptor,
}

impl WasmAudioTranscoderClient {
    pub fn new(wasm_bytes: Vec<u8>, descriptor: PluginDescriptor) -> Self {
        Self {
            wasm_bytes: Arc::new(wasm_bytes),
            descriptor,
        }
    }
}

#[async_trait]
impl AudioTranscoderClient for WasmAudioTranscoderClient {
    async fn transcode_sync_flac(
        &self,
        job: AudioTranscodeJob,
    ) -> AppResult<AudioTranscodeArtifact> {
        let input_path = job.input_path.clone();
        let output_path = job.output_path.clone();
        let output_dir = output_path.parent().ok_or_else(|| {
            AppError::Validation(format!(
                "audio transcode output path '{}' has no parent directory",
                output_path.display()
            ))
        })?;
        std::fs::create_dir_all(output_dir).map_err(|error| {
            AppError::Repository(format!(
                "failed to create audio transcode output directory '{}': {error}",
                output_dir.display()
            ))
        })?;

        let guest_input_path = guest_file_path(GUEST_INPUT_ROOT, &input_path)?;
        let guest_output_path = guest_file_path(GUEST_OUTPUT_ROOT, &output_path)?;
        let request = AudioTranscodeRequest {
            input: AudioTranscodeInputRef {
                path: guest_input_path,
            },
            output: AudioTranscodeOutputRef {
                path: guest_output_path,
            },
            profile: AudioTranscodeProfile::SyncFlac,
            selector: Some(AudioStreamSelector::Default),
            expected_codec: Some(job.expected_codec),
        };
        let input = serde_json::to_string(&request).map_err(|error| {
            AppError::Repository(format!(
                "failed to serialize audio transcode request: {error}"
            ))
        })?;

        let manifest =
            build_audio_transcode_manifest(self.wasm_bytes.as_slice(), &input_path, output_dir);
        let output = tokio::task::spawn_blocking(move || {
            let mut plugin = build_plugin(manifest).map_err(|error| {
                AppError::Repository(format!(
                    "failed to compile audio transcoder plugin: {error}"
                ))
            })?;
            if !plugin.function_exists(EXPORT_AUDIO_TRANSCODE) {
                return Err(AppError::Repository(format!(
                    "plugin does not export {EXPORT_AUDIO_TRANSCODE}"
                )));
            }
            plugin
                .call::<&str, String>(EXPORT_AUDIO_TRANSCODE, &input)
                .map_err(|error| plugin_call_error(EXPORT_AUDIO_TRANSCODE, error))
        })
        .await
        .map_err(|error| AppError::Repository(format!("plugin task panicked: {error}")))??;

        let mut response: AudioTranscodeResponse =
            decode_plugin_result(&output, EXPORT_AUDIO_TRANSCODE)?;
        if response.status == AudioTranscodeStatus::Decoded {
            response.output = Some(AudioTranscodeOutputRef {
                path: output_path.clone(),
            });
        }

        Ok(AudioTranscodeArtifact {
            output_path,
            response,
        })
    }
}

impl std::fmt::Debug for WasmAudioTranscoderClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmAudioTranscoderClient")
            .field("plugin_id", &self.descriptor.id)
            .field("plugin_name", &self.descriptor.name)
            .finish()
    }
}

pub(crate) fn build_audio_transcode_manifest(
    wasm_bytes: &[u8],
    input_path: &Path,
    output_dir: &Path,
) -> extism::Manifest {
    extism::Manifest::new([extism::Wasm::data(wasm_bytes.to_vec())])
        .with_timeout(std::time::Duration::from_secs(
            AUDIO_TRANSCODE_TIMEOUT_SECONDS,
        ))
        .with_allowed_path(format!("ro:{}", input_path.display()), GUEST_INPUT_ROOT)
        .with_allowed_path(output_dir.display().to_string(), GUEST_OUTPUT_ROOT)
}

fn guest_file_path(root: &str, host_path: &Path) -> AppResult<PathBuf> {
    let file_name = host_path.file_name().ok_or_else(|| {
        AppError::Validation(format!(
            "audio transcode path '{}' has no file name",
            host_path.display()
        ))
    })?;
    Ok(Path::new(root).join(file_name))
}

fn plugin_call_error(export: &str, error: extism::Error) -> AppError {
    AppError::Repository(format!("{export}() failed: {error}"))
}

#[allow(dead_code)]
fn _assert_plugin_result_response(_: PluginResult<AudioTranscodeResponse>) {}
