#![recursion_limit = "256"]

mod common;

use chrono::{Duration, Utc};
use common::TestContext;
use scryer_application::{
    AcquisitionStateRepository, AppError, DownloadSourceIdentity, DownloadSourceKind,
    DownloadSubmission, DownloadSubmissionRepository, PendingReleaseRepository,
    PendingReleaseStatus, SubmissionScope, SuccessfulGrabCommit, TitleRepository,
    WantedCompleteTransition, WantedItemRepository, WantedSearchTransition, WantedStatus,
};
use scryer_domain::{MediaFacet, Title};
use sqlx::{Row, query};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a title so FK constraints are satisfied.
async fn seed_title(ctx: &TestContext, id: &str) {
    let title = Title {
        id: id.to_string(),
        name: "Test Title".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
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
    ctx.catalog.create(title).await.expect("seed title");
}

/// Insert a wanted item directly via the repo and return its ID.
async fn seed_wanted_item(
    ctx: &TestContext,
    title_id: &str,
    status: scryer_application::WantedStatus,
) -> scryer_application::WantedItem {
    let item = scryer_application::WantedItem {
        id: scryer_domain::Id::new().0,
        title_id: title_id.to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_wanted_item(&item)
        .await
        .expect("seed wanted");
    item
}

/// Insert a pending release directly via the repo.
async fn seed_pending_release(
    ctx: &TestContext,
    wanted_item_id: &str,
    title_id: &str,
    score: i32,
    delay_minutes: i64,
    status: PendingReleaseStatus,
) -> scryer_application::PendingRelease {
    let now = Utc::now();
    let delay_until = now + Duration::minutes(delay_minutes);
    let pr = scryer_application::PendingRelease {
        id: scryer_domain::Id::new().0,
        wanted_item_id: wanted_item_id.to_string(),
        title_id: title_id.to_string(),
        release_title: format!("Test.Release.Score{score}.1080p.WEB-DL"),
        release_url: Some("https://example.com/nzb/123".to_string()),
        source_kind: Some(scryer_application::DownloadSourceKind::NzbUrl),
        release_size_bytes: Some(1_500_000_000),
        release_score: score,
        scoring_log_json: None,
        indexer_source: Some("nzbgeek".to_string()),
        release_guid: Some(format!("guid-{}", scryer_domain::Id::new().0)),
        added_at: now.to_rfc3339(),
        delay_until: delay_until.to_rfc3339(),
        status,
        grabbed_at: None,
        source_password: None,
        published_at: None,
        info_hash: None,
    };
    ctx.db
        .insert_pending_release(&pr)
        .await
        .expect("seed pending release");
    pr
}

// ---------------------------------------------------------------------------
// list_pending_releases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_pending_releases_returns_only_waiting() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;
    seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        300,
        6,
        PendingReleaseStatus::Grabbed,
    )
    .await;
    seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        200,
        6,
        PendingReleaseStatus::Dismissed,
    )
    .await;

    let pending = app.list_pending_releases().await.expect("list");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].release_score, 500);
}

#[tokio::test]
async fn standby_listing_returns_only_standby_rows() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let standby = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        0,
        PendingReleaseStatus::Standby,
    )
    .await;
    seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        300,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;

    let pending = ctx
        .library_state
        .list_standby_pending_releases_for_wanted_item(&wi.id)
        .await
        .expect("standby list");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, standby.id);
}

#[tokio::test]
async fn delete_standby_for_wanted_item_leaves_waiting_rows_intact() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let standby = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        0,
        PendingReleaseStatus::Standby,
    )
    .await;
    let waiting = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        300,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;

    ctx.db
        .delete_standby_pending_releases_for_wanted_item(&wi.id)
        .await
        .expect("delete standby");

    assert!(
        ctx.library_state
            .get_pending_release(&standby.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ctx.library_state
            .get_pending_release(&waiting.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PendingReleaseStatus::Waiting
    );
}

#[tokio::test]
async fn compare_and_set_pending_release_status_claims_once() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let standby = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        0,
        PendingReleaseStatus::Standby,
    )
    .await;

    let first = ctx
        .library_state
        .compare_and_set_pending_release_status(
            &standby.id,
            PendingReleaseStatus::Standby,
            PendingReleaseStatus::Processing,
            None,
        )
        .await
        .expect("first claim");
    let second = ctx
        .library_state
        .compare_and_set_pending_release_status(
            &standby.id,
            PendingReleaseStatus::Standby,
            PendingReleaseStatus::Processing,
            None,
        )
        .await
        .expect("second claim");

    assert!(first);
    assert!(!second);
    assert_eq!(
        ctx.library_state
            .get_pending_release(&standby.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PendingReleaseStatus::Processing
    );
}

#[tokio::test]
async fn commit_successful_grab_supersedes_all_pending_siblings_for_normal_grab() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let waiting = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;
    let standby = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        400,
        0,
        PendingReleaseStatus::Standby,
    )
    .await;
    let grabbed_at = Utc::now().to_rfc3339();
    let grabbed_release = serde_json::json!({
        "title": "Best.Release.1080p.WEB-DL",
        "score": 900,
        "grabbed_at": grabbed_at.clone(),
    })
    .to_string();
    let workflow_store = scryer_infrastructure::SqliteWorkflowStore::new(&ctx.db);

    workflow_store
        .commit_successful_grab(&SuccessfulGrabCommit {
            wanted_item_id: wi.id.clone(),
            covered_wanted_item_ids: Vec::new(),
            search_count: 1,
            current_score: None,
            grabbed_release: grabbed_release.clone(),
            last_search_at: Some(grabbed_at.clone()),
            download_submission: DownloadSubmission {
                title_id: wi.title_id.clone(),
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "job-1".to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some("Best.Release.1080p.WEB-DL".to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            grabbed_pending_release_id: None,
            grabbed_at: Some(grabbed_at.clone()),
        })
        .await
        .expect("commit successful grab");

    let wanted = ctx
        .library_state
        .get_wanted_item_by_id(&wi.id)
        .await
        .expect("get wanted")
        .expect("wanted item exists");
    assert_eq!(wanted.status, scryer_application::WantedStatus::Grabbed);
    assert_eq!(wanted.search_count, 1);
    assert_eq!(wanted.next_search_at, None);
    assert_eq!(wanted.last_search_at.as_deref(), Some(grabbed_at.as_str()));
    assert_eq!(
        wanted.grabbed_release.as_deref(),
        Some(grabbed_release.as_str())
    );

    let submission = workflow_store
        .find_by_client_item_id(&DownloadSourceIdentity::new(None, "nzbget", "job-1"))
        .await
        .expect("find submission")
        .expect("submission exists");
    assert_eq!(submission.title_id, wi.title_id);
    assert_eq!(
        submission.source_title.as_deref(),
        Some("Best.Release.1080p.WEB-DL")
    );

    assert_eq!(
        ctx.library_state
            .get_pending_release(&waiting.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        ctx.library_state
            .get_pending_release(&standby.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        PendingReleaseStatus::Superseded
    );
}

#[tokio::test]
async fn commit_successful_grab_marks_selected_pending_release_grabbed() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let claimed = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;
    let sibling = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        400,
        0,
        PendingReleaseStatus::Standby,
    )
    .await;
    let grabbed_at = Utc::now().to_rfc3339();
    let workflow_store = scryer_infrastructure::SqliteWorkflowStore::new(&ctx.db);

    workflow_store
        .commit_successful_grab(&SuccessfulGrabCommit {
            wanted_item_id: wi.id.clone(),
            covered_wanted_item_ids: Vec::new(),
            search_count: 0,
            current_score: None,
            grabbed_release: serde_json::json!({
                "title": claimed.release_title,
                "score": claimed.release_score,
                "grabbed_at": grabbed_at.clone(),
                "source": "pending_release",
            })
            .to_string(),
            last_search_at: Some(grabbed_at.clone()),
            download_submission: DownloadSubmission {
                title_id: wi.title_id.clone(),
                facet: "movie".to_string(),
                download_client_id: None,
                download_client_type: "nzbget".to_string(),
                download_client_item_id: "job-2".to_string(),
                source_hint: None,
                source_kind: None,
                source_title: Some(claimed.release_title.clone()),
                request_signature: None,
                scope: SubmissionScope::Title,
            },
            grabbed_pending_release_id: Some(claimed.id.clone()),
            grabbed_at: Some(grabbed_at.clone()),
        })
        .await
        .expect("commit successful grab");

    let claimed_release = ctx
        .library_state
        .get_pending_release(&claimed.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed_release.status, PendingReleaseStatus::Grabbed);
    assert_eq!(
        claimed_release.grabbed_at.as_deref(),
        Some(grabbed_at.as_str())
    );

    let sibling_release = ctx
        .library_state
        .get_pending_release(&sibling.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sibling_release.status, PendingReleaseStatus::Superseded);
}

#[tokio::test]
async fn download_submission_roundtrips_episode_scope() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-episode-scope").await;
    let workflow_store = scryer_infrastructure::SqliteWorkflowStore::new(&ctx.db);

    workflow_store
        .record_submission(DownloadSubmission {
            title_id: "title-episode-scope".to_string(),
            facet: "series".to_string(),
            download_client_id: None,
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-episode-scope".to_string(),
            source_hint: Some("https://example.invalid/releases/episode-scope.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Episode.Scope.S01E01.1080p.WEB-DL".to_string()),
            request_signature: Some("episode-scope-signature".to_string()),
            scope: SubmissionScope::Episode {
                episode_id: "episode-1".to_string(),
            },
        })
        .await
        .expect("record submission");

    let persisted = query(
        "SELECT episode_id, collection_id FROM download_submissions WHERE download_client_item_id = ?",
    )
    .bind("job-episode-scope")
    .fetch_one(ctx.db.pool())
    .await
    .expect("load raw submission row");

    let submission = workflow_store
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            None,
            "nzbget",
            "job-episode-scope",
        ))
        .await
        .expect("find submission")
        .expect("submission exists");

    assert_eq!(submission.title_id, "title-episode-scope");
    assert_eq!(
        submission.scope,
        SubmissionScope::Episode {
            episode_id: "episode-1".to_string(),
        }
    );
    assert_eq!(
        persisted
            .try_get::<Option<String>, _>("episode_id")
            .unwrap(),
        Some("episode-1".to_string())
    );
    assert_eq!(
        persisted
            .try_get::<Option<String>, _>("collection_id")
            .unwrap(),
        None
    );
    assert_eq!(
        submission.request_signature.as_deref(),
        Some("episode-scope-signature")
    );
}

#[tokio::test]
async fn download_submission_legacy_rows_without_episode_id_still_load() {
    let ctx = TestContext::new().await;
    seed_title(&ctx, "title-legacy-scope").await;
    let workflow_store = scryer_infrastructure::SqliteWorkflowStore::new(&ctx.db);

    query(
        "INSERT INTO download_submissions
         (id, title_id, facet, download_client_type, download_client_item_id, source_title,
          source_hint, source_kind, request_signature, collection_id)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind("submission-legacy")
    .bind("title-legacy-scope")
    .bind("series")
    .bind("nzbget")
    .bind("job-legacy-scope")
    .bind("Legacy.Scope.S01E01.1080p.WEB-DL")
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .execute(ctx.db.pool())
    .await
    .expect("insert legacy submission row");

    let submission = workflow_store
        .find_by_client_item_id(&DownloadSourceIdentity::new(
            None,
            "nzbget",
            "job-legacy-scope",
        ))
        .await
        .expect("find legacy submission")
        .expect("legacy submission exists");

    assert_eq!(submission.title_id, "title-legacy-scope");
    assert_eq!(submission.scope, SubmissionScope::Title);
    assert_eq!(submission.request_signature, None);
}

#[tokio::test]
async fn list_wanted_items_does_not_duplicate_movies_across_syncs() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;

    let (first_items, first_total) = app
        .list_wanted_items(scryer_application::WantedItemsQuery {
            limit: 50,
            offset: 0,
            ..scryer_application::WantedItemsQuery::default()
        })
        .await
        .expect("first wanted list");
    assert_eq!(first_total, 1);
    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].title_id, "title-1");

    let (second_items, second_total) = app
        .list_wanted_items(scryer_application::WantedItemsQuery {
            limit: 50,
            offset: 0,
            ..scryer_application::WantedItemsQuery::default()
        })
        .await
        .expect("second wanted list");
    assert_eq!(second_total, 1);
    assert_eq!(second_items.len(), 1);
    assert_eq!(second_items[0].title_id, "title-1");
}

#[tokio::test]
async fn ensure_wanted_item_seeded_preserves_paused_status_and_existing_schedule() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", WantedStatus::Wanted).await;
    let preserved_next_search_at = (Utc::now() + Duration::hours(3)).to_rfc3339();
    let preserved_last_search_at = (Utc::now() - Duration::minutes(30)).to_rfc3339();

    ctx.library_state
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: wanted.id.clone(),
            next_search_at: Some(preserved_next_search_at.clone()),
            last_search_at: Some(preserved_last_search_at.clone()),
            search_count: 2,
            current_score: Some(90),
            grabbed_release: None,
        })
        .await
        .expect("schedule wanted item");

    app.pause_wanted_item(&wanted.id)
        .await
        .expect("pause wanted item");

    let reseed = scryer_application::WantedItem {
        id: scryer_domain::Id::new().0,
        title_id: "title-1".to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "secondary".to_string(),
        next_search_at: Some(Utc::now().to_rfc3339()),
        last_search_at: None,
        search_count: 0,
        baseline_date: Some("2024-01-02".to_string()),
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    let seeded_id = ctx
        .library_state
        .ensure_wanted_item_seeded(&reseed)
        .await
        .expect("reseed paused wanted item");
    assert_eq!(seeded_id, wanted.id);

    let fetched = ctx
        .library_state
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, WantedStatus::Paused);
    assert_eq!(fetched.next_search_at, None);
    assert_eq!(fetched.search_phase, "secondary");
    assert_eq!(fetched.baseline_date.as_deref(), Some("2024-01-02"));
}

#[tokio::test]
async fn ensure_wanted_item_seeded_preserves_existing_schedule_after_search_activity() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", WantedStatus::Wanted).await;
    let preserved_next_search_at = (Utc::now() + Duration::hours(3)).to_rfc3339();
    let preserved_last_search_at = (Utc::now() - Duration::minutes(30)).to_rfc3339();

    ctx.library_state
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: wanted.id.clone(),
            next_search_at: Some(preserved_next_search_at.clone()),
            last_search_at: Some(preserved_last_search_at),
            search_count: 2,
            current_score: Some(90),
            grabbed_release: None,
        })
        .await
        .expect("schedule wanted item");

    let reseed = scryer_application::WantedItem {
        id: scryer_domain::Id::new().0,
        title_id: "title-1".to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "secondary".to_string(),
        next_search_at: Some(Utc::now().to_rfc3339()),
        last_search_at: None,
        search_count: 0,
        baseline_date: Some("2024-01-03".to_string()),
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    ctx.library_state
        .ensure_wanted_item_seeded(&reseed)
        .await
        .expect("reseed searched wanted item");

    let fetched = ctx
        .library_state
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, WantedStatus::Wanted);
    assert_eq!(
        fetched.next_search_at.as_deref(),
        Some(preserved_next_search_at.as_str())
    );
    assert_eq!(fetched.search_phase, "secondary");
    assert_eq!(fetched.baseline_date.as_deref(), Some("2024-01-03"));
}

#[tokio::test]
async fn ensure_wanted_item_seeded_preserves_completed_status() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", WantedStatus::Wanted).await;

    ctx.library_state
        .transition_wanted_to_completed(&WantedCompleteTransition {
            id: wanted.id.clone(),
            last_search_at: Some(Utc::now().to_rfc3339()),
            search_count: 1,
            current_score: Some(120),
            grabbed_release: Some(
                serde_json::json!({
                    "title": "Completed.Release.1080p.WEB-DL",
                    "score": 120,
                })
                .to_string(),
            ),
        })
        .await
        .expect("complete wanted item");

    let reseed = scryer_application::WantedItem {
        id: scryer_domain::Id::new().0,
        title_id: "title-1".to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: Some(Utc::now().to_rfc3339()),
        last_search_at: None,
        search_count: 0,
        baseline_date: Some("2024-03-10".to_string()),
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    ctx.library_state
        .ensure_wanted_item_seeded(&reseed)
        .await
        .expect("reseed completed wanted item");

    let fetched = ctx
        .library_state
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, WantedStatus::Completed);
    assert_eq!(fetched.search_phase, "primary");
    assert_eq!(fetched.baseline_date.as_deref(), Some("2024-03-10"));
}

#[tokio::test]
async fn direct_upsert_wanted_item_still_preserves_guarded_state() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", WantedStatus::Wanted).await;
    let preserved_next_search_at = (Utc::now() + Duration::hours(2)).to_rfc3339();

    ctx.library_state
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: wanted.id.clone(),
            next_search_at: Some(preserved_next_search_at.clone()),
            last_search_at: Some(Utc::now().to_rfc3339()),
            search_count: 3,
            current_score: Some(100),
            grabbed_release: None,
        })
        .await
        .expect("schedule wanted item");

    ctx.app
        .clone()
        .pause_wanted_item(&wanted.id)
        .await
        .expect("pause wanted item");

    ctx.library_state
        .upsert_wanted_item(&scryer_application::WantedItem {
            id: scryer_domain::Id::new().0,
            title_id: "title-1".to_string(),
            title_name: Some("Test Title".to_string()),
            title_slug: None,
            title_facet: Some("movie".to_string()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "secondary".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-04-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("direct upsert wanted item");

    let fetched = ctx
        .library_state
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, WantedStatus::Paused);
    assert_eq!(fetched.next_search_at, None);
    assert_eq!(fetched.search_phase, "secondary");
    assert_eq!(fetched.baseline_date.as_deref(), Some("2024-04-01"));
}

#[tokio::test]
async fn direct_upsert_wanted_item_preserves_existing_schedule_after_search_activity() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", WantedStatus::Wanted).await;
    let preserved_next_search_at = (Utc::now() + Duration::hours(2)).to_rfc3339();

    ctx.library_state
        .schedule_wanted_item_search(&WantedSearchTransition {
            id: wanted.id.clone(),
            next_search_at: Some(preserved_next_search_at.clone()),
            last_search_at: Some(Utc::now().to_rfc3339()),
            search_count: 3,
            current_score: Some(100),
            grabbed_release: None,
        })
        .await
        .expect("schedule wanted item");

    ctx.library_state
        .upsert_wanted_item(&scryer_application::WantedItem {
            id: scryer_domain::Id::new().0,
            title_id: "title-1".to_string(),
            title_name: Some("Test Title".to_string()),
            title_slug: None,
            title_facet: Some("movie".to_string()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "secondary".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-04-02".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("direct upsert wanted item");

    let fetched = ctx
        .library_state
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, WantedStatus::Wanted);
    assert_eq!(
        fetched.next_search_at.as_deref(),
        Some(preserved_next_search_at.as_str())
    );
    assert_eq!(fetched.search_phase, "secondary");
    assert_eq!(fetched.baseline_date.as_deref(), Some("2024-04-02"));
}

// ---------------------------------------------------------------------------
// dismiss_pending_release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dismiss_sets_status_to_dismissed() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;

    let result = app.dismiss_pending_release(&pr.id).await.expect("dismiss");
    assert!(result);

    // Should no longer appear in waiting list
    let pending = app.list_pending_releases().await.unwrap();
    assert!(pending.is_empty());

    // Verify status in DB
    let fetched = ctx
        .library_state
        .get_pending_release(&pr.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, PendingReleaseStatus::Dismissed);
}

#[tokio::test]
async fn dismiss_nonexistent_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    let err = app
        .dismiss_pending_release("nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn dismiss_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Grabbed,
    )
    .await;

    let err = app.dismiss_pending_release(&pr.id).await.unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

// ---------------------------------------------------------------------------
// force_grab_pending_release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn force_grab_nonexistent_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    let err = app
        .force_grab_pending_release("nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn force_grab_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Dismissed,
    )
    .await;

    let err = app.force_grab_pending_release(&pr.id).await.unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

// ---------------------------------------------------------------------------
// process_expired_pending_releases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn process_expired_skips_when_none_expired() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    // delay_until is 6 hours from now — not expired
    seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn process_expired_marks_expired_when_wanted_item_gone() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    // Create pending release referencing a wanted item, then delete the wanted item
    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Wanted).await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        -1,
        PendingReleaseStatus::Waiting,
    )
    .await;
    // Delete the wanted item
    ctx.library_state
        .delete_wanted_items_for_title("title-1")
        .await
        .expect("delete wanted");

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);

    // PR should be marked expired
    let fetched = ctx
        .library_state
        .get_pending_release(&pr.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, PendingReleaseStatus::Expired);
}

#[tokio::test]
async fn process_expired_supersedes_when_already_grabbed() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(&ctx, "title-1", scryer_application::WantedStatus::Grabbed).await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        -1,
        PendingReleaseStatus::Waiting,
    )
    .await;

    let count = app
        .process_expired_pending_releases()
        .await
        .expect("process");
    assert_eq!(count, 0);

    // PR should be superseded (wanted item already grabbed)
    let fetched = ctx
        .library_state
        .get_pending_release(&pr.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, PendingReleaseStatus::Superseded);
}
