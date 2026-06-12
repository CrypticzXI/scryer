#![recursion_limit = "256"]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::TestContext;
use scryer_application::recycle_bin::{
    RECYCLE_STATUS_COMMITTED, RecycleBinConfig, RecycleManifest,
};
use scryer_application::testing::{
    AppUseCaseTestExt, UpgradeForTestInput, execute_upgrade_for_test,
};
use scryer_application::upgrade::UpgradeResult;
use scryer_application::{
    ActivityKind, ActivitySeverity, AppError, AppResult, CutoffUnmetQualitySummary,
    EpisodeScopedMediaFile, InsertMediaFileInput, MediaFileAnalysis, MediaFileRepository,
    TitleEpisodeProgressSummary, TitleMediaFile, TitleMediaSizeSummary, TitleQualitySummary,
    TitleRepository,
};
use scryer_domain::{LibraryPermissionMask, MediaFacet, Title, User, UserAuthorization};
use scryer_infrastructure::{FsFileImporter, MediaFileStore};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn app_with_real_fs(ctx: &TestContext) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(ctx.media_files.clone()))
            .with_file_importer(Arc::new(FsFileImporter))
    })
}

fn app_with_failing_media_path_update(
    ctx: &TestContext,
    fail_path: String,
) -> scryer_application::AppUseCase {
    ctx.app.with_test_overrides(|builder| {
        builder
            .with_media_files(Arc::new(FailingPathUpdateMediaFileRepo {
                inner: ctx.media_files.clone(),
                fail_path,
            }))
            .with_file_importer(Arc::new(FsFileImporter))
    })
}

struct FailingPathUpdateMediaFileRepo {
    inner: MediaFileStore,
    fail_path: String,
}

#[async_trait]
impl MediaFileRepository for FailingPathUpdateMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        self.inner.insert_media_file(input).await
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        self.inner.link_file_to_episode(file_id, episode_id).await
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        self.inner.list_media_files_for_title(title_id).await
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        self.inner
            .list_live_media_files_for_episode_ids(title_id, episode_ids)
            .await
    }

    async fn list_title_media_size_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        self.inner.list_title_media_size_summaries(title_ids).await
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        self.inner.list_title_quality_summaries(title_ids).await
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        self.inner
            .list_cutoff_unmet_quality_summaries(title_ids)
            .await
    }

    async fn list_title_episode_progress_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        self.inner
            .list_title_episode_progress_summaries(title_ids)
            .await
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_analysis(file_id, analysis)
            .await
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        self.inner
            .update_media_file_source_signature(
                file_id,
                size_bytes,
                source_signature_scheme,
                source_signature_value,
            )
            .await
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        if file_path == self.fail_path {
            return Err(AppError::Repository(format!(
                "injected media file path failure for {file_id} -> {file_path}"
            )));
        }

        self.inner.update_media_file_path(file_id, file_path).await
    }

    async fn replace_media_file_for_upgrade(
        &self,
        old_file_id: &str,
        replacement_file_id: &str,
        replacement_file_path: &str,
    ) -> AppResult<()> {
        if replacement_file_path == self.fail_path {
            return Err(AppError::Repository(format!(
                "injected media file replacement failure for {old_file_id} -> {replacement_file_id} at {replacement_file_path}"
            )));
        }

        self.inner
            .replace_media_file_for_upgrade(old_file_id, replacement_file_id, replacement_file_path)
            .await
    }

    async fn mark_scan_failed(&self, file_id: &str, error: &str) -> AppResult<()> {
        self.inner.mark_scan_failed(file_id, error).await
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_id(file_id).await
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        self.inner.get_media_file_by_path(file_path).await
    }

    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        self.inner.delete_media_file(file_id).await
    }
}

async fn seed_title(ctx: &TestContext, id: &str) -> Title {
    let title = Title {
        id: id.to_string(),
        name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2024),
        overview: None,
        poster_url: None,
        poster_source_url: None,
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
    ctx.titles.create(title.clone()).await.expect("seed title");
    title
}

fn make_recycle_config(base: &std::path::Path) -> RecycleBinConfig {
    RecycleBinConfig {
        enabled: true,
        base_path: base.to_path_buf(),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    }
}

/// Insert a media file record in the DB and create the physical file.
async fn seed_media_file(
    ctx: &TestContext,
    title_id: &str,
    file_path: &std::path::Path,
    size: i64,
    score: i32,
) -> scryer_application::TitleMediaFile {
    let input = InsertMediaFileInput {
        title_id: title_id.to_string(),
        file_path: file_path.to_string_lossy().to_string(),
        size_bytes: size,
        quality_label: Some("720p".to_string()),
        acquisition_score: Some(score),
        ..Default::default()
    };
    let file_id = ctx
        .media_files
        .insert_media_file(&input)
        .await
        .expect("insert");
    let files = ctx
        .media_files
        .list_media_files_for_title(title_id)
        .await
        .unwrap();
    files.into_iter().find(|f| f.id == file_id).unwrap()
}

fn last_upgrade_event(
    events: &[scryer_application::ActivityEvent],
) -> Option<&scryer_application::ActivityEvent> {
    events.iter().find(|e| e.kind == ActivityKind::FileUpgraded)
}

fn test_actor() -> User {
    User {
        id: scryer_domain::Id::new().0,
        username: "admin".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            loaded: true,
            default_library: LibraryPermissionMask::from_permissions([
                scryer_domain::LibraryPermission::View,
            ]),
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_replaces_old_file_with_new() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-1").await;
    let actor = test_actor();

    // Set up directories
    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    // Create "old" file in media library
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content 720p").expect("write old");

    // Create "new" higher-quality source file
    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new video content 1080p better quality").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    // Seed old file in DB
    let existing = seed_media_file(&ctx, "title-1", &old_path, 22, 400).await;

    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL.x264");
    let recycle_config = make_recycle_config(recycle_dir.path());

    let outcome = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("execute_upgrade");

    let UpgradeResult::Upgraded(outcome) = outcome else {
        panic!("expected upgrade to succeed");
    };

    assert_eq!(outcome.old_score, 400);
    assert_eq!(outcome.new_score, 650);
    assert!(
        outcome.recycle_entry_committed,
        "successful upgrade should commit recycle proof"
    );

    // New file should exist at destination
    assert!(new_dest.exists(), "new file should exist");

    // Old file should be gone from original location (recycled)
    assert!(!old_path.exists(), "old file should be recycled");

    // Recycle dir should contain a committed entry for the replaced file.
    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(
        recycle_entries.len(),
        1,
        "recycle bin should have one entry"
    );
    let manifest_bytes = std::fs::read(recycle_entries[0].path().join("manifest.json")).unwrap();
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert!(
        manifest.entry_id.is_some(),
        "committed entry should have an id"
    );
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(
        manifest.original_file_id.as_deref(),
        Some(existing.id.as_str())
    );
    assert_eq!(
        manifest.media_root.as_deref(),
        Some(media_dir.path().to_string_lossy().as_ref())
    );
    assert_eq!(
        manifest.replacement_file_id.as_deref(),
        Some(outcome.new_file_id.as_str())
    );
    assert_eq!(
        manifest.replacement_path.as_deref(),
        Some(new_dest.to_string_lossy().as_ref())
    );

    // DB should have the new file, not the old one
    let files = ctx
        .media_files
        .list_media_files_for_title("title-1")
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].acquisition_score, Some(650));

    // Activity event should be recorded
    let events = app
        .recent_activity(&actor, 10, 0)
        .await
        .expect("recent activity");
    let upgrade_event = last_upgrade_event(&events).expect("should have upgrade event");
    assert_eq!(upgrade_event.severity, ActivitySeverity::Success);
    assert!(upgrade_event.message.contains("400"));
    assert!(upgrade_event.message.contains("650"));
    assert!(upgrade_event.message.contains("Test Movie"));
}

// ---------------------------------------------------------------------------
// Rollback on import failure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_restores_old_file_on_import_failure() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-2").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");

    // Create old file
    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old video content").expect("write old");

    // Source file does NOT exist — this will cause import to fail
    let bad_source = std::path::PathBuf::from("/nonexistent/path/does/not/exist.mkv");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-2", &old_path, 17, 400).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path());

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &bad_source,
            dest_path: &new_dest,
            parsed,
            final_score: 700,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await;

    // Should fail
    assert!(
        result.is_err(),
        "upgrade should fail when source is missing"
    );

    // Old file should be RESTORED (not lost)
    assert!(
        old_path.exists(),
        "old file should be restored after failed upgrade"
    );

    // Content should match original
    let content = std::fs::read_to_string(&old_path).unwrap();
    assert_eq!(content, "old video content");

    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(
        recycle_entries.len(),
        0,
        "failed import should not recycle the old file before replacement validation"
    );
}

// ---------------------------------------------------------------------------
// Disabled recycle bin (safe refusal)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upgrade_with_disabled_recycle_bin() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-3").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content 1080p better").expect("write new");

    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-3", &old_path, 11, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");

    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 600,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    let outcome =
        result.expect("upgrade should direct-delete old file when recycle bin is disabled");
    let UpgradeResult::Upgraded(outcome) = outcome else {
        panic!("upgrade should be accepted");
    };
    assert!(
        !outcome.recycle_entry_committed,
        "disabled recycle bin should not report a committed recycle entry"
    );
    assert!(!old_path.exists(), "old file should be removed directly");
    assert!(new_dest.exists(), "new file should be imported");
}

#[tokio::test]
async fn disabled_recycle_bin_same_path_upgrade_keeps_backup_until_verified() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-4").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4", &old_path, 21, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await
    .expect("same-path disabled recycle upgrade should succeed");

    let UpgradeResult::Upgraded(outcome) = result else {
        panic!("upgrade should be accepted");
    };
    assert!(!outcome.recycle_entry_committed);
    assert_eq!(
        std::fs::read(&old_path).expect("read final path"),
        b"new same-path content"
    );
    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn recycle_bin_same_path_upgrade_recycles_original_filename() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-4a").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let recycle_dir = tempfile::tempdir().expect("recycle dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4a", &old_path, 21, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let recycle_config = make_recycle_config(recycle_dir.path());

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &recycle_config,
        },
    )
    .await
    .expect("same-path recycle upgrade should succeed");

    let UpgradeResult::Upgraded(outcome) = result else {
        panic!("upgrade should be accepted");
    };
    assert!(outcome.recycle_entry_committed);
    assert_eq!(
        std::fs::read(&old_path).expect("read final path"),
        b"new same-path content"
    );

    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let recycle_entries: Vec<_> = std::fs::read_dir(recycle_dir.path())
        .expect("read recycle dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .collect();
    assert_eq!(recycle_entries.len(), 1, "old file should be recycled once");
    let entry_dir = recycle_entries[0].path();
    assert!(
        entry_dir.join("Movie.mkv").exists(),
        "same-path recycle should store the original filename, not the guard filename"
    );

    let manifest_bytes = std::fs::read(entry_dir.join("manifest.json")).unwrap();
    let manifest: RecycleManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(manifest.status.as_deref(), Some(RECYCLE_STATUS_COMMITTED));
    assert_eq!(manifest.original_path, old_path.to_string_lossy());
    assert_eq!(
        manifest.replacement_file_id.as_deref(),
        Some(outcome.new_file_id.as_str())
    );
    assert_eq!(
        manifest.replacement_path.as_deref(),
        Some(old_path.to_string_lossy().as_ref())
    );

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4a")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, outcome.new_file_id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn disabled_recycle_bin_same_path_path_update_failure_preserves_old_file() {
    let ctx = TestContext::new().await;
    let title = seed_title(&ctx, "title-4b").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");

    let old_path = media_dir.path().join("Movie.mkv");
    std::fs::write(&old_path, b"old same-path content").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new same-path content").expect("write new");

    let existing = seed_media_file(&ctx, "title-4b", &old_path, 21, 300).await;
    let app = app_with_failing_media_path_update(&ctx, old_path.to_string_lossy().to_string());
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &old_path,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(media_dir.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    assert!(result.is_err(), "path update failure should abort upgrade");
    assert_eq!(
        std::fs::read(&old_path).expect("read original path"),
        b"old same-path content"
    );
    let leftovers = std::fs::read_dir(media_dir.path())
        .expect("read media dir")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".scryer-upgrade-")
        })
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "guard files should be cleaned up");

    let files = ctx
        .media_files
        .list_media_files_for_title("title-4b")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
    assert_eq!(files[0].file_path, old_path.to_string_lossy());
}

#[tokio::test]
async fn disabled_recycle_bin_upgrade_validation_failure_preserves_old_file() {
    let ctx = TestContext::new().await;
    let app = app_with_real_fs(&ctx);
    let title = seed_title(&ctx, "title-5").await;
    let actor = test_actor();

    let media_dir = tempfile::tempdir().expect("media dir");
    let source_dir = tempfile::tempdir().expect("source dir");
    let wrong_root = tempfile::tempdir().expect("wrong root");

    let old_path = media_dir.path().join("Movie.720p.mkv");
    std::fs::write(&old_path, b"old content guarded").expect("write old");

    let new_source = source_dir.path().join("Movie.1080p.mkv");
    std::fs::write(&new_source, b"new content guarded").expect("write new");
    let new_dest = media_dir.path().join("Movie.1080p.mkv");

    let existing = seed_media_file(&ctx, "title-5", &old_path, 19, 300).await;
    let parsed = scryer_application::parse_release_metadata("Movie.1080p.WEB-DL");
    let disabled_config = RecycleBinConfig {
        enabled: false,
        base_path: std::path::PathBuf::from("/tmp/unused"),
        retention_days: 7,
        cleanup_enabled: true,
        validation_error: None,
    };

    let result = execute_upgrade_for_test(
        &app,
        UpgradeForTestInput {
            actor: &actor,
            title: &title,
            existing_file: &existing,
            source_path: &new_source,
            dest_path: &new_dest,
            parsed,
            final_score: 650,
            target_episode_ids: &[],
            media_root: Some(wrong_root.path().to_string_lossy().as_ref()),
            recycle_config: &disabled_config,
        },
    )
    .await;

    assert!(result.is_err(), "replacement validation should fail");
    assert_eq!(
        std::fs::read(&old_path).expect("old file still exists"),
        b"old content guarded"
    );
    assert!(
        !new_dest.exists(),
        "unverified replacement should be rolled back before old deletion"
    );
    let files = ctx
        .media_files
        .list_media_files_for_title("title-5")
        .await
        .expect("list media files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, existing.id);
}
