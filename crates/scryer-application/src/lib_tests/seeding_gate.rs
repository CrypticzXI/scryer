//! Removal-gate and import-mode wiring for torrent seeding.
//!
//! The gate's own decision table is unit-tested in
//! `crate::seeding_gate`; these tests cover the wiring: which downloads reach
//! the gate, what the terminal-cleanup path does with each verdict, and that a
//! configured `Move` is downgraded while a torrent is still seeding.

use super::*;
use crate::import::import::TerminalDownloadCleanupOutcome;
use crate::tracked_downloads::{TrackedDownload, tracked_download_id};
use scryer_domain::{DownloadQueueState, ImportMode, MediaFacet, NewTitle};

/// A plugin provider that reports torrent inputs, so `client_type_is_torrent`
/// classifies the fixture clients the way a real install would. Without one,
/// only the built-in usenet clients are known and every plugin client would
/// look protocol-less.
struct TorrentPluginProvider {
    torrent_types: Vec<String>,
}

impl TorrentPluginProvider {
    fn new(types: &[&str]) -> Self {
        Self {
            torrent_types: types.iter().map(|value| (*value).to_string()).collect(),
        }
    }
}

impl DownloadClientPluginProvider for TorrentPluginProvider {
    fn client_for_config(
        &self,
        _config: &scryer_domain::DownloadClientConfig,
    ) -> Option<Arc<dyn DownloadClient>> {
        None
    }

    fn available_provider_types(&self) -> Vec<String> {
        self.torrent_types.clone()
    }

    fn accepted_inputs_for_provider(&self, provider_type: &str) -> Vec<String> {
        if self
            .torrent_types
            .iter()
            .any(|value| value.eq_ignore_ascii_case(provider_type))
        {
            vec!["magnet_uri".to_string(), "torrent_file".to_string()]
        } else {
            vec![]
        }
    }
}

fn bootstrap_with_torrent_clients(
    download_client: Arc<StubDownloadClient>,
) -> (AppUseCase, User, Arc<TrackingDownloadSubmissionRepo>) {
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let (mut app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );
    // Protocol classification comes from the client's declared accepted
    // inputs, so the fixture needs a provider that declares torrent inputs;
    // without one every plugin client would look protocol-less and the gate
    // would never engage.
    app.services.integrations.download_client_plugin_provider =
        crate::RuntimeFeature::enabled(Arc::new(TorrentPluginProvider::new(&[
            "qbittorrent",
            "torrent-blackhole",
        ])) as Arc<dyn DownloadClientPluginProvider>);
    (app, user, download_submissions)
}

async fn movie_title(app: &AppUseCase, user: &User, name: &str) -> scryer_domain::Title {
    app.add_title(
        user,
        NewTitle {
            name: name.to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
    )
    .await
    .expect("create monitored movie title")
}

fn tracked_for(
    client_id: &str,
    client_type: &str,
    item_id: &str,
    title: &scryer_domain::Title,
    state: TrackedDownloadState,
    is_trackable: bool,
) -> TrackedDownload {
    let mut client_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    client_item.client_id = client_id.to_string();
    client_item.client_type = client_type.to_string();
    client_item.title_id = Some(title.id.clone());
    client_item.title_name = title.name.clone();
    client_item.facet = Some("movie".to_string());
    TrackedDownload {
        id: tracked_download_id(Some(client_id), client_type, item_id),
        client_id: client_id.to_string(),
        client_type: client_type.to_string(),
        client_item,
        completed_source: None,
        state,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some(title.name.clone()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable,
        import_attempted: true,
        waiting_for_completed_history: false,
        path_missing_since: None,
        no_video_import_retry: None,
        import_execution_retry: None,
        import_hold: None,
        skip_reacquire_on_failure: false,
        snapshot_missing_since: None,
    }
}

#[tokio::test]
async fn an_imported_torrent_is_held_instead_of_removed_when_seeding_cannot_be_proven_done() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Hold").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-hold-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(
        !crate::import::import::terminal_download_cleanup_is_complete(outcome),
        "a held torrent must not settle: it has to re-enter the gate next poll"
    );
    assert!(
        download_client.deleted_requests.lock().await.is_empty(),
        "the client entry must not be removed while the torrent may still owe seeding"
    );
}

#[tokio::test]
async fn an_already_held_torrent_re_enters_the_gate_and_is_still_not_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Rehold").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-hold-2",
        &title,
        TrackedDownloadState::ImportedSeeding,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn a_torrent_that_left_the_client_settles_without_a_removal_call() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seeding Vanished").await;

    // `is_trackable: false` is how the tracker records "absent from the
    // client's snapshot past the grace window" — a `removes_on_seed_limit`
    // client, or an operator who pulled it by hand.
    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-gone-1",
        &title,
        TrackedDownloadState::ImportedSeeding,
        false,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::ImportedSeeding,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::AlreadyGone);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn torrent_blackhole_entries_are_never_auto_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config =
        create_enabled_download_client_config(&app, &user, "Watch Folder", "torrent-blackhole")
            .await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Blackhole Movie").await;

    let tracked = tracked_for(
        &config.id,
        "torrent-blackhole",
        "watch-entry-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
    )
    .await;

    // Removal here is `remove_dir_all` against a directory an external client
    // is still seeding from, so the entry settles without being touched.
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::SeedingEntryKept);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn usenet_downloads_are_removed_on_import_exactly_as_before() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Usenet Movie").await;

    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, "nzb-1".to_string(), true)]
    );
}

#[tokio::test]
async fn a_failed_torrent_is_removed_immediately_so_blocklisting_never_waits_on_seeding() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Failed Torrent").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-failed-1",
        &title,
        TrackedDownloadState::Failed,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::Removed);
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "torrent-failed-1".to_string(),
            true
        )]
    );
}

#[tokio::test]
async fn a_torrent_with_removal_disabled_is_left_alone_without_engaging_the_gate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, false, true).await;
    let title = movie_title(&app, &user, "Keep Everything").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-keep-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
    )
    .await;

    // Nothing was ever going to be removed, so the download settles as
    // `Imported` rather than being parked in `ImportedSeeding` forever.
    assert_eq!(outcome, TerminalDownloadCleanupOutcome::NotConfigured);
    assert!(crate::import::import::terminal_download_cleanup_is_complete(outcome));
}

// ── import mode ───────────────────────────────────────────────────────────

fn completed_for(client_id: &str, client_type: &str, item_id: &str) -> CompletedDownload {
    CompletedDownload {
        client_type: client_type.to_string(),
        client_id: client_id.to_string(),
        download_client_item_id: item_id.to_string(),
        download_id: None,
        name: "Example.Release.2024.1080p".to_string(),
        release_name: None,
        dest_dir: "/downloads/complete/example".to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

#[tokio::test]
async fn a_configured_move_is_downgraded_to_copy_while_a_torrent_may_still_be_seeding() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Move Guard").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");
    assert_eq!(
        app.resolve_import_mode(Some(&title.library_id), &title.facet)
            .await
            .expect("resolve configured import mode"),
        ImportMode::Move,
        "the fixture must actually be configured for Move, or this test proves nothing"
    );

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", "torrent-move-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

#[tokio::test]
async fn a_configured_move_survives_for_usenet_imports() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    let title = movie_title(&app, &user, "Move Usenet").await;
    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("configure move import mode");

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "nzbget", "nzb-move-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::Move);
}

#[tokio::test]
async fn a_configured_hardlink_or_copy_is_never_upgraded_by_the_gate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client);
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    let title = movie_title(&app, &user, "Copy Stays Copy").await;

    let effective = crate::seeding_gate::resolve_seeding_safe_import_mode(
        &app,
        Some(&title.library_id),
        &title.facet,
        Some(&completed_for(&config.id, "qbittorrent", "torrent-copy-1")),
    )
    .await
    .expect("resolve seeding-safe import mode");

    assert_eq!(effective, ImportMode::HardlinkOrCopy);
}

// ── tracked-state transition ──────────────────────────────────────────────

#[tokio::test]
async fn a_held_torrent_is_parked_in_imported_seeding_and_stays_tracked() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, submissions) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Park Me").await;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        "torrent-park-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    let parked = tracker
        .find(&id)
        .expect("a held torrent must stay tracked so it re-enters the gate and stays visible");
    assert_eq!(parked.state, TrackedDownloadState::ImportedSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
    assert_eq!(
        submissions
            .tracked_states
            .lock()
            .await
            .values()
            .next()
            .cloned(),
        Some(TrackedDownloadState::ImportedSeeding.as_str().to_string()),
        "the parked state must be persisted so a restart does not re-derive and remove it"
    );
}

#[tokio::test]
async fn a_usenet_download_still_stops_being_tracked_once_it_is_removed() {
    let download_client = Arc::new(StubDownloadClient::default());
    let (app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Settle Me").await;

    let tracked = tracked_for(
        &config.id,
        "nzbget",
        "nzb-settle-1",
        &title,
        TrackedDownloadState::Imported,
        true,
    );
    let id = tracked.id.clone();
    let mut tracker = crate::tracked_downloads::TrackedDownloadService::new();
    tracker.insert_for_tests(tracked);

    crate::app_usecase_integration::finalize_tracked_terminal_state(
        &app,
        &mut tracker,
        &id,
        TrackedDownloadState::Imported,
    )
    .await;

    assert!(tracker.find(&id).is_none());
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(
            Some(config.id.clone()),
            None,
            "nzb-settle-1".to_string(),
            true
        )]
    );
}

// ── persisted seed-goal read ──────────────────────────────────────────────

/// Serves only the seed-goal reads; every other repository method is inert.
/// This pins the exact lookup the gate performs against the grab-time
/// persistence contract.
#[derive(Default)]
struct SeedGoalOnlySubmissionRepo {
    by_identity: std::sync::Mutex<HashMap<String, PersistedSeedGoals>>,
    by_info_hash: std::sync::Mutex<HashMap<String, PersistedSeedGoals>>,
    identity_lookups: std::sync::Mutex<Vec<DownloadSourceIdentity>>,
    info_hash_lookups: std::sync::Mutex<Vec<String>>,
}

#[async_trait]
impl DownloadSubmissionRepository for SeedGoalOnlySubmissionRepo {
    async fn record_submission(&self, _: DownloadSubmission) -> AppResult<()> {
        Ok(())
    }

    async fn find_by_client_item_id(
        &self,
        _: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }

    async fn list_for_client_items(
        &self,
        _: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }

    async fn list_for_title(&self, _: &str) -> AppResult<Vec<DownloadSubmission>> {
        Ok(vec![])
    }

    async fn find_by_title_and_request_signature(
        &self,
        _: &str,
        _: &str,
        _: DownloadSubmissionPurpose,
        _: &SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        Ok(None)
    }

    async fn delete_for_title(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn delete_by_client_item_id(&self, _: &DownloadSourceIdentity) -> AppResult<()> {
        Ok(())
    }

    async fn update_tracked_state(&self, _: &DownloadSourceIdentity, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn get_tracked_state(&self, _: &DownloadSourceIdentity) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_seed_goals(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        self.identity_lookups
            .lock()
            .expect("identity lookup log")
            .push(identity.clone());
        Ok(self
            .by_identity
            .lock()
            .expect("seed goals by identity")
            .get(&identity.item_id)
            .cloned())
    }

    async fn find_seed_goals_by_info_hash(
        &self,
        info_hash: &str,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        self.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .push(info_hash.to_string());
        Ok(self
            .by_info_hash
            .lock()
            .expect("seed goals by info hash")
            .get(info_hash)
            .cloned())
    }
}

fn persisted_goals(never_remove: bool) -> PersistedSeedGoals {
    PersistedSeedGoals {
        seeding_profile_id: Some("profile-1".to_string()),
        seed_goal_ratio: Some(2.0),
        seed_goal_seconds: None,
        never_remove,
        goal_met_action: Some(scryer_domain::SeedGoalMetAction::RemoveEntry),
        resolution_source: crate::SeedGoalResolutionSource::Indexer,
        info_hash: None,
    }
}

#[tokio::test]
async fn the_gate_reads_the_goals_a_grab_was_persisted_under() {
    use crate::seeding_gate::{SeedGoalLookupKey, SeedGoalsRead};

    let item_id = "abcdef0123456789abcdef0123456789abcdef01";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert(item_id.to_string(), persisted_goals(true));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let key = SeedGoalLookupKey {
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: item_id.to_string(),
        info_hash: Some(item_id.to_string()),
    };
    let goals = app
        .resolved_seed_goals(&key)
        .await
        .expect("persisted goals should be found by client identity");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert!(goals.never_remove);
    assert_eq!(
        repo.identity_lookups
            .lock()
            .expect("identity lookup log")
            .len(),
        1
    );
    assert!(
        repo.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .is_empty(),
        "the info-hash fallback must not run when client identity already answered"
    );
}

#[tokio::test]
async fn the_gate_falls_back_to_the_info_hash_when_the_client_item_id_moved() {
    use crate::seeding_gate::{SeedGoalLookupKey, SeedGoalsRead};

    let info_hash = "abcdef0123456789abcdef0123456789abcdef01";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_info_hash
        .lock()
        .expect("seed goals by info hash")
        .insert(info_hash.to_string(), persisted_goals(false));

    let (mut app, _user, _) =
        bootstrap_with_torrent_clients(Arc::new(StubDownloadClient::default()));
    app.services.workflow.download_submissions = repo.clone();

    let key = SeedGoalLookupKey {
        client_id: "client-1".to_string(),
        client_type: "qbittorrent".to_string(),
        client_item_id: "some-other-item-id".to_string(),
        info_hash: Some(info_hash.to_string()),
    };
    let goals = app
        .resolved_seed_goals(&key)
        .await
        .expect("persisted goals should be found by info hash");

    assert_eq!(goals.seed_goal_ratio, Some(2.0));
    assert_eq!(
        repo.info_hash_lookups
            .lock()
            .expect("info hash lookup log")
            .clone(),
        vec![info_hash.to_string()]
    );
}

#[tokio::test]
async fn a_never_remove_profile_holds_a_torrent_the_client_says_is_removable() {
    let item_id = "abcdef0123456789abcdef0123456789abcdef02";
    let repo = Arc::new(SeedGoalOnlySubmissionRepo::default());
    repo.by_identity
        .lock()
        .expect("seed goals by identity")
        .insert(item_id.to_string(), persisted_goals(true));

    let download_client = Arc::new(StubDownloadClient::default());
    let (mut app, user, _) = bootstrap_with_torrent_clients(download_client.clone());
    let config = create_enabled_download_client_config(&app, &user, "qBit", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, true).await;
    let title = movie_title(&app, &user, "Seed Forever").await;
    app.services.workflow.download_submissions = repo;

    let tracked = tracked_for(
        &config.id,
        "qbittorrent",
        item_id,
        &title,
        TrackedDownloadState::Imported,
        true,
    );

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Imported,
    )
    .await;

    assert_eq!(outcome, TerminalDownloadCleanupOutcome::HeldForSeeding);
    assert!(download_client.deleted_requests.lock().await.is_empty());
}
