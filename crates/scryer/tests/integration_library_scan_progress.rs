#![recursion_limit = "256"]

mod common;

use serde_json::json;
use tokio::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};

use common::TestContext;
use scryer_application::{LibraryScanSession, LibraryScanStatus};
use scryer_domain::{Id, MediaFacet};
use scryer_infrastructure::SettingDefinitionSeed;

async fn gql(ctx: &TestContext, query: &str, variables: serde_json::Value) -> serde_json::Value {
    let client = ctx.http_client();
    let resp = client
        .post(ctx.graphql_url())
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    resp.json().await.expect("should be valid JSON")
}

fn assert_no_errors(body: &serde_json::Value) {
    assert!(
        body.get("errors").is_none(),
        "unexpected GraphQL errors: {body}"
    );
}

async fn seed_media_path_settings(ctx: &TestContext) {
    ctx.settings_store
        .batch_ensure_setting_definitions(vec![
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "movies.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/movies\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "series.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/series\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
            SettingDefinitionSeed {
                category: "media".into(),
                scope: "media".into(),
                key_name: "anime.path".into(),
                data_type: "string".into(),
                default_value_json: "\"/data/anime\"".into(),
                is_sensitive: false,
                validation_json: None,
            },
        ])
        .await
        .expect("seed media path setting definitions");
}

async fn set_media_path(ctx: &TestContext, key_name: &str, value: &str) {
    ctx.settings_store
        .upsert_setting_value(
            "media",
            key_name,
            None,
            serde_json::to_string(value).expect("serialize setting value"),
            "integration_test",
            None,
        )
        .await
        .expect("upsert media path setting");
}

async fn wait_for_scan_status(
    receiver: &mut tokio::sync::broadcast::Receiver<LibraryScanSession>,
    session_id: &str,
    expected_status: LibraryScanStatus,
) -> LibraryScanSession {
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for scan session {session_id} to reach status {:?}",
            expected_status
        );

        match tokio::time::timeout(remaining, receiver.recv()).await {
            Ok(Ok(session))
                if session.session_id == session_id && session.status == expected_status =>
            {
                return session;
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!(
                    "scan progress stream closed before session {session_id} reached terminal status"
                );
            }
            Err(_) => {
                panic!(
                    "timed out waiting for scan session {session_id} to reach status {:?}",
                    expected_status
                );
            }
        }
    }
}

#[tokio::test]
async fn active_library_scans_query_returns_progress_snapshot() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;

    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)"))
        .expect("create unknown series folder");
    set_media_path(&ctx, "series.path", series_root.to_string_lossy().as_ref()).await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(750))
                .set_body_json(json!({
                    "data": {
                        "searchTvdbBatch": []
                    }
                })),
        )
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;

    let start_resp = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"mutation ScanLibrary($facet: MediaFacetValue!) {
                scanLibrary(facet: $facet) {
                    sessionId
                    facet
                    status
                }
            }"#,
            "variables": { "facet": "series" }
        }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(start_resp.status(), 201);
    let start: serde_json::Value = start_resp.json().await.expect("should be valid JSON");
    assert_no_errors(&start);
    let session_id = start["data"]["scanLibrary"]["sessionId"]
        .as_str()
        .expect("scanLibrary should return a session id")
        .to_string();
    assert_eq!(start["data"]["scanLibrary"]["facet"], "series");

    let deadline = Instant::now() + Duration::from_secs(5);
    let scan = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "timed out waiting for activeLibraryScans to expose session {session_id}"
        );

        let body = gql(
            &ctx,
            r#"query { activeLibraryScans { sessionId facet status foundTitles titleMatchTotalKnown titleMatchProgress { total completed failed } hydrationProgress { total completed failed } mediaAnalysisProgress { total completed failed } } }"#,
            json!({}),
        )
        .await;

        assert_no_errors(&body);
        if let Some(scan) = body["data"]["activeLibraryScans"]
            .as_array()
            .and_then(|scans| {
                scans
                    .iter()
                    .find(|scan| scan["sessionId"].as_str() == Some(session_id.as_str()))
                    .cloned()
            })
        {
            break scan;
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    assert_eq!(scan["sessionId"], session_id);
    assert_eq!(scan["facet"], "series");
    assert!(
        matches!(
            scan["status"].as_str(),
            Some("discovering") | Some("running")
        ),
        "expected active scan status, got {scan}"
    );
    assert!(scan["foundTitles"].as_u64().is_some());
    assert!(scan["titleMatchTotalKnown"].as_bool().is_some());
    assert!(scan["titleMatchProgress"]["total"].as_u64().is_some());
    assert!(scan["titleMatchProgress"]["completed"].as_u64().is_some());
    assert!(scan["titleMatchProgress"]["failed"].as_u64().is_some());
    assert!(scan["hydrationProgress"]["total"].as_u64().is_some());
    assert!(scan["hydrationProgress"]["completed"].as_u64().is_some());
    assert!(scan["hydrationProgress"]["failed"].as_u64().is_some());
    assert!(scan["mediaAnalysisProgress"]["total"].as_u64().is_some());
    assert!(
        scan["mediaAnalysisProgress"]["completed"]
            .as_u64()
            .is_some()
    );
    assert!(scan["mediaAnalysisProgress"]["failed"].as_u64().is_some());
}

#[tokio::test]
async fn scan_library_mutation_returns_created_status_and_started_session() {
    let ctx = TestContext::new().await;

    let resp = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"mutation ScanLibrary($facet: MediaFacetValue!) {
                scanLibrary(facet: $facet) {
                    sessionId
                    facet
                    mode
                    status
                }
            }"#,
            "variables": { "facet": "movie" }
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 201);

    let body: serde_json::Value = resp.json().await.expect("should be valid JSON");
    assert_no_errors(&body);

    let session = &body["data"]["scanLibrary"];
    assert!(
        session["sessionId"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(session["facet"], "movie");
    assert_eq!(session["mode"], "full");
    assert_eq!(session["status"], "discovering");
}

#[tokio::test]
async fn scan_library_mutation_marks_nonexistent_library_path_failed() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    let mut progress_rx = ctx
        .app
        .subscribe_library_scan_progress(&admin)
        .expect("subscribe to library scan progress");

    let missing_path = format!("/definitely/missing/anime-{}", Id::new().0);
    set_media_path(&ctx, "anime.path", &missing_path).await;

    let resp = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"mutation ScanLibrary($facet: MediaFacetValue!) {
                scanLibrary(facet: $facet) {
                    sessionId
                    facet
                    mode
                    status
                }
            }"#,
            "variables": { "facet": "anime" }
        }))
        .send()
        .await
        .expect("request should succeed");

    assert_eq!(resp.status(), 201);

    let body: serde_json::Value = resp.json().await.expect("should be valid JSON");
    assert_no_errors(&body);

    let session_id = body["data"]["scanLibrary"]["sessionId"]
        .as_str()
        .expect("scanLibrary should return a session id")
        .to_string();

    let failed_session =
        wait_for_scan_status(&mut progress_rx, &session_id, LibraryScanStatus::Failed).await;
    assert_eq!(failed_session.facet, MediaFacet::Anime);
    assert_eq!(failed_session.status, LibraryScanStatus::Failed);
}

#[tokio::test]
async fn cancel_library_scan_mutation_marks_active_full_scan_canceled() {
    let ctx = TestContext::new().await;
    seed_media_path_settings(&ctx).await;
    let admin = ctx
        .app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    let mut progress_rx = ctx
        .app
        .subscribe_library_scan_progress(&admin)
        .expect("subscribe to library scan progress");

    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)"))
        .expect("create unknown series folder");
    set_media_path(&ctx, "series.path", series_root.to_string_lossy().as_ref()).await;

    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(750))
                .set_body_json(json!({
                    "data": {
                        "searchTvdbBatch": []
                    }
                })),
        )
        .with_priority(1)
        .mount(&ctx.smg_server)
        .await;

    let start_resp = ctx
        .http_client()
        .post(ctx.graphql_url())
        .json(&json!({
            "query": r#"mutation ScanLibrary($facet: MediaFacetValue!) {
                scanLibrary(facet: $facet) {
                    sessionId
                    status
                }
            }"#,
            "variables": { "facet": "series" }
        }))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(start_resp.status(), 201);

    let start_body: serde_json::Value = start_resp.json().await.expect("should be valid JSON");
    assert_no_errors(&start_body);

    let session_id = start_body["data"]["scanLibrary"]["sessionId"]
        .as_str()
        .expect("scanLibrary should return a session id")
        .to_string();

    let cancel_body = gql(
        &ctx,
        r#"mutation CancelLibraryScan($input: CancelLibraryScanInput!) {
            cancelLibraryScan(input: $input) {
                sessionId
                accepted
            }
        }"#,
        json!({
            "input": {
                "sessionId": session_id,
            }
        }),
    )
    .await;
    assert_no_errors(&cancel_body);
    assert_eq!(
        cancel_body["data"]["cancelLibraryScan"]["accepted"],
        serde_json::Value::Bool(true)
    );

    let canceled_session =
        wait_for_scan_status(&mut progress_rx, &session_id, LibraryScanStatus::Canceled).await;
    assert_eq!(canceled_session.facet, MediaFacet::Series);
    assert_eq!(canceled_session.status, LibraryScanStatus::Canceled);
}
