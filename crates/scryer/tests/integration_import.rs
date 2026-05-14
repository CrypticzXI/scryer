#![recursion_limit = "256"]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::TestContext;
use scryer_application::testing::AppUseCaseTestExt;
use scryer_application::{
    BlocklistRepository, ImportRepository, MediaFileRepository, ReleaseAttemptRepository,
    ShowRepository, TitleRepository, WantedItemRepository, import_completed_download,
};
use scryer_domain::{
    Collection, CompletedDownload, Episode, Id, ImportDecision, ImportSkipReason, MediaFacet, Title,
};
use scryer_infrastructure::{FsFileImporter, SqliteWorkflowStore};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build an AppUseCase with a real SQLite import repository and filesystem
/// file importer so that tests can exercise the full import pipeline.
fn app_with_real_imports(ctx: &TestContext) -> scryer_application::AppUseCase {
    let workflow_store = Arc::new(SqliteWorkflowStore::new(&ctx.db));
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_imports(workflow_store)
            .with_file_importer(Arc::new(FsFileImporter))
            .with_media_files(Arc::new(ctx.library_state.clone()))
            .with_wanted_items(Arc::new(ctx.library_state.clone()))
    })
}

/// Build a minimal CompletedDownload with scryer-origin parameters.
fn scryer_completed(
    item_id: &str,
    dest_dir: &str,
    title_id: &str,
    facet_id: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: item_id.to_string(),
        name: format!("Test.Download.{item_id}"),
        dest_dir: dest_dir.to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![
            ("*scryer_title_id".to_string(), title_id.to_string()),
            ("*scryer_facet".to_string(), facet_id.to_string()),
        ],
    }
}

/// Add a minimal movie Title to the DB, tagging `media_root` so import
/// uses it as the destination library folder without needing settings.
async fn add_movie_title(ctx: &TestContext, id: &str, name: &str, media_root: &str) -> Title {
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        // The root-folder tag overrides the settings lookup.
        tags: vec![format!("scryer:root-folder:{}", media_root)],
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
    ctx.catalog.create(title).await.expect("add movie title")
}

async fn add_series_title(ctx: &TestContext, id: &str, name: &str, media_root: &str) -> Title {
    let title = Title {
        id: id.to_string(),
        name: name.to_string(),
        facet: MediaFacet::Series,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        monitored: true,
        tags: vec![format!("scryer:root-folder:{}", media_root)],
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
        runtime_minutes: Some(24),
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
    ctx.catalog.create(title).await.expect("add series title")
}

fn mediainfo_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scryer-mediainfo")
        .join("tests")
        .join("media")
        .join(name)
}

fn copy_fixture(dest_dir: &Path, fixture_name: &str, dest_name: &str) -> PathBuf {
    let dest = dest_dir.join(dest_name);
    std::fs::copy(mediainfo_fixture(fixture_name), &dest).expect("copy fixture");
    dest
}

async fn seed_movie_wanted_item(
    ctx: &TestContext,
    title_id: &str,
    status: scryer_application::WantedStatus,
    current_score: Option<i32>,
) -> scryer_application::WantedItem {
    let item = scryer_application::WantedItem {
        id: Id::new().0,
        title_id: title_id.to_string(),
        title_name: Some("Test Title".to_string()),
        title_slug: None,
        title_facet: Some("movie".to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
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
        current_score,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_wanted_item(&item)
        .await
        .expect("seed movie wanted");
    item
}

async fn seed_series_episode(ctx: &TestContext, title: &Title) -> Episode {
    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: scryer_domain::CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: None,
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.catalog
        .create_collection(collection.clone())
        .await
        .expect("create collection");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection.id.clone()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.catalog
        .create_episode(episode.clone())
        .await
        .expect("create episode");
    episode
}

async fn seed_episode_wanted_item(
    ctx: &TestContext,
    title: &Title,
    episode: &Episode,
    status: scryer_application::WantedStatus,
) -> scryer_application::WantedItem {
    let item = scryer_application::WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: title.slug.clone(),
        title_facet: Some(title.facet.as_str().to_string()),
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: Some(episode.id.clone()),
        collection_id: None,
        season_number: Some("1".to_string()),
        episode_number: episode.episode_number.clone(),
        media_type: "series".to_string(),
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
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    ctx.library_state
        .upsert_wanted_item(&item)
        .await
        .expect("seed episode wanted");
    item
}

async fn install_rule(
    app: &scryer_application::AppUseCase,
    user: &scryer_domain::User,
    rego_source: &str,
    applied_facets: Vec<MediaFacet>,
) {
    app.create_rule_set(
        user,
        "Test Rule".to_string(),
        "integration test".to_string(),
        rego_source.to_string(),
        applied_facets,
        0,
    )
    .await
    .expect("create rule set");
}

fn pad_file_past_series_sample_threshold(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};

    let target_len = 52 * 1024 * 1024;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open fixture for padding");
    file.seek(SeekFrom::Start(target_len))
        .expect("seek fixture");
    file.write_all(&[0])
        .expect("extend fixture beyond sample threshold");
}

async fn seed_second_series_episode(
    ctx: &TestContext,
    title: &Title,
    collection_id: &str,
) -> Episode {
    seed_series_episode_in_collection(ctx, title, collection_id, 2).await
}

async fn seed_series_episode_in_collection(
    ctx: &TestContext,
    title: &Title,
    collection_id: &str,
    episode_number: u32,
) -> Episode {
    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(collection_id.to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some(episode_number.to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some(format!("S01E{episode_number:02}")),
        title: Some(format!("Episode {episode_number}")),
        air_date: None,
        duration_seconds: Some(1440),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: chrono::Utc::now(),
    };
    ctx.catalog
        .create_episode(episode.clone())
        .await
        .expect("create seeded episode");
    episode
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Directly mark a download as "completed" in the import repository, then
/// attempt to import the same download again.  The second call should be
/// short-circuited as AlreadyImported without re-running the pipeline.
#[tokio::test]
async fn import_deduplicates_completed_imports() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let workflow_store = SqliteWorkflowStore::new(&ctx.db);

    // Seed a completed import record for (nzbget, "dl-dedup").
    let import_id = workflow_store
        .queue_import_request(
            "nzbget".to_string(),
            "dl-dedup".to_string(),
            "movie_download".to_string(),
            "{}".to_string(),
        )
        .await
        .expect("queue_import_request");
    workflow_store
        .update_import_status(&import_id, scryer_domain::ImportStatus::Completed, None)
        .await
        .expect("update_import_status");

    // Now attempt to import the same download — dedup should fire immediately.
    let completed = CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-dedup".to_string(),
        name: "Already.Imported.Movie".to_string(),
        dest_dir: "/tmp/wherever".to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![("*scryer_title_id".to_string(), "any-id".to_string())],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Skipped);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::AlreadyImported));
}

// ---------------------------------------------------------------------------
// Title matching
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_returns_unmatched_when_title_not_found() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        source_dir.path().join("Unknown.Movie.2024.mkv"),
        b"fake video",
    )
    .expect("write");

    let completed = CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "test-client".to_string(),
        download_client_item_id: "dl-no-title".to_string(),
        name: "Unknown.Movie.2024".to_string(),
        dest_dir: source_dir.path().to_str().unwrap().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![("*scryer_title_id".to_string(), "nonexistent-id".to_string())],
    };

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Unmatched);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::UnresolvedIdentity)
    );
}

// ---------------------------------------------------------------------------
// Video file detection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_fails_when_no_video_files_in_dest_dir() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source dir exists but contains only a text file — no video files.
    let source_dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(source_dir.path().join("readme.txt"), b"no video here").expect("write");

    let dest_dir = tempfile::tempdir().expect("tempdir");
    let title = add_movie_title(
        &ctx,
        "title-no-video",
        "No Video Movie",
        dest_dir.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-no-video",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Skipped);
    assert_eq!(result.skip_reason, Some(ImportSkipReason::NoVideoFiles));
}

#[tokio::test]
async fn import_movie_strm_file_is_treated_as_video_artifact() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    let strm = source_dir
        .path()
        .join("Test.Movie.2024.1080p.WEB-DL.H264.strm");
    std::fs::write(
        &strm,
        b"https://nzbdav.example/stream/Test.Movie.2024.1080p.WEB-DL.H264",
    )
    .expect("write strm");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-movie-strm-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-strm-1",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(result.decision, ImportDecision::Imported);
    let dest_path = result.dest_path.expect("dest path");
    assert!(dest_path.ends_with(".strm"));
    assert!(std::path::Path::new(&dest_path).exists());
    assert_eq!(
        std::fs::read_to_string(&dest_path).expect("read imported strm"),
        "https://nzbdav.example/stream/Test.Movie.2024.1080p.WEB-DL.H264"
    );
}

// ---------------------------------------------------------------------------
// Happy path: movie import
// ---------------------------------------------------------------------------

#[tokio::test]
async fn import_movie_succeeds_and_copies_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    // Source: a temp dir containing a plausible movie .mkv file.
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let mkv = source_dir
        .path()
        .join("Test.Movie.2024.1080p.WEB-DL.H264.mkv");
    std::fs::write(&mkv, b"fake video content").expect("write mkv");

    // Destination: a different temp dir used as the media library root.
    let dest_root = tempfile::tempdir().expect("dest tempdir");

    let title = add_movie_title(
        &ctx,
        "title-movie-1",
        "Test Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-movie-1",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import_completed_download");

    assert_eq!(
        result.decision,
        ImportDecision::Imported,
        "expected Imported"
    );
    assert!(
        result.dest_path.is_some(),
        "dest_path should be set after import"
    );

    // The imported file must physically exist.
    let dest_path = result.dest_path.unwrap();
    assert!(
        std::path::Path::new(&dest_path).exists(),
        "imported file should exist at {dest_path}"
    );
}

// ---------------------------------------------------------------------------
// Dedup after a real successful import
// ---------------------------------------------------------------------------

/// Run a complete movie import, then confirm that a second attempt with the
/// same download_client_item_id is immediately short-circuited.
#[tokio::test]
async fn import_movie_second_attempt_is_deduped() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();

    let source_dir = tempfile::tempdir().expect("source tempdir");
    std::fs::write(
        source_dir.path().join("Movie.2024.1080p.mkv"),
        b"fake video",
    )
    .expect("write mkv");

    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-dedup-2",
        "Dedup Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    let completed = scryer_completed(
        "dl-dedup-2",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    // First import — should succeed.
    let first = import_completed_download(&app, &user, &completed)
        .await
        .expect("first import");
    assert_eq!(first.decision, ImportDecision::Imported);

    // Second import — same download_client_item_id → AlreadyImported.
    let second = import_completed_download(&app, &user, &completed)
        .await
        .expect("second import");
    assert_eq!(second.decision, ImportDecision::Skipped);
    assert_eq!(second.skip_reason, Some(ImportSkipReason::AlreadyImported));
}

#[tokio::test]
async fn import_movie_rejected_by_post_download_rule_leaves_no_library_file_and_blocklists_release()
{
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Blocked.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-rule-blocked",
        "Blocked Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let wanted = seed_movie_wanted_item(
        &ctx,
        &title.id,
        scryer_application::WantedStatus::Grabbed,
        None,
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let completed = scryer_completed(
        "dl-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        result.dest_path.is_none(),
        "rejected import should not report a finalized destination path"
    );
    assert!(
        std::fs::read_dir(dest_root.path())
            .expect("read destination root")
            .next()
            .is_none(),
        "rejected movie should not create library artifacts"
    );
    assert!(
        ctx.library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "rejected movie should not leave a finalized media file"
    );

    let updated_wanted = ctx
        .library_state
        .get_wanted_item_for_title(&title.id, None)
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.id, wanted.id);
    assert_eq!(
        updated_wanted.status,
        scryer_application::WantedStatus::Wanted
    );

    let failures = scryer_infrastructure::SqliteReleaseStore::new(&ctx.db)
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("failed signatures");
    assert!(failures.iter().any(|failure| {
        failure.source_title.as_deref() == Some("test.download.dl-rule-blocked")
            && failure
                .error_message
                .as_deref()
                .is_some_and(|message| message.contains("too_few_chapters"))
    }));

    let blocklist = ctx
        .library_state
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert!(blocklist.iter().any(|entry| {
        entry.source_title.as_deref() == Some("test.download.dl-rule-blocked")
            && entry
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("too_few_chapters"))
    }));
}

#[tokio::test]
async fn import_series_rejected_by_post_download_rule_resets_episode_wanted_item() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Blocked.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-series-rule-blocked",
        "Blocked Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_series_episode(&ctx, &title).await;
    let wanted = seed_episode_wanted_item(
        &ctx,
        &title,
        &episode,
        scryer_application::WantedStatus::Grabbed,
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Series],
    )
    .await;

    let completed = scryer_completed(
        "dl-series-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        ctx.library_state
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty(),
        "rejected episode should not leave a finalized media file"
    );

    let updated_wanted = ctx
        .library_state
        .get_wanted_item_for_title(&title.id, Some(&episode.id))
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.id, wanted.id);
    assert_eq!(
        updated_wanted.status,
        scryer_application::WantedStatus::Wanted
    );
}

#[tokio::test]
async fn manual_import_series_persists_media_analysis_and_acquisition_score() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-success",
        "Manual Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode = seed_series_episode(&ctx, &title).await;
    let completed = scryer_completed(
        "dl-manual-series-success",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );

    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: episode.id.clone(),
            quality: Some("1080P".to_string()),
        }],
    )
    .await
    .expect("execute manual import");

    assert_eq!(results.len(), 1);
    assert!(results[0].success, "manual import should succeed");

    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    let imported = &media_files[0];
    assert_eq!(imported.episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(imported.scan_status, "scanned");
    assert!(imported.acquisition_score.is_some());
    assert!(imported.duration_seconds.is_some());
    assert!(imported.video_codec.is_some());
}

#[tokio::test]
async fn manual_import_series_rejects_when_incumbent_covers_broader_episode_set() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    let source_file = copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Manual.Show.S01E01.1080p.WEB-DL.H264.mkv",
    );
    pad_file_past_series_sample_threshold(&source_file);
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_series_title(
        &ctx,
        "title-manual-series-broader-incumbent",
        "Manual Show",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let episode1 = seed_series_episode(&ctx, &title).await;
    let episode2 = seed_second_series_episode(
        &ctx,
        &title,
        episode1.collection_id.as_deref().expect("collection id"),
    )
    .await;
    let existing_file_id = ctx
        .library_state
        .insert_media_file(&scryer_application::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: dest_root
                .path()
                .join("Manual Show")
                .join("Season 01")
                .join("Manual Show - S01E01-E02.mkv")
                .to_string_lossy()
                .to_string(),
            size_bytes: 52 * 1024 * 1024,
            quality_label: Some("1080P".to_string()),
            acquisition_score: Some(500),
            ..Default::default()
        })
        .await
        .expect("insert incumbent pack");
    ctx.library_state
        .link_file_to_episode(&existing_file_id, &episode1.id)
        .await
        .expect("link incumbent episode 1");
    ctx.library_state
        .link_file_to_episode(&existing_file_id, &episode2.id)
        .await
        .expect("link incumbent episode 2");

    let completed = scryer_completed(
        "dl-manual-series-broader-incumbent",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "series",
    );
    let results = scryer_application::execute_manual_import(
        &app,
        &user,
        &title.id,
        Some(&completed),
        vec![scryer_application::ManualImportFileMapping {
            file_path: source_file.to_string_lossy().to_string(),
            episode_id: episode1.id.clone(),
            quality: Some("1080P".to_string()),
        }],
    )
    .await
    .expect("execute manual import");

    assert_eq!(results.len(), 1);
    assert!(
        !results[0].success,
        "manual import should reject narrower replacement"
    );
    assert!(
        results[0]
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains("broader episode set"))
    );

    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    let media_file_ids = media_files
        .iter()
        .map(|media_file| media_file.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        media_file_ids,
        std::collections::BTreeSet::from([existing_file_id.as_str()]),
        "rejected manual import should not add a new file"
    );
}

#[tokio::test]
async fn import_movie_rule_eval_error_fails_open() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Rule.Error.Movie.2024.1080p.WEB-DL.H264.mkv",
    );
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-rule-error",
        "Rule Error Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["bad_runtime"] := count(input.file.video_width) if {
    input.file != null
    input.file.num_chapters == 0
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let completed = scryer_completed(
        "dl-rule-error",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Imported);
    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
}

#[tokio::test]
async fn import_upgrade_rejected_by_post_download_rule_restores_prior_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_imports(&ctx);
    let user = ctx.app.find_or_create_default_user().await.unwrap();
    let dest_root = tempfile::tempdir().expect("dest tempdir");
    let title = add_movie_title(
        &ctx,
        "title-upgrade-rule-blocked",
        "Upgrade Movie",
        dest_root.path().to_str().unwrap(),
    )
    .await;
    let _wanted = seed_movie_wanted_item(
        &ctx,
        &title.id,
        scryer_application::WantedStatus::Grabbed,
        Some(100),
    )
    .await;

    let old_path = dest_root
        .path()
        .join("Upgrade Movie (2024)")
        .join("Upgrade.Movie.2024.1080p.WEB-DL.H264.mkv");
    std::fs::create_dir_all(old_path.parent().expect("old path parent")).expect("create old dir");
    std::fs::copy(mediainfo_fixture("h264_aac.mkv"), &old_path).expect("seed old movie file");
    ctx.library_state
        .insert_media_file(&scryer_application::InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: old_path.to_string_lossy().to_string(),
            size_bytes: std::fs::metadata(&old_path).expect("old metadata").len() as i64,
            quality_label: Some("1080P".to_string()),
            acquisition_score: Some(100),
            ..Default::default()
        })
        .await
        .expect("insert old media file");

    install_rule(
        &app,
        &user,
        r#"
import rego.v1

score_entry["too_few_chapters"] := scryer.block_score() if {
    input.file != null
    input.file.num_chapters < 2
}
"#,
        vec![MediaFacet::Movie],
    )
    .await;

    let source_dir = tempfile::tempdir().expect("source tempdir");
    copy_fixture(
        source_dir.path(),
        "h264_aac.mkv",
        "Upgrade.Movie.2024.2160p.WEB-DL.H264.mkv",
    );
    let completed = scryer_completed(
        "dl-upgrade-rule-blocked",
        source_dir.path().to_str().unwrap(),
        &title.id,
        "movie",
    );

    let result = import_completed_download(&app, &user, &completed)
        .await
        .expect("import completed download");

    assert_eq!(result.decision, ImportDecision::Rejected);
    assert_eq!(
        result.skip_reason,
        Some(ImportSkipReason::PostDownloadRuleBlocked)
    );
    assert!(
        old_path.exists(),
        "old file should have been restored after rejected upgrade"
    );
    let media_files = ctx
        .library_state
        .list_media_files_for_title(&title.id)
        .await
        .expect("list media files");
    assert_eq!(media_files.len(), 1);
    assert_eq!(
        media_files[0].file_path,
        old_path.to_string_lossy().to_string()
    );

    let blocklist = ctx
        .library_state
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert!(blocklist.iter().any(|entry| {
        entry.source_title.as_deref() == Some("test.download.dl-upgrade-rule-blocked")
            && entry
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("too_few_chapters"))
    }));
}
