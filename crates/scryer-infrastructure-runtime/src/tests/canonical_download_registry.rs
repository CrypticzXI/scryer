use super::*;
use scryer_application::{
    ClientJobLocator, DownloadRegistryRepository, DownloadSubmissionPurpose, ObservationResolution,
    ObservedClientJob, PersistedSeedGoals, SeedGoalGrabRecord, SeedGoalResolutionSource,
};
use scryer_domain::PostImportTracking;

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
async fn legacy_submission_writes_keep_the_canonical_registry_in_sync() {
    let db = std::env::temp_dir().join(format!(
        "scryer_canonical_download_registry_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("database should migrate through 0178");
    let store = DownloadSubmissionStore::new(services.datastore());
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'Primary qBittorrent', 'qbittorrent', '{}', ?1, ?2)",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(services.pool())
    .await
    .expect("configured client should insert");

    store
        .record_seed_goals(SeedGoalGrabRecord {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            client_id: Some("client-one".to_string()),
            client_type: "qbittorrent".to_string(),
            client_item_id: "seed-item".to_string(),
            title_id: "title-seed".to_string(),
            facet: "series".to_string(),
            purpose: DownloadSubmissionPurpose::Standard,
            goals: PersistedSeedGoals {
                seeding_profile_id: None,
                seed_goal_ratio: Some(1.5),
                seed_goal_seconds: None,
                never_remove: false,
                goal_met_action: None,
                post_import_tracking: PostImportTracking::Park,
                resolution_source: SeedGoalResolutionSource::Indexer,
                info_hash: None,
            },
        })
        .await
        .expect("seed-goal stub should sync canonical rows");
    let seed_row: (String, String, String) = sqlx::query_as(
        "SELECT s.id, d.origin, b.client_name_snapshot
           FROM download_submissions s
           JOIN downloads d ON d.id = s.id
           JOIN download_client_bindings b ON b.download_id = s.id
          WHERE s.download_client_item_id = 'seed-item'",
    )
    .fetch_one(services.pool())
    .await
    .expect("seed-goal canonical rows should load");
    assert_eq!(seed_row.1, "scryer_submission");
    assert_eq!(seed_row.2, "Primary qBittorrent");

    store
        .record_submission(submission("plain-item", "title-plain"))
        .await
        .expect("plain submission should sync canonical rows");
    let plain_row: (String, String) = sqlx::query_as(
        "SELECT s.id, d.id
           FROM download_submissions s
           JOIN downloads d ON d.id = s.id
          WHERE s.download_client_item_id = 'plain-item'",
    )
    .fetch_one(services.pool())
    .await
    .expect("plain canonical row should load");
    assert_eq!(plain_row.0, plain_row.1);

    let token = "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA";
    let token_identity = DownloadSubmissionIdentity {
        download_id: Some(format!("scryer-download:{token}")),
    };
    store
        .record_submission_with_identity(
            submission("token-item", "title-token"),
            token_identity.clone(),
        )
        .await
        .expect("token submission should adopt the token UUID");
    let token_id = token.to_ascii_lowercase();
    let token_row: (String, String, String) = sqlx::query_as(
        "SELECT s.id, d.id, b.download_id
           FROM download_submissions s
           JOIN downloads d ON d.id = s.id
           JOIN download_client_bindings b ON b.download_id = s.id
          WHERE s.download_client_item_id = 'token-item'",
    )
    .fetch_one(services.pool())
    .await
    .expect("token canonical rows should load");
    assert_eq!(
        token_row,
        (token_id.clone(), token_id.clone(), token_id.clone())
    );
    let created_before: String = sqlx::query_scalar(
        "SELECT created_at FROM download_client_bindings WHERE download_id = ?1",
    )
    .bind(&token_id)
    .fetch_one(services.pool())
    .await
    .expect("token binding should load");
    store
        .record_submission_identity(
            &DownloadSourceIdentity::new(Some("client-one"), "qbittorrent", "token-item"),
            &token_identity,
        )
        .await
        .expect("the same token identity should be idempotent");
    let created_after: String = sqlx::query_scalar(
        "SELECT created_at FROM download_client_bindings WHERE download_id = ?1",
    )
    .bind(&token_id)
    .fetch_one(services.pool())
    .await
    .expect("token binding should still load");
    assert_eq!(created_after, created_before);

    store
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some("client-one"), "qbittorrent", "tracked-item"),
            "queued",
        )
        .await
        .expect("tracked-state stub should sync canonical rows");
    let tracked_origin: String = sqlx::query_scalar(
        "SELECT d.origin
           FROM download_submissions s
           JOIN downloads d ON d.id = s.id
          WHERE s.download_client_item_id = 'tracked-item'",
    )
    .fetch_one(services.pool())
    .await
    .expect("tracked-state canonical row should load");
    assert_eq!(tracked_origin, "foreign_observation");

    drop(services);
    let _ = std::fs::remove_file(db);
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

/// This is the contract that runtime dual-write == the 0178 backfill. UUIDs
/// allocated for legacy/hash rows are normalized by their immutable client
/// locator because each execution intentionally generates fresh UUIDs; token
/// UUIDs remain literal. Timestamps are deliberately excluded from snapshots.
#[tokio::test]
async fn canonical_dual_write_matches_0178_backfill_for_legacy_submission_shapes() {
    crate::spellfix::register_spellfix_auto_extension()
        .expect("spellfix auto-extension should register");
    let suffix = chrono::Utc::now().timestamp_micros();
    let backfill_db =
        std::env::temp_dir().join(format!("scryer_canonical_backfill_equivalence_{suffix}.db"));
    let replay_db =
        std::env::temp_dir().join(format!("scryer_canonical_replay_equivalence_{suffix}.db"));
    let backfill_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&sqlite_url_with_create(
            backfill_db.to_string_lossy().as_ref(),
        ))
        .await
        .expect("0177 fixture database should open");
    crate::migrations::replay_source_catalog_for_fresh_install(&backfill_pool, Some(177), true)
        .await
        .expect("0177 fixture database should migrate");
    insert_equivalence_clients(&backfill_pool).await;
    insert_0177_equivalence_submissions(&backfill_pool).await;
    crate::migrations::run_migrations(&backfill_pool, crate::types::MigrationMode::Apply)
        .await
        .expect("0178 backfill should apply");
    let backfill_snapshot = canonical_snapshot(&backfill_pool).await;

    let replay_services = SqliteServices::new(replay_db.to_string_lossy())
        .await
        .expect("latest replay database should migrate");
    insert_equivalence_clients(replay_services.pool()).await;
    let replay_store = DownloadSubmissionStore::new(replay_services.datastore());
    replay_equivalence_submissions(&replay_store).await;
    let replay_snapshot = canonical_snapshot(replay_services.pool()).await;

    assert_eq!(replay_snapshot, backfill_snapshot);

    drop(replay_services);
    drop(backfill_pool);
    let _ = std::fs::remove_file(backfill_db);
    let _ = std::fs::remove_file(replay_db);
}

async fn insert_equivalence_clients(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "INSERT INTO download_clients (
            id, name, client_type, config_json, created_at, updated_at
         ) VALUES ('client-one', 'NZBGet One', 'nzbget', '{}', ?1, ?1)",
    )
    .bind("2026-08-24T12:00:00Z")
    .execute(pool)
    .await
    .expect("configured fixture client should insert");
}

async fn insert_0177_equivalence_submissions(pool: &sqlx::SqlitePool) {
    let token = "11111111-1111-4111-8111-111111111111";
    let rows = [
        (
            "22222222-2222-4222-8222-222222222222",
            "title-token",
            "client-one",
            "nzbget",
            "native-token",
            Some(format!("scryer-download:{token}")),
        ),
        (
            "33333333-3333-4333-8333-333333333333",
            "title-hash",
            "client-one",
            "qbittorrent",
            "native-hash",
            Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        ),
        (
            "44444444-4444-4444-8444-444444444444",
            "",
            "client-one",
            "weaver",
            "native-stub",
            None,
        ),
        (
            "55555555-5555-4555-8555-555555555555",
            "title-deleted-client",
            "deleted-client",
            "qbittorrent",
            "native-deleted-client",
            None,
        ),
        (
            "66666666-6666-4666-8666-666666666666",
            "title-empty-config",
            "",
            "sabnzbd",
            "native-empty-config",
            None,
        ),
        (
            "legacy-row-id",
            "title-legacy-id",
            "client-one",
            "nzbget",
            "native-legacy-id",
            None,
        ),
    ];
    for (id, title_id, client_id, client_type, item_id, download_id) in rows {
        sqlx::query(
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id, submitted_at
             ) VALUES (?1, ?2, 'series', ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(id)
        .bind(title_id)
        .bind(client_id)
        .bind(client_type)
        .bind(item_id)
        .bind(download_id)
        .bind("2026-08-24T12:00:00Z")
        .execute(pool)
        .await
        .expect("0177 fixture submission should insert");
    }
}

async fn replay_equivalence_submissions(store: &DownloadSubmissionStore) {
    let token = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:11111111-1111-4111-8111-111111111111".to_string()),
    };
    store
        .record_submission_with_identity(
            submission_for("native-token", "title-token", Some("client-one"), "nzbget"),
            token,
        )
        .await
        .expect("token fixture should replay");
    store
        .record_submission_with_identity(
            submission_for(
                "native-hash",
                "title-hash",
                Some("client-one"),
                "qbittorrent",
            ),
            DownloadSubmissionIdentity {
                download_id: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
            },
        )
        .await
        .expect("hash fixture should replay");
    store
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some("client-one"), "weaver", "native-stub"),
            "queued",
        )
        .await
        .expect("stub fixture should replay");
    for (item_id, title_id, client_id, client_type) in [
        (
            "native-deleted-client",
            "title-deleted-client",
            Some("deleted-client"),
            "qbittorrent",
        ),
        ("native-empty-config", "title-empty-config", None, "sabnzbd"),
        (
            "native-legacy-id",
            "title-legacy-id",
            Some("client-one"),
            "nzbget",
        ),
    ] {
        store
            .record_submission(submission_for(item_id, title_id, client_id, client_type))
            .await
            .expect("legacy-shaped fixture should replay");
    }
}

fn submission_for(
    item_id: &str,
    title_id: &str,
    client_id: Option<&str>,
    client_type: &str,
) -> DownloadSubmission {
    DownloadSubmission {
        download_id: scryer_domain::download_identity::DownloadId::new(),
        download_client_id: client_id.map(str::to_string),
        download_client_type: client_type.to_string(),
        download_client_item_id: item_id.to_string(),
        title_id: title_id.to_string(),
        ..submission(item_id, title_id)
    }
}

async fn canonical_snapshot(pool: &sqlx::SqlitePool) -> Vec<CanonicalSnapshot> {
    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT d.id, d.origin, b.client_config_id, b.client_type_snapshot,
                b.client_name_snapshot, b.native_item_id, b.ended_at
           FROM downloads d
           JOIN download_client_bindings b ON b.download_id = d.id
          ORDER BY b.native_item_id, d.id",
    )
    .fetch_all(pool)
    .await
    .expect("canonical snapshot should load");
    rows.into_iter()
        .map(
            |(
                download_id,
                origin,
                client_config_id,
                client_type_snapshot,
                client_name_snapshot,
                native_item_id,
                ended_at,
            )| CanonicalSnapshot {
                download_id: normalized_snapshot_download_id(
                    &download_id,
                    native_item_id.as_deref(),
                ),
                origin,
                client_config_id,
                client_type_snapshot,
                client_name_snapshot,
                native_item_id,
                is_ended: ended_at.is_some(),
            },
        )
        .collect()
}

fn normalized_snapshot_download_id(download_id: &str, native_item_id: Option<&str>) -> String {
    match native_item_id {
        Some("native-token") => download_id.to_string(),
        Some(native_item_id) => format!("generated-for:{native_item_id}"),
        None => "generated-for:null-native-item".to_string(),
    }
}
