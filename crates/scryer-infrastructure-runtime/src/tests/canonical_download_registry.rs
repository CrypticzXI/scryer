use super::*;
use scryer_application::{
    ClientJobLocator, DownloadRegistryRepository, DownloadSubmissionPurpose,
    DownloadSubmissionRepository, ObservationResolution, ObservedClientJob,
};

#[derive(Debug, PartialEq, Eq)]
struct CanonicalSnapshot {
    download_id: String,
    origin: String,
    client_config_id: Option<String>,
    client_type_snapshot: Option<String>,
    client_name_snapshot: Option<String>,
    native_item_id: Option<String>,
    is_ended: bool,
}

fn submission(item_id: &str, title_id: &str) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        scope: SubmissionScope::Title,
        title_id: title_id.to_string(),
        facet: "series".to_string(),
        download_client_id: Some("client-one".to_string()),
        download_client_type: "qbittorrent".to_string(),
        download_client_item_id: item_id.to_string(),
        source_hint: None,
        source_provider_id: None,
        source_provider_name: None,
        source_kind: None,
        source_title: None,
        release_size_bytes: None,
        request_signature: None,
        purpose: DownloadSubmissionPurpose::Standard,
    }
}

#[tokio::test]
async fn ambiguous_submissions_attach_only_when_the_observation_is_unambiguous() {
    let db = std::env::temp_dir().join(format!(
        "scryer_ambiguous_download_observation_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through 0178");
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?1)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");
    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());

    let mut first = submission("unused-first", "title-first");
    first.source_title = Some("Paper Lantern".to_string());
    let first_id = first.download_id;
    submissions
        .record_ambiguous_submission(first)
        .await
        .expect("ambiguous submission should be stored unbound");

    let queue_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-first"),
        wire_token: None,
        observed_name: Some("  paper lantern  ".to_string()),
        observed_at: chrono::Utc::now(),
    };
    assert_eq!(
        registry
            .resolve_observation(&queue_observation)
            .await
            .expect("queue observation should attach"),
        ObservationResolution::Resolved {
            download_id: first_id,
            newly_foreign: false,
            attached: true,
        }
    );
    let completed_observation = ObservedClientJob {
        observed_name: Some("unrelated completed display name".to_string()),
        observed_at: chrono::Utc::now(),
        ..queue_observation.clone()
    };
    assert_eq!(
        registry
            .resolve_observation(&completed_observation)
            .await
            .expect("completed observation should use its locator"),
        ObservationResolution::Resolved {
            download_id: first_id,
            newly_foreign: false,
            attached: false,
        }
    );

    let mut second = submission("unused-second", "title-second");
    second.source_title = Some("Paper Lantern".to_string());
    let second_id = second.download_id;
    submissions
        .record_ambiguous_submission(second)
        .await
        .expect("second ambiguous submission should store");
    let mut third = submission("unused-third", "title-third");
    third.source_title = Some("Paper Lantern".to_string());
    submissions
        .record_ambiguous_submission(third)
        .await
        .expect("third ambiguous submission should store");

    let collision_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-collision"),
        wire_token: None,
        observed_name: Some("paper lantern".to_string()),
        observed_at: chrono::Utc::now(),
    };
    let ObservationResolution::Resolved {
        download_id: collision_id,
        newly_foreign,
        attached,
    } = registry
        .resolve_observation(&collision_observation)
        .await
        .expect("ambiguous name collision should fall through to foreign");
    assert!(newly_foreign);
    assert!(!attached);
    assert_ne!(collision_id, second_id);

    let token_observation = ObservedClientJob {
        locator: ClientJobLocator::new(Some("client-one"), "qbittorrent", "native-second"),
        wire_token: Some(second_id.to_wire()),
        observed_name: Some("does not need to match".to_string()),
        observed_at: chrono::Utc::now(),
    };
    assert_eq!(
        registry
            .resolve_observation(&token_observation)
            .await
            .expect("token should choose its own unbound submission"),
        ObservationResolution::Resolved {
            download_id: second_id,
            newly_foreign: false,
            attached: true,
        }
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn readding_a_completed_delete_locator_creates_a_new_canonical_submission() {
    let db = std::env::temp_dir().join(format!(
        "scryer_readded_download_locator_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through 0179");
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?1)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");

    let submissions = DownloadSubmissionStore::new(services.datastore());
    let registry = DownloadRegistryStore::new(services.datastore());
    let locator = ClientJobLocator::new(Some("client-one"), "qbittorrent", "reused-native-id");
    let first_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-4000-8000-000000000001")
            .expect("first fixed download id should parse");
    let second_id =
        scryer_domain::download_identity::DownloadId::parse("00000000-0000-4000-8000-000000000002")
            .expect("second fixed download id should parse");

    for download_id in [first_id, second_id] {
        let observation = ObservedClientJob {
            locator: locator.clone(),
            wire_token: Some(download_id.to_wire()),
            observed_name: Some("reused torrent".to_string()),
            observed_at: chrono::Utc::now(),
        };
        if download_id == first_id {
            registry
                .resolve_observation(&observation)
                .await
                .expect("first submission binding should resolve");
            let mut first = submission("reused-native-id", "title-first");
            first.download_id = first_id;
            submissions
                .record_submission(first)
                .await
                .expect("first submission should persist");

            // This is the store-level action the completed delete-command path
            // invokes after the client has removed the native item.
            registry
                .end_binding(&first_id)
                .await
                .expect("completed delete should end the first binding");
        } else {
            registry
                .resolve_observation(&observation)
                .await
                .expect("re-added submission binding should resolve");
            let mut second = submission("reused-native-id", "title-second");
            second.download_id = second_id;
            submissions
                .record_submission(second)
                .await
                .expect("re-added submission should persist");
        }
    }

    let submission_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM download_submissions
          WHERE download_client_id = 'client-one'
            AND download_client_type = 'qbittorrent'
            AND download_client_item_id = 'reused-native-id'",
    )
    .fetch_one(services.pool())
    .await
    .expect("coexisting submissions should count");
    assert_eq!(submission_count, 2);

    let bindings: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT download_id, ended_at
           FROM download_client_bindings
          WHERE download_id IN (?1, ?2)
          ORDER BY download_id",
    )
    .bind(first_id.to_string())
    .bind(second_id.to_string())
    .fetch_all(services.pool())
    .await
    .expect("bindings should load");
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].0, first_id.to_string());
    assert!(bindings[0].1.is_some());
    assert_eq!(bindings[1], (second_id.to_string(), None));

    let projected = submissions
        .list_for_client_items(&[locator])
        .await
        .expect("tuple projection should load");
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].download_id, second_id);

    drop(services);
    let _ = std::fs::remove_file(db);
}
