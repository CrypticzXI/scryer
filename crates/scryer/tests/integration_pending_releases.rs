#![recursion_limit = "256"]

mod common;

use chrono::{Duration, Utc};
use common::TestContext;
use scryer_application::{
    AcquisitionScopeCompleteTransition, AcquisitionScopeStateRepository, AcquisitionScopeStatus,
    AcquisitionStateRepository, AppError, DownloadSourceIdentity, DownloadSourceKind,
    DownloadSubmission, DownloadSubmissionPurpose, DownloadSubmissionRepository, LibraryRepository,
    LibraryRootDraft, PendingReleaseRepository, PendingReleaseStatus, SubmissionScope,
    SuccessfulGrabCommit, TitleRepository, UserRepository,
};
use scryer_domain::{
    Id, Library, LibraryGrant, LibraryPermission, LibraryPermissionMask, MediaFacet, Title, User,
};
use scryer_infrastructure::{AcquisitionStore, DownloadSubmissionStore};
use sqlx::{Row, query};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a title so FK constraints are satisfied.
async fn seed_title(ctx: &TestContext, id: &str) {
    seed_title_in_library(
        ctx,
        id,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
    )
    .await;
}

async fn seed_title_in_library(ctx: &TestContext, id: &str, library_id: &str) {
    let title = Title {
        id: id.to_string(),
        name: "Test Title".to_string(),
        facet: MediaFacet::Movie,
        library_id: library_id.to_string(),
        monitored: true,
        tags: vec![],
        canonical_tags: vec![],
        external_ids: vec![],
        root_folder_id: scryer_domain::root_folder_id_for_path("/data/movies"),
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        catalog_sort_key: String::new(),
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        popularity: None,
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
    TitleRepository::create(&ctx.titles, title)
        .await
        .expect("seed title");
}

/// Insert a wanted item directly via the repo and return its ID.
async fn seed_wanted_item(
    ctx: &TestContext,
    title_id: &str,
    status: scryer_application::AcquisitionScopeStatus,
) -> scryer_application::AcquisitionScopeState {
    let item = scryer_application::AcquisitionScopeState {
        id: scryer_domain::Id::new().0,
        title_id: title_id.to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_acquisition_scope_state(&item)
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
    scryer_infrastructure::PendingReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    )
    .insert_pending_release(&pr)
    .await
    .expect("seed pending release");
    pr
}

fn admin() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        default_library: LibraryPermissionMask::from_permissions([
            LibraryPermission::View,
            LibraryPermission::ManageTitles,
        ]),
        actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
        loaded: true,
        ..Default::default()
    };
    user
}

#[tokio::test]
async fn direct_wanted_item_lookup_requires_access_to_item_library() {
    let ctx = TestContext::new().await;
    let adult_root = tempfile::tempdir().expect("adult library root");
    let now = Utc::now();
    LibraryRepository::create(
        &ctx.libraries,
        Library {
            id: "movie_adult_library".to_string(),
            facet: MediaFacet::Movie,
            name: "Adult".to_string(),
            slug: "adult".to_string(),
            is_default: false,
            roots: vec![],
            created_at: now,
            updated_at: now,
        },
        vec![LibraryRootDraft {
            path: adult_root.path().to_string_lossy().to_string(),
            is_default: true,
        }],
    )
    .await
    .expect("create adult library");

    seed_title_in_library(&ctx, "adult-title", "movie_adult_library").await;
    let mut wanted = seed_wanted_item(&ctx, "adult-title", AcquisitionScopeStatus::Wanted).await;
    wanted.library_id = Some("movie_adult_library".to_string());
    ctx.library_state
        .upsert_acquisition_scope_state(&wanted)
        .await
        .expect("update wanted library id");

    let user_id = Id::new().0;
    let viewer = UserRepository::create(
        &ctx.users,
        User {
            id: user_id.clone(),
            username: "default-viewer".to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: Default::default(),
        },
    )
    .await
    .expect("create viewer");
    LibraryRepository::set_grants_for_user(
        &ctx.libraries,
        &user_id,
        vec![LibraryGrant {
            user_id: user_id.clone(),
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
            permissions: LibraryPermissionMask::VIEW,
        }],
    )
    .await
    .expect("grant default library only");
    let viewer = ctx
        .app
        .attach_user_authorization(viewer)
        .await
        .expect("attach authorization");

    let error = ctx
        .app
        .get_wanted_item(&viewer, &wanted.id)
        .await
        .expect_err("viewer should not see another library wanted item");
    assert!(matches!(error, AppError::Unauthorized(_)));
}

// ---------------------------------------------------------------------------
// list_pending_releases
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_pending_releases_returns_only_waiting() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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

    let actor = admin();
    let pending = app.list_pending_releases(&actor).await.expect("list");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].release_score, 500);
}

#[tokio::test]
async fn standby_listing_returns_only_standby_rows() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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

    scryer_infrastructure::PendingReleaseStore::new(
        ctx.db.datastore(),
        ctx.db.encryption_key_state(),
    )
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
    let acquisition_store = AcquisitionStore::new(ctx.db.datastore());
    let download_submission_store = DownloadSubmissionStore::new(ctx.db.datastore());

    acquisition_store
        .commit_successful_grab(&SuccessfulGrabCommit {
            wanted_item_id: wi.id.clone(),
            covered_wanted_item_ids: Vec::new(),
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
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Best.Release.1080p.WEB-DL".to_string()),
                request_signature: None,
                purpose: DownloadSubmissionPurpose::Standard,
                scope: SubmissionScope::Title,
            },
            download_submission_identity: None,
            grabbed_pending_release_id: None,
            grabbed_at: Some(grabbed_at.clone()),
        })
        .await
        .expect("commit successful grab");

    let wanted = ctx
        .library_state
        .get_acquisition_scope_state_by_id(&wi.id)
        .await
        .expect("get wanted")
        .expect("wanted item exists");
    assert_eq!(
        wanted.status,
        scryer_application::AcquisitionScopeStatus::Grabbed
    );
    assert_eq!(wanted.last_search_at.as_deref(), Some(grabbed_at.as_str()));
    assert_eq!(
        wanted.grabbed_release.as_deref(),
        Some(grabbed_release.as_str())
    );

    let submission = download_submission_store
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
    let workflow_store = AcquisitionStore::new(ctx.db.datastore());

    workflow_store
        .commit_successful_grab(&SuccessfulGrabCommit {
            wanted_item_id: wi.id.clone(),
            covered_wanted_item_ids: Vec::new(),
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
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some(claimed.release_title.clone()),
                request_signature: None,
                purpose: DownloadSubmissionPurpose::Standard,
                scope: SubmissionScope::Title,
            },
            download_submission_identity: None,
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
    let workflow_store = DownloadSubmissionStore::new(ctx.db.datastore());

    workflow_store
        .record_submission(DownloadSubmission {
            title_id: "title-episode-scope".to_string(),
            facet: "series".to_string(),
            download_client_id: None,
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-episode-scope".to_string(),
            source_hint: Some("https://example.invalid/releases/episode-scope.nzb".to_string()),
            source_provider_id: None,
            source_provider_name: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Episode.Scope.S01E01.1080p.WEB-DL".to_string()),
            request_signature: Some("episode-scope-signature".to_string()),
            purpose: DownloadSubmissionPurpose::Standard,
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
    let workflow_store = DownloadSubmissionStore::new(ctx.db.datastore());

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
async fn ensure_wanted_state_row_preserves_completed_status() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", AcquisitionScopeStatus::Wanted).await;

    ctx.library_state
        .transition_acquisition_scope_to_completed(&AcquisitionScopeCompleteTransition {
            id: wanted.id.clone(),
            last_search_at: Some(Utc::now().to_rfc3339()),
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

    // `ensure_acquisition_scope_state` is a pure get-or-create — a scope
    // that already owns a row gets that row back untouched, never re-seeded.
    let reseed = scryer_application::AcquisitionScopeState {
        id: scryer_domain::Id::new().0,
        title_id: "title-1".to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        series_movie_link_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        last_search_at: None,
        status: AcquisitionScopeStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    let seeded_id = ctx
        .library_state
        .ensure_acquisition_scope_state(&reseed)
        .await
        .expect("get-or-create completed wanted item");
    assert_eq!(seeded_id, wanted.id);

    let fetched = ctx
        .library_state
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, AcquisitionScopeStatus::Completed);
}

#[tokio::test]
async fn direct_upsert_wanted_item_still_preserves_guarded_state() {
    let ctx = TestContext::new().await;

    seed_title(&ctx, "title-1").await;
    let wanted = seed_wanted_item(&ctx, "title-1", AcquisitionScopeStatus::Wanted).await;

    ctx.app
        .clone()
        .pause_wanted_item(&admin(), &wanted.id)
        .await
        .expect("pause wanted item");

    // A raw upsert must not clobber the guarded status of an
    // existing scope — a re-seed carrying `Wanted` leaves a paused row paused.
    ctx.library_state
        .upsert_acquisition_scope_state(&scryer_application::AcquisitionScopeState {
            id: scryer_domain::Id::new().0,
            title_id: "title-1".to_string(),
            title_name: Some("Test Title".to_string()),
            title_slug: None,
            title_facet: Some("movie".to_string()),
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: None,
            collection_id: None,
            series_movie_link_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            last_search_at: None,
            status: AcquisitionScopeStatus::Wanted,
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
        .get_acquisition_scope_state_by_id(&wanted.id)
        .await
        .expect("fetch wanted")
        .expect("wanted item exists");
    assert_eq!(fetched.status, AcquisitionScopeStatus::Paused);
}

// ---------------------------------------------------------------------------
// dismiss_pending_release
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dismiss_sets_status_to_dismissed() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Waiting,
    )
    .await;

    let actor = admin();
    let result = app
        .dismiss_pending_release(&actor, &pr.id)
        .await
        .expect("dismiss");
    assert!(result);

    // Should no longer appear in waiting list
    let pending = app.list_pending_releases(&actor).await.unwrap();
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
        .dismiss_pending_release(&admin(), "nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn dismiss_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Grabbed,
    )
    .await;

    let err = app
        .dismiss_pending_release(&admin(), &pr.id)
        .await
        .unwrap_err();
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
        .force_grab_pending_release(&admin(), "nonexistent-id")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Repository(_)));
}

#[tokio::test]
async fn force_grab_non_waiting_returns_error() {
    let ctx = TestContext::new().await;
    let app = ctx.app.clone();

    seed_title(&ctx, "title-1").await;
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
    let pr = seed_pending_release(
        &ctx,
        &wi.id,
        "title-1",
        500,
        6,
        PendingReleaseStatus::Dismissed,
    )
    .await;

    let err = app
        .force_grab_pending_release(&admin(), &pr.id)
        .await
        .unwrap_err();
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Wanted,
    )
    .await;
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
        .delete_acquisition_scope_states_for_title("title-1")
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
    let wi = seed_wanted_item(
        &ctx,
        "title-1",
        scryer_application::AcquisitionScopeStatus::Grabbed,
    )
    .await;
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
