use super::*;
use chrono::Utc;
use scryer_application::{
    CollectionUpdate, DomainEventRepository, DownloadClientConfigRepository,
    DownloadQueueCommandRepository, DownloadSourceIdentity, DownloadSubmission,
    DownloadSubmissionRepository, EpisodeUpdate, HousekeepingRepository, ImportRepository,
    InsertMediaFileInput, LibraryScanUnmatchedItem, LibraryScanUnmatchedItemRepository,
    LibraryScanUnmatchedSearchAttempt, MediaFileRepository, NotificationChannelRepository,
    NotificationSubscriptionRepository, PendingImportStatus, PluginInstallationRepository,
    ReleaseAttemptRepository, ReleaseDecision, ReleaseDownloadAttemptOutcome, ScopedExternalId,
    SettingsRepository, ShowRepository, SubmissionScope, SubtitleDownloadRepository,
    SubtitleProviderConfigRepository, SubtitleProviderConfigUpdate, TitleImageBlob, TitleImageKind,
    TitleImageReplacement, TitleImageRepository, TitleImageStorageMode, TitleImageVariantRecord,
    TitleMetadataUpdate, TitleRepository, UserRepository, WantedItem, WantedItemRepository,
    WantedItemsQuery, WantedStatus,
    subtitles::{ExternalSubtitleDetectionSource, ExternalSubtitleProbeCacheEntry},
};
use scryer_domain::{
    ChannelType, Collection, CollectionType, DomainEventFilter, DomainEventPayload,
    DomainEventStream, DownloadClientConfig, DownloadClientStatus, Episode, ExternalId, Id,
    ImportStatus, ImportType, InterstitialMovieMetadata, MediaFacet, NewDomainEvent,
    NotificationChannelConfig, NotificationEventType, NotificationSubscription,
    SubtitleProviderConfig, TaggedAlias, Title, TitleContextSnapshot, TitleUpdatedEventData,
};
use sqlx::{Row, sqlite::SqlitePoolOptions};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use tokio::time::{Duration, timeout};

fn test_descriptor_json(
    plugin_id: &str,
    version: &str,
    plugin_type: &str,
    provider_type: &str,
) -> String {
    fn indexer_config_fields() -> Vec<scryer_plugin_sdk::ConfigFieldDef> {
        vec![scryer_plugin_sdk::ConfigFieldDef {
            key: "base_url".to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_plugin_sdk::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_plugin_sdk::ConfigFieldValueSource::User,
            role: Some(scryer_plugin_sdk::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: Vec::new(),
            help_text: None,
        }]
    }

    let provider = match plugin_type {
        "indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Generic,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
            })
        }
        "usenet_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Usenet,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
            })
        }
        "torrent_indexer" => {
            scryer_plugin_sdk::ProviderDescriptor::Indexer(scryer_plugin_sdk::IndexerDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                source_kind: scryer_plugin_sdk::IndexerSourceKind::Torrent,
                capabilities: Default::default(),
                scoring_policies: Vec::new(),
                config_fields: indexer_config_fields(),
                allowed_hosts: Vec::new(),
                rate_limit_seconds: None,
            })
        }
        "notification" => scryer_plugin_sdk::ProviderDescriptor::Notification(
            scryer_plugin_sdk::NotificationDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
            },
        ),
        "download_client" => scryer_plugin_sdk::ProviderDescriptor::DownloadClient(
            scryer_plugin_sdk::DownloadClientDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                accepted_inputs: Vec::new(),
                isolation_modes: Vec::new(),
                capabilities: Default::default(),
            },
        ),
        "subtitle_provider" => {
            scryer_plugin_sdk::ProviderDescriptor::Subtitle(scryer_plugin_sdk::SubtitleDescriptor {
                provider_type: provider_type.to_string(),
                provider_aliases: Vec::new(),
                config_fields: Vec::new(),
                default_base_url: None,
                allowed_hosts: Vec::new(),
                capabilities: Default::default(),
            })
        }
        other => panic!("unsupported test plugin type: {other}"),
    };

    serde_json::to_string(&scryer_plugin_sdk::PluginDescriptor {
        id: plugin_id.to_string(),
        name: format!("{plugin_id} Plugin"),
        version: version.to_string(),
        sdk_version: scryer_plugin_sdk::SDK_VERSION.to_string(),
        sdk_constraint: scryer_plugin_sdk::current_sdk_constraint(),
        socket_permissions: Vec::new(),
        provider,
    })
    .expect("serialize test descriptor")
}

#[tokio::test]
async fn sqlite_can_initialize() {
    let db = std::env::temp_dir().join(format!(
        "scryer_store_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let users = UserRepository::list_all(&user_store(&services))
        .await
        .expect("query should return users after initialization");

    assert!(!users.is_empty());
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn seed_builtin_refreshes_existing_builtin_metadata_without_resetting_enabled_state() {
    let (services, db) = temp_services("scryer_plugin_builtin_refresh").await;
    let customization = PluginStore::new(services.datastore());

    customization
        .seed_builtin(
            "newznab",
            "Old Newznab",
            "old description",
            "0.1.0",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "indexer",
            "newznab",
        )
        .await
        .expect("initial builtin seed should succeed");

    let mut installation = customization
        .get_plugin_installation("newznab")
        .await
        .expect("load seeded builtin")
        .expect("builtin installation should exist");
    installation.is_enabled = false;
    customization
        .update_plugin_installation(&installation, None)
        .await
        .expect("disable builtin installation");

    customization
        .seed_builtin(
            "newznab",
            "Newznab Indexer",
            "new description",
            "0.2.2",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "usenet_indexer",
            "newznab",
        )
        .await
        .expect("refresh builtin seed should succeed");

    let refreshed = customization
        .get_plugin_installation("newznab")
        .await
        .expect("load refreshed builtin")
        .expect("refreshed builtin installation should exist");
    assert_eq!(refreshed.name, "Newznab Indexer");
    assert_eq!(refreshed.description, "new description");
    assert_eq!(refreshed.version, "0.2.2");
    assert_eq!(refreshed.plugin_type, "usenet_indexer");
    assert_eq!(refreshed.provider_type, "newznab");
    assert!(!refreshed.is_enabled);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn reverting_downloaded_builtin_clears_downloaded_artifact_state() {
    let (services, db) = temp_services("scryer_plugin_builtin_revert").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();
    let installation = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "newznab".to_string(),
        name: "Newznab".to_string(),
        description: "downloaded override".to_string(),
        version: "0.2.2".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "usenet_indexer".to_string(),
        provider_type: "newznab".to_string(),
        is_enabled: true,
        is_builtin: true,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/newznab-0.2.2.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: Some("abc123".to_string()),
        artifact_digest: None,
        descriptor_json: Some(test_descriptor_json(
            "newznab",
            "0.2.2",
            "usenet_indexer",
            "newznab",
        )),
        installed_at: now,
        updated_at: now,
    };

    customization
        .create_plugin_installation(&installation, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed downloaded builtin override");

    let mut reverted = installation.clone();
    reverted.source_kind = scryer_domain::PluginSourceKind::Bundled;
    reverted.wasm_encoding = scryer_domain::PluginWasmEncoding::Identity;
    reverted.wasm_digest_algo = None;
    reverted.source_url = None;
    reverted.wasm_digest = None;

    let reverted = customization
        .update_plugin_installation(&reverted, None)
        .await
        .expect("revert builtin override");

    assert_eq!(
        reverted.source_kind,
        scryer_domain::PluginSourceKind::Bundled
    );
    assert_eq!(
        reverted.wasm_encoding,
        scryer_domain::PluginWasmEncoding::Identity
    );
    assert!(reverted.wasm_digest_algo.is_none());
    assert!(reverted.wasm_digest.is_none());
    assert!(reverted.source_url.is_none());

    let enabled = customization
        .get_enabled_plugin_wasm_bytes()
        .await
        .expect("list enabled plugin wasm bytes");
    let (_, wasm_bytes) = enabled
        .into_iter()
        .find(|(item, _)| item.plugin_id == "newznab")
        .expect("reverted builtin should remain installed");
    assert!(wasm_bytes.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn cleanup_deletes_legacy_external_plugin_rows_and_preserves_builtins() {
    let (services, db) = temp_services("scryer_plugin_cleanup_legacy").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();

    let legacy_external = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "legacy-external".to_string(),
        name: "Legacy External".to_string(),
        description: "old registry install".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "legacy_external".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
        wasm_digest_algo: None,
        source_url: Some("https://example.com/legacy.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: None,
        artifact_digest: None,
        descriptor_json: None,
        installed_at: now,
        updated_at: now,
    };
    customization
        .create_plugin_installation(&legacy_external, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed legacy external install");

    customization
        .seed_builtin(
            "newznab",
            "Newznab",
            "builtin seed",
            "0.2.0",
            "1.3.0",
            ">=1.3.0, <1.4.0",
            "usenet_indexer",
            "newznab",
        )
        .await
        .expect("seed builtin install");

    let removed = customization
        .delete_incompatible_external_plugin_installations()
        .await
        .expect("cleanup incompatible external installs");

    assert_eq!(removed, vec!["legacy-external".to_string()]);
    assert!(
        customization
            .get_plugin_installation("legacy-external")
            .await
            .expect("read legacy install")
            .is_none()
    );
    assert!(
        customization
            .get_plugin_installation("newznab")
            .await
            .expect("read builtin install")
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn legacy_external_rows_are_hidden_and_do_not_block_reinstall() {
    let (services, db) = temp_services("scryer_plugin_reinstall_legacy").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();

    let legacy_external = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Legacy Email".to_string(),
        description: "old registry install".to_string(),
        version: "0.1.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Identity,
        wasm_digest_algo: None,
        source_url: Some("https://example.com/legacy-email.wasm".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: None,
        docs_url: None,
        source_repo: None,
        manifest_url: None,
        wasm_digest: None,
        artifact_digest: None,
        descriptor_json: None,
        installed_at: now,
        updated_at: now,
    };
    customization
        .create_plugin_installation(&legacy_external, Some(&[1_u8, 2, 3]))
        .await
        .expect("seed legacy external install");

    assert!(
        customization
            .get_plugin_installation("email")
            .await
            .expect("read hidden legacy install")
            .is_none()
    );
    assert!(
        customization
            .list_plugin_installations()
            .await
            .expect("list plugin installations")
            .into_iter()
            .all(|installation| installation.plugin_id != "email")
    );

    let compressed = zstd::encode_all(&b"catalog plugin bytes"[..], 1).expect("compress plugin");
    let replacement = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Email".to_string(),
        description: "catalog install".to_string(),
        version: "1.0.0".to_string(),
        sdk_version: "1.3.0".to_string(),
        sdk_constraint: ">=1.3.0, <1.4.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/catalog-email.wasm.zst".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: Some("Scryer".to_string()),
        docs_url: Some("https://example.com/docs".to_string()),
        source_repo: Some("https://github.com/example/email".to_string()),
        manifest_url: Some("https://example.com/email.manifest.json".to_string()),
        wasm_digest: Some("abcdef0123456789".to_string()),
        artifact_digest: Some("sha256:artifact".to_string()),
        descriptor_json: Some(test_descriptor_json(
            "email",
            "1.0.0",
            "notification",
            "email",
        )),
        installed_at: now,
        updated_at: now,
    };

    let created = customization
        .create_plugin_installation(&replacement, Some(&compressed))
        .await
        .expect("create replacement install");
    assert_eq!(created.version, "1.0.0");
    assert_eq!(
        created.wasm_encoding,
        scryer_domain::PluginWasmEncoding::Zstd
    );
    assert_eq!(created.wasm_digest_algo.as_deref(), Some("blake3"));

    let row_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM plugin_installations WHERE plugin_id = 'email'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("count email rows");
    assert_eq!(row_count, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn plugin_catalog_source_rows_survive_cleanup() {
    let (services, db) = temp_services("scryer_plugin_catalog_source_shape").await;
    let customization = PluginStore::new(services.datastore());
    let source = scryer_domain::PluginCatalogSource {
        source_key: "__central_catalog_v2".to_string(),
        source_kind: "central".to_string(),
        source_url: "https://example.com/catalog-v2.min.json.zst".to_string(),
        github_repo: Some("scryer-media/scryer-plugins".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        catalog_json: Some(
            r#"{"schema_version":"scryer.plugin.catalog.v2","plugins":[],"rule_packs":[]}"#
                .to_string(),
        ),
        last_success_at: Some(Utc::now()),
        last_error: None,
        updated_at: Utc::now(),
    };

    customization
        .upsert_plugin_catalog_source(&source)
        .await
        .expect("store plugin catalog source");

    assert_eq!(
        customization
            .get_plugin_catalog_source("__central_catalog_v2")
            .await
            .expect("read plugin catalog source"),
        Some(source.clone())
    );

    let removed = customization
        .delete_incompatible_external_plugin_installations()
        .await
        .expect("cleanup incompatible external installs");
    assert!(removed.is_empty());
    assert_eq!(
        customization
            .get_plugin_catalog_source("__central_catalog_v2")
            .await
            .expect("read plugin catalog source after cleanup"),
        Some(source.clone())
    );

    let row = sqlx::query(
        "SELECT source_kind, support_tier FROM plugin_catalog_sources WHERE source_key = '__central_catalog_v2'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("load plugin catalog source row");
    assert_eq!(row.get::<String, _>("source_kind"), "central");
    assert_eq!(row.get::<String, _>("support_tier"), "official");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn enabled_plugin_payloads_preserve_zstd_encoding() {
    let (services, db) = temp_services("scryer_plugin_payload_encoding").await;
    let customization = PluginStore::new(services.datastore());
    let now = Utc::now();
    let wasm_bytes = b"hello compressed plugin";
    let compressed = zstd::encode_all(&wasm_bytes[..], 1).expect("compress plugin bytes");

    let installation = scryer_domain::PluginInstallation {
        id: scryer_domain::Id::new().0,
        plugin_id: "email".to_string(),
        name: "Email".to_string(),
        description: "catalog install".to_string(),
        version: "0.1.2".to_string(),
        sdk_version: "1.6.0".to_string(),
        sdk_constraint: ">=1.6.0, <2.0.0".to_string(),
        scryer_constraint: None,
        plugin_type: "notification".to_string(),
        provider_type: "email".to_string(),
        is_enabled: true,
        is_builtin: false,
        source_kind: scryer_domain::PluginSourceKind::Downloaded,
        wasm_encoding: scryer_domain::PluginWasmEncoding::Zstd,
        wasm_digest_algo: Some("blake3".to_string()),
        source_url: Some("https://example.com/email/plugin.wasm.zst".to_string()),
        support_tier: scryer_domain::PluginSupportTier::Official,
        publisher: Some("scryer-media".to_string()),
        docs_url: None,
        source_repo: Some("https://github.com/scryer-media/scryer-plugins".to_string()),
        manifest_url: Some("https://example.com/email/plugin.manifest.json".to_string()),
        wasm_digest: Some(
            scryer_application::plugin_wasm_blake3_digest(wasm_bytes)
                .split_once(':')
                .expect("digest should include an algorithm prefix")
                .1
                .to_string(),
        ),
        artifact_digest: Some("blake3:abcd".to_string()),
        descriptor_json: Some(test_descriptor_json(
            "email",
            "0.1.2",
            "notification",
            "email",
        )),
        installed_at: now,
        updated_at: now,
    };

    customization
        .create_plugin_installation(&installation, Some(compressed.as_slice()))
        .await
        .expect("seed encoded plugin install");

    let enabled = customization
        .get_enabled_plugin_wasm_bytes()
        .await
        .expect("list enabled plugin payloads");
    let (_, payload) = enabled
        .into_iter()
        .find(|(item, _)| item.plugin_id == "email")
        .expect("email installation should be present");
    let payload = payload.expect("payload should be present");

    assert_eq!(payload.encoding, scryer_domain::PluginWasmEncoding::Zstd);
    assert_eq!(payload.bytes, compressed);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_imports_for_identities_handles_multiple_pairs() {
    let db = std::env::temp_dir().join(format!(
        "scryer_import_sources_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = ImportStore::new(services.datastore());

    workflow
        .queue_import_request(
            DownloadSourceIdentity::new(Some("client-a"), "weaver", "10000"),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("first import should queue");
    workflow
        .queue_import_request(
            DownloadSourceIdentity::new(Some("client-b"), "weaver", "10001"),
            ImportType::ManualImport.as_str().to_string(),
            "{}".to_string(),
        )
        .await
        .expect("second import should queue");

    let records = workflow
        .list_imports_for_identities(&[
            DownloadSourceIdentity::new(Some("client-a"), "weaver", "10000"),
            DownloadSourceIdentity::new(Some("client-b"), "weaver", "10001"),
        ])
        .await
        .expect("batch lookup should succeed");

    assert_eq!(records.len(), 2);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn queue_import_request_reuses_existing_row_for_same_identity() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("pool should initialize");
    sqlx::query(
        "CREATE TABLE imports (
            id TEXT PRIMARY KEY,
            source_client_id TEXT,
            source_system TEXT NOT NULL,
            source_ref TEXT NOT NULL,
            import_type TEXT NOT NULL,
            status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            result_json TEXT,
            rename_plan_json TEXT,
            started_at TEXT,
            finished_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("imports table should create");
    sqlx::query(
        "CREATE UNIQUE INDEX idx_imports_source_ref
         ON imports (COALESCE(source_client_id, ''), source_system, source_ref, import_type)",
    )
    .execute(&pool)
    .await
    .expect("imports identity index should create");

    let workflow = ImportStore::new(crate::queries::sql_runtime::StoreDatastore::Sqlite {
        pool: pool.clone(),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    });
    let identity = DownloadSourceIdentity::new(Some("client-a"), "weaver", "10000");

    let first_id = workflow
        .queue_import_request(
            identity.clone(),
            ImportType::ManualImport.as_str().to_string(),
            "{\"attempt\":1}".to_string(),
        )
        .await
        .expect("first import should queue");
    workflow
        .update_import_status(
            &first_id,
            ImportStatus::Completed,
            Some("{\"result\":\"done\"}".to_string()),
        )
        .await
        .expect("import status should update");

    let second_id = workflow
        .queue_import_request(
            identity,
            ImportType::ManualImport.as_str().to_string(),
            "{\"attempt\":2}".to_string(),
        )
        .await
        .expect("second import should requeue");

    assert_eq!(second_id, first_id);

    let record = workflow
        .get_import_by_id(&second_id)
        .await
        .expect("import lookup should succeed")
        .expect("import should exist");
    assert_eq!(record.status, ImportStatus::Pending);
    assert_eq!(record.payload_json, "{\"attempt\":2}");
    assert_eq!(record.result_json, None);

    let row_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM imports
         WHERE COALESCE(source_client_id, '') = ?
           AND source_system = ?
           AND source_ref = ?
           AND import_type = ?",
    )
    .bind("client-a")
    .bind("weaver")
    .bind("10000")
    .bind(ImportType::ManualImport.as_str())
    .fetch_one(&pool)
    .await
    .expect("import count should load");
    assert_eq!(row_count, 1);
}

#[test]
fn download_submission_lookup_chunks_and_deduplicates_client_items() {
    let mut client_items = (0..805)
        .map(|idx| DownloadSourceIdentity::new(None, "weaver", format!("job-{idx}")))
        .collect::<Vec<_>>();
    client_items.push(DownloadSourceIdentity::new(None, "weaver", "job-12"));
    client_items.push(DownloadSourceIdentity::new(None, "weaver", "job-400"));

    let chunks = crate::workflow_store::chunk_download_submission_client_items(&client_items);

    assert_eq!(chunks.len(), 3);
    assert_eq!(chunks[0].len(), 400);
    assert_eq!(chunks[1].len(), 400);
    assert_eq!(chunks[2].len(), 5);
    assert_eq!(
        chunks[0][12],
        DownloadSourceIdentity::new(None, "weaver", "job-12")
    );
    assert_eq!(
        chunks
            .iter()
            .flat_map(|chunk| chunk.iter())
            .filter(|identity| identity.client_type == "weaver" && identity.item_id == "job-12")
            .count(),
        1
    );
}

#[tokio::test]
async fn list_download_submissions_for_client_items_handles_large_batched_lookup() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_sources_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    for idx in 0..805 {
        workflow
            .record_submission(DownloadSubmission {
                title_id: format!("title-{idx}"),
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "weaver".to_string(),
                download_client_item_id: format!("job-{idx}"),
                source_hint: None,
                source_kind: None,
                source_title: Some(format!("Release {idx}")),
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record submission should succeed");
    }

    let mut lookup = (0..805)
        .map(|idx| DownloadSourceIdentity::new(None, "weaver", format!("job-{idx}")))
        .collect::<Vec<_>>();
    lookup.push(DownloadSourceIdentity::new(None, "weaver", "job-12"));
    lookup.push(DownloadSourceIdentity::new(None, "weaver", "job-400"));

    let records = workflow
        .list_for_client_items(&lookup)
        .await
        .expect("batched lookup should succeed");

    assert_eq!(records.len(), 805);
    assert!(records.iter().any(|record| {
        record.download_client_type == "weaver" && record.download_client_item_id == "job-804"
    }));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn download_submission_identity_does_not_fall_back_to_legacy_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_identity_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    workflow
        .record_submission(DownloadSubmission {
            title_id: "legacy-title".to_string(),
            facet: "movie".to_string(),
            download_client_id: None,
            download_client_type: "weaver".to_string(),
            download_client_item_id: "shared-job".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Legacy Release".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("legacy submission should persist");

    let exact_client_lookup = workflow
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            Some("client-a"),
            "weaver",
            "shared-job",
        ))
        .await
        .expect("exact client lookup should succeed");
    assert!(exact_client_lookup.is_none());

    let legacy_lookup = workflow
        .find_by_client_item_id(&DownloadSourceIdentity::new(None, "weaver", "shared-job"))
        .await
        .expect("legacy lookup should succeed")
        .expect("legacy row should still be discoverable by a legacy identity");
    assert_eq!(legacy_lookup.title_id, "legacy-title");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn record_download_submission_persists_episode_set_scope() {
    let db = std::env::temp_dir().join(format!(
        "scryer_download_submission_episode_set_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = DownloadSubmissionStore::new(services.datastore());

    workflow
        .record_submission(DownloadSubmission {
            title_id: "title-1".to_string(),
            facet: "anime".to_string(),
            download_client_id: Some("client-a".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "job-range".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("BASTARD 01-13".to_string()),
            request_signature: None,
            scope: SubmissionScope::EpisodeSet {
                episode_ids: vec!["ep-13".to_string(), "ep-1".to_string()],
            },
        })
        .await
        .expect("record submission should succeed");

    let record = workflow
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            Some("client-a"),
            "weaver",
            "job-range",
        ))
        .await
        .expect("lookup should succeed")
        .expect("submission should exist");

    assert_eq!(
        record.scope,
        SubmissionScope::EpisodeSet {
            episode_ids: vec!["ep-1".to_string(), "ep-13".to_string()]
        }
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_settings_batch_and_encrypted_upserts() {
    let (services, db) = temp_services("scryer_settings_writer").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([7; 32]))
        .await
        .expect("encryption key should set");
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "general".to_string(),
            scope: "system".to_string(),
            key_name: "secret.value".to_string(),
            data_type: "string".to_string(),
            default_value_json: "\"default\"".to_string(),
            is_sensitive: true,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    settings
        .batch_upsert_settings_if_not_overridden(vec![(
            "system".to_string(),
            "secret.value".to_string(),
            "\"seeded\"".to_string(),
            "migration".to_string(),
        )])
        .await
        .expect("batch upsert should succeed");

    let seeded = settings
        .get_setting_with_defaults("system", "secret.value", None)
        .await
        .expect("seeded setting should load")
        .expect("seeded setting should exist");
    assert_eq!(seeded.effective_value_json, "\"seeded\"");

    let updated = settings
        .upsert_setting_value(
            "system",
            "secret.value",
            None,
            "\"overridden\"",
            "user",
            None,
        )
        .await
        .expect("direct upsert should succeed");
    assert_eq!(updated.effective_value_json, "\"overridden\"");

    settings
        .delete_setting_value("system", "secret.value", None)
        .await
        .expect("delete override should succeed");

    let reverted = settings
        .get_setting_with_defaults("system", "secret.value", None)
        .await
        .expect("setting should still load")
        .expect("setting should still exist");
    assert_eq!(reverted.effective_value_json, "\"default\"");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn settings_with_defaults_store_reads_scoped_overrides() {
    let (services, db) = temp_services("scryer_settings_parity").await;
    let encryption_key = crate::encryption::EncryptionKey::from_bytes([11; 32]);
    services
        .set_encryption_key(encryption_key.clone())
        .await
        .expect("encryption key should set");
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![
            crate::types::SettingDefinitionSeed {
                category: "general".to_string(),
                scope: "system".to_string(),
                key_name: "secret.global".to_string(),
                data_type: "string".to_string(),
                default_value_json: "\"default-global\"".to_string(),
                is_sensitive: true,
                validation_json: None,
            },
            crate::types::SettingDefinitionSeed {
                category: "general".to_string(),
                scope: "system".to_string(),
                key_name: "secret.scoped".to_string(),
                data_type: "string".to_string(),
                default_value_json: "\"default-scoped\"".to_string(),
                is_sensitive: true,
                validation_json: None,
            },
        ])
        .await
        .expect("definitions should seed");

    settings
        .upsert_setting_value(
            "system",
            "secret.global",
            None,
            "\"overridden-global\"",
            "user",
            None,
        )
        .await
        .expect("global override should succeed");
    settings
        .upsert_setting_value(
            "system",
            "secret.scoped",
            Some("movie".to_string()),
            "\"overridden-scoped\"",
            "user",
            None,
        )
        .await
        .expect("scoped override should succeed");

    let query_rows = settings
        .list_settings_with_defaults("system", Some("movie".to_string()))
        .await
        .expect("settings should load");

    let summarize = |rows: Vec<crate::types::SettingsValueRecord>| {
        let mut summary = rows
            .into_iter()
            .map(|row| {
                (
                    row.key_name,
                    row.scope_id,
                    row.effective_value_json,
                    row.value_json,
                    row.source,
                    row.is_sensitive,
                )
            })
            .collect::<Vec<_>>();
        summary.sort_by(|left, right| left.0.cmp(&right.0));
        summary
    };
    assert!(summarize(query_rows).contains(&(
        "secret.scoped".to_string(),
        Some("movie".to_string()),
        "\"overridden-scoped\"".to_string(),
        Some("\"overridden-scoped\"".to_string()),
        Some("user".to_string()),
        true,
    )));

    let query_record = settings
        .get_setting_with_defaults("system", "secret.scoped", Some("movie".to_string()))
        .await
        .expect("single setting should load");
    let summarize_record = |row: Option<crate::types::SettingsValueRecord>| {
        row.map(|record| {
            (
                record.key_name,
                record.scope_id,
                record.effective_value_json,
                record.value_json,
                record.source,
                record.is_sensitive,
            )
        })
    };
    assert_eq!(
        summarize_record(query_record),
        Some((
            "secret.scoped".to_string(),
            Some("movie".to_string()),
            "\"overridden-scoped\"".to_string(),
            Some("\"overridden-scoped\"".to_string()),
            Some("user".to_string()),
            true,
        ))
    );
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn explicit_setting_query_skips_definition_defaults_for_missing_scopes() {
    let (services, db) = temp_services("scryer_settings_explicit").await;
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "media".to_string(),
            scope: "system".to_string(),
            key_name: "quality.profile_id".to_string(),
            data_type: "string".to_string(),
            default_value_json: "\"4k\"".to_string(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    settings
        .upsert_setting_value(
            "system",
            "quality.profile_id",
            Some("series".to_string()),
            "\"wizard-series\"",
            "user",
            None,
        )
        .await
        .expect("facet override should save");

    let inherited = settings
        .get_setting_json(
            "system",
            "quality.profile_id",
            Some("series_default_library".to_string()),
        )
        .await
        .expect("inherited lookup should succeed");
    assert_eq!(inherited.as_deref(), Some("\"4k\""));

    let explicit = settings
        .get_setting_json_explicit(
            "system",
            "quality.profile_id",
            Some("series_default_library".to_string()),
        )
        .await
        .expect("explicit lookup should succeed");
    assert_eq!(explicit, None);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_library_scoped_download_client_routing_round_trips_explicit_json() {
    let (services, db) = temp_services("scryer_library_download_client_routing").await;
    let settings = SettingsStore::new(services.datastore(), services.encryption_key_state());

    settings
        .batch_ensure_setting_definitions(vec![crate::types::SettingDefinitionSeed {
            category: "media".to_string(),
            scope: "system".to_string(),
            key_name: "download_client.routing".to_string(),
            data_type: "json".to_string(),
            default_value_json: "{}".to_string(),
            is_sensitive: false,
            validation_json: None,
        }])
        .await
        .expect("definitions should seed");

    let library_id = "series_default_library";
    let value_json = serde_json::json!({
        "weaver": {
            "enabled": true,
            "category": "series",
            "recentQueuePriority": "high",
            "olderQueuePriority": "normal",
            "removeCompleted": true,
            "removeFailed": false
        }
    })
    .to_string();

    SettingsRepository::upsert_setting_json(
        &settings,
        "system",
        "download_client.routing",
        Some(library_id.to_string()),
        value_json.clone(),
        "test",
        None,
    )
    .await
    .expect("library-scoped routing should save");

    let explicit = SettingsRepository::get_setting_json_explicit(
        &settings,
        "system",
        "download_client.routing",
        Some(library_id.to_string()),
    )
    .await
    .expect("explicit lookup should succeed");
    assert_eq!(explicit.as_deref(), Some(value_json.as_str()));

    let default_lookup = SettingsRepository::get_setting_json(
        &settings,
        "system",
        "download_client.routing",
        Some("another_library".to_string()),
    )
    .await
    .expect("default lookup should succeed");
    assert_eq!(default_lookup.as_deref(), Some("{}"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_notification_channel_and_subscription_round_trip() {
    let (services, db) = temp_services("scryer_notification_writer").await;
    services
        .set_encryption_key(crate::encryption::EncryptionKey::from_bytes([9; 32]))
        .await
        .expect("encryption key should set");
    let store = NotificationStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();

    let channel = NotificationChannelConfig {
        id: "channel-1".to_string(),
        name: "Discord".to_string(),
        channel_type: ChannelType::parse("discord").expect("channel type"),
        config_json: r#"{"url":"https://example.com/webhook"}"#.to_string(),
        is_enabled: true,
        created_at: now,
        updated_at: now,
    };
    NotificationChannelRepository::create_channel(&store, channel.clone())
        .await
        .expect("channel should create");

    let fetched = NotificationChannelRepository::get_channel(&store, &channel.id)
        .await
        .expect("channel lookup should succeed")
        .expect("channel should exist");
    assert_eq!(fetched.config_json, channel.config_json);

    let updated_channel = NotificationChannelConfig {
        name: "Discord Alerts".to_string(),
        config_json: r#"{"url":"https://example.com/updated"}"#.to_string(),
        is_enabled: false,
        updated_at: Utc::now(),
        ..fetched.clone()
    };
    let updated = NotificationChannelRepository::update_channel(&store, updated_channel.clone())
        .await
        .expect("channel should update");
    assert_eq!(updated.name, "Discord Alerts");
    assert_eq!(updated.config_json, updated_channel.config_json);

    let subscription = NotificationSubscription {
        id: "subscription-1".to_string(),
        channel_id: updated.id.clone(),
        event_type: NotificationEventType::ImportComplete,
        scope: "global".to_string(),
        scope_id: None,
        is_enabled: true,
        created_at: now,
        updated_at: now,
    };
    NotificationSubscriptionRepository::create_subscription(&store, subscription.clone())
        .await
        .expect("subscription should create");

    let updated_subscription = NotificationSubscription {
        is_enabled: false,
        updated_at: Utc::now(),
        ..subscription.clone()
    };
    NotificationSubscriptionRepository::update_subscription(&store, updated_subscription.clone())
        .await
        .expect("subscription should update");

    let later_subscription = NotificationSubscription {
        id: "subscription-2".to_string(),
        scope: "movie".to_string(),
        scope_id: Some("title-1".to_string()),
        created_at: now + chrono::Duration::seconds(1),
        updated_at: now + chrono::Duration::seconds(1),
        ..subscription.clone()
    };
    NotificationSubscriptionRepository::create_subscription(&store, later_subscription.clone())
        .await
        .expect("second subscription should create");

    let by_event = NotificationSubscriptionRepository::list_subscriptions_for_event(
        &store,
        NotificationEventType::ImportComplete,
    )
    .await
    .expect("event subscriptions should load");
    assert_eq!(by_event.len(), 2);
    assert_eq!(by_event[0].id, later_subscription.id);
    assert_eq!(by_event[1].id, subscription.id);
    assert!(
        !by_event[1].is_enabled,
        "event listing should preserve disabled rows for dispatcher-side filtering"
    );

    let by_channel =
        NotificationSubscriptionRepository::list_subscriptions_for_channel(&store, &updated.id)
            .await
            .expect("subscription list should load");
    assert_eq!(by_channel.len(), 2);

    NotificationSubscriptionRepository::delete_subscription(&store, &subscription.id)
        .await
        .expect("subscription should delete");
    NotificationSubscriptionRepository::delete_subscription(&store, &later_subscription.id)
        .await
        .expect("second subscription should delete");
    assert!(matches!(
        NotificationSubscriptionRepository::delete_subscription(&store, &subscription.id).await,
        Err(scryer_application::AppError::NotFound(_))
    ));
    NotificationChannelRepository::delete_channel(&store, &updated.id)
        .await
        .expect("channel should delete");
    assert!(matches!(
        NotificationChannelRepository::delete_channel(&store, &updated.id).await,
        Err(scryer_application::AppError::NotFound(_))
    ));

    let remaining =
        NotificationSubscriptionRepository::list_subscriptions_for_channel(&store, &updated.id)
            .await
            .expect("subscription list should still load");
    assert!(remaining.is_empty());
    assert!(
        NotificationChannelRepository::get_channel(&store, &updated.id)
            .await
            .expect("channel lookup should succeed")
            .is_none()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_download_client_reorder() {
    let (services, db) = temp_services("scryer_download_client_writer").await;
    let store =
        DownloadClientConfigStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();

    let client_a = DownloadClientConfig {
        id: "client-a".to_string(),
        name: "Client A".to_string(),
        client_type: "weaver".to_string(),
        config_json: "{}".to_string(),
        client_priority: 0,
        is_enabled: true,
        status: DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: now,
        updated_at: now,
    };
    let client_b = DownloadClientConfig {
        id: "client-b".to_string(),
        name: "Client B".to_string(),
        client_type: "sabnzbd".to_string(),
        config_json: "{}".to_string(),
        client_priority: 1,
        is_enabled: true,
        status: DownloadClientStatus::Healthy,
        last_error: None,
        last_seen_at: None,
        created_at: now,
        updated_at: now,
    };

    DownloadClientConfigRepository::create(&store, client_a.clone())
        .await
        .expect("first client should create");
    DownloadClientConfigRepository::create(&store, client_b.clone())
        .await
        .expect("second client should create");

    DownloadClientConfigRepository::reorder(&store, vec![client_b.id.clone(), client_a.id.clone()])
        .await
        .expect("reorder should succeed");

    let ordered = DownloadClientConfigRepository::list(&store, None)
        .await
        .expect("clients should list");
    let ordered_ids: Vec<String> = ordered.into_iter().map(|client| client.id).collect();
    assert_eq!(ordered_ids, vec![client_b.id, client_a.id]);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn serialized_writer_handles_release_attempts_and_vacuum_into() {
    let (services, db) = temp_services("scryer_release_writer").await;
    let release_store = ReleaseStore::new(services.datastore());

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        None,
        Some("weaver".to_string()),
        Some("Farwander.S08E05".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom".to_string()),
        Some("secret".to_string()),
    )
    .await
    .expect("release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].source_hint.as_deref(), Some("weaver"));

    let latest_password = ReleaseAttemptRepository::get_latest_source_password(
        &release_store,
        None,
        Some("weaver"),
        Some("Farwander.S08E05"),
    )
    .await
    .expect("latest password should load");
    assert_eq!(latest_password.as_deref(), Some("secret"));

    let vacuum_dest = std::env::temp_dir().join(format!(
        "scryer_release_writer_copy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    services
        .vacuum_into(vacuum_dest.to_string_lossy().as_ref())
        .await
        .expect("vacuum into should succeed");
    assert!(vacuum_dest.exists());

    let _ = std::fs::remove_file(vacuum_dest);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn release_attempt_queries_dedupe_failed_signatures_by_normalized_source_title() {
    let (services, db) = temp_services("scryer_release_dedupe").await;
    let release_store = ReleaseStore::new(services.datastore());
    let catalog = title_store(&services);

    catalog
        .create_or_get_existing(make_test_title("title-1", None))
        .await
        .expect("title should insert");

    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-1".to_string()),
        Some("weaver-1".to_string()),
        Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom-1".to_string()),
        None,
    )
    .await
    .expect("first release attempt should record");
    ReleaseAttemptRepository::record_release_attempt(
        &release_store,
        Some("title-1".to_string()),
        Some("weaver-2".to_string()),
        Some(" friends.s05.720p.bluray.dd5.1.x264-ntb ".to_string()),
        ReleaseDownloadAttemptOutcome::Failed,
        Some("boom-2".to_string()),
        None,
    )
    .await
    .expect("second release attempt should record");

    let failures = ReleaseAttemptRepository::list_failed_release_signatures(&release_store, 10)
        .await
        .expect("failed signatures should list");
    assert_eq!(failures.len(), 1);

    let title_failures = ReleaseAttemptRepository::list_failed_release_signatures_for_title(
        &release_store,
        "title-1",
        10,
    )
    .await
    .expect("title failed signatures should list");
    assert_eq!(title_failures.len(), 1);

    let _ = std::fs::remove_file(db);
}

fn make_test_title(id: &str, poster_url: Option<&str>) -> Title {
    Title {
        id: id.to_string(),
        name: "Poster Test".to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: Some(2026),
        overview: Some("overview".to_string()),
        poster_url: poster_url.map(str::to_string),
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn title_store(services: &SqliteServices) -> TitleStore {
    TitleStore::new(services.datastore())
}

fn show_store(services: &SqliteServices) -> ShowStore {
    ShowStore::new(services.datastore())
}

fn user_store(services: &SqliteServices) -> UserStore {
    UserStore::new(services.datastore())
}

fn wanted_store(services: &SqliteServices) -> WantedStore {
    WantedStore::new(services.datastore())
}

fn housekeeping_store(services: &SqliteServices) -> HousekeepingStore {
    HousekeepingStore::new(services.datastore())
}

fn subtitle_download_store(services: &SqliteServices) -> SubtitleDownloadStore {
    SubtitleDownloadStore::new(services.datastore())
}

fn media_file_store(services: &SqliteServices) -> MediaFileStore {
    MediaFileStore::new(services.datastore())
}

fn library_scan_unmatched_store(services: &SqliteServices) -> LibraryScanUnmatchedStore {
    LibraryScanUnmatchedStore::new(services.datastore())
}

fn title_image_store(services: &SqliteServices) -> TitleImageStore {
    TitleImageStore::new(services.datastore())
}

async fn temp_services(prefix: &str) -> (SqliteServices, std::path::PathBuf) {
    let db = std::env::temp_dir().join(format!(
        "{}_{}.db",
        prefix,
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    (services, db)
}

#[tokio::test]
async fn sqlite_database_maintenance_runs_without_command_bus() {
    let (services, db) = temp_services("scryer_sqlite_database_maintenance").await;
    let housekeeping = housekeeping_store(&services);

    housekeeping
        .run_database_maintenance()
        .await
        .expect("database maintenance should complete");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn external_subtitle_probe_cache_round_trips_replace_and_delete() {
    let (services, db) = temp_services("scryer_external_subtitle_probe_cache").await;
    let catalog = title_store(&services);
    let media_files = media_file_store(&services);
    let subtitles = subtitle_download_store(&services);

    let title = make_test_title("title-probe-cache", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let media_file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Example.Movie.mkv".to_string(),
            size_bytes: 4_096,
            ..Default::default()
        })
        .await
        .expect("media file should insert");

    let initial = ExternalSubtitleProbeCacheEntry {
        media_file_id: media_file_id.clone(),
        file_path: "/tmp/Example.Movie.srt".to_string(),
        size_bytes: 512,
        modified_at: Some("2026-04-29T00:00:00Z".to_string()),
        language: None,
        hearing_impaired: None,
        detection_source_language: ExternalSubtitleDetectionSource::Unknown,
        detection_source_hi: ExternalSubtitleDetectionSource::Unknown,
        probe_version: 2,
        updated_at: "2026-04-29T00:00:01Z".to_string(),
    };

    subtitles
        .upsert_probe_cache_entry(&initial)
        .await
        .expect("initial probe cache row should insert");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("probe cache rows should list");
    assert_eq!(listed, vec![initial.clone()]);

    let replaced = ExternalSubtitleProbeCacheEntry {
        language: Some("eng".to_string()),
        hearing_impaired: Some(true),
        detection_source_language: ExternalSubtitleDetectionSource::Content,
        detection_source_hi: ExternalSubtitleDetectionSource::Content,
        updated_at: "2026-04-29T00:00:02Z".to_string(),
        ..initial
    };

    subtitles
        .upsert_probe_cache_entry(&replaced)
        .await
        .expect("probe cache row should replace");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("replaced probe cache row should list");
    assert_eq!(listed, vec![replaced.clone()]);

    subtitles
        .delete_probe_cache_entry(&media_file_id, "/tmp/Example.Movie.srt")
        .await
        .expect("probe cache row should delete");

    let listed = subtitles
        .list_probe_cache_for_media_file(&media_file_id)
        .await
        .expect("probe cache rows should list after delete");
    assert!(listed.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn scoped_anibridge_external_ids_round_trip_for_collections_and_episodes() {
    let (services, db) = temp_services("scryer_scoped_anibridge_ids").await;
    let catalog = title_store(&services);
    let shows = show_store(&services);

    let mut title = make_test_title("title-anime", None);
    title.facet = MediaFacet::Anime;
    title.external_ids = vec![ExternalId {
        source: "tvdb_id".to_string(),
        value: "431162".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let collection = Collection {
        id: "season-2".to_string(),
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "2".to_string(),
        label: Some("Season 2".to_string()),
        ordered_path: None,
        narrative_order: Some("2".to_string()),
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("24".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&shows, collection.clone())
        .await
        .expect("collection should insert");

    let episode = Episode {
        id: "episode-s02e23".to_string(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("23".to_string()),
        season_number: Some("2".to_string()),
        episode_label: Some("S02E23".to_string()),
        title: Some("Episode 23".to_string()),
        air_date: Some("2025-06-13".to_string()),
        duration_seconds: Some(1_440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: Some("47".to_string()),
        overview: None,
        tvdb_id: Some("1234567".to_string()),
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_episode(&shows, episode.clone())
        .await
        .expect("episode should insert");

    ShowRepository::replace_anibridge_scoped_external_ids_for_title(
        &shows,
        &title.id,
        vec![ScopedExternalId {
            scope_id: collection.id.clone(),
            source: "anilist".to_string(),
            external_id: "176301".to_string(),
            provenance: "anibridge".to_string(),
            source_scope: Some("R".to_string()),
        }],
        vec![ScopedExternalId {
            scope_id: episode.id.clone(),
            source: "anidb".to_string(),
            external_id: "18562".to_string(),
            provenance: "anibridge".to_string(),
            source_scope: Some("R".to_string()),
        }],
    )
    .await
    .expect("replace scoped ids should succeed");

    let collection_ids = ShowRepository::list_collection_external_ids(&shows, &collection.id)
        .await
        .expect("collection ids should load");
    assert_eq!(collection_ids.len(), 1);
    assert_eq!(collection_ids[0].scope_id, collection.id);
    assert_eq!(collection_ids[0].source, "anilist");
    assert_eq!(collection_ids[0].external_id, "176301");
    assert_eq!(collection_ids[0].source_scope.as_deref(), Some("R"));

    let episode_ids = ShowRepository::list_episode_external_ids(&shows, &episode.id)
        .await
        .expect("episode ids should load");
    assert_eq!(episode_ids.len(), 1);
    assert_eq!(episode_ids[0].scope_id, episode.id);
    assert_eq!(episode_ids[0].source, "anidb");
    assert_eq!(episode_ids[0].external_id, "18562");
    assert_eq!(episode_ids[0].source_scope.as_deref(), Some("R"));

    let missing =
        TitleRepository::list_anime_title_ids_missing_anibridge_scoped_external_ids(&catalog, 10)
            .await
            .expect("missing scoped-id backfill query should run");
    assert!(!missing.contains(&title.id));

    let missing_title_anidb =
        TitleRepository::list_anime_title_ids_missing_title_anidb_external_ids(&catalog, 10)
            .await
            .expect("missing title AniDB backfill query should run");
    assert!(missing_title_anidb.contains(&title.id));

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("eng".to_string()),
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            extra_external_ids: vec![ExternalId {
                source: "anidb".to_string(),
                value: "18562".to_string(),
            }],
            ..TitleMetadataUpdate::default()
        },
    )
    .await
    .expect("hydrated title metadata should persist title-level AniDB");

    let missing_title_anidb =
        TitleRepository::list_anime_title_ids_missing_title_anidb_external_ids(&catalog, 10)
            .await
            .expect("missing title AniDB backfill query should rerun");
    assert!(!missing_title_anidb.contains(&title.id));

    let _ = std::fs::remove_file(db);
}

async fn run_embedded_migration(pool: &sqlx::SqlitePool, sql: &str) {
    for statement in sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(sqlx::AssertSqlSafe(statement.to_owned()))
            .execute(pool)
            .await
            .expect("migration statement should succeed");
    }
}

#[tokio::test]
async fn release_metadata_enum_canonicalization_migration_normalizes_legacy_values() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE media_files (
            id TEXT PRIMARY KEY,
            source_type TEXT,
            video_codec TEXT,
            video_codec_parsed TEXT,
            audio_codec_parsed TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("media_files fixture table should be created");

    for statement in [
        "CREATE TABLE quality_profile_source_allowlist (
            profile_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, source)
        )",
        "CREATE TABLE quality_profile_source_blocklist (
            profile_id TEXT NOT NULL,
            source TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, source)
        )",
        "CREATE TABLE quality_profile_audio_codec_allowlist (
            profile_id TEXT NOT NULL,
            codec TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, codec)
        )",
        "CREATE TABLE quality_profile_audio_codec_blocklist (
            profile_id TEXT NOT NULL,
            codec TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (profile_id, codec)
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("quality profile fixture table should be created");
    }

    sqlx::query(
        "INSERT INTO media_files
            (id, source_type, video_codec, video_codec_parsed, audio_codec_parsed)
         VALUES
            ('known', 'webdl', 'x264', 'HEVC', 'DTS-HD MA'),
            ('unknown', 'mystery-source', 'mystery-video', 'mystery-parsed', 'mystery-audio')",
    )
    .execute(&pool)
    .await
    .expect("media file fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_source_allowlist(profile_id, source, created_at)
         VALUES
            ('p1', 'webdl', '2026-01-01T00:00:00Z'),
            ('p1', 'WEB-DL', '2026-01-02T00:00:00Z'),
            ('p1', 'not-a-source', '2026-01-03T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("source allowlist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_source_blocklist(profile_id, source, created_at)
         VALUES
            ('p1', 'bdmv', '2026-01-01T00:00:00Z'),
            ('p1', 'BRDISK', '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("source blocklist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_audio_codec_allowlist(profile_id, codec, created_at)
         VALUES
            ('p1', 'DTS-HD MA', '2026-01-01T00:00:00Z'),
            ('p1', 'DTSMA', '2026-01-02T00:00:00Z'),
            ('p1', 'not-a-codec', '2026-01-03T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("audio allowlist fixture rows should be inserted");

    sqlx::query(
        "INSERT INTO quality_profile_audio_codec_blocklist(profile_id, codec, created_at)
         VALUES
            ('p1', 'DD+', '2026-01-01T00:00:00Z'),
            ('p1', 'DDP', '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("audio blocklist fixture rows should be inserted");

    run_embedded_migration(
        &pool,
        include_str!(
            "../../scryer/src/db/migrations/0125_release_metadata_enum_canonicalization.sql"
        ),
    )
    .await;

    let known_media: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT source_type, video_codec, video_codec_parsed, audio_codec_parsed
               FROM media_files
              WHERE id = 'known'",
    )
    .fetch_one(&pool)
    .await
    .expect("known media row should remain");
    assert_eq!(
        known_media,
        (
            Some("WEB-DL".to_string()),
            Some("H.264".to_string()),
            Some("H.265".to_string()),
            Some("DTSMA".to_string())
        )
    );

    let unknown_media: (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        "SELECT source_type, video_codec, video_codec_parsed, audio_codec_parsed
               FROM media_files
              WHERE id = 'unknown'",
    )
    .fetch_one(&pool)
    .await
    .expect("unknown media row should remain");
    assert_eq!(
        unknown_media,
        (
            Some("mystery-source".to_string()),
            Some("mystery-video".to_string()),
            Some("mystery-parsed".to_string()),
            Some("mystery-audio".to_string())
        )
    );

    let source_allowlist: Vec<String> =
        sqlx::query_scalar("SELECT source FROM quality_profile_source_allowlist ORDER BY source")
            .fetch_all(&pool)
            .await
            .expect("source allowlist should query");
    assert_eq!(source_allowlist, vec!["WEB-DL".to_string()]);

    let source_blocklist: Vec<String> =
        sqlx::query_scalar("SELECT source FROM quality_profile_source_blocklist ORDER BY source")
            .fetch_all(&pool)
            .await
            .expect("source blocklist should query");
    assert_eq!(source_blocklist, vec!["BRDISK".to_string()]);

    let audio_allowlist: Vec<String> = sqlx::query_scalar(
        "SELECT codec FROM quality_profile_audio_codec_allowlist ORDER BY codec",
    )
    .fetch_all(&pool)
    .await
    .expect("audio allowlist should query");
    assert_eq!(audio_allowlist, vec!["DTSMA".to_string()]);

    let audio_blocklist: Vec<String> = sqlx::query_scalar(
        "SELECT codec FROM quality_profile_audio_codec_blocklist ORDER BY codec",
    )
    .fetch_all(&pool)
    .await
    .expect("audio blocklist should query");
    assert_eq!(audio_blocklist, vec!["DDP".to_string()]);
}

#[test]
fn embedded_migration_bundle_includes_external_import_monitor_snapshot_chunk_table() {
    let keys = crate::migrations::list_embedded_migration_keys();
    assert!(
        keys.iter()
            .any(|key| key == "0117_external_import_monitor_snapshot_chunks"),
        "embedded migration bundle is missing 0117_external_import_monitor_snapshot_chunks: {keys:?}"
    );
}

#[tokio::test]
async fn review_regression_download_client_identity_migration_deduplicates_legacy_submissions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            source_title TEXT,
            submitted_at TEXT NOT NULL,
            collection_id TEXT,
            tracked_state TEXT,
            tracked_state_at TEXT,
            source_hint TEXT,
            source_kind TEXT,
            request_signature TEXT,
            episode_id TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy download_submissions should be created");
    sqlx::query(
        "CREATE TABLE download_queue_commands (
            id TEXT PRIMARY KEY,
            action TEXT NOT NULL,
            client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            is_history INTEGER NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("legacy download_queue_commands should be created");

    for (id, submitted_at) in [
        ("old-submission", "2025-01-01T00:00:00Z"),
        ("new-submission", "2025-01-02T00:00:00Z"),
    ] {
        sqlx::query(
            "INSERT INTO download_submissions
             (id, title_id, facet, download_client_type, download_client_item_id, submitted_at)
             VALUES (?, 'title-1', 'series', 'sabnzbd', 'native-id-1', ?)",
        )
        .bind(id)
        .bind(submitted_at)
        .execute(&pool)
        .await
        .expect("legacy submission should insert");
    }

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0087_download_queue_client_identity.sql"),
    )
    .await;

    let kept_id: String = sqlx::query_scalar("SELECT id FROM download_submissions")
        .fetch_one(&pool)
        .await
        .expect("migrated submission should exist");
    assert_eq!(kept_id, "new-submission");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions")
        .fetch_one(&pool)
        .await
        .expect("migrated submission count should load");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn review_regression_release_name_blocklist_watershed_resets_legacy_failed_state() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE blocklist (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            source_title TEXT,
            source_hint TEXT,
            quality TEXT,
            download_id TEXT,
            reason TEXT,
            data_json TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("blocklist table should be created");
    sqlx::query(
        "CREATE TABLE release_download_attempts (
            id TEXT PRIMARY KEY,
            title_id TEXT,
            source_hint TEXT,
            source_title TEXT,
            outcome TEXT NOT NULL,
            error_message TEXT,
            source_password TEXT,
            attempted_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("release attempts table should be created");
    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            facet TEXT NOT NULL,
            download_client_id TEXT NOT NULL DEFAULT '',
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            source_title TEXT,
            submitted_at TEXT NOT NULL,
            collection_id TEXT,
            tracked_state TEXT,
            tracked_state_at TEXT,
            source_hint TEXT,
            source_kind TEXT,
            request_signature TEXT,
            episode_id TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("download submissions table should be created");

    sqlx::query(
        "INSERT INTO blocklist
         (id, title_id, source_title, created_at)
         VALUES ('block-1', 'title-1', 'friends.s05.720p.bluray.dd5.1.x264-ntb', '2025-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("blocklist row should insert");
    sqlx::query(
        "INSERT INTO release_download_attempts
         (id, title_id, source_hint, source_title, outcome, attempted_at)
         VALUES
         ('failed-1', 'title-1', 'weaver-1', 'friends.s05.720p.bluray.dd5.1.x264-ntb', 'failed', '2025-01-01T00:00:00Z'),
         ('success-1', 'title-1', 'weaver-1', 'friends.s05.720p.bluray.dd5.1.x264-ntb', 'success', '2025-01-01T01:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("release attempts should insert");
    sqlx::query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_title, submitted_at, tracked_state, tracked_state_at, source_hint, request_signature)
         VALUES
         ('stub-failed', '', '', 'primary', 'weaver', 'job-1', NULL, '2025-01-01T00:00:00Z', 'failed', '2025-01-01T00:05:00Z', NULL, NULL),
         ('rich-failed', 'title-1', 'series', 'primary', 'weaver', 'job-2', 'Friends.S05.720p.BluRay.DD5.1.x264-NTb', '2025-01-01T00:00:00Z', 'failed', '2025-01-01T00:05:00Z', 'weaver://job-2', 'sig-2')",
    )
    .execute(&pool)
    .await
    .expect("download submissions should insert");

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0102_release_name_blocklist_watershed.sql"),
    )
    .await;

    let blocklist_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM blocklist")
        .fetch_one(&pool)
        .await
        .expect("blocklist count should load");
    assert_eq!(blocklist_count, 0);

    let failed_attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM release_download_attempts WHERE outcome = 'failed'",
    )
    .fetch_one(&pool)
    .await
    .expect("failed attempt count should load");
    assert_eq!(failed_attempt_count, 0);

    let successful_attempt_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM release_download_attempts WHERE outcome = 'success'",
    )
    .fetch_one(&pool)
    .await
    .expect("successful attempt count should load");
    assert_eq!(successful_attempt_count, 1);

    let blank_stub_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions WHERE id = 'stub-failed'")
            .fetch_one(&pool)
            .await
            .expect("blank stub count should load");
    assert_eq!(blank_stub_count, 0);

    let rich_failed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_submissions WHERE id = 'rich-failed'")
            .fetch_one(&pool)
            .await
            .expect("rich failed submission count should load");
    assert_eq!(rich_failed_count, 1);
}

#[tokio::test]
async fn review_regression_download_submission_episode_links_cascade_with_parent_records() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys should enable");
    sqlx::query(
        "CREATE TABLE download_submissions (
            id TEXT PRIMARY KEY,
            download_client_id TEXT NOT NULL DEFAULT '',
            download_client_type TEXT NOT NULL,
            download_client_item_id TEXT NOT NULL,
            UNIQUE(download_client_id, download_client_type, download_client_item_id)
        )",
    )
    .execute(&pool)
    .await
    .expect("download_submissions should be created");
    sqlx::query("CREATE TABLE episodes (id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("episodes should be created");

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0089_download_submission_episode_links.sql"),
    )
    .await;

    sqlx::query(
        "INSERT INTO download_submissions
         (id, download_client_id, download_client_type, download_client_item_id)
         VALUES ('submission-1', 'client-1', 'sabnzbd', 'native-id-1')",
    )
    .execute(&pool)
    .await
    .expect("submission should insert");
    sqlx::query("INSERT INTO episodes (id) VALUES ('episode-1')")
        .execute(&pool)
        .await
        .expect("episode should insert");
    sqlx::query(
        "INSERT INTO download_submission_episode_links
         (download_client_id, download_client_type, download_client_item_id, episode_id)
         VALUES ('client-1', 'sabnzbd', 'native-id-1', 'episode-1')",
    )
    .execute(&pool)
    .await
    .expect("episode link should insert");

    sqlx::query("DELETE FROM download_submissions WHERE id = 'submission-1'")
        .execute(&pool)
        .await
        .expect("submission should delete");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submission_episode_links")
        .fetch_one(&pool)
        .await
        .expect("link count should load");
    assert_eq!(count, 0);

    sqlx::query(
        "INSERT INTO download_submissions
         (id, download_client_id, download_client_type, download_client_item_id)
         VALUES ('submission-2', 'client-1', 'sabnzbd', 'native-id-1')",
    )
    .execute(&pool)
    .await
    .expect("submission should reinsert");
    sqlx::query(
        "INSERT INTO download_submission_episode_links
         (download_client_id, download_client_type, download_client_item_id, episode_id)
         VALUES ('client-1', 'sabnzbd', 'native-id-1', 'episode-1')",
    )
    .execute(&pool)
    .await
    .expect("episode link should reinsert");
    sqlx::query("DELETE FROM episodes WHERE id = 'episode-1'")
        .execute(&pool)
        .await
        .expect("episode should delete");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_submission_episode_links")
        .fetch_one(&pool)
        .await
        .expect("link count should load");
    // Episode deletion does not cascade — the link table is a submission-time
    // audit record and outlives episode catalog churn. Cascade applies only
    // to the download_submissions parent.
    assert_eq!(
        count, 1,
        "link survives episode deletion: episode_id has no FK cascade"
    );
}

#[tokio::test]
async fn review_regression_subtitle_provider_update_sets_and_clears_disabled_until() {
    let (services, db) = single_connection_services("scryer_subtitle_disabled_until").await;
    let store =
        SubtitleProviderConfigStore::new(services.datastore(), services.encryption_key_state());
    let now = Utc::now();
    let config = SubtitleProviderConfig {
        id: "subtitle-provider-1".to_string(),
        name: "Subtitles".to_string(),
        provider_type: "mock".to_string(),
        config_json: "{}".to_string(),
        enabled_facets: vec!["movie".to_string()],
        is_enabled: true,
        last_health_status: None,
        last_error: None,
        last_error_at: None,
        disabled_until: None,
        created_at: now,
        updated_at: now,
    };
    SubtitleProviderConfigRepository::create(&store, config)
        .await
        .expect("subtitle provider should be created");

    let disabled_until = chrono::DateTime::parse_from_rfc3339("2030-01-02T03:04:05Z")
        .expect("fixed timestamp should parse")
        .with_timezone(&Utc);
    let updated = SubtitleProviderConfigRepository::update(
        &store,
        SubtitleProviderConfigUpdate {
            id: "subtitle-provider-1".to_string(),
            disabled_until: Some(Some(disabled_until)),
            ..Default::default()
        },
    )
    .await
    .expect("subtitle provider disabled_until should update");
    assert_eq!(updated.disabled_until, Some(disabled_until));

    let updated = SubtitleProviderConfigRepository::update(
        &store,
        SubtitleProviderConfigUpdate {
            id: "subtitle-provider-1".to_string(),
            disabled_until: Some(None),
            ..Default::default()
        },
    )
    .await
    .expect("subtitle provider disabled_until should clear");
    assert_eq!(updated.disabled_until, None);

    let _ = std::fs::remove_file(db);
}

async fn single_connection_services(name: &str) -> (SqliteServices, std::path::PathBuf) {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register before migrations");

    let db = std::env::temp_dir().join(format!(
        "{}_{}.db",
        name,
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("single-connection pool should open");

    crate::migrations::run_migrations(&pool, crate::types::MigrationMode::Apply)
        .await
        .expect("migrations should apply");

    let services = SqliteServices {
        pool,
        encryption_key: Arc::new(RwLock::new(None)),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    };

    (services, db)
}

async fn create_pre_0079_title_projection_schema(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TABLE titles (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            facet TEXT NOT NULL,
            external_ids TEXT NOT NULL DEFAULT '[]',
            metadata_fetched_at TEXT
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy titles");

    sqlx::query(
        "CREATE TABLE title_external_ids (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            source TEXT NOT NULL,
            external_id TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids");

    sqlx::query(
        "CREATE UNIQUE INDEX idx_title_external_ids_lookup
         ON title_external_ids(source, external_id)",
    )
    .execute(pool)
    .await
    .expect("create legacy title_external_ids lookup");
}

async fn create_pre_0084_media_file_schema(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TABLE media_files (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            file_path TEXT NOT NULL UNIQUE,
            size_bytes INTEGER NOT NULL,
            quality_id TEXT,
            hash_sha256 TEXT,
            audio_languages_json TEXT,
            subtitle_languages_json TEXT,
            has_multiaudio INTEGER DEFAULT 0,
            scan_status TEXT NOT NULL DEFAULT 'pending',
            scan_error TEXT,
            created_at TEXT NOT NULL,
            video_codec TEXT,
            video_width INTEGER,
            video_height INTEGER,
            video_bitrate_kbps INTEGER,
            video_bit_depth INTEGER,
            video_hdr_format TEXT,
            audio_codec TEXT,
            audio_channels INTEGER,
            duration_seconds INTEGER,
            container_format TEXT,
            ffprobe_json TEXT,
            video_frame_rate TEXT,
            video_profile TEXT,
            audio_bitrate_kbps INTEGER,
            subtitle_codecs_json TEXT,
            audio_streams_json TEXT,
            scene_name TEXT,
            release_group TEXT,
            source_type TEXT,
            resolution TEXT,
            video_codec_parsed TEXT,
            audio_codec_parsed TEXT,
            audio_channels_parsed TEXT,
            acquisition_score INTEGER,
            scoring_log TEXT,
            indexer_source TEXT,
            grabbed_release_title TEXT,
            grabbed_at TEXT,
            edition TEXT,
            original_file_path TEXT,
            release_hash TEXT,
            num_chapters INTEGER,
            subtitle_streams_json TEXT,
            source_signature_scheme TEXT,
            source_signature_value TEXT,
            audio_profile TEXT
        )",
    )
    .execute(pool)
    .await
    .expect("create legacy media_files");
}

#[tokio::test]
async fn nzbget_client_is_sendable() {
    let client = NzbgetDownloadClient::new(
        "http://127.0.0.1:6789".to_string(),
        Some("user".into()),
        Some("pass".into()),
        "SCORE".to_string(),
    );
    // We only validate that it can be built and is callable in type system.
    let _ = client.endpoint();
}

#[tokio::test]
async fn title_queries_prefer_local_cached_poster_url() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-1", Some("https://tvdb.example/poster.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let before_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        before_cache.poster_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );

    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster.jpg".to_string(),
                source_etag: Some("\"etag-1\"".to_string()),
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![7, 8, 9],
                    sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let after_cache = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        after_cache.poster_url.as_deref(),
        Some("/images/titles/title-1/poster/w500?v=bbbbbbbbbbbbbbbb")
    );
    assert_eq!(
        after_cache.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );

    let listed = TitleRepository::list(&catalog, None, None)
        .await
        .expect("title list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].poster_url.as_deref(),
        Some("/images/titles/title-1/poster/w500?v=bbbbbbbbbbbbbbbb")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_with_extra_external_ids_completes_on_single_connection_sqlite() {
    let (services, db) =
        single_connection_services("scryer_title_hydration_single_connection").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-hydration-extra-ids", None);
    title.facet = MediaFacet::Anime;
    title.external_ids = vec![
        ExternalId {
            source: "tvdb".to_string(),
            value: "12345".to_string(),
        },
        ExternalId {
            source: "mal".to_string(),
            value: "old-mal".to_string(),
        },
    ];
    title.tags = vec!["score:old".to_string(), "keep".to_string()];

    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let update = TitleMetadataUpdate {
        metadata_language: Some("eng".to_string()),
        metadata_fetched_at: Some(Utc::now().to_rfc3339()),
        extra_external_ids: vec![
            ExternalId {
                source: "mal".to_string(),
                value: "834".to_string(),
            },
            ExternalId {
                source: "anilist".to_string(),
                value: "269".to_string(),
            },
        ],
        extra_tags: vec!["score:9.1".to_string()],
        ..TitleMetadataUpdate::default()
    };

    let updated = timeout(
        Duration::from_secs(1),
        TitleRepository::update_title_hydrated_metadata(&catalog, &title.id, update),
    )
    .await
    .expect("hydrated metadata update should not self-deadlock on single-connection sqlite")
    .expect("hydrated metadata update should succeed");

    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "mal" && external_id.value == "834" })
    );
    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "anilist" && external_id.value == "269" })
    );
    assert_eq!(
        updated
            .external_ids
            .iter()
            .filter(|external_id| external_id.source == "mal")
            .map(|external_id| external_id.value.as_str())
            .collect::<Vec<_>>(),
        vec!["834"]
    );
    assert!(!updated.tags.iter().any(|tag| tag == "score:old"));
    assert!(updated.tags.iter().any(|tag| tag == "score:9.1"));
    assert!(updated.tags.iter().any(|tag| tag == "keep"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn hydrated_title_metadata_preserves_retry_until_fetch_marker_sqlite() {
    let (services, db) = temp_services("scryer_title_hydration_retry_preserve").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-hydration-retry-preserve", None);
    title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = ?
         WHERE id = ?",
    )
    .bind("2026-01-01T00:00:00Z")
    .bind(7_i64)
    .bind(&title.id)
    .execute(services.pool())
    .await
    .expect("retry state should update");

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_language: Some("eng".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("partial metadata update should succeed");

    let retry_state: (Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
         FROM titles
         WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(services.pool())
    .await
    .expect("retry state should load");
    assert_eq!(retry_state.0.as_deref(), Some("2026-01-01T00:00:00Z"));
    assert_eq!(retry_state.1, 7);

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some("2026-02-01T00:00:00Z".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("fetched metadata update should succeed");

    let cleared_retry_state: (Option<String>, i64) = sqlx::query_as(
        "SELECT metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
         FROM titles
         WHERE id = ?",
    )
    .bind(&title.id)
    .fetch_one(services.pool())
    .await
    .expect("cleared retry state should load");
    assert_eq!(cleared_retry_state.0, None);
    assert_eq!(cleared_retry_state.1, 0);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn replace_title_match_state_completes_on_single_connection_sqlite() {
    let (services, db) =
        single_connection_services("scryer_replace_match_state_single_connection").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-replace-match-state", None);
    title.facet = MediaFacet::Anime;
    title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    title.year = Some(2024);
    title.overview = Some("overview before clear".to_string());

    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let updated = timeout(
        Duration::from_secs(1),
        TitleRepository::replace_match_state(
            &catalog,
            &title.id,
            vec![ExternalId {
                source: "tvdb".to_string(),
                value: "99999".to_string(),
            }],
            vec!["score:9.1".to_string()],
        ),
    )
    .await
    .expect("replace match state should not self-deadlock on single-connection sqlite")
    .expect("replace match state should succeed");

    assert_eq!(updated.year, None);
    assert_eq!(updated.overview, None);
    assert!(
        updated
            .external_ids
            .iter()
            .any(|external_id| { external_id.source == "tvdb" && external_id.value == "99999" })
    );
    assert!(updated.tags.iter().any(|tag| tag == "score:9.1"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_update_metadata_keeps_validation_and_not_found_errors() {
    let (services, db) = temp_services("scryer_title_update_metadata_errors").await;
    let catalog = title_store(&services);

    let title = make_test_title("title-update-metadata-errors", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let empty_err = TitleRepository::update_metadata(&catalog, &title.id, None, None, None)
        .await
        .expect_err("empty update should fail validation");
    assert!(matches!(
        empty_err,
        scryer_application::AppError::Validation(message)
            if message.contains("at least one title field")
    ));

    let blank_name_err =
        TitleRepository::update_metadata(&catalog, &title.id, Some("   ".to_string()), None, None)
            .await
            .expect_err("blank title name should fail validation");
    assert!(matches!(
        blank_name_err,
        scryer_application::AppError::Validation(message)
            if message.contains("title name cannot be empty")
    ));

    let missing_err = TitleRepository::update_metadata(
        &catalog,
        "missing-title",
        Some("Renamed".to_string()),
        None,
        None,
    )
    .await
    .expect_err("missing title update should fail not found");
    assert!(matches!(
        missing_err,
        scryer_application::AppError::NotFound(message) if message.contains("missing-title")
    ));

    let renamed = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        Some(" Renamed Title ".to_string()),
        None,
        None,
    )
    .await
    .expect("valid metadata update should succeed");
    assert_eq!(renamed.name, "Renamed Title");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_change_local_version_when_cached_poster_changes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_version_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-2", Some("https://tvdb.example/poster-a.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    for (source_url, sha) in [
        (
            "https://tvdb.example/poster-a.jpg",
            "11111111111111111111111111111111",
        ),
        (
            "https://tvdb.example/poster-b.jpg",
            "22222222222222222222222222222222",
        ),
    ] {
        title_images
            .replace_title_image(
                &title.id,
                TitleImageReplacement {
                    kind: TitleImageKind::Poster,
                    source_url: source_url.to_string(),
                    source_etag: None,
                    source_last_modified: None,
                    source_format: "jpeg".to_string(),
                    source_width: 1000,
                    source_height: 1500,
                    storage_mode: TitleImageStorageMode::AvifMaster,
                    master_format: "avif".to_string(),
                    master_sha256: sha.to_string(),
                    master_width: 1000,
                    master_height: 1500,
                    master_bytes: vec![1, 2, 3],
                    variants: vec![TitleImageVariantRecord {
                        variant_key: "w500".to_string(),
                        format: "avif".to_string(),
                        width: 500,
                        height: 750,
                        bytes: vec![7, 8, 9],
                        sha256: sha.to_string(),
                    }],
                },
            )
            .await
            .expect("title image should upsert");

        sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
            .bind(source_url)
            .bind(&title.id)
            .execute(&services.pool)
            .await
            .expect("source url should update");
    }

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-2/poster/w500?v=2222222222222222")
    );
    assert_eq!(
        updated.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-b.jpg")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn create_title_only_marks_tvdb_titles_for_background_hydration() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_seed_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut tvdb_title = make_test_title("title-tvdb", None);
    tvdb_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123".to_string(),
    }];
    TitleRepository::create(&catalog, tvdb_title)
        .await
        .expect("tvdb title should insert");

    let mut imdb_title = make_test_title("title-imdb", None);
    imdb_title.external_ids = vec![ExternalId {
        source: "imdb".to_string(),
        value: "tt1234567".to_string(),
    }];
    TitleRepository::create(&catalog, imdb_title)
        .await
        .expect("imdb title should insert");

    let markers: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind("title-imdb")
    .bind("title-tvdb")
    .fetch_all(&services.pool)
    .await
    .expect("load hydration markers");

    assert_eq!(markers[0], ("title-imdb".to_string(), None));
    assert!(
        markers[1].1.is_some(),
        "tvdb-backed titles should be queued for background hydration"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_titles_due_for_hydration_excludes_active_facets_in_due_order() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_hydration_excluded_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut anime_title = make_test_title("anime-due", None);
    anime_title.facet = MediaFacet::Anime;
    anime_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "301".to_string(),
    }];
    TitleRepository::create(&catalog, anime_title)
        .await
        .expect("anime title should insert");

    let mut movie_title = make_test_title("movie-due", None);
    movie_title.facet = MediaFacet::Movie;
    movie_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "101".to_string(),
    }];
    TitleRepository::create(&catalog, movie_title)
        .await
        .expect("movie title should insert");

    let mut series_title = make_test_title("series-due", None);
    series_title.facet = MediaFacet::Series;
    series_title.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "201".to_string(),
    }];
    TitleRepository::create(&catalog, series_title)
        .await
        .expect("series title should insert");

    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = 0
         WHERE id IN (?, ?, ?)",
    )
    .bind("2026-01-01T00:00:00Z")
    .bind("anime-due")
    .bind("movie-due")
    .bind("series-due")
    .execute(&services.pool)
    .await
    .expect("normalize due timestamps");

    let due_titles =
        TitleRepository::list_titles_due_for_hydration(&catalog, 10, &[MediaFacet::Series])
            .await
            .expect("load due titles excluding active series facet");

    let due_ids = due_titles
        .into_iter()
        .map(|pending| pending.title.id)
        .collect::<Vec<_>>();
    assert_eq!(
        due_ids,
        vec!["anime-due".to_string(), "movie-due".to_string()]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_find_by_external_id() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let mut title = make_test_title(
        "title-external-id",
        Some("https://tvdb.example/poster-external.jpg"),
    );
    title.external_ids = vec![ExternalId {
        source: "TVDB".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-external.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 1, 1],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![2, 2, 2],
                    sha256: "ffffffffffffffffffffffffffffffff".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let found = catalog
        .find_by_external_id("tvdb", "123456")
        .await
        .expect("lookup should succeed")
        .expect("title should exist");

    assert_eq!(found.id, title.id);
    assert_eq!(
        found.poster_url.as_deref(),
        Some("/images/titles/title-external-id/poster/w500?v=ffffffffffffffff")
    );
    assert_eq!(
        found.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-external.jpg")
    );

    let uppercase_source = catalog
        .find_by_external_id("TVDB", "123456")
        .await
        .expect("uppercase source lookup should succeed")
        .expect("title should exist for uppercase source");
    assert_eq!(uppercase_source.id, title.id);

    let padded_source = catalog
        .find_by_external_id(" tvdb ", "123456")
        .await
        .expect("padded source lookup should succeed");
    assert!(padded_source.is_none());

    let padded_value = catalog
        .find_by_external_id("tvdb", " 123456 ")
        .await
        .expect("padded value lookup should succeed");
    assert!(padded_value.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_list_by_external_ids_preserve_request_order_for_unique_first_matches() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_external_id_batch_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let mut first = make_test_title("title-a", Some("https://tvdb.example/a.jpg"));
    first.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "123456".to_string(),
    }];
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let mut second = make_test_title("title-b", Some("https://tvdb.example/b.jpg"));
    second.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "345678".to_string(),
    }];
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let values = vec![
        "345678".to_string(),
        "123456".to_string(),
        "123456".to_string(),
        "000000".to_string(),
    ];
    let matches = catalog
        .list_by_external_ids("tvdb", &values)
        .await
        .expect("batch lookup should succeed");

    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].id, second.id);
    assert_eq!(matches[1].id, first.id);

    let padded_values = vec![" 345678 ".to_string()];
    let padded_value_matches = catalog
        .list_by_external_ids("tvdb", &padded_values)
        .await
        .expect("padded value batch lookup should succeed");
    assert!(padded_value_matches.is_empty());

    let exact_values = vec!["345678".to_string()];
    let padded_source_matches = catalog
        .list_by_external_ids(" tvdb ", &exact_values)
        .await
        .expect("padded source batch lookup should succeed");
    assert!(padded_source_matches.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_with_empty_library_allowlist_return_no_results() {
    let (services, db) = temp_services("scryer_title_empty_library_allowlist").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-empty-library-allowlist", None);
    title.name = "Alpha Allowlist".to_string();
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    let empty_library_ids = Vec::<String>::new();

    let listed = TitleRepository::list_for_libraries(&catalog, None, &empty_library_ids, None)
        .await
        .expect("plain library listing should succeed");
    assert!(listed.is_empty());

    let searched = TitleRepository::list_for_libraries_without_external_ids(
        &catalog,
        None,
        &empty_library_ids,
        Some("alpha".to_string()),
    )
    .await
    .expect("ranked library listing should succeed");
    assert!(searched.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_get_by_facet_and_slug_trim_input_and_reject_duplicates() {
    let (services, db) = temp_services("scryer_title_slug_lookup").await;
    let catalog = title_store(&services);

    let mut first = make_test_title("title-slug-primary", None);
    first.facet = MediaFacet::Movie;
    first.library_id = "library-a".to_string();
    first.slug = Some("earth-defenders".to_string());
    TitleRepository::create(&catalog, first.clone())
        .await
        .expect("first title should insert");

    let found =
        TitleRepository::get_by_facet_and_slug(&catalog, MediaFacet::Movie, " earth-defenders ")
            .await
            .expect("trimmed slug lookup should succeed")
            .expect("trimmed slug lookup should find a title");
    assert_eq!(found.id, first.id);

    let mut duplicate = make_test_title("title-slug-duplicate", None);
    duplicate.facet = MediaFacet::Movie;
    duplicate.library_id = "library-b".to_string();
    duplicate.slug = Some("earth-defenders".to_string());
    TitleRepository::create(&catalog, duplicate)
        .await
        .expect("duplicate title should insert in a different library");

    let err =
        TitleRepository::get_by_facet_and_slug(&catalog, MediaFacet::Movie, "earth-defenders")
            .await
            .expect_err("duplicate slug lookup should fail validation");
    assert!(matches!(
        err,
        scryer_application::AppError::Validation(message)
            if message.contains("earth-defenders") && message.contains("multiple titles")
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_get_by_facet_libraries_and_slug_trim_input_and_reject_duplicates() {
    let (services, db) = temp_services("scryer_title_library_slug_lookup").await;
    let catalog = title_store(&services);

    let mut first = make_test_title("title-library-slug-a", None);
    first.facet = MediaFacet::Movie;
    first.library_id = "library-a".to_string();
    first.slug = Some("planet-heroes".to_string());
    TitleRepository::create(&catalog, first)
        .await
        .expect("first title should insert");

    let mut second = make_test_title("title-library-slug-b", None);
    second.facet = MediaFacet::Movie;
    second.library_id = "library-b".to_string();
    second.slug = Some("planet-heroes".to_string());
    TitleRepository::create(&catalog, second.clone())
        .await
        .expect("second title should insert");

    let library_b = vec!["library-b".to_string()];
    let found = TitleRepository::get_by_facet_libraries_and_slug(
        &catalog,
        MediaFacet::Movie,
        &library_b,
        " planet-heroes ",
    )
    .await
    .expect("trimmed library slug lookup should succeed")
    .expect("trimmed library slug lookup should find a title");
    assert_eq!(found.id, second.id);

    let libraries = vec!["library-a".to_string(), "library-b".to_string()];
    let err = TitleRepository::get_by_facet_libraries_and_slug(
        &catalog,
        MediaFacet::Movie,
        &libraries,
        "planet-heroes",
    )
    .await
    .expect_err("duplicate library slug lookup should fail validation");
    assert!(matches!(
        err,
        scryer_application::AppError::Validation(message)
            if message.contains("planet-heroes") && message.contains("multiple titles")
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_query_modes_keep_spellfix_search_scoped_to_presentation_sqlite() {
    let (services, db) = temp_services("scryer_title_query_mode_search_scope").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-query-mode-search-scope", None);
    title.name = "Canonical Search Name".to_string();
    title.aliases = vec!["Hidden Search Alias".to_string()];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let presentation_hits =
        TitleRepository::list(&catalog, None, Some("hidden search alias".to_string()))
            .await
            .expect("presentation search should load");
    assert_eq!(
        presentation_hits.first().map(|title| title.id.as_str()),
        Some(title.id.as_str())
    );

    let matching_alias_hits =
        TitleRepository::list_for_matching(&catalog, None, Some("hidden search alias".to_string()))
            .await
            .expect("matching search should load");
    assert!(
        !matching_alias_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let matching_name_hits =
        TitleRepository::list_for_matching(&catalog, None, Some("canonical search".to_string()))
            .await
            .expect("matching name search should load");
    assert!(
        matching_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let padded_matching_name_hits =
        TitleRepository::list_for_matching(&catalog, None, Some(" canonical search ".to_string()))
            .await
            .expect("padded matching name search should load");
    assert!(
        !padded_matching_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let library_ids = vec![title.library_id.clone()];
    let library_alias_hits = TitleRepository::list_for_libraries(
        &catalog,
        None,
        &library_ids,
        Some("hidden search alias".to_string()),
    )
    .await
    .expect("library search should load");
    assert!(
        !library_alias_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let library_padded_name_hits = TitleRepository::list_for_libraries(
        &catalog,
        None,
        &library_ids,
        Some(" canonical search ".to_string()),
    )
    .await
    .expect("padded library search should load");
    assert!(
        !library_padded_name_hits
            .iter()
            .any(|candidate| candidate.id == title.id)
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_create_or_get_existing_reuses_external_ids_not_slug_only() {
    let (services, db) = temp_services("scryer_title_create_or_get_existing_parity").await;
    let catalog = title_store(&services);

    let mut existing = make_test_title("title-existing-external-id", None);
    existing.slug = Some("shared-slug".to_string());
    existing.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "12345".to_string(),
    }];
    TitleRepository::create(&catalog, existing.clone())
        .await
        .expect("existing title should insert");

    let mut same_slug = make_test_title("title-same-slug-new-external-id", None);
    same_slug.slug = Some("shared-slug".to_string());
    same_slug.external_ids = vec![ExternalId {
        source: "tvdb".to_string(),
        value: "67890".to_string(),
    }];
    let same_slug_outcome = TitleRepository::create_or_get_existing(&catalog, same_slug.clone())
        .await
        .expect("same-slug title should create");
    assert!(!same_slug_outcome.reused_existing);
    assert_eq!(same_slug_outcome.title.id, same_slug.id);

    let mut same_external_id = make_test_title("title-same-external-id", None);
    same_external_id.slug = Some("different-slug".to_string());
    same_external_id.external_ids = existing.external_ids.clone();
    let same_external_id_outcome =
        TitleRepository::create_or_get_existing(&catalog, same_external_id)
            .await
            .expect("same external id title should reuse");
    assert!(same_external_id_outcome.reused_existing);
    assert_eq!(same_external_id_outcome.title.id, existing.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_list_for_matching_keeps_source_image_urls() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_list_for_matching_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-list-matching",
        Some("https://tvdb.example/poster.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "abababababababababababababababab".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![4, 5, 6],
                    sha256: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let titles = TitleRepository::list_for_matching(&catalog, None, None)
        .await
        .expect("matching list should succeed");
    let listed = titles
        .into_iter()
        .find(|candidate| candidate.id == title.id)
        .expect("title should be listed");

    assert_eq!(
        listed.poster_url.as_deref(),
        Some("https://tvdb.example/poster.jpg")
    );
    assert!(listed.poster_source_url.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn media_file_source_signature_refresh_preserves_scan_status() {
    let db = std::env::temp_dir().join(format!(
        "scryer_media_file_signature_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let media_files = media_file_store(&services);

    let title = make_test_title("title-media-file", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Movie.Title.2024.mkv".to_string(),
            size_bytes: 4_096,
            ..Default::default()
        })
        .await
        .expect("media file should insert");

    sqlx::query("UPDATE media_files SET scan_status = 'scanned' WHERE id = ?")
        .bind(&file_id)
        .execute(&services.pool)
        .await
        .expect("scan status should update");

    media_files
        .update_media_file_source_signature(
            &file_id,
            4_096,
            Some("unix_mtime_nsec_v1".to_string()),
            Some("1:2".to_string()),
        )
        .await
        .expect("source signature should refresh");

    let media_file = media_files
        .get_media_file_by_id(&file_id)
        .await
        .expect("lookup should succeed")
        .expect("media file should exist");

    assert_eq!(media_file.scan_status, "scanned");
    assert_eq!(
        media_file.source_signature_scheme.as_deref(),
        Some("unix_mtime_nsec_v1")
    );
    assert_eq!(media_file.source_signature_value.as_deref(), Some("1:2"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_use_local_original_url_for_original_storage_mode() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_original_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-3", Some("https://tvdb.example/poster-original.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-original.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 400,
                source_height: 600,
                storage_mode: TitleImageStorageMode::Original,
                master_format: "jpeg".to_string(),
                master_sha256: "cccccccccccccccccccccccccccccccc".to_string(),
                master_width: 400,
                master_height: 600,
                master_bytes: vec![3, 2, 1],
                variants: Vec::new(),
            },
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-3/poster/original?v=cccccccccccccccc")
    );

    let original = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
        .await
        .expect("original blob lookup should succeed");
    assert_eq!(
        original,
        Some(TitleImageBlob {
            content_type: "image/jpeg".to_string(),
            etag: "cccccccccccccccccccccccccccccccc".to_string(),
            bytes: vec![3, 2, 1],
        })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn replace_title_image_and_append_event_commits_image_and_event_atomically() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_event_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);
    let domain_events = DomainEventStore::new(services.datastore());

    let title = make_test_title("title-image-event", Some("https://tvdb.example/poster.jpg"));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let event = NewDomainEvent {
        event_id: Id::new().0,
        occurred_at: Utc::now(),
        actor_user_id: None,
        title_id: Some(title.id.clone()),
        facet: Some(title.facet.clone()),
        correlation_id: None,
        causation_id: None,
        schema_version: 1,
        stream: DomainEventStream::Title {
            title_id: title.id.clone(),
        },
        payload: DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
            title: TitleContextSnapshot {
                title_name: title.name.clone(),
                facet: title.facet.clone(),
                external_ids: Default::default(),
                poster_url: title.poster_url.clone(),
                year: title.year,
            },
        }),
    };

    let stored = title_images
        .replace_title_image_and_append_event(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 400,
                source_height: 600,
                storage_mode: TitleImageStorageMode::Original,
                master_format: "jpeg".to_string(),
                master_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
                master_width: 400,
                master_height: 600,
                master_bytes: vec![4, 5, 6],
                variants: Vec::new(),
            },
            event.clone(),
        )
        .await
        .expect("title image and event should commit");

    assert_eq!(stored.event_id, event.event_id);
    let blob = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Poster, "original")
        .await
        .expect("blob lookup should succeed")
        .expect("blob should exist");
    assert_eq!(blob.bytes, vec![4, 5, 6]);

    let events = domain_events
        .list(&DomainEventFilter {
            title_id: Some(title.id.clone()),
            limit: 10,
            ..Default::default()
        })
        .await
        .expect("domain event list should succeed");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, event.event_id);
    assert!(matches!(
        events[0].payload,
        DomainEventPayload::TitleUpdated(_)
    ));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_queries_fall_back_to_original_when_w500_variant_is_missing() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_poster_incomplete_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-4",
        Some("https://tvdb.example/poster-incomplete.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: "https://tvdb.example/poster-incomplete.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "dddddddddddddddddddddddddddddddd".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![9, 8, 7],
                variants: Vec::new(),
            },
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.poster_url.as_deref(),
        Some("/images/titles/title-4/poster/original?v=dddddddddddddddd")
    );

    let pending = title_images
        .list_titles_requiring_image_refresh(TitleImageKind::Poster, 10)
        .await
        .expect("list pending poster refresh should succeed");
    assert!(
        pending.iter().any(|task| task.title_id == title.id),
        "incomplete AVIF cache rows should be re-queued for repair"
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn banner_and_fanart_queries_use_master_alias_without_variant_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_banner_fanart_alias_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-banner-fanart", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query("UPDATE titles SET banner_url = ?, background_url = ? WHERE id = ?")
        .bind("https://tvdb.example/banner.jpg")
        .bind("https://tvdb.example/fanart.jpg")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("source urls should update");

    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Banner,
                source_url: "https://tvdb.example/banner.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1920,
                source_height: 1080,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "11111111111111111111111111111111".to_string(),
                master_width: 1920,
                master_height: 1080,
                master_bytes: vec![1, 2, 3, 4],
                variants: Vec::new(),
            },
        )
        .await
        .expect("banner image should insert");
    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Fanart,
                source_url: "https://tvdb.example/fanart.jpg".to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 2560,
                source_height: 1440,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "22222222222222222222222222222222".to_string(),
                master_width: 2560,
                master_height: 1440,
                master_bytes: vec![5, 6, 7, 8],
                variants: Vec::new(),
            },
        )
        .await
        .expect("fanart image should insert");

    let updated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        updated.banner_url.as_deref(),
        Some("/images/titles/title-banner-fanart/banner/master?v=1111111111111111")
    );
    assert_eq!(
        updated.banner_source_url.as_deref(),
        Some("https://tvdb.example/banner.jpg")
    );
    assert_eq!(
        updated.background_url.as_deref(),
        Some("/images/titles/title-banner-fanart/fanart/master?v=2222222222222222")
    );
    assert_eq!(
        updated.background_source_url.as_deref(),
        Some("https://tvdb.example/fanart.jpg")
    );

    let banner = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Banner, "master")
        .await
        .expect("banner blob lookup should succeed");
    assert_eq!(
        banner,
        Some(TitleImageBlob {
            content_type: "image/avif".to_string(),
            etag: "11111111111111111111111111111111".to_string(),
            bytes: vec![1, 2, 3, 4],
        })
    );

    let fanart = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Fanart, "master")
        .await
        .expect("fanart blob lookup should succeed");
    assert_eq!(
        fanart,
        Some(TitleImageBlob {
            content_type: "image/avif".to_string(),
            etag: "22222222222222222222222222222222".to_string(),
            bytes: vec![5, 6, 7, 8],
        })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_titles_requiring_image_refresh_does_not_require_banner_or_fanart_master_variant_rows()
{
    let db = std::env::temp_dir().join(format!(
        "scryer_title_banner_fanart_refresh_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-banner-fanart-refresh", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query("UPDATE titles SET banner_url = ?, background_url = ? WHERE id = ?")
        .bind("https://tvdb.example/banner-refresh.jpg")
        .bind("https://tvdb.example/fanart-refresh.jpg")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("source urls should update");

    for (kind, source_url, sha) in [
        (
            TitleImageKind::Banner,
            "https://tvdb.example/banner-refresh.jpg",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
        (
            TitleImageKind::Fanart,
            "https://tvdb.example/fanart-refresh.jpg",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        ),
    ] {
        title_images
            .replace_title_image(
                &title.id,
                TitleImageReplacement {
                    kind,
                    source_url: source_url.to_string(),
                    source_etag: None,
                    source_last_modified: None,
                    source_format: "jpeg".to_string(),
                    source_width: 1920,
                    source_height: 1080,
                    storage_mode: TitleImageStorageMode::AvifMaster,
                    master_format: "avif".to_string(),
                    master_sha256: sha.to_string(),
                    master_width: 1920,
                    master_height: 1080,
                    master_bytes: vec![1, 2, 3],
                    variants: Vec::new(),
                },
            )
            .await
            .expect("title image should insert");
    }

    let pending_banner = title_images
        .list_titles_requiring_image_refresh(TitleImageKind::Banner, 10)
        .await
        .expect("list pending banner refresh should succeed");
    assert!(pending_banner.is_empty());

    let pending_fanart = title_images
        .list_titles_requiring_image_refresh(TitleImageKind::Fanart, 10)
        .await
        .expect("list pending fanart refresh should succeed");
    assert!(pending_fanart.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_update_metadata_preserves_provider_image_url_after_local_image_projection() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_provider_preserve_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let source_url = "https://tvdb.example/provider-poster.jpg";
    let title = make_test_title("title-provider-preserve", Some(source_url));
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    title_images
        .replace_title_image(
            &title.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: source_url.to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "abababababababababababababababab".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![4, 5, 6],
                    sha256: "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");

    let updated = TitleRepository::update_metadata(
        &catalog,
        &title.id,
        None,
        None,
        Some(vec!["favorite".to_string()]),
    )
    .await
    .expect("title metadata should update");
    assert_eq!(updated.poster_source_url.as_deref(), Some(source_url));
    assert!(
        updated
            .poster_url
            .as_deref()
            .is_some_and(|url| url.starts_with("/images/titles/"))
    );

    let row = sqlx::query("SELECT poster_url, poster_local_path FROM titles WHERE id = ?")
        .bind(&title.id)
        .fetch_one(&services.pool)
        .await
        .expect("title row should load");
    let stored_source: Option<String> = row.get("poster_url");
    let stored_local_path: Option<String> = row.get("poster_local_path");
    assert_eq!(stored_source.as_deref(), Some(source_url));
    assert!(
        stored_local_path
            .as_deref()
            .is_some_and(|url| url.starts_with("/images/titles/"))
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_titles_requiring_image_refresh_ignores_local_title_image_routes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_local_route_refresh_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title(
        "title-local-route-refresh",
        Some("/images/titles/title-local-route-refresh/poster/w500?v=deadbeef"),
    );
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    let upstream = make_test_title(
        "title-http-route-segment-refresh",
        Some("https://cdn.example/images/titles/upstream-poster.jpg"),
    );
    TitleRepository::create(&catalog, upstream.clone())
        .await
        .expect("upstream title should insert");

    let pending = title_images
        .list_titles_requiring_image_refresh(TitleImageKind::Poster, 10)
        .await
        .expect("list pending poster refresh should succeed");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].title_id, upstream.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn clear_title_image_cache_repairs_polluted_urls_and_clears_db_cache() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_cache_clear_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let source_url = "https://tvdb.example/cache-clear-poster.jpg";
    let repaired = make_test_title("title-cache-clear-repair", Some(source_url));
    TitleRepository::create(&catalog, repaired.clone())
        .await
        .expect("repair title should insert");
    title_images
        .replace_title_image(
            &repaired.id,
            TitleImageReplacement {
                kind: TitleImageKind::Poster,
                source_url: source_url.to_string(),
                source_etag: None,
                source_last_modified: None,
                source_format: "jpeg".to_string(),
                source_width: 1000,
                source_height: 1500,
                storage_mode: TitleImageStorageMode::AvifMaster,
                master_format: "avif".to_string(),
                master_sha256: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string(),
                master_width: 1000,
                master_height: 1500,
                master_bytes: vec![1, 2, 3],
                variants: vec![TitleImageVariantRecord {
                    variant_key: "w500".to_string(),
                    format: "avif".to_string(),
                    width: 500,
                    height: 750,
                    bytes: vec![4, 5, 6],
                    sha256: "ffffffffffffffffffffffffffffffff".to_string(),
                }],
            },
        )
        .await
        .expect("title image should insert");
    sqlx::query("UPDATE titles SET poster_url = ? WHERE id = ?")
        .bind("/images/titles/title-cache-clear-repair/poster/w500?v=ffffffffffffffff")
        .bind(&repaired.id)
        .execute(&services.pool)
        .await
        .expect("polluted source should update");

    let unrecoverable = make_test_title(
        "title-cache-clear-unrecoverable",
        Some("/images/titles/title-cache-clear-unrecoverable/poster/w500?v=badbadbad"),
    );
    TitleRepository::create(&catalog, unrecoverable.clone())
        .await
        .expect("unrecoverable title should insert");

    title_images
        .clear_title_image_cache()
        .await
        .expect("title image cache should clear");

    let repaired_row = sqlx::query("SELECT poster_url, poster_local_path FROM titles WHERE id = ?")
        .bind(&repaired.id)
        .fetch_one(&services.pool)
        .await
        .expect("repaired row should load");
    let repaired_source: Option<String> = repaired_row.get("poster_url");
    let repaired_local_path: Option<String> = repaired_row.get("poster_local_path");
    assert_eq!(repaired_source.as_deref(), Some(source_url));
    assert!(repaired_local_path.is_none());

    let unrecoverable_row = sqlx::query(
        "SELECT poster_url, metadata_hydration_next_attempt_at FROM titles WHERE id = ?",
    )
    .bind(&unrecoverable.id)
    .fetch_one(&services.pool)
    .await
    .expect("unrecoverable row should load");
    let unrecoverable_source: Option<String> = unrecoverable_row.get("poster_url");
    let next_attempt: Option<String> = unrecoverable_row.get("metadata_hydration_next_attempt_at");
    assert!(unrecoverable_source.is_none());
    assert!(next_attempt.is_some());

    let image_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_images")
        .fetch_one(&services.pool)
        .await
        .expect("image count should load");
    let variant_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_image_variants")
        .fetch_one(&services.pool)
        .await
        .expect("variant count should load");
    assert_eq!(image_count, 0);
    assert_eq!(variant_count, 0);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_master_dedup_migration_rewrites_paths_and_deletes_banner_fanart_variants() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_master_dedup_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);
    let title_images = title_image_store(&services);

    let title = make_test_title("title-master-dedup", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query(
        "UPDATE titles SET
            banner_url = ?,
            background_url = ?,
            banner_local_path = ?,
            background_local_path = ?
         WHERE id = ?",
    )
    .bind("https://tvdb.example/banner-master.jpg")
    .bind("https://tvdb.example/fanart-master.jpg")
    .bind("/images/titles/title-master-dedup/banner/master?v=bbbbbbbbbbbbbbbb")
    .bind("/images/titles/title-master-dedup/fanart/master?v=dddddddddddddddd")
    .bind(&title.id)
    .execute(&services.pool)
    .await
    .expect("title source and legacy local paths should update");

    sqlx::query(
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("img-banner-master")
    .bind(&title.id)
    .bind("tvdb")
    .bind(Option::<String>::None)
    .bind("banner")
    .bind("https://tvdb.example/banner-master.jpg")
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind("jpeg")
    .bind(1920i32)
    .bind(1080i32)
    .bind("avif_master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    .bind(1920i32)
    .bind(1080i32)
    .bind(vec![1_u8, 2, 3, 4])
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("banner image row should insert");

    sqlx::query(
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("img-fanart-master")
    .bind(&title.id)
    .bind("tvdb")
    .bind(Option::<String>::None)
    .bind("fanart")
    .bind("https://tvdb.example/fanart-master.jpg")
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind("jpeg")
    .bind(2560i32)
    .bind(1440i32)
    .bind("avif_master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind("cccccccccccccccccccccccccccccccc")
    .bind(2560i32)
    .bind(1440i32)
    .bind(vec![5_u8, 6, 7, 8])
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("fanart image row should insert");

    sqlx::query(
        "INSERT INTO title_image_variants (
            id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("variant-banner-master")
    .bind("img-banner-master")
    .bind("master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind(1920i32)
    .bind(1080i32)
    .bind(vec![9_u8, 9, 9])
    .bind("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("banner master variant row should insert");

    sqlx::query(
        "INSERT INTO title_image_variants (
            id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("variant-fanart-master")
    .bind("img-fanart-master")
    .bind("master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind(2560i32)
    .bind(1440i32)
    .bind(vec![8_u8, 8, 8])
    .bind("dddddddddddddddddddddddddddddddd")
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("fanart master variant row should insert");

    run_embedded_migration(
        &services.pool,
        include_str!("../../scryer/src/db/migrations/0081_title_image_master_dedup.sql"),
    )
    .await;

    let paths =
        sqlx::query("SELECT banner_local_path, background_local_path FROM titles WHERE id = ?")
            .bind(&title.id)
            .fetch_one(&services.pool)
            .await
            .expect("load migrated local paths");
    let banner_local_path: Option<String> = paths.try_get("banner_local_path").unwrap_or(None);
    let background_local_path: Option<String> =
        paths.try_get("background_local_path").unwrap_or(None);
    assert_eq!(
        banner_local_path.as_deref(),
        Some("/images/titles/title-master-dedup/banner/master?v=aaaaaaaaaaaaaaaa")
    );
    assert_eq!(
        background_local_path.as_deref(),
        Some("/images/titles/title-master-dedup/fanart/master?v=cccccccccccccccc")
    );

    let remaining_master_variants: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM title_image_variants tiv
         INNER JOIN title_images ti ON ti.id = tiv.title_image_id
         WHERE tiv.variant_key = 'master'
           AND ti.kind IN ('banner', 'fanart')",
    )
    .fetch_one(&services.pool)
    .await
    .expect("count remaining master variants");
    assert_eq!(remaining_master_variants, 0);

    let banner = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Banner, "master")
        .await
        .expect("banner blob lookup should succeed after migration");
    assert_eq!(
        banner,
        Some(TitleImageBlob {
            content_type: "image/avif".to_string(),
            etag: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            bytes: vec![1, 2, 3, 4],
        })
    );

    let fanart = title_images
        .get_title_image_blob(&title.id, TitleImageKind::Fanart, "master")
        .await
        .expect("fanart blob lookup should succeed after migration");
    assert_eq!(
        fanart,
        Some(TitleImageBlob {
            content_type: "image/avif".to_string(),
            etag: "cccccccccccccccccccccccccccccccc".to_string(),
            bytes: vec![5, 6, 7, 8],
        })
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_image_local_path_backfill_matches_legacy_served_path() {
    let db = std::env::temp_dir().join(format!(
        "scryer_title_image_backfill_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let catalog = title_store(&services);

    let title = make_test_title(
        "title-backfill",
        Some("https://tvdb.example/poster-backfill.jpg"),
    );
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO title_images (
            id, title_id, provider, provider_image_id, kind, source_url, source_etag,
            source_last_modified, source_format, source_width, source_height, storage_mode,
            master_path, master_format, master_sha256, master_width, master_height, bytes,
            created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("img-backfill")
    .bind(&title.id)
    .bind("tvdb")
    .bind(Option::<String>::None)
    .bind("poster")
    .bind("https://tvdb.example/poster-backfill.jpg")
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind("jpeg")
    .bind(1000i32)
    .bind(1500i32)
    .bind("avif_master")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind("12121212121212121212121212121212")
    .bind(1000i32)
    .bind(1500i32)
    .bind(vec![1_u8, 2, 3])
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("legacy title image row should insert");

    sqlx::query(
        "INSERT INTO title_image_variants (
            id, title_image_id, variant_key, path, format, width, height, bytes, sha256, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("variant-backfill")
    .bind("img-backfill")
    .bind("w500")
    .bind(Option::<String>::None)
    .bind("avif")
    .bind(500i32)
    .bind(750i32)
    .bind(vec![4_u8, 5, 6])
    .bind("34343434343434343434343434343434")
    .bind(Utc::now().to_rfc3339())
    .bind(Utc::now().to_rfc3339())
    .execute(&services.pool)
    .await
    .expect("legacy title image variant row should insert");

    sqlx::query("UPDATE titles SET poster_local_path = NULL WHERE id = ?")
        .bind(&title.id)
        .execute(&services.pool)
        .await
        .expect("local path should be cleared to simulate legacy state");

    for statement in
        include_str!("../../scryer/src/db/migrations/0075_title_image_local_paths.sql").split(";\n")
    {
        let statement = statement.trim();
        if statement.is_empty() {
            continue;
        }
        if statement.starts_with("ALTER TABLE titles ADD COLUMN") {
            let _ = sqlx::query(statement).execute(&services.pool).await;
            continue;
        }
        sqlx::query(statement)
            .execute(&services.pool)
            .await
            .expect("backfill statement should succeed");
    }

    let materialized_path: Option<String> =
        sqlx::query_scalar("SELECT poster_local_path FROM titles WHERE id = ?")
            .bind(&title.id)
            .fetch_one(&services.pool)
            .await
            .expect("materialized local path should exist");
    assert_eq!(
        materialized_path.as_deref(),
        Some("/images/titles/title-backfill/poster/w500?v=3434343434343434")
    );

    let hydrated = TitleRepository::get_by_id(&catalog, &title.id)
        .await
        .expect("title lookup should succeed")
        .expect("title should exist");
    assert_eq!(
        hydrated.poster_url.as_deref(),
        Some("/images/titles/title-backfill/poster/w500?v=3434343434343434")
    );
    assert_eq!(
        hydrated.poster_source_url.as_deref(),
        Some("https://tvdb.example/poster-backfill.jpg")
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_validate_mode_rejects_pending_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    assert!(
        result.is_err(),
        "validate mode should reject unapplied migrations"
    );
    let err = match result {
        Ok(_) => panic!("validate mode should reject unapplied migrations"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("pending migration"));
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_validate_mode_does_not_mutate_legacy_sqlx_ledger() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_mode_legacy_ledger_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    sqlx::query("ALTER TABLE _sqlx_migrations RENAME TO _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("legacy ledger rename should succeed");
    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration ledger should be created");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         SELECT version, description, installed_on, success, checksum, execution_time
           FROM _sqlx_migrations_current
          WHERE version <= 102",
    )
    .execute(&services.pool)
    .await
    .expect("legacy migration rows should be copied");
    sqlx::query("DROP TABLE _sqlx_migrations_current")
        .execute(&services.pool)
        .await
        .expect("temporary migration ledger should be dropped");

    drop(services);

    let result =
        SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly).await;
    let err = match result {
        Ok(_) => panic!("validate mode should reject missing migration 0103"),
        Err(err) => err,
    };
    assert!(
        err.to_string()
            .contains("0103_custom_migrator_runtime_cutover"),
        "validate mode should report the pending custom migration, got {err:?}"
    );

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    let applied_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("migration row count should load");
    assert_eq!(applied_rows, 102);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_bootstrap_rejects_unknown_or_newer_schema_history() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_compat_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let too_new_key = "999999_too_new";
    sqlx::query(
        "UPDATE _sqlx_migrations
            SET checksum = ?
          WHERE version = ?",
    )
    .bind(Vec::<u8>::new())
    .bind(1i64)
    .execute(&pool)
    .await
    .expect("tamper first migration checksum");
    sqlx::query(
        "INSERT INTO _sqlx_migrations
        (version, description, installed_on, success, checksum, execution_time)
        VALUES (?, ?, CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(999999i64)
    .bind(too_new_key)
    .bind(Vec::<u8>::new())
    .execute(&pool)
    .await
    .expect("insert new migration");

    let result = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::Apply).await;
    assert!(result.is_err());
    let err = match result {
        Ok(_) => panic!("bad migration history should fail compatibility check"),
        Err(err) => err,
    };

    let message = err.to_string();
    assert!(message.contains("checksum mismatch"));
    assert!(message.contains("migrations newer than supported"));
    assert!(message.contains("Please update scryer"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_status_listing_reads_legacy_ledger_without_mutating_schema() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_status_legacy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        r#"
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
)
        "#,
    )
    .execute(&pool)
    .await
    .expect("legacy migration ledger should be created");

    sqlx::query(
        "INSERT INTO _sqlx_migrations
            (version, description, installed_on, success, checksum, execution_time)
         VALUES (1, 'init', CURRENT_TIMESTAMP, 1, ?, 0)",
    )
    .bind(vec![1u8, 2, 3])
    .execute(&pool)
    .await
    .expect("legacy migration row should be inserted");

    let statuses = crate::migrations::list_applied_migrations(&pool)
        .await
        .expect("status listing should succeed");
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].migration_checksum_algo, "inferred");

    let checksum_algo_columns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM pragma_table_info('_sqlx_migrations')
          WHERE name = 'checksum_algo'",
    )
    .fetch_one(&pool)
    .await
    .expect("pragma_table_info should succeed");
    assert_eq!(checksum_algo_columns, 0);

    drop(pool);
    let _ = std::fs::remove_file(db);
}

#[test]
fn compile_source_bundle_rejects_unknown_rust_hook_ids() {
    let db_root = std::env::temp_dir().join(format!(
        "scryer_migration_hook_fixture_{}",
        chrono::Utc::now().timestamp_micros()
    ));
    std::fs::create_dir_all(db_root.join("migrations")).expect("fixture migrations dir");
    std::fs::write(
        db_root.join("migrations/0001_initial.sql"),
        "CREATE TABLE example (id INTEGER PRIMARY KEY);\n",
    )
    .expect("write legacy migration");
    std::fs::write(
        db_root.join("migration_manifest.toml"),
        r#"
format_version = 1

[legacy_sql]
path = "migrations"
through_version = 1

[[migration]]
version = 2
description = "bad hook"
checksum_algo = "blake3"
steps = [
  { kind = "rust", hook_id = "missing_hook", engine = "all", scope = "all" },
]
"#,
    )
    .expect("write manifest");

    let error = crate::migration_assets::compile_source_bundle(&db_root)
        .expect_err("unknown hook id should fail manifest compilation");
    assert!(error.contains("unknown migration hook id 'missing_hook'"));

    let _ = std::fs::remove_dir_all(db_root);
}

#[tokio::test]
async fn specials_convergence_migration_repoints_legacy_season_zero_references() {
    let db = std::env::temp_dir().join(format!(
        "scryer_specials_convergence_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS title_history (
            id TEXT PRIMARY KEY,
            title_id TEXT NOT NULL,
            episode_id TEXT,
            collection_id TEXT,
            event_type TEXT NOT NULL,
            source_title TEXT,
            quality TEXT,
            download_id TEXT,
            data_json TEXT,
            occurred_at TEXT NOT NULL,
            created_at TEXT NOT NULL,
            FOREIGN KEY (title_id) REFERENCES titles(id) ON DELETE CASCADE
        )",
    )
    .execute(&pool)
    .await
    .expect("create legacy title_history compatibility table");

    for statement in [
        "CREATE TABLE IF NOT EXISTS releases (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS workflow_operations (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
        "CREATE TABLE IF NOT EXISTS download_submissions (
            id TEXT PRIMARY KEY,
            collection_id TEXT
        )",
    ] {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("create legacy compatibility table");
    }

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO titles (id, name, name_normalized, facet, monitored, status, tags, external_ids, created_at)
         VALUES (?, ?, ?, ?, 1, 'active', '[]', '[]', ?)",
    )
    .bind("title-series")
    .bind("Legacy Series")
    .bind("legacy series")
    .bind("series")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert title");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-specials")
    .bind("title-series")
    .bind("season")
    .bind("0")
    .bind("Season 0")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert legacy specials");

    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, monitored, created_at, special_movies_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("canonical-specials")
    .bind("title-series")
    .bind("specials")
    .bind("0")
    .bind("Specials")
    .bind(0i64)
    .bind(&now)
    .bind("[]")
    .execute(&pool)
    .await
    .expect("insert canonical specials");

    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("episode-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("special")
    .bind("1")
    .bind("0")
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy episode");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-legacy")
    .bind("title-series")
    .bind("episode")
    .bind("primary")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("legacy-specials")
    .execute(&pool)
    .await
    .expect("insert legacy wanted item");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, created_at, updated_at, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-canonical")
    .bind("title-series")
    .bind("episode")
    .bind("primary")
    .bind("wanted")
    .bind(&now)
    .bind(&now)
    .bind("canonical-specials")
    .execute(&pool)
    .await
    .expect("insert canonical wanted item");

    sqlx::query(
        "INSERT INTO title_history
         (id, title_id, collection_id, event_type, occurred_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("history-legacy")
    .bind("title-series")
    .bind("legacy-specials")
    .bind("imported")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy title history row");

    let migration_sql =
        include_str!("../../scryer/src/db/migrations/0070_specials_collection_convergence.sql");
    for statement in migration_sql
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
    {
        sqlx::query(statement)
            .execute(&pool)
            .await
            .expect("run migration statement");
    }

    let collections: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, collection_type FROM collections WHERE title_id = ? ORDER BY id",
    )
    .bind("title-series")
    .fetch_all(&pool)
    .await
    .expect("load collections");
    assert_eq!(
        collections,
        vec![("canonical-specials".to_string(), "specials".to_string())]
    );

    let episode_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM episodes WHERE id = ?")
            .bind("episode-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated episode collection");
    assert_eq!(episode_collection, "canonical-specials");

    let wanted_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM wanted_items WHERE collection_id = ? ORDER BY id")
            .bind("canonical-specials")
            .fetch_all(&pool)
            .await
            .expect("load wanted items");
    assert_eq!(wanted_ids, vec!["wanted-canonical".to_string()]);

    let history_collection: String =
        sqlx::query_scalar("SELECT collection_id FROM title_history WHERE id = ?")
            .bind("history-legacy")
            .fetch_one(&pool)
            .await
            .expect("load migrated title history collection");
    assert_eq!(history_collection, "canonical-specials");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migrations_apply_then_validate_is_idempotent() {
    let db = std::env::temp_dir().join(format!(
        "scryer_validate_then_apply_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    drop(services);

    let _ = SqliteServices::new_with_mode(db.to_string_lossy(), MigrationMode::ValidateOnly)
        .await
        .expect("applied DB should pass validate mode");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn complete_wanted_item_for_title_updates_matching_row_in_one_step() {
    let db = std::env::temp_dir().join(format!(
        "scryer_complete_wanted_item_for_title_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now().to_rfc3339();

    let title = make_test_title("title-series", None);
    TitleRepository::create(&catalog, title)
        .await
        .expect("title should insert");

    sqlx::query(
        "INSERT INTO wanted_items
         (id, title_id, media_type, search_phase, status, search_count,
          current_score, grabbed_release, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("wanted-episode")
    .bind("title-series")
    .bind("movie")
    .bind("primary")
    .bind("wanted")
    .bind(7i64)
    .bind(42i64)
    .bind("Existing Release")
    .bind(&now)
    .bind(&now)
    .execute(services.pool())
    .await
    .expect("wanted item should insert");

    let completed = workflow
        .complete_wanted_item_for_title("title-series", None, Some("2026-04-20T00:00:00Z"), None)
        .await
        .expect("completion should succeed");

    assert!(completed);

    let row = sqlx::query(
        "SELECT status, next_search_at, last_search_at, search_count, current_score, grabbed_release
         FROM wanted_items
         WHERE id = ?",
    )
    .bind("wanted-episode")
    .fetch_one(services.pool())
    .await
    .expect("wanted item should load");

    assert_eq!(row.get::<String, _>("status"), "completed");
    assert_eq!(row.get::<Option<String>, _>("next_search_at"), None);
    assert_eq!(
        row.get::<Option<String>, _>("last_search_at"),
        Some("2026-04-20T00:00:00Z".to_string())
    );
    assert_eq!(row.get::<i64, _>("search_count"), 7);
    assert_eq!(row.get::<Option<i64>, _>("current_score"), Some(42));
    assert_eq!(
        row.get::<Option<String>, _>("grabbed_release"),
        Some("Existing Release".to_string())
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_due_wanted_items_excludes_blocked_facets_before_limit() {
    let db = std::env::temp_dir().join(format!(
        "scryer_due_wanted_items_by_facet_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let shows = show_store(&services);
    let now = Utc::now().to_rfc3339();

    let mut movie_title = make_test_title("title-movie", None);
    movie_title.name = "Blocked Movie".to_string();
    movie_title.facet = MediaFacet::Movie;
    TitleRepository::create(&catalog, movie_title.clone())
        .await
        .expect("movie title should insert");

    let mut series_title = make_test_title("title-series", None);
    series_title.name = "Eligible Series".to_string();
    series_title.facet = MediaFacet::Series;
    TitleRepository::create(&catalog, series_title.clone())
        .await
        .expect("series title should insert");

    let mut anime_title = make_test_title("title-anime", None);
    anime_title.name = "Blocked Anime".to_string();
    anime_title.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, anime_title.clone())
        .await
        .expect("anime title should insert");

    let series_collection = ShowRepository::create_collection(
        &shows,
        Collection {
            id: "series-season-1".to_string(),
            title_id: series_title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        },
    )
    .await
    .expect("series collection should insert");

    let series_episode = ShowRepository::create_episode(
        &shows,
        Episode {
            id: "series-episode-1".to_string(),
            title_id: series_title.id.clone(),
            collection_id: Some(series_collection.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_800),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        },
    )
    .await
    .expect("series episode should insert");

    let anime_collection = ShowRepository::create_collection(
        &shows,
        Collection {
            id: "anime-interstitial-1".to_string(),
            title_id: anime_title.id.clone(),
            collection_type: CollectionType::Interstitial,
            collection_index: "0".to_string(),
            label: Some("Movie".to_string()),
            ordered_path: None,
            narrative_order: Some("0".to_string()),
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(InterstitialMovieMetadata {
                tvdb_id: "anime-movie-1".to_string(),
                name: "Franchise Movie".to_string(),
                slug: "franchise-movie".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Franchise movie between anime arcs".to_string(),
                poster_url: String::new(),
                language: "ja".to_string(),
                runtime_minutes: 100,
                sort_title: "Franchise Movie".to_string(),
                continuity_status: Some("canon".to_string()),
                genres: vec!["anime".to_string()],
                studio: "Studio".to_string(),
                digital_release_date: Some("2024-01-01".to_string()),
                imdb_id: String::new(),
                association_confidence: Some("high".to_string()),
                movie_form: None,
                confidence: Some("high".to_string()),
                signal_summary: Some("Inserted by test fixture".to_string()),
                placement: Some("between_seasons".to_string()),
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        },
    )
    .await
    .expect("anime interstitial collection should insert");

    for item in [
        scryer_application::WantedItem {
            id: "wanted-movie".to_string(),
            title_id: movie_title.id.clone(),
            title_name: Some(movie_title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some("2024-01-01T00:00:00Z".to_string()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        scryer_application::WantedItem {
            id: "wanted-series-episode".to_string(),
            title_id: series_title.id.clone(),
            title_name: Some(series_title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(series_episode.id.clone()),
            collection_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some("2024-01-01T00:00:01Z".to_string()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
        scryer_application::WantedItem {
            id: "wanted-anime-movie".to_string(),
            title_id: anime_title.id.clone(),
            title_name: Some(anime_title.name.clone()),
            title_slug: None,
            title_facet: None,
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: Some(anime_collection.id.clone()),
            season_number: Some("0".to_string()),
            episode_number: None,
            media_type: "interstitial_movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some("2024-01-01T00:00:00Z".to_string()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: now.clone(),
            updated_at: now.clone(),
        },
    ] {
        workflow
            .upsert_wanted_item(&item)
            .await
            .expect("wanted item should insert");
    }

    let rows = workflow
        .list_due_wanted_items(
            "2024-01-02T00:00:00Z",
            2,
            &[MediaFacet::Movie, MediaFacet::Anime],
        )
        .await
        .expect("due wanted items query should succeed");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "wanted-series-episode");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_on_latest_decision_code() {
    let (services, db) = temp_services("scryer_wanted_latest_decision").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let title = make_test_title("title-latest-decision", None);
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");
    let other_title = make_test_title("title-latest-decision-other", None);
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_mismatch = WantedItem {
        id: "wanted-mismatch".to_string(),
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_quality_blocked = WantedItem {
        id: "wanted-quality-blocked".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some(other_title.name.clone()),
        ..wanted_mismatch.clone()
    };

    workflow
        .upsert_wanted_item(&wanted_mismatch)
        .await
        .expect("first wanted item should insert");
    workflow
        .upsert_wanted_item(&wanted_quality_blocked)
        .await
        .expect("second wanted item should insert");

    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-1".to_string(),
            wanted_item_id: wanted_mismatch.id.clone(),
            title_id: title.id.clone(),
            release_title: "Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-2".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "Old Mismatch Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: (now - chrono::Duration::minutes(2)).to_rfc3339(),
        })
        .await
        .expect("older mismatch decision should insert");
    workflow
        .insert_release_decision(&ReleaseDecision {
            id: "decision-3".to_string(),
            wanted_item_id: wanted_quality_blocked.id.clone(),
            title_id: other_title.id.clone(),
            release_title: "New Blocked Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "quality_blocked".to_string(),
            candidate_score: 0,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: now.to_rfc3339(),
        })
        .await
        .expect("latest blocked decision should insert");

    let items = workflow
        .list_wanted_items(WantedItemsQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_wanted_items(WantedItemsQuery {
            latest_decision_codes: vec!["title_mismatch".into()],
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_mismatch.id);
    assert!(items[0].mismatch_recovery_eligible);
    let latest_decision = items[0]
        .latest_release_decision
        .as_ref()
        .expect("latest decision should be hydrated");
    assert_eq!(latest_decision.decision_code, "title_mismatch");
    assert_eq!(latest_decision.release_title, "Mismatch Release");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_matches_aliases_slug_and_typos_with_direct_priority() {
    let (services, db) = temp_services("scryer_catalog_title_search").await;
    let catalog = title_store(&services);

    let mut direct_title = make_test_title("title-search-direct", None);
    direct_title.name = "Schoolhouse Rock! Earth".to_string();
    direct_title.slug = Some("schoolhouse-rock-earth".to_string());
    direct_title.aliases = vec!["School House Rock".to_string()];
    direct_title.tagged_aliases = vec![TaggedAlias {
        name: "Schoolhouse Planet Earth".to_string(),
        language: "eng".to_string(),
    }];
    TitleRepository::create(&catalog, direct_title.clone())
        .await
        .expect("direct title should insert");

    let mut typo_title = make_test_title("title-search-typo", None);
    typo_title.name = "Schoolhouze Rock Earth".to_string();
    TitleRepository::create(&catalog, typo_title.clone())
        .await
        .expect("typo title should insert");

    let alias_hits = TitleRepository::list(&catalog, None, Some("school house rock".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let slug_hits =
        TitleRepository::list(&catalog, None, Some("schoolhouse rock earth".to_string()))
            .await
            .expect("slug search should load");
    assert_eq!(
        slug_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );

    let typo_hits =
        TitleRepository::list(&catalog, None, Some("scholhouse rock earth".to_string()))
            .await
            .expect("typo search should load");
    assert_eq!(
        typo_hits.first().map(|title| title.id.as_str()),
        Some(direct_title.id.as_str())
    );
    assert!(typo_hits.iter().any(|title| title.id == typo_title.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_short_typo_does_not_return_loose_spellfix_neighbors() {
    let (services, db) = temp_services("scryer_catalog_title_search_short_typo").await;
    let catalog = title_store(&services);

    let mut aoashi = make_test_title("title-search-aoashi", None);
    aoashi.name = "Aoashi".to_string();
    aoashi.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, aoashi.clone())
        .await
        .expect("close typo target should insert");

    let mut ranma = make_test_title("title-search-ranma", None);
    ranma.name = "Ranma 1/2 (2024)".to_string();
    ranma.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, ranma.clone())
        .await
        .expect("loose neighbor should insert");

    let mut blue_box = make_test_title("title-search-blue-box", None);
    blue_box.name = "Blue Box".to_string();
    blue_box.facet = MediaFacet::Anime;
    TitleRepository::create(&catalog, blue_box.clone())
        .await
        .expect("loose neighbor should insert");

    let mut her_blue_sky = make_test_title("title-search-her-blue-sky", None);
    her_blue_sky.name = "Her Blue Sky".to_string();
    TitleRepository::create(&catalog, her_blue_sky.clone())
        .await
        .expect("movie loose neighbor should insert");

    let hits = TitleRepository::list(&catalog, None, Some("aashi".to_string()))
        .await
        .expect("short typo search should load");
    let hit_ids = hits
        .into_iter()
        .map(|title| title.id)
        .collect::<HashSet<_>>();

    assert!(hit_ids.contains(&aoashi.id));
    assert!(!hit_ids.contains(&ranma.id));
    assert!(!hit_ids.contains(&blue_box.id));
    assert!(!hit_ids.contains(&her_blue_sky.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_returns_valid_single_substitution_typo_for_frieren() {
    let (services, db) = temp_services("scryer_catalog_title_search_frieren_typo").await;
    let catalog = title_store(&services);

    let mut frieren = make_test_title("title-search-frieren", None);
    frieren.name = "Silver Horizon: Beyond Journey's End".to_string();
    frieren.facet = MediaFacet::Anime;
    frieren.aliases = vec!["Sora no Vale".to_string(), "Frieren".to_string()];
    TitleRepository::create(&catalog, frieren.clone())
        .await
        .expect("frieren should insert");

    let mut friend = make_test_title("title-search-friend", None);
    friend.name = "Friend".to_string();
    TitleRepository::create(&catalog, friend.clone())
        .await
        .expect("friend should insert");

    let mut firefly = make_test_title("title-search-firefly", None);
    firefly.name = "Signal Run".to_string();
    TitleRepository::create(&catalog, firefly.clone())
        .await
        .expect("firefly should insert");

    let hits = TitleRepository::list(&catalog, None, Some("friefen".to_string()))
        .await
        .expect("frieren typo search should load");

    assert_eq!(
        hits.first().map(|title| title.id.as_str()),
        Some(frieren.id.as_str())
    );
    assert!(!hits.iter().any(|title| title.id == friend.id));
    assert!(!hits.iter().any(|title| title.id == firefly.id));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn title_search_projection_refreshes_after_hydrated_metadata_update_and_delete() {
    let (services, db) = temp_services("scryer_title_search_projection_refresh").await;
    let catalog = title_store(&services);

    let mut title = make_test_title("title-projection-refresh", None);
    title.name = "Example Show".to_string();
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("title should insert");

    let missing_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("pre-update search should load");
    assert!(missing_hits.is_empty());

    TitleRepository::update_title_hydrated_metadata(
        &catalog,
        &title.id,
        TitleMetadataUpdate {
            slug: Some("earth-defenders".to_string()),
            aliases: vec!["Earth's Defenders".to_string()],
            tagged_aliases: vec![TaggedAlias {
                name: "Earth Defenders".to_string(),
                language: "eng".to_string(),
            }],
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            ..Default::default()
        },
    )
    .await
    .expect("hydrated metadata should update");

    let alias_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("alias search should load");
    assert_eq!(
        alias_hits
            .first()
            .map(|match_title| match_title.id.as_str()),
        Some(title.id.as_str())
    );

    TitleRepository::delete(&catalog, &title.id)
        .await
        .expect("title should delete");

    let deleted_hits = TitleRepository::list(&catalog, None, Some("earth defenders".to_string()))
        .await
        .expect("post-delete search should load");
    assert!(deleted_hits.is_empty());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_wanted_items_filters_with_fuzzy_title_search() {
    let (services, db) = temp_services("scryer_wanted_title_search").await;
    let workflow = wanted_store(&services);
    let catalog = title_store(&services);
    let now = Utc::now();

    let mut title = make_test_title("title-search-match", None);
    title.name = "Schoolhouse Rock! Earth".to_string();
    title.aliases = vec!["School House Rock".to_string()];
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("matching title should insert");
    let mut other_title = make_test_title("title-search-other", None);
    other_title.name = "Different Show".to_string();
    TitleRepository::create(&catalog, other_title.clone())
        .await
        .expect("other title should insert");

    let wanted_match = WantedItem {
        id: "wanted-search-match".to_string(),
        title_id: title.id.clone(),
        title_name: Some("Schoolhouse Rock! Earth".to_string()),
        title_slug: None,
        title_facet: None,
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "episode".to_string(),
        search_phase: "long_tail".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };
    let wanted_other = WantedItem {
        id: "wanted-search-other".to_string(),
        title_id: other_title.id.clone(),
        title_name: Some("Different Show".to_string()),
        ..wanted_match.clone()
    };

    workflow
        .upsert_wanted_item(&wanted_match)
        .await
        .expect("matching wanted item should insert");
    workflow
        .upsert_wanted_item(&wanted_other)
        .await
        .expect("other wanted item should insert");

    let items = workflow
        .list_wanted_items(WantedItemsQuery {
            title_search: Some("scholhouse erth".into()),
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted items should load");
    let count = workflow
        .count_wanted_items(WantedItemsQuery {
            title_search: Some("scholhouse erth".into()),
            ..WantedItemsQuery::default()
        })
        .await
        .expect("filtered wanted count should load");

    assert_eq!(items.len(), 1);
    assert_eq!(count, 1);
    assert_eq!(items[0].id, wanted_match.id);

    let short_items = workflow
        .list_wanted_items(WantedItemsQuery {
            title_search: Some("roc".into()),
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("short filtered wanted items should load");
    let short_count = workflow
        .count_wanted_items(WantedItemsQuery {
            title_search: Some("roc".into()),
            ..WantedItemsQuery::default()
        })
        .await
        .expect("short filtered wanted count should load");

    assert_eq!(short_items.len(), 1);
    assert_eq!(short_count, 1);
    assert_eq!(short_items[0].id, wanted_match.id);

    let short_title_hits = TitleRepository::list(&catalog, None, Some("roc".to_string()))
        .await
        .expect("short title list search should load");
    assert_eq!(short_title_hits.len(), 1);
    assert_eq!(short_title_hits[0].id, title.id);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_faceted_projection_allows_cross_facet_duplicates_and_seeds_only_tvdb_titles()
 {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_facets_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-1")
    .bind("Series")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-1")
    .bind("Movie")
    .bind("movie")
    .bind(r#"[{"source":"tvdb","value":"123"}]"#)
    .bind("movie-imdb")
    .bind("IMDb Only")
    .bind("movie")
    .bind(r#"[{"source":"imdb","value":"tt1234567"}]"#)
    .execute(&pool)
    .await
    .expect("insert legacy titles");

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"),
    )
    .await;

    let faceted_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT title_id, facet, external_id
         FROM title_external_ids
         WHERE source = 'tvdb'
         ORDER BY facet, title_id",
    )
    .fetch_all(&pool)
    .await
    .expect("load projected faceted tvdb ids");
    assert_eq!(
        faceted_rows,
        vec![
            (
                "movie-1".to_string(),
                "movie".to_string(),
                "123".to_string()
            ),
            (
                "series-1".to_string(),
                "series".to_string(),
                "123".to_string()
            ),
        ]
    );

    let due_now: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT id, metadata_hydration_next_attempt_at
         FROM titles
         ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .expect("load hydration due markers");
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-imdb")
            .expect("imdb title marker")
            .1
            .is_none()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "movie-1")
            .expect("movie tvdb marker")
            .1
            .is_some()
    );
    assert!(
        due_now
            .iter()
            .find(|(id, _)| id == "series-1")
            .expect("series tvdb marker")
            .1
            .is_some()
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_same_facet_duplicate_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_duplicate_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let migration_sql = include_str!(
        "../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on same-facet duplicate")
    };
    assert!(
        err.to_string().contains("UNIQUE"),
        "expected uniqueness failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let legacy_external_id: String =
        sqlx::query_scalar("SELECT external_id FROM title_external_ids WHERE id = 'legacy-row'")
            .fetch_one(&pool)
            .await
            .expect("legacy row should remain");
    assert_eq!(legacy_external_id, "legacy");

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_conflict_hint_lists_colliding_title_ids() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_conflict_hint_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL), (?, ?, ?, ?, NULL)",
    )
    .bind("series-a")
    .bind("Series A")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .bind("series-b")
    .bind("Series B")
    .bind("series")
    .bind(r#"[{"source":"tvdb","value":"999"}]"#)
    .execute(&pool)
    .await
    .expect("insert conflicting legacy titles");

    let hint = crate::migrations::title_external_id_projection_conflict_hint(&pool)
        .await
        .expect("conflict hint should be present");
    assert!(hint.contains("series/tvdb/999"));
    assert!(hint.contains("series-a"));
    assert!(hint.contains("series-b"));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0079_rejects_invalid_projection_before_delete() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0079_invalid_json_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0079_title_projection_schema(&pool).await;

    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO title_external_ids
         (id, title_id, source, external_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind("legacy-row")
    .bind("legacy-title")
    .bind("tvdb")
    .bind("legacy")
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert legacy projection row");

    sqlx::query(
        "INSERT INTO titles (id, name, facet, external_ids, metadata_fetched_at)
         VALUES (?, ?, ?, ?, NULL)",
    )
    .bind("series-bad")
    .bind("Broken Series")
    .bind("series")
    .bind("{not-valid-json")
    .execute(&pool)
    .await
    .expect("insert malformed legacy title");

    let migration_sql = include_str!(
        "../../scryer/src/db/migrations/0079_title_external_id_projection_and_metadata_hydration_retry.sql"
    );
    let err = {
        let mut failed = None;
        for statement in migration_sql
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            if let Err(error) = sqlx::query(statement).execute(&pool).await {
                failed = Some(error);
                break;
            }
        }
        failed.expect("migration should fail on malformed external_ids json")
    };
    assert!(
        err.to_string().contains("malformed"),
        "expected malformed json failure, got: {err}"
    );

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM title_external_ids")
        .fetch_one(&pool)
        .await
        .expect("load remaining legacy projection rows");
    assert_eq!(remaining, 1);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0084_backfills_analysis_json_and_preserves_stream_reads() {
    let db = std::env::temp_dir().join(format!(
        "scryer_migration_0084_media_analysis_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    create_pre_0084_media_file_schema(&pool).await;

    // Seeded from a real pre-0084 media_files row pulled from the running local scryer
    // container on 2026-04-21 so the migration test exercises production-shaped data.
    let legacy_ffprobe_json = r#"{"format":"matroska","duration_seconds":1440.055,"num_chapters":4,"tracks":[{"kind":"video","codec_id":"V_MPEG4/ISO/AVC","codec_name":"h264","audio_profile":null,"width":1920,"height":1080,"channels":null,"bit_rate_bps":8253642,"language":null,"frame_rate_fps":23.976024167640553},{"kind":"audio","codec_id":"A_AAC","codec_name":"aac","audio_profile":"LC","width":null,"height":null,"channels":2,"bit_rate_bps":null,"language":"jpn","frame_rate_fps":43.0664074528313},{"kind":"audio","codec_id":"A_AAC","codec_name":"aac","audio_profile":"LC","width":null,"height":null,"channels":2,"bit_rate_bps":null,"language":"eng","frame_rate_fps":43.0664074528313},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"eng","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"eng","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/UTF8","codec_name":"subrip","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"eng","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"ara","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"ger","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"spa","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"spa","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"fre","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"ita","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"por","frame_rate_fps":null},{"kind":"subtitle","codec_id":"S_TEXT/ASS","codec_name":"ass","audio_profile":null,"width":null,"height":null,"channels":null,"bit_rate_bps":null,"language":"rus","frame_rate_fps":null}]}"#;
    let legacy_audio_languages_json = r#"["jpn","eng"]"#;
    let legacy_subtitle_languages_json = r#"["eng","ara","deu","spa","fra","ita","por","rus"]"#;
    let legacy_subtitle_codecs_json =
        r#"["ass","ass","subrip","ass","ass","ass","ass","ass","ass","ass","ass"]"#;
    let legacy_audio_streams_json = r#"[{"codec":"aac","profile":"LC","channels":2,"language":"jpn","bitrate_kbps":null},{"codec":"aac","profile":"LC","channels":2,"language":"eng","bitrate_kbps":null}]"#;
    let legacy_subtitle_streams_json = r#"[{"codec":"ass","language":"eng","name":"Forced","forced":true,"default":false},{"codec":"ass","language":"eng","name":null,"forced":false,"default":true},{"codec":"subrip","language":"eng","name":"CC","forced":false,"default":false},{"codec":"ass","language":"ara","name":"Saudi Arabia","forced":false,"default":false},{"codec":"ass","language":"deu","name":null,"forced":false,"default":false},{"codec":"ass","language":"spa","name":"Latin American","forced":false,"default":false},{"codec":"ass","language":"spa","name":"European","forced":false,"default":false},{"codec":"ass","language":"fra","name":null,"forced":false,"default":false},{"codec":"ass","language":"ita","name":null,"forced":false,"default":false},{"codec":"ass","language":"por","name":"Brazilian","forced":false,"default":false},{"codec":"ass","language":"rus","name":null,"forced":false,"default":false}]"#;

    sqlx::query(
        "INSERT INTO media_files (
            id, title_id, file_path, size_bytes, quality_id, has_multiaudio,
            scan_status, created_at, video_codec, video_width, video_height,
            video_bitrate_kbps, video_bit_depth, video_hdr_format, audio_codec,
            audio_channels, duration_seconds, container_format, ffprobe_json,
            video_frame_rate, video_profile, audio_bitrate_kbps, subtitle_codecs_json,
            audio_streams_json, subtitle_languages_json, subtitle_streams_json,
            audio_languages_json, num_chapters, audio_profile
        ) VALUES (
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?, ?,
            ?, ?, ?, ?, ?
        )",
    )
    .bind("file-legacy")
    .bind("title-legacy")
    .bind("/data/anime/The Apothecary Diaries/The Apothecary Diaries.S02E03.Corpse Fungus.mkv")
    .bind(1_485_712_325i64)
    .bind(Option::<String>::None)
    .bind(1i64)
    .bind("scanned")
    .bind("2026-04-21T18:56:54.286796797+00:00")
    .bind("h264")
    .bind(1_920i64)
    .bind(1_080i64)
    .bind(8_253i64)
    .bind(8i64)
    .bind(Option::<String>::None)
    .bind("aac")
    .bind(2i64)
    .bind(1_440i64)
    .bind("matroska")
    .bind(legacy_ffprobe_json)
    .bind("23.976")
    .bind("High")
    .bind(Option::<i64>::None)
    .bind(legacy_subtitle_codecs_json)
    .bind(legacy_audio_streams_json)
    .bind(legacy_subtitle_languages_json)
    .bind(legacy_subtitle_streams_json)
    .bind(legacy_audio_languages_json)
    .bind(4i64)
    .bind("LC")
    .execute(&pool)
    .await
    .expect("insert legacy media file row");

    run_embedded_migration(
        &pool,
        include_str!("../../scryer/src/db/migrations/0084_media_analysis_json.sql"),
    )
    .await;

    let columns: Vec<String> = sqlx::query("PRAGMA table_info(media_files)")
        .fetch_all(&pool)
        .await
        .expect("load migrated media_files columns")
        .into_iter()
        .map(|row| {
            row.try_get::<String, _>("name")
                .expect("table info row should have name")
        })
        .collect();
    assert!(columns.iter().any(|column| column == "analysis_json"));
    assert!(!columns.iter().any(|column| column == "ffprobe_json"));
    assert!(!columns.iter().any(|column| column == "audio_streams_json"));
    assert!(
        !columns
            .iter()
            .any(|column| column == "subtitle_streams_json")
    );

    let stored_analysis_json: Option<String> =
        sqlx::query_scalar("SELECT analysis_json FROM media_files WHERE id = ?")
            .bind("file-legacy")
            .fetch_one(&pool)
            .await
            .expect("analysis json should load");
    let stored_analysis_json = stored_analysis_json.expect("analysis json should be present");
    let stored_analysis_json: serde_json::Value =
        serde_json::from_str(&stored_analysis_json).expect("analysis json should parse");
    assert_eq!(
        stored_analysis_json["audio_languages"],
        serde_json::json!(["jpn", "eng"])
    );
    assert_eq!(
        stored_analysis_json["audio_profile"],
        serde_json::json!("LC")
    );
    assert_eq!(
        stored_analysis_json["has_multiaudio"],
        serde_json::json!(true)
    );
    assert!(stored_analysis_json.get("audio_streams").is_some());
    assert!(stored_analysis_json.get("tracks").is_none());

    let media_files = MediaFileStore::new(crate::queries::sql_runtime::StoreDatastore::Sqlite {
        pool: pool.clone(),
        writer_gate: Arc::new(tokio::sync::Mutex::new(())),
    });
    let media_file = media_files
        .get_media_file_by_id("file-legacy")
        .await
        .expect("lookup should succeed")
        .expect("media file should exist");
    assert_eq!(
        media_file.audio_languages,
        vec!["jpn".to_string(), "eng".to_string()]
    );
    assert_eq!(media_file.audio_streams[0].language.as_deref(), Some("jpn"));
    assert_eq!(media_file.audio_streams[0].profile.as_deref(), Some("LC"));
    assert_eq!(media_file.subtitle_codecs.len(), 11);
    assert!(
        media_file
            .subtitle_codecs
            .iter()
            .any(|codec| codec == "subrip")
    );
    assert_eq!(
        media_file.subtitle_streams[0].name.as_deref(),
        Some("Forced")
    );
    assert!(media_file.subtitle_streams[0].forced);
    assert_eq!(media_file.audio_profile.as_deref(), Some("LC"));
    assert_eq!(media_file.num_chapters, Some(4));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn tracked_state_upsert_creates_download_submission_row_when_missing() {
    let db = std::env::temp_dir().join(format!(
        "scryer_tracked_state_upsert_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());

    workflow_store
        .update_tracked_state(
            &DownloadSourceIdentity::new(None, "weaver", "job-123"),
            "failed",
        )
        .await
        .expect("tracked state upsert should succeed without a preexisting submission row");

    let tracked_state = workflow_store
        .get_tracked_state(&DownloadSourceIdentity::new(None, "weaver", "job-123"))
        .await
        .expect("tracked state query should succeed");
    assert_eq!(tracked_state.as_deref(), Some("failed"));

    let row = sqlx::query(
        "SELECT title_id, facet FROM download_submissions WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind("weaver")
    .bind("job-123")
    .fetch_one(services.pool())
    .await
    .expect("download submission row should exist");

    let title_id: String = row.get("title_id");
    let facet: String = row.get("facet");
    assert!(title_id.is_empty());
    assert!(facet.is_empty());

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn migration_0104_accepts_plain_path_settings_without_choking_on_unrelated_invalid_json() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");

    sqlx::query(
        "CREATE TABLE settings_definitions (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            scope TEXT NOT NULL,
            key_name TEXT NOT NULL,
            data_type TEXT NOT NULL,
            default_value_json TEXT,
            is_sensitive INTEGER NOT NULL DEFAULT 0,
            validation_json TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_definitions should create");

    sqlx::query(
        "CREATE TABLE settings_values (
            id TEXT PRIMARY KEY,
            setting_definition_id TEXT NOT NULL,
            scope TEXT NOT NULL,
            scope_id TEXT,
            value_json TEXT NOT NULL,
            source TEXT NOT NULL,
            updated_by_user_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("settings_values should create");

    sqlx::query(
        "CREATE TEMP TABLE _default_library_roots (
            library_id TEXT NOT NULL,
            path TEXT NOT NULL,
            is_default INTEGER NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("_default_library_roots should create");

    for (id, key_name) in [
        ("def-movies-path", "movies.path"),
        ("def-series-path", "series.path"),
        ("def-unrelated", "service:system:smg.client_key"),
    ] {
        sqlx::query(
            "INSERT INTO settings_definitions (
                id, category, scope, key_name, data_type, default_value_json,
                is_sensitive, validation_json, created_at, updated_at
            ) VALUES (?, 'test', 'system', ?, 'string', '\"\"', 0, NULL, 'now', 'now')",
        )
        .bind(id)
        .bind(key_name)
        .execute(&pool)
        .await
        .expect("setting definition should insert");
    }

    sqlx::query(
        "INSERT INTO settings_values (
            id, setting_definition_id, scope, scope_id, value_json, source,
            updated_by_user_id, created_at, updated_at
        ) VALUES
            ('row-movies', 'def-movies-path', 'media', NULL, '\"/Volumes/Media/Movies\"', 'test', NULL, 'now', 'now'),
            ('row-series', 'def-series-path', 'media', NULL, '/Volumes/Media/TV', 'test', NULL, 'now', 'now'),
            ('row-unrelated', 'def-unrelated', 'system', NULL, 'enc:v1:not-json', 'test', NULL, 'now', 'now')",
    )
    .execute(&pool)
    .await
    .expect("setting values should insert");

    let migration_sql = include_str!(
        "../../scryer/src/db/migrations/0104_first_class_libraries_and_permissions.sql"
    );
    let statement = migration_sql
        .split(';')
        .map(str::trim)
        .find(|statement| statement.starts_with("INSERT INTO _default_library_roots (library_id, path, is_default)\nSELECT\n    CASE sd.key_name\n        WHEN 'movies.path'"))
        .expect("0104 path backfill statement should exist");

    sqlx::query(statement)
        .execute(&pool)
        .await
        .expect("legacy plain path values should backfill without malformed json errors");

    let roots: Vec<(String, String)> =
        sqlx::query_as("SELECT library_id, path FROM _default_library_roots ORDER BY library_id")
            .fetch_all(&pool)
            .await
            .expect("backfilled roots should load");
    assert_eq!(
        roots,
        vec![
            (
                "movie_default_library".to_string(),
                "/Volumes/Media/Movies".to_string()
            ),
            (
                "series_default_library".to_string(),
                "/Volumes/Media/TV".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn queued_delete_stale_recovery_only_recovers_stale_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_delete_recovery_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadQueueCommandStore::new(services.datastore());

    let stale = workflow_store
        .queue_delete_command(None, "nzbget", "job-stale", false, Some("admin"))
        .await
        .expect("stale delete should queue");
    let fresh = workflow_store
        .queue_delete_command(None, "nzbget", "job-fresh", true, Some("admin"))
        .await
        .expect("fresh delete should queue");

    workflow_store
        .mark_delete_command_running(&stale.id)
        .await
        .expect("stale delete should mark running");
    workflow_store
        .mark_delete_command_running(&fresh.id)
        .await
        .expect("fresh delete should mark running");

    let stale_updated_at = (Utc::now() - chrono::Duration::seconds(300)).to_rfc3339();
    sqlx::query("UPDATE download_queue_commands SET updated_at = ? WHERE id = ?")
        .bind(&stale_updated_at)
        .bind(&stale.id)
        .execute(&services.pool)
        .await
        .expect("age stale running delete");

    let recovered = workflow_store
        .recover_stale_running_delete_commands(120)
        .await
        .expect("stale recovery should succeed");
    assert_eq!(recovered, 1);

    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, started_at
         FROM download_queue_commands
         WHERE id IN (?, ?)
         ORDER BY id",
    )
    .bind(&fresh.id)
    .bind(&stale.id)
    .fetch_all(&services.pool)
    .await
    .expect("load delete rows after stale recovery");

    assert_eq!(rows.len(), 2);
    let fresh_row = rows
        .iter()
        .find(|row| row.0 == fresh.id)
        .expect("fresh row should exist");
    assert_eq!(fresh_row.1, "running");
    assert!(
        fresh_row.2.is_some(),
        "fresh running delete should remain running"
    );
    let stale_row = rows
        .iter()
        .find(|row| row.0 == stale.id)
        .expect("stale row should exist");
    assert_eq!(stale_row, &(stale.id, "queued".to_string(), None));

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn unique_constraints_enforce_settings_and_user_permission_masks() {
    let db = std::env::temp_dir().join(format!(
        "scryer_unique_constraints_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let _ = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(db.to_string_lossy().as_ref()))
        .await
        .expect("pool should open");

    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO settings_definitions
        (id, category, scope, key_name, data_type, default_value_json, is_sensitive, validation_json, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sd-settings")
    .bind("app")
    .bind("global")
    .bind("theme")
    .bind("string")
    .bind("{}")
    .bind(0)
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert settings definition");

    sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-1")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert first settings value");

    let duplicate_setting_value = sqlx::query(
        "INSERT INTO settings_values
        (id, setting_definition_id, scope, scope_id, value_json, source, updated_by_user_id, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("sv-2")
    .bind("sd-settings")
    .bind("global")
    .bind(Option::<String>::None)
    .bind("{}",)
    .bind("seed")
    .bind(Option::<String>::None)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await;
    assert!(duplicate_setting_value.is_err());

    sqlx::query("INSERT INTO users (id, username) VALUES (?, ?)")
        .bind("user-1")
        .bind("constraint_user")
        .execute(&pool)
        .await
        .expect("insert user");

    sqlx::query(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
        VALUES (?, ?, ?)",
    )
    .bind("user-1")
    .bind(1_i64)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("insert first app permission mask");

    let duplicate_app_permission_mask = sqlx::query(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
        VALUES (?, ?, ?)",
    )
    .bind("user-1")
    .bind(1_i64)
    .bind(&now)
    .execute(&pool)
    .await;
    assert!(duplicate_app_permission_mask.is_err());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn user_crud_queries_work() {
    let db = std::env::temp_dir().join(format!(
        "scryer_user_queries_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let users = user_store(&services);

    let created = UserRepository::create(
        &users,
        scryer_domain::User {
            id: "u-1".to_string(),
            username: "editor".to_string(),
            password_hash: None,
            authorization: Default::default(),
        },
    )
    .await
    .expect("create user");

    let from_db = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("query by id")
        .expect("id should exist");
    assert_eq!(from_db.username, created.username);

    let updated =
        UserRepository::update_password_hash(&users, &created.id, "hashed-password".to_string())
            .await
            .expect("update password hash");
    assert_eq!(updated.password_hash.as_deref(), Some("hashed-password"));

    UserRepository::delete(&users, &created.id)
        .await
        .expect("delete user");
    let missing = UserRepository::get_by_id(&users, &created.id)
        .await
        .expect("query after delete");
    assert!(missing.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn sqlite_show_queries_roundtrip() {
    let db = std::env::temp_dir().join(format!(
        "scryer_show_roundtrip_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy()).await.unwrap();
    let catalog = title_store(&services);
    let shows = show_store(&services);

    let title = Title {
        id: "title-show-1".into(),
        name: "Sample Show".into(),
        facet: MediaFacet::Series,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: Utc::now(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    };
    TitleRepository::create(&catalog, title.clone())
        .await
        .expect("insert title");

    let collection = Collection {
        id: "collection-show-1".into(),
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".into(),
        label: Some("Season One".into()),
        ordered_path: None,
        narrative_order: Some("1".into()),
        first_episode_number: Some("1".into()),
        last_episode_number: Some("12".into()),
        interstitial_movie: Some(InterstitialMovieMetadata {
            tvdb_id: "12345".into(),
            name: "Test Movie".into(),
            slug: "test-movie".into(),
            year: Some(2024),
            content_status: "released".into(),
            overview: "Interstitial overview".into(),
            poster_url: "https://example.com/poster.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 97,
            sort_title: "Test Movie".into(),
            imdb_id: "tt1234567".into(),
            genres: vec!["Action".into(), "Anime".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2024-01-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("canon".into()),
            movie_form: Some("movie".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB marked special as critical to story".into()),
            placement: Some("ordered".into()),
            movie_tmdb_id: Some("99001".into()),
            movie_mal_id: Some("5001".into()),
            movie_anidb_id: None,
        }),
        specials_movies: vec![InterstitialMovieMetadata {
            tvdb_id: "67890".into(),
            name: "Recap Movie".into(),
            slug: "recap-movie".into(),
            year: Some(2014),
            content_status: "released".into(),
            overview: "Recap of the first half.".into(),
            poster_url: "https://example.com/recap.jpg".into(),
            language: "eng".into(),
            runtime_minutes: 90,
            sort_title: "Recap Movie".into(),
            imdb_id: "tt7654321".into(),
            genres: vec!["Action".into()],
            studio: "Studio Test".into(),
            digital_release_date: Some("2014-11-01".into()),
            association_confidence: Some("high".into()),
            continuity_status: Some("unknown".into()),
            movie_form: Some("recap".into()),
            confidence: Some("high".into()),
            signal_summary: Some("TVDB special category marks this as a recap".into()),
            placement: Some("specials".into()),
            movie_tmdb_id: None,
            movie_mal_id: None,
            movie_anidb_id: None,
        }],
        interstitial_season_episode: None,
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_collection(&shows, collection.clone())
        .await
        .expect("insert collection");

    let episode = Episode {
        id: "episode-show-1".into(),
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".into()),
        season_number: Some("1".into()),
        episode_label: Some("Pilot".into()),
        title: Some("Pilot".into()),
        air_date: None,
        duration_seconds: Some(1000),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: Some("The pilot episode.".into()),
        tvdb_id: None,
        image_url: Some("https://cdn.example.test/episode-created.jpg".into()),
        monitored: true,
        created_at: Utc::now(),
    };
    ShowRepository::create_episode(&shows, episode.clone())
        .await
        .expect("insert episode");

    let collections = ShowRepository::list_collections_for_title(&shows, &title.id)
        .await
        .expect("list collections");
    let episodes = ShowRepository::list_episodes_for_collection(&shows, &collection.id)
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    assert_eq!(
        collections[0]
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.as_str()),
        Some("Test Movie")
    );
    let loaded_collection = ShowRepository::get_collection_by_id(&shows, &collection.id)
        .await
        .expect("get collection by id")
        .expect("collection should exist");
    assert_eq!(loaded_collection.id, collection.id);
    assert_eq!(
        loaded_collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.imdb_id.as_str()),
        Some("tt1234567")
    );
    assert_eq!(loaded_collection.specials_movies.len(), 1);
    assert_eq!(
        loaded_collection.specials_movies[0].movie_form.as_deref(),
        Some("recap")
    );
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
    let loaded_episode = ShowRepository::get_episode_by_id(&shows, &episode.id)
        .await
        .expect("get episode by id")
        .expect("episode should exist");
    assert_eq!(loaded_episode.id, episode.id);
    assert_eq!(
        loaded_episode.image_url,
        Some("https://cdn.example.test/episode-created.jpg".into())
    );

    let updated_collection = ShowRepository::update_collection(
        &shows,
        &collection.id,
        CollectionUpdate {
            collection_type: Some(CollectionType::Arc),
            collection_index: Some("1.1".into()),
            label: Some("Arc One".into()),
            ordered_path: Some("arc/season".into()),
            last_episode_number: Some("12".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update collection");
    assert_eq!(updated_collection.collection_type, CollectionType::Arc);
    assert_eq!(updated_collection.collection_index, "1.1");
    assert_eq!(updated_collection.label, Some("Arc One".into()));
    assert_eq!(updated_collection.ordered_path, Some("arc/season".into()));
    assert_eq!(updated_collection.last_episode_number, Some("12".into()));

    let updated_episode = ShowRepository::update_episode(
        &shows,
        &episode.id,
        EpisodeUpdate {
            episode_type: Some(scryer_domain::EpisodeType::Special),
            episode_number: Some("E1".into()),
            season_number: Some("2".into()),
            episode_label: Some("Special".into()),
            title: Some("Pilot Special".into()),
            air_date: Some("2026-01-01".into()),
            duration_seconds: Some(2_400),
            has_multi_audio: Some(true),
            has_subtitle: Some(false),
            collection_id: Some(collection.id.clone()),
            overview: Some("Updated overview".into()),
            tvdb_id: Some("349232".into()),
            image_url: Some("https://cdn.example.test/episode-updated.jpg".into()),
            ..Default::default()
        },
    )
    .await
    .expect("update episode");
    assert_eq!(
        updated_episode.episode_type,
        scryer_domain::EpisodeType::Special
    );
    assert_eq!(updated_episode.episode_number, Some("E1".into()));
    assert_eq!(updated_episode.season_number, Some("2".into()));
    assert_eq!(updated_episode.episode_label, Some("Special".into()));
    assert_eq!(updated_episode.title, Some("Pilot Special".into()));
    assert_eq!(updated_episode.air_date, Some("2026-01-01".into()));
    assert_eq!(updated_episode.duration_seconds, Some(2_400));
    assert!(updated_episode.has_multi_audio);
    assert!(!updated_episode.has_subtitle);
    assert_eq!(
        updated_episode.image_url,
        Some("https://cdn.example.test/episode-updated.jpg".into())
    );

    let cleared_episode = ShowRepository::update_episode(
        &shows,
        &episode.id,
        EpisodeUpdate {
            clear_image_url: true,
            ..Default::default()
        },
    )
    .await
    .expect("clear episode image url");
    assert_eq!(cleared_episode.image_url, None);

    ShowRepository::delete_episode(&shows, &episode.id)
        .await
        .expect("delete episode");
    let episodes_after_delete =
        ShowRepository::list_episodes_for_collection(&shows, &collection.id)
            .await
            .expect("list episodes after delete");
    assert!(episodes_after_delete.is_empty());
    let missing_episode = ShowRepository::get_episode_by_id(&shows, &episode.id)
        .await
        .expect("get episode by id after delete");
    assert!(missing_episode.is_none());

    ShowRepository::delete_collection(&shows, &collection.id)
        .await
        .expect("delete collection");
    let collections_after_delete = ShowRepository::list_collections_for_title(&shows, &title.id)
        .await
        .expect("list collections after delete");
    assert!(collections_after_delete.is_empty());
    let missing_collection = ShowRepository::get_collection_by_id(&shows, &collection.id)
        .await
        .expect("get collection by id after delete");
    assert!(missing_collection.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_items_round_trip_and_preserve_created_at() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    let created_at = "2026-04-07T00:00:00Z".to_string();
    let updated_at = "2026-04-07T00:00:00Z".to_string();
    let item = LibraryScanUnmatchedItem {
        id: "library_scan_unmatched:test".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "session-1".to_string(),
        scan_root: "/library".to_string(),
        item_path: "/library/Unknown.Movie.2020.mkv".to_string(),
        display_name: "Unknown.Movie.2020".to_string(),
        query: "Unknown Movie".to_string(),
        year_hint: Some(2020),
        reason_code: "no_metadata_search_results".to_string(),
        error_message: None,
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie".to_string(),
            result_count: 0,
            top_results: Vec::new(),
        }],
        created_at: created_at.clone(),
        updated_at: updated_at.clone(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&item)
        .await
        .expect("insert unmatched item");

    let count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after insert");
    assert_eq!(count, 1);

    let listed = library_scan_unmatched
        .list_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
            10,
            0,
        )
        .await
        .expect("list unmatched items after insert");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].search_attempts.len(), 1);
    assert_eq!(listed[0].search_attempts[0].query, "Unknown Movie");
    assert_eq!(listed[0].created_at, created_at);

    let updated = LibraryScanUnmatchedItem {
        scan_session_id: "session-2".to_string(),
        reason_code: "no_acceptable_metadata_match".to_string(),
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Unknown Movie 2020".to_string(),
            result_count: 2,
            top_results: vec![
                "Known Movie (2019)".to_string(),
                "Known Movie 2 (2020)".to_string(),
            ],
        }],
        created_at: "2026-04-08T00:00:00Z".to_string(),
        updated_at: "2026-04-08T01:00:00Z".to_string(),
        ..item.clone()
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&updated)
        .await
        .expect("update unmatched item");

    let listed_after_update = library_scan_unmatched
        .list_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
            10,
            0,
        )
        .await
        .expect("list unmatched items after update");
    assert_eq!(listed_after_update.len(), 1);
    assert_eq!(listed_after_update[0].scan_session_id, "session-2");
    assert_eq!(
        listed_after_update[0].reason_code,
        "no_acceptable_metadata_match"
    );
    assert_eq!(listed_after_update[0].created_at, item.created_at);
    assert_eq!(listed_after_update[0].updated_at, updated.updated_at);
    assert_eq!(listed_after_update[0].search_attempts[0].result_count, 2);

    library_scan_unmatched
        .delete_library_scan_unmatched_item(&item.library_id, MediaFacet::Movie, &item.item_path)
        .await
        .expect("delete unmatched item");

    let count_after_delete = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after delete");
    assert_eq!(count_after_delete, 0);

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_upsert_heals_legacy_id_on_library_path_conflict() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_legacy_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    fn unmatched_id(input: &str) -> String {
        let hash = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, input.as_bytes());
        let hex = hash
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        format!("library_scan_unmatched:{}", &hex[..24])
    }

    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let item_path = "/library/Harbor Pals/Harbor.Pals.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
    let created_at = "2026-04-07T00:00:00Z".to_string();
    let legacy_id = unmatched_id(format!("series:{item_path}").as_str());
    let current_id = unmatched_id(format!("series:{library_id}:{item_path}").as_str());

    let legacy_item = LibraryScanUnmatchedItem {
        id: legacy_id.clone(),
        library_id: library_id.clone(),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: Some("title-harbor-pals".to_string()),
        scan_session_id: "legacy-session".to_string(),
        scan_root: "/library/Harbor Pals".to_string(),
        item_path: item_path.to_string(),
        display_name: "4f8e2c7a91b6d3e0".to_string(),
        query: "Harbor Pals".to_string(),
        year_hint: None,
        reason_code: "legacy_row".to_string(),
        error_message: None,
        search_attempts: Vec::new(),
        created_at: created_at.clone(),
        updated_at: created_at.clone(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&legacy_item)
        .await
        .expect("insert legacy unmatched item");

    let refreshed_item = LibraryScanUnmatchedItem {
        id: current_id.clone(),
        library_id: library_id.clone(),
        facet: MediaFacet::Series,
        status: PendingImportStatus::Pending,
        title_id: Some("title-harbor-pals".to_string()),
        scan_session_id: "current-session".to_string(),
        scan_root: "/library/Harbor Pals".to_string(),
        item_path: item_path.to_string(),
        display_name: "4f8e2c7a91b6d3e0".to_string(),
        query: "Harbor Pals".to_string(),
        year_hint: None,
        reason_code: "scan_refresh".to_string(),
        error_message: None,
        search_attempts: vec![LibraryScanUnmatchedSearchAttempt {
            query: "Harbor Pals".to_string(),
            result_count: 1,
            top_results: vec!["Harbor Pals".to_string()],
        }],
        created_at: "2026-04-08T00:00:00Z".to_string(),
        updated_at: "2026-04-08T01:00:00Z".to_string(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&refreshed_item)
        .await
        .expect("upsert current unmatched item over legacy row");

    let count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Series),
            Some("/library/Harbor Pals"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count unmatched items after heal");
    assert_eq!(count, 1);

    let healed = library_scan_unmatched
        .get_library_scan_unmatched_item(&current_id)
        .await
        .expect("load healed unmatched item")
        .expect("healed unmatched item should exist");
    assert_eq!(healed.id, current_id);
    assert_eq!(healed.scan_session_id, "current-session");
    assert_eq!(healed.reason_code, "scan_refresh");
    assert_eq!(healed.created_at, created_at);

    let legacy_lookup = library_scan_unmatched
        .get_library_scan_unmatched_item(&legacy_id)
        .await
        .expect("load legacy unmatched item after heal");
    assert!(legacy_lookup.is_none());

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn library_scan_unmatched_upsert_preserves_ignored_status_for_scan_refresh() {
    let db = std::env::temp_dir().join(format!(
        "scryer_scan_unmatched_status_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let library_scan_unmatched = library_scan_unmatched_store(&services);

    let ignored_item = LibraryScanUnmatchedItem {
        id: "library_scan_unmatched:ignored".to_string(),
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        facet: MediaFacet::Movie,
        status: PendingImportStatus::Ignored,
        title_id: None,
        scan_session_id: "session-1".to_string(),
        scan_root: "/library".to_string(),
        item_path: "/library/Unknown.Movie.2020.mkv".to_string(),
        display_name: "Unknown.Movie.2020".to_string(),
        query: "Unknown Movie".to_string(),
        year_hint: Some(2020),
        reason_code: "no_metadata_search_results".to_string(),
        error_message: None,
        search_attempts: vec![],
        created_at: "2026-04-07T00:00:00Z".to_string(),
        updated_at: "2026-04-07T00:00:00Z".to_string(),
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&ignored_item)
        .await
        .expect("seed ignored item");

    let scan_refresh = LibraryScanUnmatchedItem {
        status: PendingImportStatus::Pending,
        scan_session_id: "session-2".to_string(),
        updated_at: "2026-04-08T00:00:00Z".to_string(),
        ..ignored_item.clone()
    };

    library_scan_unmatched
        .upsert_library_scan_unmatched_item(&scan_refresh)
        .await
        .expect("refresh ignored item from scan");

    let pending_count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Pending),
        )
        .await
        .expect("count pending items");
    let ignored_count = library_scan_unmatched
        .count_library_scan_unmatched_items(
            Some(MediaFacet::Movie),
            Some("/library"),
            Some(PendingImportStatus::Ignored),
        )
        .await
        .expect("count ignored items");
    assert_eq!(pending_count, 0);
    assert_eq!(ignored_count, 1);

    let stored = library_scan_unmatched
        .get_library_scan_unmatched_item(&ignored_item.id)
        .await
        .expect("load stored item")
        .expect("item should still exist");
    assert_eq!(stored.status, PendingImportStatus::Ignored);
    assert_eq!(stored.scan_session_id, "session-2");

    let _ = std::fs::remove_file(db);
}
