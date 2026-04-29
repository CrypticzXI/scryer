use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    NotificationEventType, PluginNotificationMediaFile, PluginNotificationMediaUpdate,
    PluginNotificationRequest,
};

pub const NOTIFICATION_REQUEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationDeliveryMode {
    Webhook,
    CustomScript,
    Chat,
    Email,
    Push,
    MediaServerUpdate,
    ExternalSync,
    Aggregator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationPayloadFormat {
    StructuredJson,
    ScriptEnvironment,
    PlainText,
    Markdown,
    Html,
    RichEmbed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    #[default]
    Info,
    Warning,
    Error,
}

impl NotificationSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationEventOptions {
    #[serde(default)]
    pub supports_upgrade_filter: bool,
    #[serde(default)]
    pub supports_delete_for_upgrade_filter: bool,
    #[serde(default)]
    pub supports_health_warning_filter: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationActor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationRichEmbedField {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationRichEmbed {
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<NotificationRichEmbedField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationMediaUpdateBatch {
    pub group_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_facet: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_types: Vec<NotificationEventType>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_updates: Vec<PluginNotificationMediaUpdate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_files: Vec<PluginNotificationMediaFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginNotificationTargetResult {
    pub target: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn to_script_environment(request: &PluginNotificationRequest) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    env.insert(
        "SCRYER_NOTIFICATION_SCHEMA_VERSION".to_string(),
        request.schema_version.to_string(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_EVENT_TYPE".to_string(),
        request.event_type.as_str().to_string(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_SUMMARY_TITLE".to_string(),
        request.summary_title.clone(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_SUMMARY_MESSAGE".to_string(),
        request.summary_message.clone(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_APP_NAME".to_string(),
        request.app.name.clone(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_APP_VERSION".to_string(),
        request.app.version.clone(),
    );
    env.insert(
        "SCRYER_NOTIFICATION_IS_TEST".to_string(),
        request.is_test.to_string(),
    );

    if let Some(event_id) = &request.event_id {
        env.insert("SCRYER_NOTIFICATION_EVENT_ID".to_string(), event_id.clone());
    }
    if let Some(occurred_at) = &request.occurred_at {
        env.insert(
            "SCRYER_NOTIFICATION_OCCURRED_AT".to_string(),
            occurred_at.clone(),
        );
    }
    if let Some(correlation_id) = &request.correlation_id {
        env.insert(
            "SCRYER_NOTIFICATION_CORRELATION_ID".to_string(),
            correlation_id.clone(),
        );
    }
    if let Some(actor) = &request.actor
        && let Some(user_id) = &actor.user_id
    {
        env.insert(
            "SCRYER_NOTIFICATION_ACTOR_USER_ID".to_string(),
            user_id.clone(),
        );
    }
    if let Some(severity) = request.severity {
        env.insert(
            "SCRYER_NOTIFICATION_SEVERITY".to_string(),
            severity.as_str().to_string(),
        );
    }

    if let Some(title) = &request.title {
        if let Some(id) = &title.id {
            env.insert("SCRYER_TITLE_ID".to_string(), id.clone());
        }
        env.insert("SCRYER_TITLE_NAME".to_string(), title.name.clone());
        env.insert("SCRYER_TITLE_FACET".to_string(), title.facet.clone());
        if let Some(year) = title.year {
            env.insert("SCRYER_TITLE_YEAR".to_string(), year.to_string());
        }
        if let Some(path) = &title.path {
            env.insert("SCRYER_TITLE_PATH".to_string(), path.clone());
        }
        if let Some(slug) = &title.slug {
            env.insert("SCRYER_TITLE_SLUG".to_string(), slug.clone());
        }
        if !title.genres.is_empty() {
            env.insert("SCRYER_TITLE_GENRES".to_string(), title.genres.join(","));
        }
        if !title.tags.is_empty() {
            env.insert("SCRYER_TITLE_TAGS".to_string(), title.tags.join(","));
        }
        if !title.aliases.is_empty() {
            env.insert(
                "SCRYER_TITLE_ALIASES".to_string(),
                title.aliases.join(" | "),
            );
        }
        let external_ids_json = serde_json::to_string(&title.external_ids).unwrap_or_default();
        if !external_ids_json.is_empty() && external_ids_json != "{}" {
            env.insert(
                "SCRYER_TITLE_EXTERNAL_IDS_JSON".to_string(),
                external_ids_json,
            );
        }
    }

    if let Some(episode) = &request.episode {
        if !episode.episode_ids.is_empty() {
            env.insert(
                "SCRYER_EPISODE_IDS".to_string(),
                episode.episode_ids.join(","),
            );
        }
        if let Some(display) = &episode.display {
            env.insert("SCRYER_EPISODE_DISPLAY".to_string(), display.clone());
        }
    }
    if !request.episodes.is_empty() {
        env.insert(
            "SCRYER_EPISODES_JSON".to_string(),
            serde_json::to_string(&request.episodes).unwrap_or_default(),
        );
    }

    if let Some(release) = &request.release {
        if let Some(source_title) = &release.source_title {
            env.insert(
                "SCRYER_RELEASE_SOURCE_TITLE".to_string(),
                source_title.clone(),
            );
        }
        if let Some(source_hint) = &release.source_hint {
            env.insert(
                "SCRYER_RELEASE_SOURCE_HINT".to_string(),
                source_hint.clone(),
            );
        }
        if let Some(quality) = &release.quality {
            env.insert("SCRYER_RELEASE_QUALITY".to_string(), quality.clone());
        }
        if let Some(provider) = &release.provider {
            env.insert("SCRYER_RELEASE_PROVIDER".to_string(), provider.clone());
        }
        if let Some(language) = &release.language {
            env.insert("SCRYER_RELEASE_LANGUAGE".to_string(), language.clone());
        }
    }

    if let Some(download) = &request.download {
        if let Some(download_id) = &download.download_id {
            env.insert("SCRYER_DOWNLOAD_ID".to_string(), download_id.clone());
        }
        if let Some(client_name) = &download.client_name {
            env.insert(
                "SCRYER_DOWNLOAD_CLIENT_NAME".to_string(),
                client_name.clone(),
            );
        }
        if let Some(client_type) = &download.client_type {
            env.insert(
                "SCRYER_DOWNLOAD_CLIENT_TYPE".to_string(),
                client_type.clone(),
            );
        }
        if let Some(status) = &download.status {
            env.insert("SCRYER_DOWNLOAD_STATUS".to_string(), status.clone());
        }
    }

    if let Some(import) = &request.import {
        if let Some(import_id) = &import.import_id {
            env.insert("SCRYER_IMPORT_ID".to_string(), import_id.clone());
        }
        if let Some(status) = &import.status {
            env.insert("SCRYER_IMPORT_STATUS".to_string(), status.clone());
        }
        if let Some(source_path) = &import.source_path {
            env.insert("SCRYER_IMPORT_SOURCE_PATH".to_string(), source_path.clone());
        }
        if let Some(dest_path) = &import.dest_path {
            env.insert("SCRYER_IMPORT_DEST_PATH".to_string(), dest_path.clone());
        }
    }

    if let Some(file) = &request.file {
        if let Some(primary_path) = &file.primary_path {
            env.insert("SCRYER_FILE_PRIMARY_PATH".to_string(), primary_path.clone());
        }
        if !file.media_updates.is_empty() {
            env.insert(
                "SCRYER_FILE_MEDIA_UPDATES_JSON".to_string(),
                serde_json::to_string(&file.media_updates).unwrap_or_default(),
            );
        }
    }
    if !request.media_files.is_empty() {
        env.insert(
            "SCRYER_MEDIA_FILES_JSON".to_string(),
            serde_json::to_string(&request.media_files).unwrap_or_default(),
        );
    }

    env
}

pub fn to_webhook_json(request: &PluginNotificationRequest) -> Value {
    serde_json::to_value(request).unwrap_or_else(|_| {
        json!({
            "event_type": request.event_type.as_str(),
            "summary_title": request.summary_title,
            "summary_message": request.summary_message,
        })
    })
}

pub fn rich_embed_from_request(request: &PluginNotificationRequest) -> NotificationRichEmbed {
    let color_hex = match request.severity.unwrap_or_default() {
        NotificationSeverity::Info => Some("#4F8CFF".to_string()),
        NotificationSeverity::Warning => Some("#E0A100".to_string()),
        NotificationSeverity::Error => Some("#D33F49".to_string()),
    };

    let mut fields = Vec::new();
    if let Some(title) = &request.title {
        fields.push(NotificationRichEmbedField {
            name: "Title".to_string(),
            value: title.name.clone(),
            inline: false,
        });
        fields.push(NotificationRichEmbedField {
            name: "Facet".to_string(),
            value: title.facet.clone(),
            inline: true,
        });
        if let Some(year) = title.year {
            fields.push(NotificationRichEmbedField {
                name: "Year".to_string(),
                value: year.to_string(),
                inline: true,
            });
        }
    }
    if let Some(release) = &request.release {
        if let Some(source_title) = &release.source_title {
            fields.push(NotificationRichEmbedField {
                name: "Release".to_string(),
                value: source_title.clone(),
                inline: false,
            });
        }
        if let Some(quality) = &release.quality {
            fields.push(NotificationRichEmbedField {
                name: "Quality".to_string(),
                value: quality.clone(),
                inline: true,
            });
        }
    }
    if let Some(download) = &request.download {
        if let Some(client_name) = &download.client_name {
            fields.push(NotificationRichEmbedField {
                name: "Client".to_string(),
                value: client_name.clone(),
                inline: true,
            });
        }
        if let Some(status) = &download.status {
            fields.push(NotificationRichEmbedField {
                name: "Status".to_string(),
                value: status.clone(),
                inline: true,
            });
        }
    }

    NotificationRichEmbed {
        title: request.summary_title.clone(),
        description: request.summary_message.clone(),
        color_hex,
        image_url: request
            .title
            .as_ref()
            .and_then(|title| title.poster_url.clone()),
        fields,
        footer: Some(format!("Scryer · {}", request.event_type.as_str())),
    }
}

pub fn coalesce_media_updates<'a>(
    requests: impl IntoIterator<Item = &'a PluginNotificationRequest>,
) -> Vec<NotificationMediaUpdateBatch> {
    let mut groups = BTreeMap::<String, NotificationMediaUpdateBatch>::new();

    for request in requests {
        let group_key = request
            .title
            .as_ref()
            .and_then(|title| title.id.clone())
            .or_else(|| request.title.as_ref().map(|title| title.name.clone()))
            .or_else(|| request.event_id.clone())
            .unwrap_or_else(|| request.event_type.as_str().to_string());

        let batch =
            groups
                .entry(group_key.clone())
                .or_insert_with(|| NotificationMediaUpdateBatch {
                    group_key: group_key.clone(),
                    title_id: request.title.as_ref().and_then(|title| title.id.clone()),
                    title_name: request.title.as_ref().map(|title| title.name.clone()),
                    title_facet: request.title.as_ref().map(|title| title.facet.clone()),
                    ..NotificationMediaUpdateBatch::default()
                });

        if let Some(event_id) = &request.event_id
            && !batch.event_ids.contains(event_id)
        {
            batch.event_ids.push(event_id.clone());
        }
        if !batch.event_types.contains(&request.event_type) {
            batch.event_types.push(request.event_type);
        }

        if let Some(file) = &request.file
            && let Some(primary_path) = &file.primary_path
            && !batch.primary_paths.contains(primary_path)
        {
            batch.primary_paths.push(primary_path.clone());
        }

        let mut seen_updates = BTreeSet::new();
        for update in &batch.media_updates {
            seen_updates.insert((update.path.clone(), update.update_type));
        }

        let mut seen_media_files = BTreeSet::new();
        for media_file in &batch.media_files {
            seen_media_files.insert(media_file.path.clone());
        }

        if let Some(file) = &request.file {
            for update in &file.media_updates {
                if seen_updates.insert((update.path.clone(), update.update_type)) {
                    batch.media_updates.push(update.clone());
                }
            }
        }

        for media_file in &request.media_files {
            if seen_media_files.insert(media_file.path.clone()) {
                batch.media_files.push(media_file.clone());
            }
        }
    }

    groups.into_values().collect()
}
