#![recursion_limit = "256"]

mod common;

use async_graphql::Request;
use async_trait::async_trait;
use chrono::Utc;
use common::TestContext;
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    AppError, AppResult, NotificationClient, NotificationMediaUpdateTypePayload,
    NotificationPayload, NotificationPluginProvider, NotificationScopeIdUpdate,
    start_notification_dispatcher,
};
use scryer_domain::{
    ConfigFieldDef, ConfigFieldOption, ConfigFieldType, ConfigFieldValueSource, DomainEventPayload,
    DomainEventStream, DomainEventType, DomainExternalIds, ExternalId, ImportCompletedEventData,
    LibraryScanProgressedEventData, MediaFacet, MediaFileDeletedEventData, MediaFileDeletedReason,
    MediaFileRenamedEventData, MediaFileUpgradedEventData, MediaPathUpdate, MediaUpdateType,
    NewDomainEvent, NewTitle, NotificationEventType, TitleContextSnapshot,
};
use scryer_infrastructure::SqliteNotificationStore;
use scryer_interface::build_schema;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast::error::TryRecvError;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wire notification repos into the test AppUseCase so CRUD methods don't
/// return "not configured".
fn app_with_notifications(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder.with_notification_store(Arc::new(SqliteNotificationStore::new(&ctx.db)))
    })
}

fn app_with_notification_provider(
    ctx: &TestContext,
    provider: Arc<dyn NotificationPluginProvider>,
) -> scryer_application::AppUseCase {
    app_with_notifications(ctx)
        .with_test_overrides(|builder| builder.with_notification_provider(provider))
}

async fn default_user(app: &scryer_application::AppUseCase) -> scryer_domain::User {
    app.find_or_create_default_user().await.unwrap()
}

#[derive(Debug, Clone, PartialEq)]
struct CapturedNotification {
    event_type: String,
    title: String,
    message: String,
    metadata: HashMap<String, Value>,
}

#[derive(Clone)]
struct FakeNotificationClient {
    captured: Arc<Mutex<Vec<CapturedNotification>>>,
}

#[async_trait]
impl NotificationClient for FakeNotificationClient {
    async fn send_notification(&self, payload: &NotificationPayload) -> AppResult<()> {
        self.captured.lock().unwrap().push(CapturedNotification {
            event_type: payload.event_type.as_str().to_string(),
            title: payload.summary_title.clone(),
            message: payload.summary_message.clone(),
            metadata: captured_metadata(payload),
        });
        Ok(())
    }
}

#[derive(Clone)]
struct FakeNotificationProvider {
    provider_type: String,
    provider_name: String,
    config_fields: Vec<ConfigFieldDef>,
    captured: Arc<Mutex<Vec<CapturedNotification>>>,
}

impl FakeNotificationProvider {
    fn jellyfin() -> Self {
        Self {
            provider_type: "jellyfin".to_string(),
            provider_name: "Jellyfin".to_string(),
            config_fields: vec![
                ConfigFieldDef {
                    key: "base_url".to_string(),
                    label: "Base URL".to_string(),
                    field_type: ConfigFieldType::String,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                },
                ConfigFieldDef {
                    key: "api_key".to_string(),
                    label: "API Key".to_string(),
                    field_type: ConfigFieldType::Password,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    host_binding: None,
                    options: vec![],
                    help_text: None,
                },
                ConfigFieldDef {
                    key: "path_mappings".to_string(),
                    label: "Path Mappings".to_string(),
                    field_type: ConfigFieldType::Multiline,
                    required: true,
                    default_value: None,
                    value_source: ConfigFieldValueSource::User,
                    host_binding: None,
                    options: vec![ConfigFieldOption {
                        value: "/data => /mnt".to_string(),
                        label: "Example".to_string(),
                    }],
                    help_text: Some("One mapping per line.".to_string()),
                },
            ],
            captured: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn captured(&self) -> Vec<CapturedNotification> {
        self.captured.lock().unwrap().clone()
    }
}

impl NotificationPluginProvider for FakeNotificationProvider {
    fn client_for_channel(
        &self,
        config: &scryer_domain::NotificationChannelConfig,
    ) -> Option<Arc<dyn NotificationClient>> {
        if config.channel_type.as_str() != self.provider_type {
            return None;
        }

        Some(Arc::new(FakeNotificationClient {
            captured: Arc::clone(&self.captured),
        }))
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec![self.provider_type.clone()]
    }

    fn config_fields_for_provider(&self, provider_type: &str) -> Vec<ConfigFieldDef> {
        if provider_type == self.provider_type {
            self.config_fields.clone()
        } else {
            vec![]
        }
    }

    fn plugin_name_for_provider(&self, provider_type: &str) -> Option<String> {
        (provider_type == self.provider_type).then(|| self.provider_name.clone())
    }
}

fn assert_no_errors(body: &Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn schema_exec(
    app: &scryer_application::AppUseCase,
    _ctx: &TestContext,
    query: &str,
) -> Value {
    let schema = build_schema(app.clone(), false);
    let user = default_user(app).await;
    let response = schema.execute(Request::new(query).data(user)).await;
    serde_json::to_value(&response).expect("serialize GraphQL response")
}

fn config_json_with_path_mappings() -> String {
    serde_json::json!({
        "base_url": "http://jellyfin:8096",
        "api_key": "secret",
        "path_mappings": "/data/Movies => /mnt/media/Movies\n/data/TV => /mnt/media/TV"
    })
    .to_string()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf()
}

fn jellyfin_dist_wasm_path() -> PathBuf {
    repo_root()
        .parent()
        .expect("workspace root")
        .join("scryer-plugins")
        .join("dist")
        .join("jellyfin_notification.wasm")
}

fn lifecycle_metadata(
    title_name: &str,
    facet: &str,
    updates: Vec<(&str, &str)>,
    external_ids: Value,
) -> HashMap<String, Value> {
    let media_updates = updates
        .iter()
        .map(|(path, update_type)| {
            json!({
                "path": path,
                "update_type": update_type,
            })
        })
        .collect::<Vec<_>>();

    HashMap::from([
        ("title_name".to_string(), json!(title_name)),
        ("title_facet".to_string(), json!(facet)),
        ("file_path".to_string(), json!(updates[0].0)),
        ("media_updates".to_string(), Value::Array(media_updates)),
        ("external_ids".to_string(), external_ids),
    ])
}

fn captured_metadata(payload: &NotificationPayload) -> HashMap<String, Value> {
    let mut metadata = HashMap::new();

    if let Some(title) = &payload.title {
        metadata.insert("title_name".to_string(), json!(title.name));
        metadata.insert("title_facet".to_string(), json!(title.facet));
        if let Some(year) = title.year {
            metadata.insert("title_year".to_string(), json!(year));
        }
        if !title.tags.is_empty() {
            metadata.insert("title_tags".to_string(), json!(title.tags));
        }

        let mut external_ids = serde_json::Map::new();
        if let Some(tmdb_id) = &title.external_ids.tmdb_id {
            external_ids.insert("tmdb_id".to_string(), json!(tmdb_id));
        }
        if let Some(imdb_id) = &title.external_ids.imdb_id {
            external_ids.insert("imdb_id".to_string(), json!(imdb_id));
        }
        if let Some(tvdb_id) = &title.external_ids.tvdb_id {
            external_ids.insert("tvdb_id".to_string(), json!(tvdb_id));
        }
        if let Some(anidb_id) = &title.external_ids.anidb_id {
            external_ids.insert("anidb_id".to_string(), json!(anidb_id));
        }
        metadata.insert("external_ids".to_string(), Value::Object(external_ids));

        if !title.external_ids.by_source.is_empty()
            && title
                .external_ids
                .by_source
                .keys()
                .any(|source| !matches!(source.as_str(), "tmdb" | "imdb" | "tvdb" | "anidb"))
        {
            metadata.insert(
                "external_ids_by_source".to_string(),
                json!(title.external_ids.by_source),
            );
        }
    }

    if let Some(episode) = payload.episodes.first() {
        metadata.insert("episode_id".to_string(), json!(episode.id));
        if let Some(season_number) = &episode.season_number {
            metadata.insert("episode_season_number".to_string(), json!(season_number));
        }
        if let Some(episode_number) = &episode.episode_number {
            metadata.insert("episode_number".to_string(), json!(episode_number));
        }
        if let Some(title) = &episode.title {
            metadata.insert("episode_title".to_string(), json!(title));
        }
        if let Some(air_date) = &episode.air_date {
            metadata.insert("episode_air_date".to_string(), json!(air_date));
        }
    }

    if let Some(file) = &payload.file {
        if let Some(primary_path) = &file.primary_path {
            metadata.insert("file_path".to_string(), json!(primary_path));
        }

        let media_updates = file
            .media_updates
            .iter()
            .map(|update| {
                json!({
                    "path": update.path,
                    "update_type": match update.update_type {
                        NotificationMediaUpdateTypePayload::Created => "created",
                        NotificationMediaUpdateTypePayload::Modified => "modified",
                        NotificationMediaUpdateTypePayload::Deleted => "deleted",
                    },
                })
            })
            .collect::<Vec<_>>();
        metadata.insert("media_updates".to_string(), Value::Array(media_updates));
    }

    metadata
}

fn import_completed_event_data(
    title: TitleContextSnapshot,
    media_updates: Vec<MediaPathUpdate>,
    imported_count: i32,
    episode_ids: Vec<String>,
) -> ImportCompletedEventData {
    ImportCompletedEventData {
        title,
        media_updates,
        imported_count,
        import_id: None,
        source_system: None,
        source_ref: None,
        source_title: None,
        source_path: None,
        dest_path: None,
        quality: None,
        episode_ids,
    }
}

fn title_context(
    title_name: &str,
    facet: &str,
    external_ids: DomainExternalIds,
) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: title_name.to_string(),
        facet: MediaFacet::parse(facet).expect("valid facet"),
        external_ids,
        poster_url: None,
        year: None,
    }
}

fn external_id(source: &str, value: &str) -> ExternalId {
    ExternalId {
        source: source.to_string(),
        value: value.to_string(),
    }
}

fn new_event(
    event_id: &str,
    title_id: &str,
    facet: &str,
    payload: DomainEventPayload,
) -> NewDomainEvent {
    NewDomainEvent {
        event_id: event_id.to_string(),
        occurred_at: Utc::now(),
        actor_user_id: Some("user-1".to_string()),
        title_id: Some(title_id.to_string()),
        facet: MediaFacet::parse(facet),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title_id.to_string(),
        },
        payload,
    }
}

async fn wait_for_captured(
    provider: &FakeNotificationProvider,
    expected: usize,
) -> Vec<CapturedNotification> {
    for _ in 0..50 {
        let captured = provider.captured();
        if captured.len() >= expected {
            return captured;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    panic!(
        "timed out waiting for {expected} notifications, captured {:?}",
        provider.captured()
    );
}

// ---------------------------------------------------------------------------
// Channel CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_channels() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Discord".into(), "webhook".into(), "{}".into(), true)
        .await
        .expect("create channel");
    assert_eq!(ch.name, "Discord");
    assert_eq!(ch.channel_type.as_str(), "webhook");
    assert!(ch.is_enabled);

    let channels = app.list_notification_channels(&user).await.expect("list");
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].id, ch.id);
}

#[tokio::test]
async fn get_channel_by_id() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Slack".into(), "webhook".into(), "{}".into(), false)
        .await
        .unwrap();

    let fetched = app
        .get_notification_channel(&user, &ch.id)
        .await
        .unwrap()
        .expect("should find channel");
    assert_eq!(fetched.name, "Slack");
    assert!(!fetched.is_enabled);
}

#[tokio::test]
async fn update_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(
            &user,
            "Old Name".into(),
            "webhook".into(),
            "{\"url\":\"http://a\"}".into(),
            true,
        )
        .await
        .unwrap();

    let updated = app
        .update_notification_channel(
            &user,
            ch.id.clone(),
            Some("New Name".into()),
            Some("{\"url\":\"http://b\"}".into()),
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.config_json, "{\"url\":\"http://b\"}");
    assert!(!updated.is_enabled);
}

#[tokio::test]
async fn delete_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Temp".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    app.delete_notification_channel(&user, &ch.id)
        .await
        .expect("delete");

    let channels = app.list_notification_channels(&user).await.unwrap();
    assert!(channels.is_empty());
}

// ---------------------------------------------------------------------------
// Channel validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_channel_rejects_empty_name() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(&user, "".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_channel_rejects_empty_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_channel(&user, "Slack".into(), "  ".into(), "{}".into(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_channel_accepts_arbitrary_provider_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "  Jellyfin  ".into(),
            "{}".into(),
            true,
        )
        .await
        .expect("create channel");

    assert_eq!(ch.channel_type.as_str(), "jellyfin");
}

#[tokio::test]
async fn update_nonexistent_channel_returns_not_found() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .update_notification_channel(&user, "nonexistent".into(), Some("x".into()), None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

// ---------------------------------------------------------------------------
// Subscription CRUD
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_list_subscriptions() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Discord".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id.clone(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .expect("create subscription");

    assert_eq!(sub.channel_id, ch.id);
    assert_eq!(sub.event_type, NotificationEventType::Grab);
    assert!(sub.is_enabled);

    let subs = app.list_notification_subscriptions(&user).await.unwrap();
    assert_eq!(subs.len(), 1);
}

#[tokio::test]
async fn update_subscription() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id.clone(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    let updated = app
        .update_notification_subscription(
            &user,
            sub.id.clone(),
            Some("import_completed".into()),
            None,
            NotificationScopeIdUpdate::NoChange,
            Some(false),
        )
        .await
        .unwrap();

    assert_eq!(updated.event_type, NotificationEventType::ImportComplete);
    assert!(!updated.is_enabled);
}

#[tokio::test]
async fn delete_subscription() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id,
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    app.delete_notification_subscription(&user, &sub.id)
        .await
        .expect("delete");

    let subs = app.list_notification_subscriptions(&user).await.unwrap();
    assert!(subs.is_empty());
}

// ---------------------------------------------------------------------------
// Subscription validation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_subscription_rejects_unknown_event_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let err = app
        .create_notification_subscription(
            &user,
            ch.id,
            "nonexistent_event".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn create_subscription_rejects_nonexistent_channel() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let err = app
        .create_notification_subscription(
            &user,
            "nonexistent-channel".into(),
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
}

#[tokio::test]
async fn update_subscription_rejects_unknown_event_type() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;

    let ch = app
        .create_notification_channel(&user, "Ch".into(), "webhook".into(), "{}".into(), true)
        .await
        .unwrap();

    let sub = app
        .create_notification_subscription(
            &user,
            ch.id,
            "release_grabbed".into(),
            "global".into(),
            None,
            true,
        )
        .await
        .unwrap();

    let err = app
        .update_notification_subscription(
            &user,
            sub.id,
            Some("bogus_event".into()),
            None,
            NotificationScopeIdUpdate::NoChange,
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
}

#[tokio::test]
async fn notification_provider_types_query_exposes_jellyfin_multiline_field() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider);

    let body = schema_exec(
        &app,
        &ctx,
        r#"
        query NotificationProviderTypes {
          notificationProviderTypes {
            providerType
            name
            configFields {
              key
              fieldType
              required
            }
          }
        }
        "#,
    )
    .await;

    assert_no_errors(&body);
    let providers = body["data"]["notificationProviderTypes"]
        .as_array()
        .expect("provider array");
    let jellyfin = providers
        .iter()
        .find(|provider| provider["providerType"] == "jellyfin")
        .expect("jellyfin provider");

    assert_eq!(jellyfin["name"], "Jellyfin");
    assert!(
        jellyfin["configFields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| {
                field["key"] == "path_mappings"
                    && field["fieldType"] == "multiline"
                    && field["required"] == true
            }),
        "expected path_mappings multiline field in {jellyfin}"
    );
}

#[tokio::test]
async fn create_channel_preserves_multiline_jellyfin_config_json() {
    let ctx = TestContext::new().await;
    let app = app_with_notifications(&ctx);
    let user = default_user(&app).await;
    let config_json = config_json_with_path_mappings();

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json.clone(),
            true,
        )
        .await
        .expect("create channel");

    let fetched = app
        .get_notification_channel(&user, &channel.id)
        .await
        .expect("load channel")
        .expect("channel should exist");
    assert_eq!(fetched.config_json, config_json);
}

#[tokio::test]
async fn jellyfin_dist_plugin_accepts_test_notification_payload() {
    let wasm_path = jellyfin_dist_wasm_path();
    if !wasm_path.exists() {
        eprintln!("skipping jellyfin dist test; missing {}", wasm_path.display());
        return;
    }

    let ctx = TestContext::new().await;
    let wasm_bytes =
        std::fs::read(&wasm_path).unwrap_or_else(|error| panic!("read {}: {error}", wasm_path.display()));
    let provider: Arc<dyn NotificationPluginProvider> = Arc::new(
        scryer_plugins::DynamicNotificationPluginProvider::new(
            scryer_plugins::WasmNotificationPluginProvider::empty()
                .with_external_bytes(&wasm_bytes),
        ),
    );
    let app = app_with_notification_provider(&ctx, provider);
    let user = default_user(&app).await;

    Mock::given(method("GET"))
        .and(path("/System/Info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ServerName": "Jellyfin Test",
            "Version": "10.10.0",
        })))
        .expect(1)
        .mount(&ctx.nzbgeek_server)
        .await;

    let config_json = json!({
        "base_url": ctx.nzbgeek_server.uri(),
        "api_key": "secret",
        "path_mappings": "/data => /mnt",
    })
    .to_string();

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json,
            true,
        )
        .await
        .expect("create channel");

    app.test_notification_channel(&user, &channel.id)
        .await
        .expect("jellyfin dist plugin should accept test payload");
}

#[tokio::test]
async fn notification_dispatcher_delivers_structured_lifecycle_metadata() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json_with_path_mappings(),
            true,
        )
        .await
        .expect("create channel");

    for event_type in [
        DomainEventType::ImportCompleted,
        DomainEventType::MediaFileUpgraded,
        DomainEventType::MediaFileRenamed,
        DomainEventType::MediaFileDeleted,
    ] {
        app.create_notification_subscription(
            &user,
            channel.id.clone(),
            event_type.as_str().to_string(),
            "global".into(),
            None,
            true,
        )
        .await
        .expect("create subscription");
    }

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    let scenarios = vec![
        (
            "import_complete",
            "Import complete: Example Show".to_string(),
            "Imported 1 file for 'Example Show'.".to_string(),
            lifecycle_metadata(
                "Example Show",
                "series",
                vec![("/data/TV/Example Show/S01E01.mkv", "created")],
                json!({ "tvdb_id": "123", "imdb_id": "tt456" }),
            ),
            new_event(
                "evt-import-complete",
                "title-1",
                "series",
                DomainEventPayload::ImportCompleted(import_completed_event_data(
                    title_context(
                        "Example Show",
                        "series",
                        DomainExternalIds {
                            imdb_id: Some("tt456".to_string()),
                            tmdb_id: None,
                            tvdb_id: Some("123".to_string()),
                            anidb_id: None,
                        },
                    ),
                    vec![MediaPathUpdate {
                        path: "/data/TV/Example Show/S01E01.mkv".to_string(),
                        update_type: MediaUpdateType::Created,
                    }],
                    1,
                    vec!["episode-1".to_string()],
                )),
            ),
        ),
        (
            "upgrade",
            "Upgraded: Example Movie".to_string(),
            "Upgraded file for 'Example Movie'.".to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![("/data/Movies/Example Movie (2024)/Example Movie.mkv", "modified")],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-upgrade",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileUpgraded(MediaFileUpgradedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Modified,
                    }],
                    previous_file_id: Some("file-old".to_string()),
                    current_file_id: Some("file-new".to_string()),
                    old_score: None,
                    new_score: None,
                }),
            ),
        ),
        (
            "rename",
            "Renamed: Example Show".to_string(),
            "Renamed 1 file(s) for 'Example Show'.".to_string(),
            lifecycle_metadata(
                "Example Show",
                "series",
                vec![
                    ("/data/TV/Example Show/Old Name.mkv", "deleted"),
                    ("/data/TV/Example Show/New Name.mkv", "created"),
                ],
                json!({ "tvdb_id": "123", "imdb_id": "tt456" }),
            ),
            new_event(
                "evt-rename",
                "title-1",
                "series",
                DomainEventPayload::MediaFileRenamed(MediaFileRenamedEventData {
                    title: title_context(
                        "Example Show",
                        "series",
                        DomainExternalIds {
                            imdb_id: Some("tt456".to_string()),
                            tmdb_id: None,
                            tvdb_id: Some("123".to_string()),
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![
                        MediaPathUpdate {
                            path: "/data/TV/Example Show/Old Name.mkv".to_string(),
                            update_type: MediaUpdateType::Deleted,
                        },
                        MediaPathUpdate {
                            path: "/data/TV/Example Show/New Name.mkv".to_string(),
                            update_type: MediaUpdateType::Created,
                        },
                    ],
                    renamed_count: 1,
                    episode_ids: vec!["episode-1".to_string()],
                }),
            ),
        ),
        (
            "file_deleted",
            "File deleted: Example Movie".to_string(),
            "Deleted media file from disk: /data/Movies/Example Movie (2024)/Example Movie.mkv"
                .to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![("/data/Movies/Example Movie (2024)/Example Movie.mkv", "deleted")],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-file-deleted",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.mkv".to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-1".to_string()),
                    reason: MediaFileDeletedReason::Deleted,
                    episode_ids: Vec::new(),
                }),
            ),
        ),
        (
            "file_deleted_for_upgrade",
            "Deleted for upgrade: Example Movie".to_string(),
            "Removed old media file during upgrade: /data/Movies/Example Movie (2024)/Example Movie.old.mkv"
                .to_string(),
            lifecycle_metadata(
                "Example Movie",
                "movie",
                vec![(
                    "/data/Movies/Example Movie (2024)/Example Movie.old.mkv",
                    "deleted",
                )],
                json!({ "tmdb_id": "987", "imdb_id": "tt6543210" }),
            ),
            new_event(
                "evt-file-deleted-upgrade",
                "title-1",
                "movie",
                DomainEventPayload::MediaFileDeleted(MediaFileDeletedEventData {
                    title: title_context(
                        "Example Movie",
                        "movie",
                        DomainExternalIds {
                            imdb_id: Some("tt6543210".to_string()),
                            tmdb_id: Some("987".to_string()),
                            tvdb_id: None,
                            anidb_id: None,
                        },
                    ),
                    media_updates: vec![MediaPathUpdate {
                        path: "/data/Movies/Example Movie (2024)/Example Movie.old.mkv"
                            .to_string(),
                        update_type: MediaUpdateType::Deleted,
                    }],
                    file_id: Some("file-old".to_string()),
                    reason: MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: Vec::new(),
                }),
            ),
        ),
    ];

    for (_plugin_event_type, _title, _body, _metadata, event) in &scenarios {
        app.append_domain_event(event.clone())
            .await
            .expect("append domain event");
    }

    let captured = wait_for_captured(&provider, scenarios.len()).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    let expected = scenarios
        .into_iter()
        .map(
            |(event_type, title, body, metadata, _event)| CapturedNotification {
                event_type: event_type.to_string(),
                title,
                message: body,
                metadata,
            },
        )
        .collect::<Vec<_>>();

    assert_eq!(captured, expected);
}

#[tokio::test]
async fn notification_dispatcher_prefers_local_catalog_metadata_over_snapshot() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Canonical Show".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2024),
                tags: vec!["local-tag".to_string()],
                external_ids: vec![
                    external_id("tvdb", "321"),
                    external_id("imdb", "tt7654321"),
                    external_id("anilist", "9999"),
                ],
                ..Default::default()
            },
        )
        .await
        .expect("add title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season 1".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("2024-01-01".into()),
            Some(1500),
            false,
            true,
        )
        .await
        .expect("create episode");

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json_with_path_mappings(),
            true,
        )
        .await
        .expect("create channel");

    app.create_notification_subscription(
        &user,
        channel.id.clone(),
        DomainEventType::ImportCompleted.as_str().to_string(),
        "global".into(),
        None,
        true,
    )
    .await
    .expect("create subscription");

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    app.append_domain_event(new_event(
        "evt-local-enrichment",
        &title.id,
        "series",
        DomainEventPayload::ImportCompleted(import_completed_event_data(
            title_context(
                "Snapshot Show",
                "series",
                DomainExternalIds {
                    imdb_id: Some("tt0000000".to_string()),
                    tmdb_id: None,
                    tvdb_id: Some("999".to_string()),
                    anidb_id: None,
                },
            ),
            vec![MediaPathUpdate {
                path: "/data/TV/Canonical Show/S01E01.mkv".to_string(),
                update_type: MediaUpdateType::Created,
            }],
            1,
            vec![episode.id.clone()],
        )),
    ))
    .await
    .expect("append domain event");

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    let metadata = &captured[0].metadata;
    assert_eq!(metadata.get("title_name"), Some(&json!("Canonical Show")));
    assert_eq!(metadata.get("title_year"), Some(&json!(2024)));
    assert_eq!(
        metadata.get("title_tags"),
        Some(&json!(vec!["local-tag".to_string()]))
    );
    assert_eq!(
        metadata.get("external_ids"),
        Some(&json!({
            "tvdb_id": "321",
            "imdb_id": "tt7654321",
        }))
    );
    assert_eq!(
        metadata.get("external_ids_by_source"),
        Some(&json!({
            "anilist": ["9999"],
            "imdb": ["tt7654321"],
            "tvdb": ["321"],
        }))
    );
    assert_eq!(metadata.get("episode_id"), Some(&json!(episode.id)));
    assert_eq!(metadata.get("episode_season_number"), Some(&json!("1")));
    assert_eq!(metadata.get("episode_number"), Some(&json!("1")));
    assert_eq!(metadata.get("episode_title"), Some(&json!("Pilot")));
    assert_eq!(metadata.get("episode_air_date"), Some(&json!("2024-01-01")));
}

#[tokio::test]
async fn notification_dispatcher_replays_notifications_after_operational_burst() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json_with_path_mappings(),
            true,
        )
        .await
        .expect("create channel");

    app.create_notification_subscription(
        &user,
        channel.id.clone(),
        DomainEventType::ImportCompleted.as_str().to_string(),
        "global".into(),
        None,
        true,
    )
    .await
    .expect("create import-complete subscription");

    for i in 0..300 {
        app.append_domain_event(new_event(
            &format!("evt-scan-{i}"),
            "title-scan",
            "movie",
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: format!("scan-{i}"),
                status: "running".to_string(),
                found_titles: i as i64 + 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: i as i64 + 1,
                titles_total: Some(300),
                files_completed: i as i64 + 1,
                files_total: Some(300),
                warning_message: None,
            }),
        ))
        .await
        .expect("operational burst event should append");
    }

    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    app.append_domain_event(new_event(
        "evt-import-after-burst",
        "title-1",
        "series",
        DomainEventPayload::ImportCompleted(import_completed_event_data(
            title_context(
                "Burst Replay Show",
                "series",
                DomainExternalIds {
                    imdb_id: Some("tt456".to_string()),
                    tmdb_id: None,
                    tvdb_id: Some("123".to_string()),
                    anidb_id: None,
                },
            ),
            vec![MediaPathUpdate {
                path: "/data/TV/Burst Replay Show/S01E01.mkv".to_string(),
                update_type: MediaUpdateType::Created,
            }],
            1,
            vec!["episode-1".to_string()],
        )),
    ))
    .await
    .expect("notification event should append");

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].event_type,
        NotificationEventType::ImportComplete.as_str()
    );
    assert_eq!(captured[0].title, "Import complete: Burst Replay Show");
    assert_eq!(
        captured[0].message,
        "Imported 1 file for 'Burst Replay Show'."
    );
}

#[tokio::test]
async fn notification_dispatcher_ignores_operational_burst_while_running() {
    let ctx = TestContext::new().await;
    let provider = Arc::new(FakeNotificationProvider::jellyfin());
    let app = app_with_notification_provider(&ctx, provider.clone());
    let user = default_user(&app).await;

    let channel = app
        .create_notification_channel(
            &user,
            "Jellyfin".into(),
            "jellyfin".into(),
            config_json_with_path_mappings(),
            true,
        )
        .await
        .expect("create channel");

    app.create_notification_subscription(
        &user,
        channel.id.clone(),
        DomainEventType::ImportCompleted.as_str().to_string(),
        "global".into(),
        None,
        true,
    )
    .await
    .expect("create import-complete subscription");

    let mut wake_rx = app.notification_wake_receiver();
    let cancel = CancellationToken::new();
    let dispatcher = tokio::spawn(start_notification_dispatcher(app.clone(), cancel.clone()));
    tokio::task::yield_now().await;

    for i in 0..300 {
        app.append_domain_event(new_event(
            &format!("evt-live-scan-{i}"),
            "title-scan",
            "movie",
            DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: format!("scan-live-{i}"),
                status: "running".to_string(),
                found_titles: i as i64 + 1,
                title_match_completed: 0,
                title_match_total_known: false,
                titles_completed: i as i64 + 1,
                titles_total: Some(300),
                files_completed: i as i64 + 1,
                files_total: Some(300),
                warning_message: None,
            }),
        ))
        .await
        .expect("operational burst event should append");
    }

    assert!(
        matches!(wake_rx.try_recv(), Err(TryRecvError::Empty)),
        "operational bursts should not enqueue notification dispatcher wakes"
    );

    let notification_event = app
        .append_domain_event(new_event(
            "evt-live-import-after-burst",
            "title-1",
            "series",
            DomainEventPayload::ImportCompleted(import_completed_event_data(
                title_context(
                    "Live Burst Show",
                    "series",
                    DomainExternalIds {
                        imdb_id: Some("tt456".to_string()),
                        tmdb_id: None,
                        tvdb_id: Some("123".to_string()),
                        anidb_id: None,
                    },
                ),
                vec![MediaPathUpdate {
                    path: "/data/TV/Live Burst Show/S01E01.mkv".to_string(),
                    update_type: MediaUpdateType::Created,
                }],
                1,
                vec!["episode-1".to_string()],
            )),
        ))
        .await
        .expect("notification event should append");

    let wake = tokio::time::timeout(Duration::from_secs(1), wake_rx.recv())
        .await
        .expect("notification wake should arrive")
        .expect("notification wake channel should stay open");
    assert_eq!(wake, notification_event.sequence);
    assert!(
        matches!(wake_rx.try_recv(), Err(TryRecvError::Empty)),
        "notification event should enqueue exactly one wake"
    );

    let captured = wait_for_captured(&provider, 1).await;
    cancel.cancel();
    dispatcher.await.expect("dispatcher task");

    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].event_type,
        NotificationEventType::ImportComplete.as_str()
    );
    assert_eq!(captured[0].title, "Import complete: Live Burst Show");
    assert_eq!(
        captured[0].message,
        "Imported 1 file for 'Live Burst Show'."
    );
}
