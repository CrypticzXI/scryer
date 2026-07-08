mod archive_adapter;
pub mod builtins;
mod download_client_adapter;
mod indexer_adapter;
mod loader;
mod notification_adapter;
mod plugin_http_host;
mod process_host;
mod runtime_backing;
mod runtime_features;
mod socket_host;
mod subtitle_adapter;
mod subtitle_sync_adapter;
mod types;
mod wasmtime_host;

// RFC 123 WP2 (archive validation): real-artifact integration suite. Lives
// in-crate because it drives the private `archive_adapter` +
// `wasmtime_host` modules against the checked-in plugin fixture.
#[cfg(test)]
mod archive_real_artifact_tests;

pub use loader::DynamicArchiveExtractorPluginProvider;
pub use loader::DynamicDownloadClientPluginProvider;
pub use loader::DynamicNotificationPluginProvider;
pub use loader::DynamicPluginProvider;
pub use loader::DynamicSubtitlePluginProvider;
pub use loader::WasmArchiveExtractorPluginProvider;
pub use loader::WasmDownloadClientPluginProvider;
pub use loader::WasmIndexerPluginProvider;
pub use loader::WasmNotificationPluginProvider;
pub use loader::WasmPluginDescriptorLoader;
pub use loader::WasmSubtitlePluginProvider;
pub use loader::build_archive_extractor_plugin_provider;
pub use loader::build_archive_extractor_plugin_provider_from_runtime_plugins;
pub use loader::build_download_client_plugin_provider;
pub use loader::build_download_client_plugin_provider_from_runtime_plugins;
pub use loader::build_indexer_plugin_provider;
pub use loader::build_indexer_plugin_provider_from_runtime_plugins;
pub use loader::build_notification_plugin_provider;
pub use loader::build_notification_plugin_provider_from_runtime_plugins;
pub use loader::build_subtitle_plugin_provider;
pub use loader::build_subtitle_plugin_provider_from_runtime_plugins;
pub use loader::load_indexer_plugins;
pub use plugin_http_host::PluginHttpRuntime;
pub use plugin_http_host::shared_plugin_http_runtime;
pub use runtime_features::PLUGIN_REQUIRED_FEATURE_RELAXED_SIMD;
pub use runtime_features::PLUGIN_REQUIRED_FEATURE_SIMD128;
pub use runtime_features::detect_supported_plugin_required_features;
pub use scryer_plugin_sdk::SDK_VERSION;
pub use scryer_plugin_sdk::host_version_matches_constraint;
pub use scryer_plugin_sdk::sdk_constraint_or_legacy;
pub use scryer_plugin_sdk::validate_sdk_contract;
pub use types::{ConfigFieldDef, ConfigFieldOption};
