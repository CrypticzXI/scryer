use super::*;

#[tokio::test]
async fn identity_tracked_state_does_not_create_submission_row_for_live_item_id() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:blocked".to_string()),
    };
    let source_identity = DownloadSourceIdentity::new(Some("client-a"), "weaver", "10010");

    workflow_store
        .record_identity_tracked_state(
            &identity,
            Some(&source_identity),
            "import_blocked",
            Some("unresolved_download_id"),
            Some("download id observed without a matching submission"),
        )
        .await
        .expect("identity tracked state should persist");

    let tracked_state = workflow_store
        .get_identity_tracked_state(&identity, None)
        .await
        .expect("identity tracked state lookup should succeed");
    assert_eq!(tracked_state.as_deref(), Some("import_blocked"));
    let detail = workflow_store
        .get_identity_tracked_state_detail(&identity, None)
        .await
        .expect("identity tracked state detail lookup should succeed");
    assert_eq!(
        detail.as_deref(),
        Some("download id observed without a matching submission")
    );

    let submission_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM download_submissions WHERE download_client_type = ? AND download_client_item_id = ?",
    )
    .bind("weaver")
    .bind("10010")
    .fetch_one(services.pool())
    .await
    .expect("submission count should load");
    assert_eq!(submission_count, 0);

    let row = sqlx::query(
        "SELECT client_id, client_type, download_client_item_id, reason \
         FROM download_identity_states WHERE download_id = ?",
    )
    .bind("scryer-download:blocked")
    .fetch_one(services.pool())
    .await
    .expect("identity state row should exist");
    let client_id: String = row.get("client_id");
    let client_type: String = row.get("client_type");
    let item_id: String = row.get("download_client_item_id");
    let reason: String = row.get("reason");
    assert_eq!(client_id, "client-a");
    assert_eq!(client_type, "weaver");
    assert_eq!(item_id, "10010");
    assert_eq!(reason, "unresolved_download_id");

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn identity_tracked_state_scopes_client_local_download_ids_by_source_client() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_scoped_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let identity = DownloadSubmissionIdentity {
        download_id: Some("10010".to_string()),
    };
    let client_a = DownloadSourceIdentity::new(Some("client-a"), "weaver", "10010");
    let client_b = DownloadSourceIdentity::new(Some("client-b"), "weaver", "10010");

    workflow_store
        .record_identity_tracked_state(&identity, Some(&client_a), "import_blocked", None, None)
        .await
        .expect("client a state should persist");
    workflow_store
        .record_identity_tracked_state(&identity, Some(&client_b), "failed", None, None)
        .await
        .expect("client b state should persist");

    let client_a_state = workflow_store
        .get_identity_tracked_state(&identity, Some(&client_a))
        .await
        .expect("client a state lookup should succeed");
    let client_b_state = workflow_store
        .get_identity_tracked_state(&identity, Some(&client_b))
        .await
        .expect("client b state lookup should succeed");
    let unscoped_state = workflow_store
        .get_identity_tracked_state(&identity, None)
        .await
        .expect("unscoped state lookup should succeed");

    assert_eq!(client_a_state.as_deref(), Some("import_blocked"));
    assert_eq!(client_b_state.as_deref(), Some("failed"));
    assert_eq!(unscoped_state, None);

    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_identity_states WHERE download_id = ?")
            .bind("10010")
            .fetch_one(services.pool())
            .await
            .expect("identity state count should load");
    assert_eq!(row_count, 2);

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn identity_tracked_state_keeps_torrent_hash_download_ids_global() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_hash_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let identity = DownloadSubmissionIdentity {
        download_id: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
    };
    let client_a = DownloadSourceIdentity::new(Some("client-a"), "weaver", "hash-item-a");
    let client_b = DownloadSourceIdentity::new(Some("client-b"), "weaver", "hash-item-b");

    workflow_store
        .record_identity_tracked_state(&identity, Some(&client_a), "import_blocked", None, None)
        .await
        .expect("hash state should persist");

    let unscoped_state = workflow_store
        .get_identity_tracked_state(&identity, None)
        .await
        .expect("unscoped hash lookup should succeed");
    let other_client_state = workflow_store
        .get_identity_tracked_state(&identity, Some(&client_b))
        .await
        .expect("other client hash lookup should succeed");

    assert_eq!(unscoped_state.as_deref(), Some("import_blocked"));
    assert_eq!(other_client_state.as_deref(), Some("import_blocked"));

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn identity_tracked_state_ignores_client_local_download_id_without_source_client() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_unscoped_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let identity = DownloadSubmissionIdentity {
        download_id: Some("10010".to_string()),
    };
    let source_identity = DownloadSourceIdentity::new(Some("client-a"), "weaver", "10010");

    workflow_store
        .record_identity_tracked_state(&identity, None, "import_blocked", None, None)
        .await
        .expect("unscoped client-local state should be ignored");

    let scoped_state = workflow_store
        .get_identity_tracked_state(&identity, Some(&source_identity))
        .await
        .expect("scoped state lookup should succeed");
    assert_eq!(scoped_state, None);

    let row_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM download_identity_states WHERE download_id = ?")
            .bind("10010")
            .fetch_one(services.pool())
            .await
            .expect("identity state count should load");
    assert_eq!(row_count, 0);

    drop(services);
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
async fn upsert_identity_tracked_state_preserves_terminal_outcomes() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_upsert_guard_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let preserved = ["imported", "failed"];

    let imported_identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:already-imported".to_string()),
    };
    let imported_source = DownloadSourceIdentity::new(Some("client-a"), "weaver", "job-imported");
    workflow_store
        .record_identity_tracked_state(
            &imported_identity,
            Some(&imported_source),
            "imported",
            None,
            None,
        )
        .await
        .expect("imported state should persist");
    let previous = workflow_store
        .upsert_identity_tracked_state_returning_previous(
            &imported_identity,
            Some(&imported_source),
            "ignored",
            &preserved,
            None,
            None,
        )
        .await
        .expect("guarded upsert should succeed");
    assert_eq!(previous.as_deref(), Some("imported"));
    let state = workflow_store
        .get_identity_tracked_state(&imported_identity, Some(&imported_source))
        .await
        .expect("state lookup should succeed");
    assert_eq!(
        state.as_deref(),
        Some("imported"),
        "a terminal imported outcome must not be flipped to ignored"
    );

    let blocked_identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:blocked-import".to_string()),
    };
    let blocked_source = DownloadSourceIdentity::new(Some("client-a"), "weaver", "job-blocked");
    workflow_store
        .record_identity_tracked_state(
            &blocked_identity,
            Some(&blocked_source),
            "import_blocked",
            None,
            None,
        )
        .await
        .expect("blocked state should persist");
    let previous = workflow_store
        .upsert_identity_tracked_state_returning_previous(
            &blocked_identity,
            Some(&blocked_source),
            "ignored",
            &preserved,
            None,
            None,
        )
        .await
        .expect("guarded upsert should succeed");
    assert_eq!(previous.as_deref(), Some("import_blocked"));
    let state = workflow_store
        .get_identity_tracked_state(&blocked_identity, Some(&blocked_source))
        .await
        .expect("state lookup should succeed");
    assert_eq!(state.as_deref(), Some("ignored"));

    let repeat = workflow_store
        .upsert_identity_tracked_state_returning_previous(
            &blocked_identity,
            Some(&blocked_source),
            "ignored",
            &preserved,
            None,
            None,
        )
        .await
        .expect("repeated upsert should succeed");
    assert_eq!(
        repeat.as_deref(),
        Some("ignored"),
        "a repeated ignore must report the prior ignored state so no duplicate event is emitted"
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn list_identity_tracked_states_orders_latest_row_last_per_client_triple() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_list_order_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());

    // Two grabs of the same client item (a re-added torrent reuses its hash
    // as the item id) leave two identity rows behind the same client triple.
    let triple = DownloadSourceIdentity::new(Some("client-a"), "qbittorrent", "hash-1");
    workflow_store
        .record_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-download:old-grab".to_string()),
            },
            Some(&triple),
            "ignored",
            None,
            None,
        )
        .await
        .expect("old grab state should persist");
    workflow_store
        .record_identity_tracked_state(
            &DownloadSubmissionIdentity {
                download_id: Some("scryer-download:new-grab".to_string()),
            },
            Some(&triple),
            "imported",
            None,
            None,
        )
        .await
        .expect("new grab state should persist");
    sqlx::query("UPDATE download_identity_states SET updated_at = ? WHERE download_id = ?")
        .bind("2026-01-01T00:00:00Z")
        .bind("scryer-download:old-grab")
        .execute(services.pool())
        .await
        .expect("old grab timestamp should update");
    sqlx::query("UPDATE download_identity_states SET updated_at = ? WHERE download_id = ?")
        .bind("2026-02-01T00:00:00Z")
        .bind("scryer-download:new-grab")
        .execute(services.pool())
        .await
        .expect("new grab timestamp should update");

    let states = workflow_store
        .list_identity_tracked_states_for_client_items(std::slice::from_ref(&triple))
        .await
        .expect("batch state lookup should succeed");

    let triple_states = states
        .iter()
        .filter(|(identity, _)| *identity == triple)
        .map(|(_, state)| state.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        triple_states,
        vec!["ignored", "imported"],
        "rows must be ordered oldest-first so the newest grab's state wins a last-write map build"
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn canonical_tracked_state_writes_keep_the_legacy_identity_key_and_read_canonical_first() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_canonical_write_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize through migration 0178");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let identity = DownloadSubmissionIdentity {
        download_id: Some("scryer-download:token-observation".to_string()),
    };
    let source = DownloadSourceIdentity::new(Some("client-a"), "weaver", "job-10010");

    workflow_store
        .record_identity_tracked_state_for_download(
            Some(&canonical_download_id),
            &identity,
            Some(&source),
            "import_blocked",
            Some("manual_review"),
            Some("the durable detail"),
        )
        .await
        .expect("canonical tracked state should persist");

    let row = sqlx::query(
        "SELECT canonical_download_id, identity_key FROM download_identity_states WHERE download_id = ?",
    )
    .bind("scryer-download:token-observation")
    .fetch_one(services.pool())
    .await
    .expect("canonical tracked state row should exist");
    let persisted_canonical_download_id: String = row.get("canonical_download_id");
    let identity_key: String = row.get("identity_key");
    assert_eq!(
        persisted_canonical_download_id,
        canonical_download_id.to_string()
    );
    assert_eq!(identity_key, "download:scryer-download:token-observation");

    assert_eq!(
        workflow_store
            .get_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &DownloadSubmissionIdentity::default(),
                None,
            )
            .await
            .expect("canonical tracked state read should succeed")
            .as_deref(),
        Some("import_blocked")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_reason_for_download(
                Some(&canonical_download_id),
                &DownloadSubmissionIdentity::default(),
                None,
            )
            .await
            .expect("canonical tracked-state reason read should succeed")
            .as_deref(),
        Some("manual_review")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_detail_for_download(
                Some(&canonical_download_id),
                &DownloadSubmissionIdentity::default(),
                None,
            )
            .await
            .expect("canonical tracked-state detail read should succeed")
            .as_deref(),
        Some("the durable detail")
    );
    assert_eq!(
        workflow_store
            .list_identity_tracked_states_for_client_items_with_download_ids(&[(
                source.clone(),
                Some(canonical_download_id),
            )])
            .await
            .expect("canonical batch tracked-state read should succeed"),
        vec![(source, "import_blocked".to_string())]
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn canonical_read_falls_back_to_legacy_hash_tracked_state() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_legacy_hash_fallback_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize through migration 0178");
    let workflow_store = DownloadSubmissionStore::new(services.datastore());
    let canonical_download_id = scryer_domain::download_identity::DownloadId::new();
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let identity = DownloadSubmissionIdentity {
        download_id: Some(hash.to_string()),
    };
    let source = DownloadSourceIdentity::new(Some("client-a"), "qbittorrent", hash);
    let terminal_states = ["imported", "failed"];

    assert_eq!(
        workflow_store
            .upsert_identity_tracked_state_returning_previous(
                &identity,
                Some(&source),
                "ignored",
                &terminal_states,
                Some("user_ignored"),
                Some("ignore detail"),
            )
            .await
            .expect("legacy ignored state should persist"),
        None
    );

    let row = sqlx::query(
        "SELECT canonical_download_id, identity_key
         FROM download_identity_states
         WHERE download_id = ?",
    )
    .bind(hash)
    .fetch_one(services.pool())
    .await
    .expect("legacy hash tracked-state row should exist");
    let persisted_canonical_download_id: Option<String> = row.get("canonical_download_id");
    let identity_key: String = row.get("identity_key");
    assert_eq!(persisted_canonical_download_id, None);
    assert_eq!(identity_key, format!("download:{hash}"));

    assert_eq!(
        workflow_store
            .get_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first state read should fall back to hash identity")
            .as_deref(),
        Some("ignored")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_reason_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first reason read should fall back to hash identity")
            .as_deref(),
        Some("user_ignored")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_detail_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first detail read should fall back to hash identity")
            .as_deref(),
        Some("ignore detail")
    );

    assert_eq!(
        workflow_store
            .upsert_identity_tracked_state_returning_previous(
                &identity,
                Some(&source),
                "imported",
                &terminal_states,
                Some("imported_reason"),
                Some("imported detail"),
            )
            .await
            .expect("terminal state should replace ignored state")
            .as_deref(),
        Some("ignored")
    );
    assert_eq!(
        workflow_store
            .upsert_identity_tracked_state_returning_previous(
                &identity,
                Some(&source),
                "ignored",
                &terminal_states,
                Some("user_ignored"),
                Some("ignore detail"),
            )
            .await
            .expect("terminal state should remain preserved")
            .as_deref(),
        Some("imported")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first state read should preserve terminal outcome")
            .as_deref(),
        Some("imported")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_reason_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first reason read should preserve terminal outcome")
            .as_deref(),
        Some("imported_reason")
    );
    assert_eq!(
        workflow_store
            .get_identity_tracked_state_detail_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source),
            )
            .await
            .expect("canonical-first detail read should preserve terminal outcome")
            .as_deref(),
        Some("imported detail")
    );

    drop(services);
    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn restart_style_canonical_reads_match_backfilled_token_hash_and_client_scoped_rows() {
    let db = std::env::temp_dir().join(format!(
        "scryer_identity_tracked_state_canonical_restart_{}.db",
        chrono::Utc::now().timestamp_micros()
    ));
    let services = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("db should initialize through migration 0178");
    let token_id = scryer_domain::download_identity::DownloadId::new();
    let hash_id = scryer_domain::download_identity::DownloadId::new();
    let client_scoped_id = scryer_domain::download_identity::DownloadId::new();
    let hash = "0123456789abcdef0123456789abcdef01234567";
    let hash_identity_key = format!("download:{hash}");
    let rows = [
        (
            "state-token",
            "download:scryer-download:token",
            "scryer-download:token",
            token_id.to_string(),
            "imported",
            "token_reason",
            "token_detail",
            Some(("client-a", "weaver", "token-job")),
        ),
        (
            "state-hash",
            hash_identity_key.as_str(),
            hash,
            hash_id.to_string(),
            "failed",
            "hash_reason",
            "hash_detail",
            Some(("client-b", "qbittorrent", "hash-job")),
        ),
        (
            "state-client",
            "client:client-c:weaver:download:10010",
            "10010",
            client_scoped_id.to_string(),
            "import_blocked",
            "client_reason",
            "client_detail",
            Some(("client-c", "weaver", "10010")),
        ),
    ];
    for (id, identity_key, download_id, canonical_download_id, state, reason, detail, source) in
        rows
    {
        let (client_id, client_type, item_id) = source.expect("test row source");
        sqlx::query(
            "INSERT INTO download_identity_states
             (id, identity_key, canonical_download_id, download_id,
              client_id, client_type, download_client_item_id,
              tracked_state, reason, detail, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(identity_key)
        .bind(canonical_download_id)
        .bind(download_id)
        .bind(client_id)
        .bind(client_type)
        .bind(item_id)
        .bind(state)
        .bind(reason)
        .bind(detail)
        .bind("2026-01-01T00:00:00Z")
        .bind("2026-01-01T00:00:00Z")
        .execute(services.pool())
        .await
        .expect("backfilled test row should insert");
    }
    drop(services);

    let restarted = SqliteServices::new(db.to_string_lossy())
        .await
        .expect("restart should reopen migrated database");
    let workflow_store = DownloadSubmissionStore::new(restarted.datastore());
    let cases = [
        (
            token_id,
            DownloadSubmissionIdentity {
                download_id: Some("scryer-download:token".to_string()),
            },
            DownloadSourceIdentity::new(Some("client-a"), "weaver", "token-job"),
            "imported",
            "token_reason",
            "token_detail",
        ),
        (
            hash_id,
            DownloadSubmissionIdentity {
                download_id: Some(hash.to_string()),
            },
            DownloadSourceIdentity::new(Some("client-b"), "qbittorrent", "hash-job"),
            "failed",
            "hash_reason",
            "hash_detail",
        ),
        (
            client_scoped_id,
            DownloadSubmissionIdentity {
                download_id: Some("10010".to_string()),
            },
            DownloadSourceIdentity::new(Some("client-c"), "weaver", "10010"),
            "import_blocked",
            "client_reason",
            "client_detail",
        ),
    ];
    for (canonical_download_id, identity, source, state, reason, detail) in cases {
        assert_eq!(
            workflow_store
                .get_identity_tracked_state_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source),
                )
                .await
                .expect("canonical restart read should succeed")
                .as_deref(),
            Some(state)
        );
        assert_eq!(
            workflow_store
                .get_identity_tracked_state(&identity, Some(&source))
                .await
                .expect("legacy restart read should succeed")
                .as_deref(),
            Some(state)
        );
        assert_eq!(
            workflow_store
                .get_identity_tracked_state_reason_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source),
                )
                .await
                .expect("canonical restart reason read should succeed")
                .as_deref(),
            Some(reason)
        );
        assert_eq!(
            workflow_store
                .get_identity_tracked_state_detail_for_download(
                    Some(&canonical_download_id),
                    &identity,
                    Some(&source),
                )
                .await
                .expect("canonical restart detail read should succeed")
                .as_deref(),
            Some(detail)
        );
    }

    drop(restarted);
    let _ = std::fs::remove_file(db);
}
