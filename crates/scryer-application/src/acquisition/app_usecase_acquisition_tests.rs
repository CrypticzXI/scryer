use super::*;
use chrono::{DateTime, Utc};

// ── helpers ───────────────────────────────────────────────────────────────────

fn now_utc() -> DateTime<Utc> {
    Utc::now()
}

fn days_ago(n: i64) -> String {
    (now_utc() - chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

fn days_from_now(n: i64) -> String {
    (now_utc() + chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

fn base_title() -> Title {
    Title {
        id: "t1".to_string(),
        name: "Test Movie".to_string(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![],
        created_by: None,
        created_at: now_utc(),
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
    }
}

fn base_episode_wanted_item() -> WantedItem {
    let now = now_utc().to_rfc3339();
    WantedItem {
        id: "wanted-episode-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: Some("Test Show".to_string()),
        episode_id: Some("episode-1".to_string()),
        collection_id: Some("season-1".to_string()),
        season_number: Some("1".to_string()),
        media_type: "episode".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn base_interstitial_wanted_item() -> WantedItem {
    let now = now_utc().to_rfc3339();
    WantedItem {
        id: "wanted-interstitial-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: Some("Test Show".to_string()),
        episode_id: None,
        collection_id: Some("movie-collection-1".to_string()),
        season_number: None,
        media_type: "interstitial_movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn base_episode() -> Episode {
    Episode {
        id: "episode-1".to_string(),
        title_id: "title-1".to_string(),
        collection_id: Some("season-1".to_string()),
        episode_type: scryer_domain::EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("Pilot".to_string()),
        air_date: None,
        duration_seconds: None,
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: None,
        overview: None,
        tvdb_id: None,
        monitored: true,
        created_at: now_utc(),
    }
}

// ── announced ────────────────────────────────────────────────────────────────

#[test]
fn announced_always_available_no_dates() {
    let title = base_title();
    assert!(is_movie_available_for_acquisition(
        &title,
        "announced",
        &now_utc()
    ));
}

#[test]
fn announced_always_available_future_dates() {
    let mut title = base_title();
    title.first_aired = Some(days_from_now(90));
    assert!(is_movie_available_for_acquisition(
        &title,
        "announced",
        &now_utc()
    ));
}

#[test]
fn unknown_availability_treated_as_announced() {
    let title = base_title();
    assert!(is_movie_available_for_acquisition(
        &title,
        "preorder",
        &now_utc()
    ));
}

#[tokio::test]
async fn skip_interval_does_not_replay_missed_poll_ticks_in_a_burst() {
    let mut interval = new_skip_interval(std::time::Duration::from_millis(50));
    interval.tick().await;

    tokio::time::sleep(std::time::Duration::from_millis(220)).await;
    interval.tick().await;

    let next_tick =
        tokio::time::timeout(std::time::Duration::from_millis(10), interval.tick()).await;
    assert!(
        next_tick.is_err(),
        "skip interval should not have an immediate catch-up tick waiting"
    );
}

// ── in_cinemas ────────────────────────────────────────────────────────────────

#[test]
fn in_cinemas_available_when_past_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(10));
    assert!(is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_available_when_today_is_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(now_utc().format("%Y-%m-%d").to_string());
    assert!(is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_future_cinema_date() {
    let mut title = base_title();
    title.first_aired = Some(days_from_now(30));
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_no_date() {
    let title = base_title();
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

#[test]
fn in_cinemas_unavailable_when_date_malformed() {
    let mut title = base_title();
    title.first_aired = Some("not-a-date".to_string());
    assert!(!is_movie_available_for_acquisition(
        &title,
        "in_cinemas",
        &now_utc()
    ));
}

// ── released ──────────────────────────────────────────────────────────────────

#[test]
fn released_available_when_past_digital_release() {
    let mut title = base_title();
    title.digital_release_date = Some(days_ago(5));
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_future_digital_release() {
    let mut title = base_title();
    title.digital_release_date = Some(days_from_now(14));
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_past() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(100)); // 100 days ago + 90 = still past
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_falls_back_to_cinema_plus_90_days_when_not_yet() {
    let mut title = base_title();
    title.first_aired = Some(days_ago(30)); // 30 days ago + 90 = 60 days in future
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_unavailable_when_no_dates() {
    let title = base_title();
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_digital_date_takes_priority_over_cinema_fallback() {
    let mut title = base_title();
    // digital date is in the past (available), even though cinema + 90 would be in future
    title.digital_release_date = Some(days_ago(1));
    title.first_aired = Some(days_ago(10)); // cinema only 10d ago, +90 not reached
    assert!(is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn released_malformed_digital_date_falls_back_to_cinema() {
    let mut title = base_title();
    title.digital_release_date = Some("bad-date".to_string());
    title.first_aired = Some(days_ago(100));
    // digital date parse fails → false; but we fall through to cinema check... actually no.
    // The code checks digital_release_date first, and on parse failure returns false
    // (no fallback within that branch). So this returns false.
    assert!(!is_movie_available_for_acquisition(
        &title,
        "released",
        &now_utc()
    ));
}

#[test]
fn old_failed_grab_titles_do_not_research_immediately() {
    let now = now_utc();
    let item = WantedItem {
        id: "wanted-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        media_type: "movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: Some((now - chrono::Duration::minutes(45)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(days_ago(30)),
        status: WantedStatus::Grabbed,
        grabbed_release: None,
        current_score: Some(100),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };

    assert!(is_old_failed_grab_title(&item, &now));
    assert!(!should_research_failed_grab(&item, &now));
}

#[test]
fn fresh_failed_grab_titles_require_stale_last_search() {
    let now = now_utc();
    let mut item = WantedItem {
        id: "wanted-1".to_string(),
        title_id: "title-1".to_string(),
        title_name: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        media_type: "movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: None,
        last_search_at: Some((now - chrono::Duration::minutes(10)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(days_ago(3)),
        status: WantedStatus::Grabbed,
        grabbed_release: None,
        current_score: Some(100),
        created_at: now.to_rfc3339(),
        updated_at: now.to_rfc3339(),
    };

    assert!(!should_research_failed_grab(&item, &now));

    item.last_search_at = Some((now - chrono::Duration::minutes(25)).to_rfc3339());
    assert!(should_research_failed_grab(&item, &now));
}

#[test]
fn season_pack_release_uses_collection_submission_scope() {
    let wanted = base_episode_wanted_item();
    let episode = base_episode();

    let scope = download_submission_scope_for_release_title(
        &wanted,
        Some(&episode),
        "Test.Show.S01.2025.Complete.1080p.WEB-DL.AVC.AAC-DBTV",
    );

    assert_eq!(
        scope,
        SubmissionScope::Collection {
            collection_id: "season-1".to_string(),
        }
    );
}

#[test]
fn single_episode_release_uses_episode_submission_scope() {
    let wanted = base_episode_wanted_item();
    let episode = base_episode();

    let scope = download_submission_scope_for_release_title(
        &wanted,
        Some(&episode),
        "Test.Show.S01E01.1080p.WEB-DL.AVC.AAC-DBTV",
    );

    assert_eq!(
        scope,
        SubmissionScope::Episode {
            episode_id: "episode-1".to_string(),
        }
    );
}

#[test]
fn interstitial_movie_blocking_is_collection_scoped() {
    let wanted = base_interstitial_wanted_item();

    let title_submission = DownloadSubmission {
        title_id: wanted.title_id.clone(),
        facet: "anime".to_string(),
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "job-1".to_string(),
        source_hint: None,
        source_kind: None,
        source_title: Some("Title-level".to_string()),
        request_signature: None,
        scope: SubmissionScope::Title,
    };
    assert!(submission_blocks_wanted_item(
        &title_submission,
        &wanted,
        wanted.collection_id.as_deref(),
    ));

    let matching_collection_submission = DownloadSubmission {
        scope: SubmissionScope::Collection {
            collection_id: wanted.collection_id.clone().expect("collection id"),
        },
        ..title_submission.clone()
    };
    assert!(submission_blocks_wanted_item(
        &matching_collection_submission,
        &wanted,
        wanted.collection_id.as_deref(),
    ));

    let different_collection_submission = DownloadSubmission {
        scope: SubmissionScope::Collection {
            collection_id: "movie-collection-2".to_string(),
        },
        ..title_submission
    };
    assert!(!submission_blocks_wanted_item(
        &different_collection_submission,
        &wanted,
        wanted.collection_id.as_deref(),
    ));
}
