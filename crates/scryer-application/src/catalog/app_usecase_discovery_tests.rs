use super::*;
use crate::DownloadSourceKind;

// ── extract_http_status_from_message ─────────────────────────────────────────

#[test]
fn extracts_404_from_error_message() {
    assert_eq!(
        extract_http_status_from_message("request failed with status 404"),
        Some(404)
    );
}

#[test]
fn extracts_503_from_error_message() {
    assert_eq!(
        extract_http_status_from_message("HTTP status 503 Service Unavailable"),
        Some(503)
    );
}

#[test]
fn extracts_status_case_insensitive() {
    assert_eq!(
        extract_http_status_from_message("received STATUS 429 too many requests"),
        Some(429)
    );
}

#[test]
fn returns_none_when_no_status_keyword() {
    assert_eq!(extract_http_status_from_message("connection refused"), None);
}

#[test]
fn returns_none_for_empty_message() {
    assert_eq!(extract_http_status_from_message(""), None);
}

#[test]
fn returns_none_when_status_keyword_has_no_digits() {
    assert_eq!(extract_http_status_from_message("status ok"), None);
}

// ── is_4xx_or_5xx_status ─────────────────────────────────────────────────────

#[test]
fn is_4xx_true_for_client_errors() {
    for status in [400u16, 401, 403, 404, 422, 429] {
        assert!(is_4xx_or_5xx_status(status), "{status} should be 4xx/5xx");
    }
}

#[test]
fn is_5xx_true_for_server_errors() {
    for status in [500u16, 502, 503, 504] {
        assert!(is_4xx_or_5xx_status(status), "{status} should be 4xx/5xx");
    }
}

#[test]
fn is_2xx_false() {
    assert!(!is_4xx_or_5xx_status(200));
    assert!(!is_4xx_or_5xx_status(201));
    assert!(!is_4xx_or_5xx_status(204));
}

#[test]
fn is_3xx_false() {
    assert!(!is_4xx_or_5xx_status(301));
    assert!(!is_4xx_or_5xx_status(302));
}

// ── is_indexer_http_error ─────────────────────────────────────────────────────

#[test]
fn repository_error_with_status_404_is_http_error() {
    let err = AppError::Repository("indexer returned status 404 not found".to_string());
    assert!(is_indexer_http_error(&err));
}

#[test]
fn repository_error_with_status_503_is_http_error() {
    let err = AppError::Repository("upstream status 503".to_string());
    assert!(is_indexer_http_error(&err));
}

#[test]
fn repository_error_with_200_is_not_http_error() {
    let err = AppError::Repository("status 200 ok".to_string());
    assert!(!is_indexer_http_error(&err));
}

#[test]
fn non_repository_error_is_not_http_error() {
    let err = AppError::Validation("bad input".to_string());
    assert!(!is_indexer_http_error(&err));
}

#[test]
fn connection_refused_is_not_http_error() {
    let err = AppError::Repository("connection refused".to_string());
    assert!(!is_indexer_http_error(&err));
}

fn make_search_result(
    source: &str,
    title: &str,
    download_url: &str,
    source_kind: DownloadSourceKind,
) -> IndexerSearchResult {
    IndexerSearchResult {
        indexer_id: None,
        source: source.to_string(),
        title: title.to_string(),
        link: None,
        download_url: Some(download_url.to_string()),
        source_kind: Some(source_kind),
        size_bytes: None,
        published_at: None,
        thumbs_up: None,
        thumbs_down: None,
        indexer_languages: None,
        indexer_subtitles: None,
        indexer_grabs: None,
        password_hint: None,
        parsed_release_metadata: Some(parse_release_metadata(title)),
        quality_profile_decision: None,
        extra: HashMap::new(),
        guid: None,
        info_url: None,
        provenance: None,
        candidate_token: None,
        queue_scope: None,
        auto_eligible: None,
        auto_decision_code: None,
        auto_decision_summary: None,
    }
}

#[test]
fn cross_indexer_release_dedup_prefers_higher_priority_source() {
    let results = vec![
        make_search_result(
            "Lower Priority",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/low",
            DownloadSourceKind::NzbUrl,
        ),
        make_search_result(
            "Higher Priority",
            "Signal.Run.S01E12.720p.WEB-DL.x264-NTb",
            "https://example.test/high",
            DownloadSourceKind::NzbUrl,
        ),
    ];

    let deduped = dedupe_cross_indexer_release_results(
        results,
        &HashMap::from([
            ("Lower Priority".to_string(), 50),
            ("Higher Priority".to_string(), 10),
        ]),
        "nzb",
    );

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].source, "Higher Priority");
    assert_eq!(
        deduped[0].download_url.as_deref(),
        Some("https://example.test/high")
    );
}

#[test]
fn structured_dispatch_queries_collapse_equivalent_episode_variants() {
    let queries = vec![
        "Synthetic Atlas 035".to_string(),
        "Synthetic Atlas S02E11".to_string(),
        "Synthetic Atlas S02".to_string(),
        "Synthetic Atlas".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries, Some(2), Some(11), Some(35));

    assert_eq!(deduped, vec!["Synthetic Atlas 035".to_string()]);
}

#[test]
fn structured_dispatch_queries_keep_distinct_base_titles() {
    let queries = vec![
        "Synthetic Atlas S02E11".to_string(),
        "Alternate Atlas S02E11".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), Some(2), Some(11), None);

    assert_eq!(deduped, queries);
}

#[test]
fn structured_dispatch_query_dedupe_does_not_run_for_plain_title_searches() {
    let queries = vec![
        "Synthetic Atlas 035".to_string(),
        "Synthetic Atlas S02E11".to_string(),
        "Synthetic Atlas".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), None, None, None);

    assert_eq!(deduped, queries);
}

#[test]
fn structured_dispatch_queries_keep_legitimate_numbered_titles_when_absolute_episode_differs() {
    let queries = vec![
        "Synthetic Atlas 2049".to_string(),
        "Synthetic Atlas S02E11".to_string(),
    ];

    let deduped = dedupe_structured_dispatch_queries(queries.clone(), Some(2), Some(11), Some(35));

    assert_eq!(deduped, queries);
}

fn synthetic_indexer_config(
    id: &str,
    provider_type: &str,
    is_enabled: bool,
    enable_interactive_search: bool,
    enable_auto_search: bool,
    managed_parent_config_id: Option<&str>,
) -> scryer_domain::IndexerConfig {
    scryer_domain::IndexerConfig {
        id: id.to_string(),
        name: format!("Synthetic {id}"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.invalid".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        is_enabled,
        enable_interactive_search,
        enable_auto_search,
        indexer_proxy_config_id: None,
        managed_parent_config_id: managed_parent_config_id.map(str::to_string),
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[test]
fn structured_query_collapse_runs_for_nab_only_search_sets() {
    let configs = vec![
        synthetic_indexer_config("direct", "nzbgeek", true, true, true, None),
        synthetic_indexer_config("proxy", "newznab", true, true, true, Some("parent")),
        synthetic_indexer_config("parent", "prowlarr", true, false, false, None),
    ];

    assert!(should_collapse_structured_nab_queries(
        &configs,
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_no_configs_are_eligible() {
    assert!(!should_collapse_structured_nab_queries(
        &[],
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_auto_mode_is_disabled_in_managed_metadata() {
    let mut proxy = synthetic_indexer_config("proxy", "newznab", true, true, true, Some("parent"));
    proxy.managed_metadata_json = Some(
        serde_json::json!({
            "enable_automatic_search": false
        })
        .to_string(),
    );

    assert!(!should_collapse_structured_nab_queries(
        &[proxy],
        None,
        SearchMode::Auto,
        chrono::Utc::now(),
    ));
}

#[test]
fn structured_query_collapse_skips_when_non_nab_indexers_are_eligible() {
    let configs = vec![
        synthetic_indexer_config("direct", "nzbgeek", true, true, true, None),
        synthetic_indexer_config("other", "id_only_anime_indexer", true, true, true, None),
    ];

    assert!(!should_collapse_structured_nab_queries(
        &configs,
        None,
        SearchMode::Interactive,
        chrono::Utc::now(),
    ));
}
