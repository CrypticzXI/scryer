use super::*;
use crate::import_title_resolution::find_monitored_movie_title_from_release;
use crate::missing_required_audio_languages;
use crate::post_download_gate::facet_to_category_hint;
use scryer_domain::{CompletedDownload, ExternalId, MediaFacet, Title};

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_title(facet: MediaFacet) -> Title {
    Title {
        id: "t1".to_string(),
        name: "Test Movie".to_string(),
        facet,
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
    }
}

fn test_parsed() -> crate::ParsedReleaseMetadata {
    crate::parse_release_metadata("Test.Movie.2024.1080p.WEB-DL.DDP5.1.H.264-Group")
}

fn test_movie_title_with_aliases_and_ids(
    id: &str,
    name: &str,
    year: Option<i32>,
    aliases: Vec<&str>,
    external_ids: Vec<(&str, &str)>,
) -> Title {
    let mut title = test_title(MediaFacet::Movie);
    title.id = id.to_string();
    title.name = name.to_string();
    title.year = year;
    title.aliases = aliases.into_iter().map(str::to_string).collect();
    title.external_ids = external_ids
        .into_iter()
        .map(|(source, value)| ExternalId {
            source: source.to_string(),
            value: value.to_string(),
        })
        .collect();
    title
}

fn test_completed_download(name: &str, dest_dir: &std::path::Path) -> CompletedDownload {
    CompletedDownload {
        client_type: "weaver".to_string(),
        client_id: "client-1".to_string(),
        download_client_item_id: "job-1".to_string(),
        name: name.to_string(),
        dest_dir: dest_dir.to_string_lossy().to_string(),
        category: None,
        size_bytes: None,
        completed_at: None,
        parameters: vec![],
    }
}

// ── has_scryer_origin ─────────────────────────────────────────────────────────

#[test]
fn has_scryer_origin_with_title_id() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert!(has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_without_title_id() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert!(!has_scryer_origin(&params));
}

#[test]
fn has_scryer_origin_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert!(!has_scryer_origin(&params));
}

// ── extract_parameter ─────────────────────────────────────────────────────────

#[test]
fn extract_parameter_found() {
    let params = vec![
        ("*scryer_title_id".to_string(), "abc-123".to_string()),
        ("category".to_string(), "movie".to_string()),
    ];
    assert_eq!(
        extract_parameter(&params, "*scryer_title_id"),
        Some("abc-123".to_string())
    );
}

#[test]
fn extract_parameter_not_found() {
    let params = vec![("category".to_string(), "movie".to_string())];
    assert_eq!(extract_parameter(&params, "*scryer_title_id"), None);
}

#[test]
fn extract_parameter_empty_params() {
    let params: Vec<(String, String)> = vec![];
    assert_eq!(extract_parameter(&params, "anything"), None);
}

#[test]
fn extract_parameter_first_match() {
    let params = vec![
        ("key".to_string(), "first".to_string()),
        ("key".to_string(), "second".to_string()),
    ];
    assert_eq!(extract_parameter(&params, "key"), Some("first".to_string()));
}

// ── normalize_imdb_id ─────────────────────────────────────────────────────────

#[test]
fn normalize_imdb_id_with_prefix() {
    assert_eq!(
        normalize_imdb_id("tt1234567"),
        Some("tt1234567".to_string())
    );
}

#[test]
fn normalize_imdb_id_digits_only() {
    assert_eq!(normalize_imdb_id("1234567"), Some("tt1234567".to_string()));
}

#[test]
fn normalize_imdb_id_with_extra_chars() {
    assert_eq!(
        normalize_imdb_id("tt0123456abc"),
        Some("tt0123456".to_string())
    );
}

#[test]
fn normalize_imdb_id_empty() {
    assert_eq!(normalize_imdb_id(""), None);
}

#[test]
fn normalize_imdb_id_no_digits() {
    assert_eq!(normalize_imdb_id("abcdef"), None);
}

// ── movie title resolution ───────────────────────────────────────────────────

#[test]
fn find_monitored_movie_title_from_release_matches_alias_variant() {
    let titles = vec![test_movie_title_with_aliases_and_ids(
        "movie-1",
        "My Cousin",
        Some(2020),
        vec!["Mon Cousin"],
        vec![],
    )];

    let parsed =
        crate::parse_release_metadata("Mon.Cousin.A.K.A.My.Cousin.2020.1080p.BluRay.x264-GRP");

    let matched = find_monitored_movie_title_from_release(&titles, &parsed)
        .expect("movie should resolve through alias/title variants");

    assert_eq!(matched.id, "movie-1");
}

#[test]
fn find_monitored_movie_title_from_release_matches_tagged_alias_variant() {
    let mut title =
        test_movie_title_with_aliases_and_ids("movie-1", "Bastard!!", Some(2022), vec![], vec![]);
    title.tagged_aliases = vec![scryer_domain::TaggedAlias {
        name: "Bastard Heavy Metal Dark Fantasy".to_string(),
        language: "eng".to_string(),
    }];

    let parsed =
        crate::parse_release_metadata("BASTARD.Heavy.Metal.Dark.Fantasy.2022.1080p.WEB-DL");

    let matched = find_monitored_movie_title_from_release(&[title], &parsed)
        .expect("movie should resolve through tagged alias variants");

    assert_eq!(matched.id, "movie-1");
}

#[test]
fn find_monitored_movie_title_from_release_prefers_imdb_id() {
    let titles = vec![
        test_movie_title_with_aliases_and_ids(
            "movie-1",
            "Dune",
            Some(1984),
            vec![],
            vec![("imdb", "tt0087182")],
        ),
        test_movie_title_with_aliases_and_ids(
            "movie-2",
            "Dune",
            Some(2021),
            vec![],
            vec![("imdb", "tt1160419"), ("tmdb", "438631")],
        ),
    ];

    let parsed =
        crate::parse_release_metadata("Dune.2021.{tmdb-438631}.[tt1160419].1080p.BluRay.x264-GRP");

    let matched = find_monitored_movie_title_from_release(&titles, &parsed)
        .expect("movie should resolve by embedded IDs");

    assert_eq!(matched.id, "movie-2");
}

#[test]
fn build_augmented_movie_import_metadata_prefers_download_title_for_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Paperman.2012.1080p.BluRay.x264-GRP");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"movie").expect("write file");
    let completed = test_completed_download("Paperman.2012.1080p.BluRay.x264-GRP", &dest_dir);

    let parsed = build_augmented_movie_import_metadata(&file_path, &completed);

    assert_eq!(parsed.year, Some(2012));
    assert_eq!(parsed.quality.as_deref(), Some("1080p"));
    assert_eq!(parsed.source.as_deref(), Some("BluRay"));
}

#[test]
fn build_augmented_episode_import_metadata_prefers_download_title_for_single_obfuscated_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, false);
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![1]);
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

#[test]
fn build_augmented_episode_import_metadata_keeps_file_episode_when_other_files_exist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Bluey.S01.Complete.720p.WEB-DL.AV1");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("Bluey.S01E03.720p.WEB-DL.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("Bluey.S01.Complete.720p.WEB-DL.AV1", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, true);
    let episode = parsed.episode.expect("episode metadata");

    assert_eq!(episode.season, Some(1));
    assert_eq!(episode.episode_numbers, vec![3]);
}

#[test]
fn build_augmented_episode_import_metadata_does_not_infer_episode_from_download_title_when_other_files_exist()
 {
    let dir = tempfile::tempdir().expect("tempdir");
    let dest_dir = dir.path().join("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb");
    std::fs::create_dir_all(&dest_dir).expect("create dest dir");
    let file_path = dest_dir.join("4f8e2c7a91b6d3e0.mkv");
    std::fs::write(&file_path, b"episode").expect("write file");
    let completed = test_completed_download("Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb", &dest_dir);

    let parsed = build_augmented_episode_import_metadata(&file_path, &completed, true);

    assert!(parsed.episode.is_none());
    assert_eq!(parsed.quality.as_deref(), Some("720p"));
}

// ── is_sample_file ────────────────────────────────────────────────────────────

#[test]
fn is_sample_file_detects_sample_in_stem() {
    assert!(is_sample_file(std::path::Path::new(
        "/data/episode.sample.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new(
        "/data/sample-show.mkv"
    )));
    assert!(is_sample_file(std::path::Path::new("/data/SAMPLE.mkv")));
}

#[test]
fn is_sample_file_allows_normal_video_file() {
    // Non-existent path → metadata fails → size defaults to 0, but file doesn't
    // contain "sample" so the filename check returns false; the size check on a
    // nonexistent file returns Ok(0) via unwrap_or(false)... actually
    // std::fs::metadata on a non-existent path returns Err, so unwrap_or(false)
    // → false. So this test should pass.
    assert!(!is_sample_file(std::path::Path::new(
        "/nonexistent/Show.S01E01.1080p.mkv"
    )));
    assert!(!is_sample_file(std::path::Path::new(
        "/nonexistent/Movie.2024.mkv"
    )));
}

// ── pick_largest_file ─────────────────────────────────────────────────────────

#[test]
fn pick_largest_file_empty_list_returns_error() {
    let result = pick_largest_file(&[]);
    assert!(result.is_err());
}

#[test]
fn pick_largest_file_single_file_returns_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("only.mkv");
    std::fs::write(&path, b"content").expect("write");
    let result = pick_largest_file(std::slice::from_ref(&path));
    assert_eq!(result.expect("pick"), path);
}

#[test]
fn pick_largest_file_returns_biggest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let small = dir.path().join("small.mkv");
    let large = dir.path().join("large.mkv");
    let tiny = dir.path().join("tiny.mkv");
    std::fs::write(&small, vec![0u8; 100]).expect("write small");
    std::fs::write(&large, vec![0u8; 1000]).expect("write large");
    std::fs::write(&tiny, vec![0u8; 10]).expect("write tiny");
    let result = pick_largest_file(&[small, large.clone(), tiny]);
    assert_eq!(result.expect("pick"), large);
}

// ── use_season_folders ────────────────────────────────────────────────────────

#[test]
fn use_season_folders_true_when_tag_absent() {
    let title = test_title(MediaFacet::Series);
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_true_when_tag_enabled() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:enabled".to_string()];
    assert!(use_season_folders(&title));
}

#[test]
fn use_season_folders_false_when_tag_disabled() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:disabled".to_string()];
    assert!(!use_season_folders(&title));
}

#[test]
fn use_season_folders_false_case_insensitive() {
    let mut title = test_title(MediaFacet::Series);
    title.tags = vec!["scryer:season-folder:DISABLED".to_string()];
    assert!(!use_season_folders(&title));
}

// ── build_rename_tokens ───────────────────────────────────────────────────────

#[test]
fn build_rename_tokens_includes_title_and_year() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("title").map(String::as_str), Some("Test Movie"));
    assert_eq!(tokens.get("ext").map(String::as_str), Some("mkv"));
    assert_eq!(tokens.get("year").map(String::as_str), Some("2024"));
}

#[test]
fn build_rename_tokens_falls_back_to_title_year_when_release_year_is_missing() {
    let title = test_title(MediaFacet::Movie);
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("year").map(String::as_str), Some("2024"));
}

#[test]
fn build_rename_tokens_includes_quality() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("quality").map(String::as_str), Some("1080p"));
}

fn test_media_analysis(video_height: Option<i32>) -> crate::MediaFileAnalysis {
    crate::MediaFileAnalysis {
        video_codec: Some("h264".to_string()),
        video_width: Some(1920),
        video_height,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: Some("aac".to_string()),
        audio_profile: None,
        audio_channels: Some(2),
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: None,
    }
}

#[test]
fn rescore_from_mediainfo_updates_quality_when_parsed_quality_is_missing() {
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(1080))),
        scan_error: None,
    };

    let (rescored, changes) =
        crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);

    assert_eq!(rescored.quality.as_deref(), Some("1080p"));
    assert!(changes.iter().any(|change| change.contains("resolution")));
}

#[test]
fn episode_import_dest_path_uses_rescored_parsed_quality_without_override() {
    let mut title = test_title(MediaFacet::Series);
    title.name = "Test Show".to_string();
    let parsed = crate::parse_release_metadata("obfuscated.release.name");
    let acceptance = crate::post_download_gate::ImportedFileAcceptance {
        analysis: Some(test_media_analysis(Some(1080))),
        scan_error: None,
    };
    let (rescored, _) = crate::post_download_gate::rescore_from_mediainfo(&parsed, &acceptance);

    let dest_path = episode_import_dest_path(
        &title,
        &rescored,
        "mkv",
        "/library",
        "Test Show",
        "{title} - S{season:2}E{episode:2} - {quality}.{ext}",
        8,
        "7",
        None,
        None,
        None,
    );

    assert_eq!(
        dest_path,
        std::path::PathBuf::from("/library/Test Show/Season 08/Test Show - S08E07 - 1080p.mkv")
    );
}

#[test]
fn build_rename_tokens_episode_is_empty_for_movie() {
    let title = test_title(MediaFacet::Movie);
    let parsed = test_parsed();
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("season").map(String::as_str), Some(""));
    assert_eq!(tokens.get("episode").map(String::as_str), Some(""));
}

#[test]
fn build_rename_tokens_episode_metadata_for_series() {
    let title = test_title(MediaFacet::Series);
    let parsed = crate::parse_release_metadata("Show.S02E05.720p.HDTV.mkv");
    let tokens = build_rename_tokens(&title, &parsed, "mkv");
    assert_eq!(tokens.get("season").map(String::as_str), Some("2"));
    assert_eq!(tokens.get("episode").map(String::as_str), Some("5"));
}

// ── find_video_files ──────────────────────────────────────────────────────────

#[test]
fn find_video_files_finds_mkv_in_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("movie.mkv"), b"data").expect("write");
    std::fs::write(dir.path().join("notes.txt"), b"text").expect("write");
    let files = find_video_files(dir.path(), false).expect("find");
    assert_eq!(files.len(), 1);
    assert!(files[0].to_str().unwrap().ends_with("movie.mkv"));
}

#[test]
fn find_video_files_filters_samples_when_flag_set() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = tempfile::tempdir().expect("tempdir");

    // movie.mkv must be >= 50 MB so the size check doesn't also flag it as a sample.
    // Use a sparse file (seek past threshold, write one byte) to avoid allocating 50 MB.
    let main_path = dir.path().join("movie.mkv");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&main_path)
        .expect("open main");
    f.seek(SeekFrom::Start(52 * 1024 * 1024)).expect("seek");
    f.write_all(b"\0").expect("write");
    drop(f);

    // sample file — name alone triggers filtering regardless of size
    std::fs::write(dir.path().join("movie.sample.mkv"), b"data").expect("write sample");

    let files = find_video_files(dir.path(), true).expect("find");
    // sample file is filtered; only movie.mkv remains
    assert_eq!(files.len(), 1);
    assert!(!files[0].to_str().unwrap().contains("sample"));
}

#[test]
fn find_video_files_returns_error_for_missing_dir() {
    let result = find_video_files(std::path::Path::new("/nonexistent/dir/abc"), false);
    assert!(result.is_err());
}

#[test]
fn find_video_files_recurses_into_subdirs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let subdir = dir.path().join("season1");
    std::fs::create_dir(&subdir).expect("mkdir");
    std::fs::write(subdir.join("ep1.mkv"), b"data").expect("write");
    std::fs::write(dir.path().join("ep2.mp4"), b"data").expect("write");
    let files = find_video_files(dir.path(), false).expect("find");
    assert_eq!(files.len(), 2);
}

#[cfg(unix)]
#[test]
fn find_video_files_follows_symlinked_directories() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("season1");
    std::fs::create_dir(&target).expect("mkdir");
    std::fs::write(target.join("ep1.mkv"), b"data").expect("write");
    symlink(&target, dir.path().join("linked-season1")).expect("symlink");

    let files = find_video_files(dir.path(), false).expect("find");

    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("linked-season1/ep1.mkv"));
}

// ── missing_audio_languages ───────────────────────────────────────────────────

#[test]
fn missing_audio_languages_all_present() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["jpn".to_string(), "eng".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_case_normalization() {
    // media analysis emits lowercase codes; profile stores uppercase
    let required = vec!["JPN".to_string()];
    let actual = vec!["jpn".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_accepts_full_iso_language_names() {
    let required = vec!["Filipino".to_string()];
    let actual = vec!["fil-PH".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_one_missing() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual = vec!["eng".to_string()];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["jpn"]);
}

#[test]
fn missing_audio_languages_all_missing() {
    let required = vec!["JPN".to_string()];
    let actual = vec!["eng".to_string(), "spa".to_string()];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing, vec!["jpn"]);
}

#[test]
fn missing_audio_languages_empty_required_always_passes() {
    let required: Vec<String> = vec![];
    let actual = vec!["eng".to_string()];
    assert!(missing_required_audio_languages(&required, &actual).is_empty());
}

#[test]
fn missing_audio_languages_empty_actual_returns_all_required() {
    let required = vec!["JPN".to_string(), "ENG".to_string()];
    let actual: Vec<String> = vec![];
    let missing = missing_required_audio_languages(&required, &actual);
    assert_eq!(missing.len(), 2);
}

// ── facet_to_category_hint ────────────────────────────────────────────────────

#[test]
fn facet_to_category_hint_values() {
    assert_eq!(facet_to_category_hint(&MediaFacet::Movie), "movie");
    assert_eq!(facet_to_category_hint(&MediaFacet::Series), "series");
    assert_eq!(facet_to_category_hint(&MediaFacet::Anime), "anime");
}
