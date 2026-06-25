use super::support_bootstrap_fixtures::{
    TestPermissionPreset, bootstrap_with_metadata_gateway_and_titles, create_authenticated_user,
};
use crate::{
    AppError, AppResult, BulkMetadataResult, DiscoveryContextChangeType,
    DiscoveryContextChangesInput, DiscoveryContextChangesResult, DiscoveryContextIncrementalCommit,
    DiscoveryContextSnapshotAckResult, DiscoveryContextSnapshotCommit,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotStatusResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryContextSnapshotSubmitResult,
    DiscoveryDashboardResult, DiscoveryDashboardSection, DiscoveryFacetRecord, DiscoveryHomeQuery,
    DiscoveryItemRecord, DiscoveryItemsQuery, DiscoveryPendingContextChangeRecord,
    DiscoveryPublicFeedCommit, DiscoveryPublicFeedInput, DiscoveryRawPageRecord,
    DiscoveryRepository, DiscoverySectionRecord, DiscoverySnapshotFacetGroup,
    DiscoverySnapshotFacetValue, DiscoverySubmittedSubjectRecord, DiscoverySyncRunRecord,
    DiscoverySyncStateRecord, DiscoveryTitle, DomainEventRepository, JobCategory, JobKey, JobRun,
    JobRunStatus, JobSection, JobTriggerSource, MetadataGateway, MetadataSearchItem,
    MetadataSearchQuery, MovieMetadata, MultiMetadataSearchResult, RichMetadataSearchItem,
    SeriesMetadata,
};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use scryer_domain::{
    DomainEventPayload, DomainExternalIds, ExternalId, JobRunStartedEventData,
    LibraryScanStartedEventData, MediaFacet, Title, TitleContextSnapshot, TitleUpdatedEventData,
};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn discovery_sync_status_returns_state_recent_runs_and_pending_count() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("run-current".to_string()),
        last_seen_domain_event_sequence: Some(42),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery.runs.lock().await.extend([
        discovery_run_record("run-old", observed_at, "complete"),
        discovery_run_record("run-current", observed_at, "complete"),
    ]);
    discovery.pending_changes.lock().await.extend([
        discovery_pending_change_record("change-default", crate::DISCOVERY_DEFAULT_SCOPE_KEY),
        discovery_pending_change_record("change-other", "other-scope"),
    ]);

    let status = app
        .discovery_sync_status(&admin)
        .await
        .expect("discovery status should be readable");

    assert_eq!(
        status.state.last_success_generation_id.as_deref(),
        Some("run-current")
    );
    assert_eq!(status.state.last_seen_domain_event_sequence, Some(42));
    assert_eq!(status.pending_context_change_count, 1);
    assert_eq!(status.recent_runs.len(), 2);
    assert_eq!(status.recent_runs[0].id, "run-current");
}

#[tokio::test]
async fn discovery_sync_recovers_committed_unacked_snapshot_before_new_submit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("run-unacked".to_string()),
        last_public_feed_generation_id: Some("public-current".to_string()),
        last_subject_fingerprint: Some("existing-fingerprint".to_string()),
        next_context_snapshot_eligible_at: Some(observed_at + chrono::Duration::hours(24)),
        next_incremental_reload_eligible_at: Some(observed_at + chrono::Duration::hours(4)),
        next_public_feed_eligible_at: Some(observed_at + chrono::Duration::hours(24)),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    let mut run = discovery_run_record("run-unacked", observed_at, "complete");
    run.smg_request_id = Some("request-unacked".to_string());
    run.raw_ack_json = None;
    discovery.runs.lock().await.push(run);

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should recover ack");

    assert_eq!(
        gateway.ack_requests.lock().await.as_slice(),
        ["request-unacked"]
    );
    assert!(
        gateway.submitted_inputs.lock().await.is_empty(),
        "ack recovery should run before any new context submit"
    );
    let runs = discovery.runs.lock().await;
    let recovered = runs
        .iter()
        .find(|run| run.id == "run-unacked")
        .expect("unacked run should remain in ledger");
    assert_eq!(recovered.status, "complete");
    assert!(recovered.raw_ack_json.is_some());
    assert!(recovered.error_text.is_none());
}

#[tokio::test]
async fn discovery_home_and_items_use_local_rows_and_library_view_rbac() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway);
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let (_public_user, public_actor) =
        create_authenticated_user(&app, &admin, "discovery-public", "password", vec![]).await;
    let (_viewer, viewer_actor) = create_authenticated_user(
        &app,
        &admin,
        "discovery-viewer",
        "password",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();

    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("context-run".to_string()),
        last_public_feed_generation_id: Some("public-run".to_string()),
        updated_at: observed_at,
        ..DiscoverySyncStateRecord::default()
    });
    discovery
        .sections
        .lock()
        .await
        .push(discovery_section_record(
            "public-run",
            "trending",
            "TRENDING",
            "public",
        ));
    discovery
        .submitted_subjects
        .lock()
        .await
        .push(DiscoverySubmittedSubjectRecord {
            run_id: "context-run".to_string(),
            subject_key: "tmdb:movie:603".to_string(),
            title_id: Some("title-603".to_string()),
            library_facet: Some("movie".to_string()),
            title_kind: Some("movie".to_string()),
            display_title: Some("Local Example Movie".to_string()),
            external_ids_json: serde_json::json!([{"source": "tmdb", "value": "603"}]).to_string(),
            raw_subject_json: serde_json::json!({"tmdbId": 603}).to_string(),
        });
    let mut private_recommendation = discovery_item_record(
        "context-run",
        "context-run",
        None,
        "tmdb:movie:200",
        "Private Recommendation",
        "movie",
        90.0,
        &["Drama"],
        &[],
        false,
        true,
    );
    private_recommendation.matched_subject_keys_json =
        serde_json::json!(["tmdb:movie:603", "tmdb:movie:missing"]).to_string();
    private_recommendation.matched_subject_titles_json =
        serde_json::json!(["SMG should not leak this"]).to_string();
    private_recommendation.matched_subject_count = 2;
    discovery.items.lock().await.extend([
        discovery_item_record(
            "public-run",
            "public-run",
            Some("trending"),
            "tmdb:movie:100",
            "Public Movie",
            "movie",
            50.0,
            &["Drama"],
            &[],
            false,
            true,
        ),
        private_recommendation,
        discovery_item_record(
            "context-run",
            "context-run",
            None,
            "tmdb:movie:201",
            "Collection Movie",
            "movie",
            95.0,
            &["Adventure"],
            &["tmdb.collection"],
            false,
            true,
        ),
        discovery_item_record(
            "context-run",
            "context-run",
            None,
            "tmdb:movie:202",
            "Owned Movie",
            "movie",
            99.0,
            &["Drama"],
            &[],
            true,
            true,
        ),
    ]);
    discovery.facets.lock().await.push(DiscoveryFacetRecord {
        run_id: "context-run".to_string(),
        facet_name: "genre".to_string(),
        facet_value: "Drama".to_string(),
        smg_count: Some(20),
        local_count: None,
        raw_json: serde_json::json!({"value": "Drama"}).to_string(),
    });

    let public_home = app
        .discovery_home(
            &public_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: false,
                limit_per_section: 10,
            },
        )
        .await
        .expect("public discovery home should load");
    assert!(!public_home.can_view_personalized);
    assert_eq!(public_home.public_sections.len(), 1);
    assert!(public_home.personalized_sections.is_empty());
    assert!(public_home.complete_collection.is_none());
    assert!(
        public_home
            .status
            .state
            .last_success_generation_id
            .is_none()
    );

    let viewer_home = app
        .discovery_home(
            &viewer_actor,
            DiscoveryHomeQuery {
                include_public: true,
                include_personalized: true,
                include_unresolved: false,
                limit_per_section: 10,
            },
        )
        .await
        .expect("viewer discovery home should load");
    assert!(viewer_home.can_view_personalized);
    assert!(viewer_home.complete_collection.is_some());
    assert_eq!(viewer_home.facets[0].local_count, Some(1));

    let filtered = app
        .discovery_items(
            &viewer_actor,
            DiscoveryItemsQuery {
                relation_subtypes: vec!["tmdb.collection".to_string()],
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("viewer discovery items should load");
    assert_eq!(filtered.total_count, 1);
    assert_eq!(filtered.items[0].display_title, "Collection Movie");

    let matched_context = app
        .discovery_items(
            &viewer_actor,
            DiscoveryItemsQuery {
                query: Some("Private".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("viewer matched context should load");
    assert_eq!(matched_context.total_count, 1);
    assert_eq!(matched_context.items[0].matched_subject_count, 1);
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&matched_context.items[0].matched_subject_titles_json)
            .expect("matched titles should decode"),
        vec!["Local Example Movie".to_string()]
    );

    let public_items = app
        .discovery_items(
            &public_actor,
            DiscoveryItemsQuery {
                query: Some("Public".to_string()),
                ..DiscoveryItemsQuery::default()
            },
        )
        .await
        .expect("public discovery items should load");
    assert_eq!(public_items.total_count, 1);
    assert_eq!(public_items.items[0].display_title, "Public Movie");
}

#[tokio::test]
async fn discovery_sync_initial_snapshot_submits_smg_and_commits_local_generation() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let submitted_inputs = gateway.submitted_inputs.lock().await;
    assert_eq!(submitted_inputs.len(), 1);
    assert_eq!(submitted_inputs[0].subjects.len(), 1);
    assert_eq!(submitted_inputs[0].subjects[0].tmdb_id, Some(603));
    assert!(submitted_inputs[0].context_fingerprint.is_some());
    drop(submitted_inputs);

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert_eq!(gateway.ack_requests.lock().await.as_slice(), ["request-1"]);

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(commit.run.status, "complete");
    assert_eq!(commit.run.smg_request_id.as_deref(), Some("request-1"));
    assert_eq!(
        commit.state.last_success_generation_id,
        Some(commit.run.id.clone())
    );
    assert_eq!(
        commit.state.last_subject_fingerprint,
        commit.run.subject_fingerprint
    );
    assert_eq!(commit.raw_pages.len(), 1);
    assert_eq!(commit.submitted_subjects.len(), 1);
    assert_eq!(commit.submitted_subjects[0].subject_key, "tmdb:movie:603");
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].target_key, "tmdb:movie:604");
    assert_eq!(commit.facets.len(), 1);
    assert_eq!(commit.facets[0].facet_name, "genre");
    drop(commits);

    let runs = discovery.runs.lock().await;
    assert!(
        runs.iter().any(|run| run.raw_ack_json.is_some()),
        "ack payload should be written back to the run ledger"
    );
}

#[tokio::test]
async fn discovery_sync_snapshot_polling_status_resumes_existing_request_and_commits() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    {
        let mut statuses = gateway.snapshot_status_queue.lock().await;
        statuses.push_back(polling_snapshot_status("request-1", "RUNNING"));
        statuses.push_back(complete_snapshot_status("request-1"));
    }
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("first discovery sync should defer while snapshot builds");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());

    {
        let mut state = discovery.state.lock().await;
        let state = state.as_mut().expect("state should persist");
        assert!(state.inflight_context_snapshot_run_id.is_some());
        assert!(state.backoff_until.is_some());
        state.backoff_until = Some(Utc::now() - chrono::Duration::minutes(1));
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("second discovery sync should resume and commit");

    assert_eq!(
        gateway.submitted_inputs.lock().await.len(),
        1,
        "resume must not submit a second snapshot request"
    );
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1", "request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert_eq!(discovery.commits.lock().await.len(), 1);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
}

#[tokio::test]
async fn discovery_sync_snapshot_queue_full_sets_backoff_without_commit_or_pages() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.snapshot_status_override.lock().await = Some(queue_full_snapshot_status("request-1"));
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer on queue full");

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(gateway.ack_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be persisted");
    assert!(state.backoff_until.is_some());
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "deferred");
    assert_eq!(run.smg_status.as_deref(), Some("QUEUE_FULL"));
    assert_eq!(run.item_count, Some(0));
}

#[tokio::test]
async fn discovery_sync_snapshot_terminal_failure_clears_inflight_without_commit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.snapshot_status_override.lock().await = Some(failed_snapshot_status("request-1"));
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-previous".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-previous".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should handle terminal snapshot failure");

    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    assert!(gateway.page_requests.lock().await.is_empty());
    assert!(gateway.ack_requests.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(
        state.last_success_generation_id.as_deref(),
        Some("generation-previous")
    );
    assert!(state.inflight_context_snapshot_run_id.is_none());
    assert!(state.inflight_subject_fingerprint.is_none());
    assert!(state.backoff_until.is_some());

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "failed");
    assert_eq!(run.smg_status.as_deref(), Some("FAILED"));
}

#[tokio::test]
async fn discovery_sync_snapshot_page_failure_preserves_inflight_for_retry() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_snapshot_page.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer on page fetch failure");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1)]
    );
    assert!(discovery.commits.lock().await.is_empty());
    {
        let mut state = discovery.state.lock().await;
        let state = state.as_mut().expect("state should persist");
        assert!(state.inflight_context_snapshot_run_id.is_some());
        assert!(state.backoff_until.is_some());
        state.backoff_until = Some(Utc::now() - chrono::Duration::minutes(1));
    }
    *gateway.fail_snapshot_page.lock().await = false;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should retry page fetch and commit");

    assert_eq!(
        gateway.submitted_inputs.lock().await.len(),
        1,
        "page retry must reuse the accepted snapshot request"
    );
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1", "request-1"]
    );
    assert_eq!(
        gateway.page_requests.lock().await.as_slice(),
        [("request-1".to_string(), 1), ("request-1".to_string(), 1)]
    );
    assert_eq!(discovery.commits.lock().await.len(), 1);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.inflight_context_snapshot_run_id.is_none());
}

#[tokio::test]
async fn discovery_sync_ack_failure_after_commit_schedules_retry() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_ack.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should commit snapshot even if ack fails");

    assert_eq!(gateway.ack_requests.lock().await.as_slice(), ["request-1"]);
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be persisted");
    assert!(
        state.backoff_until.is_some(),
        "ack failure should schedule prompt retry"
    );
    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_snapshot")
        .expect("snapshot run should be recorded");
    assert_eq!(run.status, "warning");
    assert!(run.raw_ack_json.is_none());
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|text| text.contains("ack failed"))
    );
}

#[tokio::test]
async fn discovery_sync_initial_tick_schedules_bootstrap_quiet_before_smg_submit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should be written");
    assert!(state.bootstrap_started_at.is_some());
    assert!(state.bootstrap_quiet_until.is_some());
    assert!(state.last_success_generation_id.is_none());
}

#[tokio::test]
async fn discovery_sync_initial_snapshot_waits_for_backoff_before_resubmitting() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(due_at),
        bootstrap_quiet_until: Some(due_at),
        backoff_until: Some(Utc::now() + chrono::Duration::hours(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_incremental_reload_calls_smg_and_commits_patch() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(
        change_inputs[0].previous_context_fingerprint.as_deref(),
        Some("fingerprint-generation-1")
    );
    assert_eq!(
        change_inputs[0].context_subject_keys,
        vec!["tmdb:movie:603"]
    );
    assert_eq!(change_inputs[0].changed_subjects.len(), 1);
    drop(change_inputs);

    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_incremental");
    assert_eq!(commit.run.status, "complete");
    assert_eq!(
        commit.run.base_generation_id.as_deref(),
        Some("generation-1")
    );
    assert_eq!(commit.tombstone_target_keys, vec!["tmdb:movie:604"]);
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].source_run_kind, "context_incremental");
    assert_eq!(commit.clear_pending_through_sequence, Some(12));
    assert_eq!(commit.state.last_seen_domain_event_sequence, Some(12));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_incremental_queue_full_sets_backoff_without_commit() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.context_changes_override.lock().await = Some(queue_full_context_changes_result());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(
        change_inputs[0].previous_context_fingerprint.as_deref(),
        Some("fingerprint-generation-1")
    );
    drop(change_inputs);

    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.backoff_until.is_some());
    assert_eq!(state.dirty_since, Some(due_at));
    assert_eq!(state.dirty_reason_mask, 1);

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_incremental")
        .expect("incremental run should be recorded");
    assert_eq!(run.status, "deferred");
    assert_eq!(run.smg_status.as_deref(), Some("QUEUE_FULL"));
    assert_eq!(run.item_count, Some(0));
}

#[tokio::test]
async fn discovery_sync_incremental_transport_failure_sets_backoff_and_keeps_pending_dirty() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_context_changes.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should defer transport failure");

    assert_eq!(gateway.change_inputs.lock().await.len(), 1);
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);

    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.backoff_until.is_some());
    assert_eq!(state.dirty_since, Some(due_at));
    assert_eq!(state.dirty_reason_mask, 1);

    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "context_incremental")
        .expect("incremental run should be recorded");
    assert_eq!(run.status, "deferred");
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|error| error.contains("forced incremental failure"))
    );
}

#[tokio::test]
async fn discovery_sync_too_many_incremental_changes_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(
            crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS as i64 + 1,
        ),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..=crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS {
        let mut change = discovery_pending_change_record(
            &format!("change-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.first_seen_sequence = Some(index as i64 + 1);
        change.last_seen_sequence = Some(index as i64 + 1);
        change.first_seen_at = due_at;
        change.last_seen_at = due_at;
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(
        commit.clear_pending_through_sequence,
        Some(crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS as i64 + 1)
    );
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_rematch_without_previous_subject_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(20),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "rematched".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(20),
            last_seen_sequence: Some(20),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.kind, "context_snapshot");
    assert_eq!(commits[0].clear_pending_through_sequence, Some(20));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_rematch_resolved_key_limit_escalates_to_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    let change_count = crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS / 2 + 1;
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(change_count as i64),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..change_count {
        let current_tmdb_id = 10_000 + index as i64;
        let previous_tmdb_id = 20_000 + index as i64;
        let mut change = discovery_pending_change_record(
            &format!("change-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.subject_key = Some(format!("tmdb:movie:{current_tmdb_id}"));
        change.previous_subject_key = Some(format!("tmdb:movie:{previous_tmdb_id}"));
        change.change_type = "rematched".to_string();
        change.raw_subject_json = Some(
            serde_json::json!({
                "tmdbId": current_tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": current_tmdb_id.to_string()}]
            })
            .to_string(),
        );
        change.raw_previous_subject_json = Some(
            serde_json::json!({
                "tmdbId": previous_tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": previous_tmdb_id.to_string()}]
            })
            .to_string(),
        );
        change.first_seen_sequence = Some(index as i64 + 1);
        change.last_seen_sequence = Some(index as i64 + 1);
        change.first_seen_at = due_at;
        change.last_seen_at = due_at;
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending rematch should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(
        change_count < crate::discovery::DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS,
        "test must stay below the row-count guard"
    );
    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].run.kind, "context_snapshot");
    assert_eq!(
        commits[0].clear_pending_through_sequence,
        Some(change_count as i64)
    );
}

#[tokio::test]
async fn discovery_sync_daily_snapshot_takes_precedence_and_clears_pending_changes() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(due_at + chrono::Duration::days(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "context_snapshot");
    assert_eq!(commit.clear_pending_through_sequence, Some(12));
    assert_eq!(
        commit.state.last_success_generation_id,
        Some(commit.run.id.clone())
    );
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_public_feed_runs_while_scan_is_active_and_filters_collection_section() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        next_public_feed_eligible_at: Some(due_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(test_active_library_scan_run(due_at))
        .await;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());

    let commits = discovery.public_feed_commits.lock().await;
    assert_eq!(commits.len(), 1);
    let commit = &commits[0];
    assert_eq!(commit.run.kind, "public_feed");
    assert_eq!(commit.sections.len(), 1);
    assert_eq!(commit.sections[0].section_type, "TRENDING_NOW");
    assert_eq!(commit.items.len(), 1);
    assert_eq!(commit.items[0].source_run_kind, "public_feed");
    assert_eq!(commit.items[0].matched_subject_keys_json, "[]");
    assert_eq!(commit.items[0].matched_subject_titles_json, "[]");
    assert_eq!(commit.items[0].matched_subject_count, 0);
    assert_eq!(
        commit.state.last_public_feed_generation_id,
        Some(commit.run.id.clone())
    );
}

#[tokio::test]
async fn discovery_sync_manual_run_forces_public_feed_when_fresh() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(future_at),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_manual_noop_writes_deferred_run_when_backoff_blocks_work() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc::now() - chrono::Duration::hours(1);
    let future_at = Utc::now() + chrono::Duration::days(1);
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(future_at),
        next_incremental_reload_eligible_at: Some(future_at),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(future_at),
        backoff_until: Some(future_at),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should defer while backoff is active");

    assert!(gateway.public_feed_inputs.lock().await.is_empty());
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.public_feed_commits.lock().await.is_empty());
    let runs = discovery.runs.lock().await;
    let run = runs
        .iter()
        .find(|run| run.kind == "deferred")
        .expect("manual no-op should write deferred run");
    assert_eq!(run.status, "deferred");
    assert!(
        run.error_text
            .as_deref()
            .is_some_and(|error| error.contains("No discovery sync work is currently eligible"))
    );
}

#[tokio::test]
async fn discovery_sync_defers_smg_work_while_library_scan_is_active() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(due_at + chrono::Duration::days(1)),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");
    app.runtime
        .jobs
        .job_run_tracker
        .upsert_active_run(test_active_library_scan_run(due_at))
        .await;

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_defers_smg_work_for_projected_active_scan() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let domain_events = Arc::new(super::MockDomainEventRepo::default());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_discovery_store(discovery.clone())
            .with_domain_events(domain_events.clone())
    });
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_context_snapshot_eligible_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        dirty_since: Some(due_at),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&DiscoveryPendingContextChangeRecord {
            id: "change-1".to_string(),
            scope_key: crate::DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some("tmdb:movie:603".to_string()),
            previous_subject_key: None,
            change_type: "updated".to_string(),
            title_id: Some("title-1".to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": 603,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": "603"}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(10),
            last_seen_sequence: Some(12),
            first_seen_at: due_at,
            last_seen_at: due_at,
        })
        .await
        .expect("pending change should seed");
    let mut job_event = crate::domain_events::new_job_run_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "scan-1".to_string(),
            job_key: JobKey::LibraryScanMovies.as_str().to_string(),
            operation_type: JobKey::LibraryScanMovies.as_str().to_string(),
            trigger_source: JobTriggerSource::Manual.as_str().to_string(),
        }),
    );
    job_event.occurred_at = due_at;
    domain_events
        .append(job_event)
        .await
        .expect("scan job start event should append");
    let mut event = crate::domain_events::new_library_scan_domain_event(
        crate::domain_events::DomainEventActor::system(),
        "scan-1",
        MediaFacet::Movie,
        DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
            session_id: "scan-1".to_string(),
            library_id: Some("library".to_string()),
            mode: "full".to_string(),
        }),
    );
    event.occurred_at = due_at;
    domain_events
        .append(event)
        .await
        .expect("scan start event should append");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should evaluate");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert_eq!(discovery.pending_changes.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_catches_up_title_events_before_incremental_reload() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let domain_events = Arc::new(super::MockDomainEventRepo::default());
    let app = app.with_test_overrides(|builder| {
        builder
            .with_discovery_store(discovery.clone())
            .with_domain_events(domain_events.clone())
    });
    let due_at = Utc.timestamp_opt(0, 0).unwrap();
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(due_at),
        next_incremental_reload_eligible_at: Some(due_at),
        last_seen_domain_event_sequence: Some(0),
        updated_at: due_at,
        ..DiscoverySyncStateRecord::default()
    });

    let title = test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    );
    titles.store.lock().await.push(title.clone());

    let mut event = crate::domain_events::new_title_domain_event(
        crate::domain_events::DomainEventActor::system(),
        &title,
        DomainEventPayload::TitleUpdated(TitleUpdatedEventData {
            title: test_title_context_snapshot(&title),
        }),
    );
    event.occurred_at = due_at;
    domain_events
        .append(event)
        .await
        .expect("domain event should append");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(change_inputs[0].changed_subjects.len(), 1);
    let changed_subject = &change_inputs[0].changed_subjects[0];
    assert_eq!(changed_subject.subject.tmdb_id, Some(603));
    assert_eq!(
        changed_subject.change_type,
        DiscoveryContextChangeType::Updated
    );
    drop(change_inputs);

    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(1));
    assert_eq!(commits[0].state.last_seen_domain_event_sequence, Some(1));
    drop(commits);

    assert!(discovery.pending_changes.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_incremental_success_preserves_dirty_when_newer_sequence_seen() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now),
        next_context_snapshot_eligible_at: Some(now + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(20),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    change.first_seen_at = now - chrono::Duration::hours(1);
    change.last_seen_at = now - chrono::Duration::hours(1);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert_eq!(gateway.change_inputs.lock().await.len(), 1);
    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(12));
    assert_eq!(commits[0].state.dirty_since, Some(now));
    assert_eq!(commits[0].state.dirty_reason_mask, 1);
    assert_eq!(commits[0].state.last_seen_domain_event_sequence, Some(20));
}

#[tokio::test]
async fn discovery_sync_snapshot_dirty_clear_requires_inflight_fingerprint_match() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-previous".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-previous".to_string()),
        inflight_context_snapshot_run_id: Some("run-inflight".to_string()),
        inflight_subject_fingerprint: Some("fingerprint-stale".to_string()),
        inflight_domain_event_sequence: Some(10),
        dirty_since: Some(now),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(10),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });
    let mut run = discovery_run_record("run-inflight", now, "deferred");
    run.smg_request_id = Some("request-1".to_string());
    run.subject_fingerprint = Some("fingerprint-stale".to_string());
    run.completed_at = None;
    discovery.runs.lock().await.push(run);

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should resume and commit");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert_eq!(
        gateway.status_requests.lock().await.as_slice(),
        ["request-1"]
    );
    let commits = discovery.commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(10));
    assert_eq!(commits[0].state.dirty_since, Some(now));
    assert_eq!(commits[0].state.dirty_reason_mask, 1);
    assert!(commits[0].state.inflight_context_snapshot_run_id.is_none());
}

#[tokio::test]
async fn discovery_sync_unchanged_fingerprint_clears_pending_without_smg() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, _titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    let fingerprint = crate::discovery::build_discovery_library_context(
        &[],
        crate::discovery::DiscoveryContextDefaults::default(),
    )
    .fingerprint;
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some(fingerprint),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(now),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    change.first_seen_at = now - chrono::Duration::hours(1);
    change.last_seen_at = now - chrono::Duration::hours(1);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should clean unchanged dirty state");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
    assert!(discovery.pending_changes.lock().await.is_empty());
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert!(state.dirty_since.is_none());
    assert_eq!(state.dirty_reason_mask, 0);
    assert_eq!(state.last_seen_domain_event_sequence, Some(12));
}

#[tokio::test]
async fn discovery_sync_reads_more_than_1000_pending_rows_for_incremental_eligibility() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(now + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(1_001),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    for index in 0..1_001 {
        let mut change = discovery_pending_change_record(
            &format!("change-shared-{index}"),
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        );
        change.subject_key = Some("tmdb:movie:603".to_string());
        change.raw_subject_json = Some(
            serde_json::json!({
                "tmdbId": 603,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": "603"}]
            })
            .to_string(),
        );
        change.first_seen_sequence = Some(index + 1);
        change.last_seen_sequence = Some(index + 1);
        change.first_seen_at = now - chrono::Duration::hours(1);
        change.last_seen_at = now - chrono::Duration::hours(1);
        discovery
            .upsert_pending_discovery_context_change(&change)
            .await
            .expect("pending change should seed");
    }

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("discovery sync should run");

    assert!(gateway.submitted_inputs.lock().await.is_empty());
    let change_inputs = gateway.change_inputs.lock().await;
    assert_eq!(change_inputs.len(), 1);
    assert_eq!(change_inputs[0].changed_subjects.len(), 1_001);
    drop(change_inputs);
    let commits = discovery.incremental_commits.lock().await;
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].clear_pending_through_sequence, Some(1_001));
}

#[tokio::test]
async fn discovery_sync_transient_backoff_escalates_and_resets_after_success() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    *gateway.fail_context_changes.lock().await = true;
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let t0 = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(t0));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(t0 - chrono::Duration::hours(1)),
        next_context_snapshot_eligible_at: Some(t0 + chrono::Duration::days(1)),
        next_incremental_reload_eligible_at: Some(t0),
        dirty_since: Some(t0 - chrono::Duration::hours(1)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(t0 + chrono::Duration::days(1)),
        updated_at: t0,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    let mut change =
        discovery_pending_change_record("change-1", crate::DISCOVERY_DEFAULT_SCOPE_KEY);
    change.first_seen_sequence = Some(10);
    change.last_seen_sequence = Some(12);
    discovery
        .upsert_pending_discovery_context_change(&change)
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("first transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 1);
    assert_eq!(
        state.backoff_until,
        Some(t0 + chrono::Duration::minutes(15))
    );

    let t1 = t0 + chrono::Duration::minutes(31);
    app.runtime.environment.set_fixed_now_for_tests(Some(t1));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("second transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 2);
    assert_eq!(state.backoff_until, Some(t1 + chrono::Duration::hours(1)));

    let t2 = t1 + chrono::Duration::minutes(61);
    app.runtime.environment.set_fixed_now_for_tests(Some(t2));
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("third transport failure should defer");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 3);
    assert_eq!(state.backoff_until, Some(t2 + chrono::Duration::hours(6)));

    let t3 = t2 + chrono::Duration::hours(6) + chrono::Duration::minutes(1);
    app.runtime.environment.set_fixed_now_for_tests(Some(t3));
    *gateway.fail_context_changes.lock().await = false;
    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::ScheduledInterval)
        .await
        .expect("successful incremental should reset transient failure count");
    let state = discovery
        .state
        .lock()
        .await
        .clone()
        .expect("state should persist");
    assert_eq!(state.transient_failure_count, 0);
    assert!(state.backoff_until.is_none());
    assert_eq!(discovery.incremental_commits.lock().await.len(), 1);
}

#[tokio::test]
async fn discovery_sync_manual_context_cooldown_defers_context_but_allows_public_feed() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        last_success_generation_id: Some("generation-1".to_string()),
        last_subject_fingerprint: Some("fingerprint-generation-1".to_string()),
        last_context_snapshot_completed_at: Some(now - chrono::Duration::minutes(5)),
        next_context_snapshot_eligible_at: Some(now),
        next_incremental_reload_eligible_at: Some(now),
        dirty_since: Some(now - chrono::Duration::minutes(5)),
        dirty_reason_mask: 1,
        last_seen_domain_event_sequence: Some(12),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));
    discovery
        .upsert_pending_discovery_context_change(&discovery_pending_change_record(
            "change-1",
            crate::DISCOVERY_DEFAULT_SCOPE_KEY,
        ))
        .await
        .expect("pending change should seed");

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should evaluate");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert!(gateway.submitted_inputs.lock().await.is_empty());
    assert!(gateway.change_inputs.lock().await.is_empty());
    assert_eq!(discovery.public_feed_commits.lock().await.len(), 1);
    assert!(discovery.commits.lock().await.is_empty());
    assert!(discovery.incremental_commits.lock().await.is_empty());
}

#[tokio::test]
async fn discovery_sync_manual_context_cooldown_allows_first_snapshot() {
    let gateway = Arc::new(SnapshotMetadataGateway::default());
    let (app, _admin, titles) = bootstrap_with_metadata_gateway_and_titles(gateway.clone());
    let discovery = Arc::new(RecordingDiscoveryRepository::default());
    let app = app.with_test_overrides(|builder| builder.with_discovery_store(discovery.clone()));
    let now = Utc.timestamp_opt(10_000, 0).unwrap();
    app.runtime.environment.set_fixed_now_for_tests(Some(now));
    *discovery.state.lock().await = Some(DiscoverySyncStateRecord {
        bootstrap_started_at: Some(now - chrono::Duration::minutes(20)),
        bootstrap_quiet_until: Some(now - chrono::Duration::minutes(1)),
        last_public_feed_generation_id: Some("public-1".to_string()),
        next_public_feed_eligible_at: Some(now + chrono::Duration::days(1)),
        updated_at: now,
        ..DiscoverySyncStateRecord::default()
    });

    titles.store.lock().await.push(test_title(
        "title-1",
        "The Example Movie",
        MediaFacet::Movie,
        vec![("tmdb_movie", "603")],
    ));

    app.run_scheduled_job_now(JobKey::DiscoverySync, JobTriggerSource::Manual)
        .await
        .expect("manual discovery sync should submit first snapshot");

    assert_eq!(gateway.public_feed_inputs.lock().await.len(), 1);
    assert_eq!(gateway.submitted_inputs.lock().await.len(), 1);
    assert_eq!(discovery.commits.lock().await.len(), 1);
}

#[derive(Default)]
struct SnapshotMetadataGateway {
    submitted_inputs: Mutex<Vec<DiscoveryContextSnapshotSubmitInput>>,
    change_inputs: Mutex<Vec<DiscoveryContextChangesInput>>,
    public_feed_inputs: Mutex<Vec<DiscoveryPublicFeedInput>>,
    status_requests: Mutex<Vec<String>>,
    page_requests: Mutex<Vec<(String, i32)>>,
    ack_requests: Mutex<Vec<String>>,
    fail_ack: Mutex<bool>,
    snapshot_status_override: Mutex<Option<DiscoveryContextSnapshotStatusResult>>,
    snapshot_status_queue: Mutex<VecDeque<DiscoveryContextSnapshotStatusResult>>,
    fail_snapshot_page: Mutex<bool>,
    context_changes_override: Mutex<Option<DiscoveryContextChangesResult>>,
    fail_context_changes: Mutex<bool>,
}

#[async_trait]
impl MetadataGateway for SnapshotMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(unused_gateway_call())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(unused_gateway_call())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(unused_gateway_call())
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(unused_gateway_call())
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Err(unused_gateway_call())
    }

    async fn discover_public_feed(
        &self,
        input: &DiscoveryPublicFeedInput,
    ) -> AppResult<DiscoveryDashboardResult> {
        self.public_feed_inputs.lock().await.push(input.clone());
        Ok(DiscoveryDashboardResult {
            subject_keys: Vec::new(),
            generated_at: "2026-06-25T00:00:04Z".to_string(),
            sections: vec![
                DiscoveryDashboardSection {
                    section_id: "trending_now".to_string(),
                    section_type: "TRENDING_NOW".to_string(),
                    title: "Trending Now".to_string(),
                    source_signals: vec!["popular".to_string()],
                    facets: Vec::new(),
                    items: vec![test_discovery_title()],
                },
                DiscoveryDashboardSection {
                    section_id: "collection".to_string(),
                    section_type: "COMPLETE_THE_COLLECTION".to_string(),
                    title: "Complete the Collection".to_string(),
                    source_signals: Vec::new(),
                    facets: Vec::new(),
                    items: vec![test_discovery_title()],
                },
            ],
        })
    }

    async fn submit_discovery_context_snapshot(
        &self,
        input: &DiscoveryContextSnapshotSubmitInput,
    ) -> AppResult<DiscoveryContextSnapshotSubmitResult> {
        self.submitted_inputs.lock().await.push(input.clone());
        Ok(DiscoveryContextSnapshotSubmitResult {
            request_id: Some("request-1".to_string()),
            status: "ACCEPTED".to_string(),
            subject_count: input.subjects.len() as i32,
            retry_after_seconds: 1,
            expires_at: "2026-06-25T00:00:00Z".to_string(),
        })
    }

    async fn discovery_context_snapshot_status(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotStatusResult> {
        self.status_requests
            .lock()
            .await
            .push(request_id.to_string());
        if let Some(result) = self.snapshot_status_queue.lock().await.pop_front() {
            return Ok(result);
        }
        if let Some(result) = self.snapshot_status_override.lock().await.clone() {
            return Ok(result);
        }
        Ok(DiscoveryContextSnapshotStatusResult {
            request_id: request_id.to_string(),
            status: "COMPLETE".to_string(),
            phase: "complete".to_string(),
            subject_count: 1,
            item_count: 1,
            page_count: 1,
            facet_count: 1,
            lazy_hydration_queued_count: 0,
            lazy_hydration_sources: Vec::new(),
            discovery_index_watermark: "watermark-1".to_string(),
            retry_after_seconds: 1,
            created_at: "2026-06-25T00:00:00Z".to_string(),
            started_at: "2026-06-25T00:00:00Z".to_string(),
            completed_at: "2026-06-25T00:00:01Z".to_string(),
            expires_at: "2026-06-26T00:00:00Z".to_string(),
            last_error: String::new(),
        })
    }

    async fn discovery_context_snapshot_page(
        &self,
        request_id: &str,
        page: i32,
    ) -> AppResult<DiscoveryContextSnapshotPageResult> {
        self.page_requests
            .lock()
            .await
            .push((request_id.to_string(), page));
        if *self.fail_snapshot_page.lock().await {
            return Err(AppError::Repository("forced page failure".to_string()));
        }
        Ok(DiscoveryContextSnapshotPageResult {
            request_id: request_id.to_string(),
            page,
            page_count: 1,
            generated_at: "2026-06-25T00:00:01Z".to_string(),
            discovery_index_watermark: "watermark-1".to_string(),
            facets: vec![DiscoverySnapshotFacetGroup {
                name: "genre".to_string(),
                values: vec![DiscoverySnapshotFacetValue {
                    value: "sci-fi".to_string(),
                    count: 1,
                }],
            }],
            items: vec![test_discovery_title()],
        })
    }

    async fn discovery_context_changes(
        &self,
        input: &DiscoveryContextChangesInput,
    ) -> AppResult<DiscoveryContextChangesResult> {
        self.change_inputs.lock().await.push(input.clone());
        if *self.fail_context_changes.lock().await {
            return Err(AppError::Repository(
                "forced incremental failure".to_string(),
            ));
        }
        if let Some(result) = self.context_changes_override.lock().await.clone() {
            return Ok(result);
        }
        Ok(DiscoveryContextChangesResult {
            status: "COMPLETE".to_string(),
            retry_after_seconds: 1,
            generated_at: "2026-06-25T00:00:03Z".to_string(),
            context_fingerprint: input.context_fingerprint.clone().unwrap_or_default(),
            previous_context_fingerprint: input
                .previous_context_fingerprint
                .clone()
                .unwrap_or_default(),
            discovery_index_watermark: "watermark-incremental".to_string(),
            context_subject_count: input.context_subject_keys.len() as i32,
            changed_subject_count: input.changed_subjects.len() as i32,
            resolved_changed_subject_keys: vec!["tmdb:movie:603".to_string()],
            removed_subject_keys: Vec::new(),
            affected_target_keys: vec!["tmdb:movie:604".to_string()],
            items: vec![test_discovery_title()],
        })
    }

    async fn acknowledge_discovery_context_snapshot(
        &self,
        request_id: &str,
    ) -> AppResult<DiscoveryContextSnapshotAckResult> {
        self.ack_requests.lock().await.push(request_id.to_string());
        if *self.fail_ack.lock().await {
            return Err(AppError::Repository("forced ack failure".to_string()));
        }
        Ok(DiscoveryContextSnapshotAckResult {
            request_id: request_id.to_string(),
            status: "EXPIRED".to_string(),
            acknowledged_at: "2026-06-25T00:00:02Z".to_string(),
        })
    }
}

#[derive(Default)]
struct RecordingDiscoveryRepository {
    state: Mutex<Option<DiscoverySyncStateRecord>>,
    runs: Mutex<Vec<DiscoverySyncRunRecord>>,
    commits: Mutex<Vec<DiscoveryContextSnapshotCommit>>,
    incremental_commits: Mutex<Vec<DiscoveryContextIncrementalCommit>>,
    public_feed_commits: Mutex<Vec<DiscoveryPublicFeedCommit>>,
    pending_changes: Mutex<Vec<DiscoveryPendingContextChangeRecord>>,
    sections: Mutex<Vec<DiscoverySectionRecord>>,
    items: Mutex<Vec<DiscoveryItemRecord>>,
    facets: Mutex<Vec<DiscoveryFacetRecord>>,
    submitted_subjects: Mutex<Vec<DiscoverySubmittedSubjectRecord>>,
}

#[async_trait]
impl DiscoveryRepository for RecordingDiscoveryRepository {
    async fn get_discovery_sync_state(
        &self,
        _scope_key: &str,
    ) -> AppResult<Option<DiscoverySyncStateRecord>> {
        Ok(self.state.lock().await.clone())
    }

    async fn upsert_discovery_sync_state(&self, state: &DiscoverySyncStateRecord) -> AppResult<()> {
        *self.state.lock().await = Some(state.clone());
        Ok(())
    }

    async fn try_acquire_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut state = self.state.lock().await;
        let mut next = state.clone().unwrap_or_default();
        next.scope_key = scope_key.to_string();
        let available = next.lease_owner_id.is_none()
            || next
                .lease_expires_at
                .is_none_or(|expires_at| expires_at <= now)
            || next.lease_owner_id.as_deref() == Some(owner_id);
        if available {
            next.lease_owner_id = Some(owner_id.to_string());
            next.lease_expires_at = Some(lease_expires_at);
            next.updated_at = now;
            *state = Some(next);
        }
        Ok(available)
    }

    async fn renew_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        lease_expires_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> AppResult<bool> {
        let mut state = self.state.lock().await;
        let Some(existing) = state.as_mut() else {
            return Ok(false);
        };
        if existing.scope_key == scope_key && existing.lease_owner_id.as_deref() == Some(owner_id) {
            existing.lease_expires_at = Some(lease_expires_at);
            existing.updated_at = now;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn release_discovery_sync_lease(
        &self,
        scope_key: &str,
        owner_id: &str,
        now: DateTime<Utc>,
    ) -> AppResult<()> {
        let mut state = self.state.lock().await;
        if let Some(existing) = state.as_mut() {
            if existing.scope_key == scope_key
                && existing.lease_owner_id.as_deref() == Some(owner_id)
            {
                existing.lease_owner_id = None;
                existing.lease_expires_at = None;
                existing.updated_at = now;
            }
        }
        Ok(())
    }

    async fn get_discovery_sync_run(&self, id: &str) -> AppResult<Option<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .rev()
            .find(|run| run.id == id)
            .cloned())
    }

    async fn upsert_discovery_sync_run(&self, run: &DiscoverySyncRunRecord) -> AppResult<()> {
        let mut runs = self.runs.lock().await;
        if let Some(existing) = runs.iter_mut().find(|existing| existing.id == run.id) {
            *existing = run.clone();
        } else {
            runs.push(run.clone());
        }
        Ok(())
    }

    async fn list_recent_discovery_sync_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .rev()
            .take(limit.clamp(1, 100) as usize)
            .cloned()
            .collect())
    }

    async fn list_unacked_discovery_context_snapshot_runs(
        &self,
        limit: i64,
    ) -> AppResult<Vec<DiscoverySyncRunRecord>> {
        Ok(self
            .runs
            .lock()
            .await
            .iter()
            .filter(|run| run.kind == "context_snapshot")
            .filter(|run| run.status == "complete" || run.status == "warning")
            .filter(|run| run.smg_request_id.is_some())
            .filter(|run| run.raw_ack_json.is_none())
            .take(limit.clamp(1, 100) as usize)
            .cloned()
            .collect())
    }

    async fn insert_discovery_raw_page(&self, _page: &DiscoveryRawPageRecord) -> AppResult<()> {
        Ok(())
    }

    async fn commit_discovery_context_snapshot(
        &self,
        commit: &DiscoveryContextSnapshotCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        self.items
            .lock()
            .await
            .retain(|item| item.run_id != commit.run.id);
        self.items.lock().await.extend(commit.items.clone());
        self.facets
            .lock()
            .await
            .retain(|facet| facet.run_id != commit.run.id);
        self.facets.lock().await.extend(commit.facets.clone());
        self.submitted_subjects
            .lock()
            .await
            .retain(|subject| subject.run_id != commit.run.id);
        self.submitted_subjects
            .lock()
            .await
            .extend(commit.submitted_subjects.clone());
        self.commits.lock().await.push(commit.clone());
        if let Some(sequence) = commit.clear_pending_through_sequence {
            self.pending_changes
                .lock()
                .await
                .retain(|change| change.last_seen_sequence.is_none_or(|seen| seen > sequence));
        }
        Ok(())
    }

    async fn commit_discovery_context_incremental(
        &self,
        commit: &DiscoveryContextIncrementalCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        let tombstoned_at = commit.run.completed_at.unwrap_or(commit.run.updated_at);
        {
            let mut items = self.items.lock().await;
            for item in items.iter_mut() {
                if commit.tombstone_target_keys.contains(&item.target_key)
                    && item.base_generation_id.as_deref()
                        == commit.run.base_generation_id.as_deref()
                    && item.tombstoned_at.is_none()
                {
                    item.tombstoned_by_run_id = Some(commit.run.id.clone());
                    item.tombstoned_at = Some(tombstoned_at);
                }
            }
            items.extend(commit.items.clone());
        }
        self.incremental_commits.lock().await.push(commit.clone());
        if let Some(sequence) = commit.clear_pending_through_sequence {
            self.pending_changes
                .lock()
                .await
                .retain(|change| change.last_seen_sequence.is_none_or(|seen| seen > sequence));
        }
        Ok(())
    }

    async fn commit_discovery_public_feed(
        &self,
        commit: &DiscoveryPublicFeedCommit,
    ) -> AppResult<()> {
        *self.state.lock().await = Some(commit.state.clone());
        self.runs.lock().await.push(commit.run.clone());
        self.sections
            .lock()
            .await
            .retain(|section| section.run_id != commit.run.id);
        self.sections.lock().await.extend(commit.sections.clone());
        self.items
            .lock()
            .await
            .retain(|item| item.run_id != commit.run.id);
        self.items.lock().await.extend(commit.items.clone());
        self.public_feed_commits.lock().await.push(commit.clone());
        Ok(())
    }

    async fn replace_discovery_submitted_subjects(
        &self,
        run_id: &str,
        subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<()> {
        self.submitted_subjects
            .lock()
            .await
            .retain(|subject| subject.run_id != run_id);
        self.submitted_subjects
            .lock()
            .await
            .extend(subjects.to_vec());
        Ok(())
    }

    async fn list_discovery_submitted_subjects(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        Ok(self
            .submitted_subjects
            .lock()
            .await
            .iter()
            .filter(|subject| subject.run_id == run_id)
            .cloned()
            .collect())
    }

    async fn upsert_pending_discovery_context_change(
        &self,
        change: &DiscoveryPendingContextChangeRecord,
    ) -> AppResult<()> {
        let mut pending_changes = self.pending_changes.lock().await;
        if let Some(existing) = pending_changes
            .iter_mut()
            .find(|existing| existing.id == change.id)
        {
            *existing = change.clone();
        } else {
            pending_changes.push(change.clone());
        }
        Ok(())
    }

    async fn get_pending_discovery_context_change(
        &self,
        id: &str,
    ) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .find(|change| change.id == id)
            .cloned())
    }

    async fn delete_pending_discovery_context_change(&self, id: &str) -> AppResult<u64> {
        let mut pending_changes = self.pending_changes.lock().await;
        let before = pending_changes.len();
        pending_changes.retain(|change| change.id != id);
        Ok((before - pending_changes.len()) as u64)
    }

    async fn list_all_pending_discovery_context_changes(
        &self,
        scope_key: &str,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .cloned()
            .collect())
    }

    async fn list_pending_discovery_context_changes(
        &self,
        scope_key: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryPendingContextChangeRecord>> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn count_pending_discovery_context_changes(&self, scope_key: &str) -> AppResult<i64> {
        Ok(self
            .pending_changes
            .lock()
            .await
            .iter()
            .filter(|change| change.scope_key == scope_key)
            .count() as i64)
    }

    async fn clear_pending_discovery_context_changes_through_sequence(
        &self,
        scope_key: &str,
        last_seen_sequence: i64,
    ) -> AppResult<u64> {
        let mut pending_changes = self.pending_changes.lock().await;
        let before = pending_changes.len();
        pending_changes.retain(|change| {
            change.scope_key != scope_key
                || change
                    .last_seen_sequence
                    .is_none_or(|seen| seen > last_seen_sequence)
        });
        Ok((before - pending_changes.len()) as u64)
    }

    async fn replace_discovery_sections(
        &self,
        run_id: &str,
        sections: &[crate::DiscoverySectionRecord],
    ) -> AppResult<()> {
        self.sections
            .lock()
            .await
            .retain(|section| section.run_id != run_id);
        self.sections.lock().await.extend(sections.to_vec());
        Ok(())
    }

    async fn replace_discovery_items(
        &self,
        run_id: &str,
        items: &[DiscoveryItemRecord],
    ) -> AppResult<()> {
        self.items.lock().await.retain(|item| item.run_id != run_id);
        self.items.lock().await.extend(items.to_vec());
        Ok(())
    }

    async fn replace_discovery_facets(
        &self,
        run_id: &str,
        facets: &[DiscoveryFacetRecord],
    ) -> AppResult<()> {
        self.facets
            .lock()
            .await
            .retain(|facet| facet.run_id != run_id);
        self.facets.lock().await.extend(facets.to_vec());
        Ok(())
    }

    async fn list_discovery_sections(
        &self,
        run_id: &str,
        surface: Option<&str>,
    ) -> AppResult<Vec<DiscoverySectionRecord>> {
        Ok(self
            .sections
            .lock()
            .await
            .iter()
            .filter(|section| section.run_id == run_id)
            .filter(|section| surface.is_none_or(|surface| section.surface == surface))
            .cloned()
            .collect())
    }

    async fn list_discovery_items_for_generation(
        &self,
        base_generation_id: &str,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| item.base_generation_id.as_deref() == Some(base_generation_id))
            .filter(|item| item.tombstoned_at.is_none())
            .cloned()
            .collect())
    }

    async fn list_discovery_facets(&self, run_id: &str) -> AppResult<Vec<DiscoveryFacetRecord>> {
        Ok(self
            .facets
            .lock()
            .await
            .iter()
            .filter(|facet| facet.run_id == run_id)
            .cloned()
            .collect())
    }

    async fn prune_discovery_history(
        &self,
        _scope_key: &str,
        _retain_successful_per_kind: usize,
        diagnostic_cutoff: DateTime<Utc>,
    ) -> AppResult<crate::DiscoveryPruneReport> {
        let mut runs = self.runs.lock().await;
        let before = runs.len();
        runs.retain(|run| {
            run.updated_at >= diagnostic_cutoff
                || run.status == "complete"
                || run.status == "warning"
                || run.status == "running"
        });
        Ok(crate::DiscoveryPruneReport {
            runs_deleted: (before - runs.len()) as u64,
        })
    }
}

fn unused_gateway_call() -> AppError {
    AppError::Repository("unexpected metadata gateway call in discovery sync test".to_string())
}

fn snapshot_status_result(
    request_id: &str,
    status: &str,
    phase: &str,
    retry_after_seconds: i32,
    page_count: i32,
    item_count: i32,
    facet_count: i32,
    last_error: &str,
) -> DiscoveryContextSnapshotStatusResult {
    DiscoveryContextSnapshotStatusResult {
        request_id: request_id.to_string(),
        status: status.to_string(),
        phase: phase.to_string(),
        subject_count: 1,
        item_count,
        page_count,
        facet_count,
        lazy_hydration_queued_count: 0,
        lazy_hydration_sources: Vec::new(),
        discovery_index_watermark: "watermark-1".to_string(),
        retry_after_seconds,
        created_at: "2026-06-25T00:00:00Z".to_string(),
        started_at: "2026-06-25T00:00:00Z".to_string(),
        completed_at: if status == "COMPLETE" {
            "2026-06-25T00:00:01Z".to_string()
        } else {
            String::new()
        },
        expires_at: "2026-06-26T00:00:00Z".to_string(),
        last_error: last_error.to_string(),
    }
}

fn polling_snapshot_status(request_id: &str, status: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(request_id, status, "building", 60, 0, 0, 0, "")
}

fn complete_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(request_id, "COMPLETE", "complete", 1, 1, 1, 1, "")
}

fn failed_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    snapshot_status_result(
        request_id,
        "FAILED",
        "failed",
        600,
        0,
        0,
        0,
        "forced snapshot failure",
    )
}

fn queue_full_snapshot_status(request_id: &str) -> DiscoveryContextSnapshotStatusResult {
    DiscoveryContextSnapshotStatusResult {
        request_id: request_id.to_string(),
        status: "QUEUE_FULL".to_string(),
        phase: "queued".to_string(),
        subject_count: 1,
        item_count: 0,
        page_count: 0,
        facet_count: 0,
        lazy_hydration_queued_count: 0,
        lazy_hydration_sources: Vec::new(),
        discovery_index_watermark: String::new(),
        retry_after_seconds: 600,
        created_at: "2026-06-25T00:00:00Z".to_string(),
        started_at: String::new(),
        completed_at: String::new(),
        expires_at: "2026-06-26T00:00:00Z".to_string(),
        last_error: String::new(),
    }
}

fn queue_full_context_changes_result() -> DiscoveryContextChangesResult {
    DiscoveryContextChangesResult {
        status: "QUEUE_FULL".to_string(),
        retry_after_seconds: 900,
        generated_at: "2026-06-25T00:00:03Z".to_string(),
        context_fingerprint: "fingerprint-current".to_string(),
        previous_context_fingerprint: "fingerprint-previous".to_string(),
        discovery_index_watermark: String::new(),
        context_subject_count: 1,
        changed_subject_count: 1,
        resolved_changed_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
        affected_target_keys: Vec::new(),
        items: Vec::new(),
    }
}

fn discovery_run_record(
    id: &str,
    observed_at: DateTime<Utc>,
    status: &str,
) -> DiscoverySyncRunRecord {
    DiscoverySyncRunRecord {
        id: id.to_string(),
        kind: "context_snapshot".to_string(),
        status: status.to_string(),
        trigger_source: "scheduled_interval".to_string(),
        region: "US".to_string(),
        language: "en".to_string(),
        subject_count: 10,
        subject_fingerprint: Some(format!("{id}-fingerprint")),
        previous_subject_fingerprint: None,
        base_generation_id: None,
        changed_subject_count: 0,
        affected_target_count: 0,
        smg_request_id: Some(format!("{id}-request")),
        smg_status: Some(status.to_string()),
        discovery_index_watermark: None,
        page_count: Some(1),
        item_count: Some(5),
        facet_count: Some(2),
        raw_submit_json: None,
        raw_changes_json: None,
        raw_final_status_json: None,
        raw_ack_json: None,
        error_text: None,
        started_at: Some(observed_at),
        completed_at: Some(observed_at),
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn discovery_pending_change_record(
    id: &str,
    scope_key: &str,
) -> DiscoveryPendingContextChangeRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    let tmdb_id = id
        .rsplit('-')
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(603);
    DiscoveryPendingContextChangeRecord {
        id: id.to_string(),
        scope_key: scope_key.to_string(),
        subject_key: Some(format!("tmdb:movie:{tmdb_id}")),
        previous_subject_key: None,
        change_type: "updated".to_string(),
        title_id: Some(id.to_string()),
        previous_title_id: None,
        library_facet: Some("movie".to_string()),
        raw_subject_json: Some(
            serde_json::json!({
                "tmdbId": tmdb_id,
                "kind": "movie",
                "facet": "movie",
                "externalIds": [{"source": "tmdb", "value": tmdb_id.to_string()}]
            })
            .to_string(),
        ),
        raw_previous_subject_json: None,
        first_seen_sequence: Some(1),
        last_seen_sequence: Some(1),
        first_seen_at: observed_at,
        last_seen_at: observed_at,
    }
}

fn discovery_section_record(
    run_id: &str,
    section_id: &str,
    section_type: &str,
    surface: &str,
) -> DiscoverySectionRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    DiscoverySectionRecord {
        id: format!("{run_id}:section:{section_id}"),
        run_id: run_id.to_string(),
        section_id: section_id.to_string(),
        section_type: section_type.to_string(),
        surface: surface.to_string(),
        title: section_id.to_string(),
        source_signals_json: "[]".to_string(),
        facets_json: "[]".to_string(),
        sort_index: 0,
        raw_json: serde_json::json!({"sectionId": section_id}).to_string(),
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    section_id: Option<&str>,
    target_key: &str,
    display_title: &str,
    target_kind: &str,
    rank_score: f64,
    genres: &[&str],
    relation_subtypes: &[&str],
    owned_in_input: bool,
    resolved: bool,
) -> DiscoveryItemRecord {
    let observed_at = Utc.timestamp_opt(1_000, 0).unwrap();
    DiscoveryItemRecord {
        id: format!("{run_id}:item:{target_key}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: if run_id == base_generation_id && run_id.starts_with("public") {
            "public_feed".to_string()
        } else {
            "context_snapshot".to_string()
        },
        section_id: section_id.map(str::to_string),
        target_key: target_key.to_string(),
        target_kind: target_kind.to_string(),
        resolved,
        resolved_title_id: None,
        display_title: display_title.to_string(),
        original_title: None,
        sort_title: Some(display_title.to_string()),
        year: Some(2026),
        poster_path: None,
        poster_url: None,
        background_url: None,
        overview: None,
        content_type: Some(target_kind.to_string()),
        genres_json: serde_json::json!(genres).to_string(),
        rating: Some(7.5),
        rating_sources_json: "[]".to_string(),
        status_tags_json: "[]".to_string(),
        source_tags_json: "[]".to_string(),
        sources_json: serde_json::json!(["smg"]).to_string(),
        best_source: Some("smg".to_string()),
        relation_types_json: "[]".to_string(),
        relation_subtypes_json: serde_json::json!(relation_subtypes).to_string(),
        chart_signals_json: "[]".to_string(),
        provider_signals_json: "[]".to_string(),
        rank_components_json: "[]".to_string(),
        source_count: Some(1),
        edge_count: Some(1),
        relation_count: Some(relation_subtypes.len() as i32),
        source_subject_count: Some(1),
        rank_score: Some(rank_score),
        matched_subject_keys_json: "[]".to_string(),
        matched_subject_titles_json: "[]".to_string(),
        matched_subject_count: 0,
        tmdb_collection_id: relation_subtypes
            .contains(&"tmdb.collection")
            .then(|| "123".to_string()),
        tmdb_collection_name: relation_subtypes
            .contains(&"tmdb.collection")
            .then(|| "Example Collection".to_string()),
        owned_in_input,
        facet_terms_json: serde_json::json!(genres).to_string(),
        context_terms_json: "[]".to_string(),
        change_subject_keys_json: "[]".to_string(),
        removed_subject_keys_json: "[]".to_string(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        raw_json: serde_json::json!({"targetKey": target_key}).to_string(),
        created_at: observed_at,
        updated_at: observed_at,
    }
}

fn test_active_library_scan_run(started_at: chrono::DateTime<Utc>) -> JobRun {
    JobRun {
        id: "active-scan-run".to_string(),
        job_key: JobKey::LibraryScanMovies,
        display_name: "Scan Movies".to_string(),
        category: JobCategory::Library,
        section: JobSection::Primary,
        status: JobRunStatus::Running,
        trigger_source: JobTriggerSource::Manual,
        started_at,
        completed_at: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        progress_json: None,
        library_scan_progress: None,
    }
}

fn test_title(id: &str, name: &str, facet: MediaFacet, external_ids: Vec<(&str, &str)>) -> Title {
    Title {
        id: id.to_string(),
        library_id: "library".to_string(),
        name: name.to_string(),
        facet,
        monitored: true,
        tags: Vec::new(),
        external_ids: external_ids
            .into_iter()
            .map(|(source, value)| ExternalId {
                source: source.to_string(),
                value: value.to_string(),
            })
            .collect(),
        root_folder_id: "root".to_string(),
        created_by: None,
        created_at: Utc.timestamp_opt(0, 0).unwrap(),
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: Vec::new(),
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: Vec::new(),
        tagged_aliases: Vec::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn test_title_context_snapshot(title: &Title) -> TitleContextSnapshot {
    TitleContextSnapshot {
        title_name: title.name.clone(),
        facet: title.facet.clone(),
        external_ids: DomainExternalIds {
            imdb_id: title.imdb_id.clone(),
            tmdb_id: test_external_id(title, &["tmdb_movie", "tmdb"]),
            tvdb_id: test_external_id(title, &["tvdb_series", "tvdb_movie", "tvdb"]),
            anidb_id: test_external_id(title, &["anidb"]),
        },
        poster_url: title.poster_url.clone(),
        year: title.year,
    }
}

fn test_external_id(title: &Title, sources: &[&str]) -> Option<String> {
    title
        .external_ids
        .iter()
        .find(|external_id| sources.iter().any(|source| external_id.source == *source))
        .map(|external_id| external_id.value.clone())
}

fn test_discovery_title() -> DiscoveryTitle {
    DiscoveryTitle {
        target_key: "tmdb:movie:604".to_string(),
        target_kind: "movie".to_string(),
        resolved: false,
        resolved_title_id: String::new(),
        display_title: "Another Example Movie".to_string(),
        original_title: String::new(),
        year: Some(2026),
        poster_path: String::new(),
        poster_url: String::new(),
        overview: "A fixture discovery title".to_string(),
        content_type: "movie".to_string(),
        genres: vec!["sci-fi".to_string()],
        rating: Some(7.5),
        rating_sources: vec!["smg".to_string()],
        status_tags: Vec::new(),
        background_url: String::new(),
        source_tags: Vec::new(),
        sources: vec!["popular".to_string()],
        relation_types: Vec::new(),
        relation_subtypes: Vec::new(),
        chart_signals: Vec::new(),
        provider_signals: Vec::new(),
        rank_components: Vec::new(),
        source_count: 1,
        edge_count: 1,
        relation_count: 0,
        source_subject_count: 1,
        rank_score: 0.8,
        best_source: "popular".to_string(),
        matched_subject_keys: vec!["tmdb:movie:603".to_string()],
        matched_subject_titles: vec!["The Example Movie".to_string()],
        matched_subject_count: 1,
        tmdb_collection_id: None,
        tmdb_collection_name: String::new(),
        owned_in_input: false,
        facet_terms: vec!["movie".to_string()],
        context_terms: Vec::new(),
        change_subject_keys: Vec::new(),
        removed_subject_keys: Vec::new(),
    }
}
