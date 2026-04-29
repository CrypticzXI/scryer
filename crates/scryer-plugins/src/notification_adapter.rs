use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, NotificationClient, NotificationMediaUpdateTypePayload,
    NotificationPayload,
};
use scryer_domain::NotificationEventType as DomainNotificationEventType;
use tracing::warn;

use crate::types::{
    EXPORT_NOTIFICATION_SEND, NotificationEventType, PluginDescriptor, PluginNotificationApp,
    PluginNotificationDownload, PluginNotificationEpisode, PluginNotificationExternalIds,
    PluginNotificationFile, PluginNotificationHealth, PluginNotificationImport,
    PluginNotificationMediaUpdate, PluginNotificationRequest, PluginNotificationResponse,
    PluginNotificationTitle, decode_plugin_result,
};

pub struct WasmNotificationClient {
    plugin: Arc<Mutex<extism::Plugin>>,
    descriptor: PluginDescriptor,
    channel_name: String,
}

impl WasmNotificationClient {
    pub fn new(plugin: extism::Plugin, descriptor: PluginDescriptor, channel_name: String) -> Self {
        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            descriptor,
            channel_name,
        }
    }
}

#[async_trait]
impl NotificationClient for WasmNotificationClient {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()> {
        let request = PluginNotificationRequest {
            event_type: map_event_type(payload.event_type),
            summary_title: payload.summary_title.clone(),
            summary_message: payload.summary_message.clone(),
            app: PluginNotificationApp {
                name: payload.app.name.clone(),
                version: payload.app.version.clone(),
            },
            title: payload.title.as_ref().map(|title| PluginNotificationTitle {
                name: title.name.clone(),
                facet: title.facet.clone(),
                year: title.year,
                poster_url: title.poster_url.clone(),
                external_ids: PluginNotificationExternalIds {
                    tmdb_id: title.external_ids.tmdb_id.clone(),
                    imdb_id: title.external_ids.imdb_id.clone(),
                    tvdb_id: title.external_ids.tvdb_id.clone(),
                    anidb_id: title.external_ids.anidb_id.clone(),
                },
            }),
            episode: payload.episode.as_ref().map(|episode| PluginNotificationEpisode {
                episode_ids: episode.episode_ids.clone(),
                display: episode.display.clone(),
            }),
            release: payload.release.as_ref().map(|release| crate::types::PluginNotificationRelease {
                source_title: release.source_title.clone(),
                source_hint: release.source_hint.clone(),
                quality: release.quality.clone(),
                provider: release.provider.clone(),
                language: release.language.clone(),
            }),
            download: payload.download.as_ref().map(|download| PluginNotificationDownload {
                download_id: download.download_id.clone(),
                client_id: download.client_id.clone(),
                client_name: download.client_name.clone(),
                client_type: download.client_type.clone(),
            }),
            import: payload.import.as_ref().map(|import| PluginNotificationImport {
                import_id: import.import_id.clone(),
                source_system: import.source_system.clone(),
                source_ref: import.source_ref.clone(),
                source_title: import.source_title.clone(),
                source_path: import.source_path.clone(),
                dest_path: import.dest_path.clone(),
                imported_count: import.imported_count,
                status: import.status.clone(),
            }),
            health: payload.health.as_ref().map(|health| PluginNotificationHealth {
                status: health.status.clone(),
                message: health.message.clone(),
            }),
            file: payload.file.as_ref().map(|file| PluginNotificationFile {
                primary_path: file.primary_path.clone(),
                media_updates: file
                    .media_updates
                    .iter()
                    .map(|update| PluginNotificationMediaUpdate {
                        path: update.path.clone(),
                        update_type: match update.update_type {
                            NotificationMediaUpdateTypePayload::Created => {
                                crate::types::NotificationMediaUpdateType::Created
                            }
                            NotificationMediaUpdateTypePayload::Modified => {
                                crate::types::NotificationMediaUpdateType::Modified
                            }
                            NotificationMediaUpdateTypePayload::Deleted => {
                                crate::types::NotificationMediaUpdateType::Deleted
                            }
                        },
                    })
                    .collect(),
            }),
        };

        let input = serde_json::to_string(&request).map_err(|e| {
            AppError::Repository(format!("failed to serialize notification request: {e}"))
        })?;

        let plugin_name = self.descriptor.name.clone();
        let channel_name = self.channel_name.clone();

        let plugin = Arc::clone(&self.plugin);
        let output = tokio::task::spawn_blocking(move || {
            let mut guard = plugin
                .lock()
                .map_err(|e| AppError::Repository(format!("plugin mutex poisoned: {e}")))?;
            guard
                .call::<&str, String>(EXPORT_NOTIFICATION_SEND, &input)
                .map_err(|e| {
                    AppError::Repository(format!("plugin {EXPORT_NOTIFICATION_SEND}() failed: {e}"))
                })
        })
        .await
        .map_err(|e| AppError::Repository(format!("notification plugin task panicked: {e}")))??;

        let response: PluginNotificationResponse =
            decode_plugin_result(&output, EXPORT_NOTIFICATION_SEND)?;

        if !response.success {
            let err_msg = response
                .error
                .unwrap_or_else(|| "unknown error".to_string());
            warn!(
                plugin = plugin_name.as_str(),
                channel = channel_name.as_str(),
                error = err_msg.as_str(),
                "notification plugin reported failure"
            );
            return Err(AppError::Repository(format!(
                "notification failed: {err_msg}"
            )));
        }

        Ok(())
    }
}

fn map_event_type(event_type: DomainNotificationEventType) -> NotificationEventType {
    match event_type {
        DomainNotificationEventType::Grab => NotificationEventType::Grab,
        DomainNotificationEventType::Download => NotificationEventType::Download,
        DomainNotificationEventType::Upgrade => NotificationEventType::Upgrade,
        DomainNotificationEventType::ImportComplete => NotificationEventType::ImportComplete,
        DomainNotificationEventType::ImportRejected => NotificationEventType::ImportRejected,
        DomainNotificationEventType::Rename => NotificationEventType::Rename,
        DomainNotificationEventType::TitleAdded => NotificationEventType::TitleAdded,
        DomainNotificationEventType::TitleDeleted => NotificationEventType::TitleDeleted,
        DomainNotificationEventType::FileDeleted => NotificationEventType::FileDeleted,
        DomainNotificationEventType::FileDeletedForUpgrade => {
            NotificationEventType::FileDeletedForUpgrade
        }
        DomainNotificationEventType::PostProcessingCompleted => {
            NotificationEventType::PostProcessingCompleted
        }
        DomainNotificationEventType::SubtitleDownloaded => {
            NotificationEventType::SubtitleDownloaded
        }
        DomainNotificationEventType::SubtitleSearchFailed => {
            NotificationEventType::SubtitleSearchFailed
        }
        DomainNotificationEventType::HealthIssue => NotificationEventType::HealthIssue,
        DomainNotificationEventType::HealthRestored => NotificationEventType::HealthRestored,
        DomainNotificationEventType::ApplicationUpdate => NotificationEventType::ApplicationUpdate,
        DomainNotificationEventType::ManualInteractionRequired => {
            NotificationEventType::ManualInteractionRequired
        }
        DomainNotificationEventType::Test => NotificationEventType::Test,
    }
}
