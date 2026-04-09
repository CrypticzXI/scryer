#![recursion_limit = "256"]

mod common;

use chrono::Utc;
use serde_json::json;

use common::TestContext;
use scryer_domain::{
    DomainEventPayload, DomainEventStream, Id, LibraryScanProgressedEventData,
    LibraryScanStartedEventData, MediaFacet, NewDomainEvent,
};

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

#[tokio::test]
async fn active_library_scans_query_returns_progress_snapshot() {
    let ctx = TestContext::new().await;

    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: None,
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::LibraryScan {
                session_id: "session-1".to_string(),
            },
            payload: DomainEventPayload::LibraryScanStarted(LibraryScanStartedEventData {
                session_id: "session-1".to_string(),
                mode: "full".to_string(),
            }),
        })
        .await
        .expect("append library scan started event");
    ctx.app
        .append_domain_event(NewDomainEvent {
            event_id: Id::new().0,
            occurred_at: Utc::now(),
            actor_user_id: None,
            title_id: None,
            facet: Some(MediaFacet::Series),
            correlation_id: None,
            causation_id: None,
            schema_version: 1,
            stream: DomainEventStream::LibraryScan {
                session_id: "session-1".to_string(),
            },
            payload: DomainEventPayload::LibraryScanProgressed(LibraryScanProgressedEventData {
                session_id: "session-1".to_string(),
                status: "running".to_string(),
                found_titles: 12,
                title_match_completed: 7,
                title_match_total_known: false,
                titles_completed: 2,
                titles_total: Some(4),
                files_completed: 5,
                files_total: Some(9),
            }),
        })
        .await
        .expect("append library scan progressed event");

    let body = gql(
        &ctx,
        r#"query { activeLibraryScans { sessionId facet status foundTitles titleMatchTotalKnown titleMatchProgress { total completed failed } hydrationProgress { total completed failed } mediaAnalysisProgress { total completed failed } } }"#,
        json!({}),
    )
    .await;

    assert_no_errors(&body);
    let scans = body["data"]["activeLibraryScans"]
        .as_array()
        .expect("activeLibraryScans should be an array");
    assert_eq!(scans.len(), 1);
    assert_eq!(scans[0]["sessionId"], "session-1");
    assert_eq!(scans[0]["facet"], "tv");
    assert_eq!(scans[0]["status"], "running");
    assert_eq!(scans[0]["foundTitles"], 12);
    assert_eq!(scans[0]["titleMatchTotalKnown"], false);
    assert_eq!(scans[0]["titleMatchProgress"]["total"], 12);
    assert_eq!(scans[0]["titleMatchProgress"]["completed"], 7);
    assert_eq!(scans[0]["titleMatchProgress"]["failed"], 0);
    assert_eq!(scans[0]["hydrationProgress"]["total"], 4);
    assert_eq!(scans[0]["hydrationProgress"]["completed"], 2);
    assert_eq!(scans[0]["hydrationProgress"]["failed"], 0);
    assert_eq!(scans[0]["mediaAnalysisProgress"]["total"], 9);
    assert_eq!(scans[0]["mediaAnalysisProgress"]["completed"], 5);
    assert_eq!(scans[0]["mediaAnalysisProgress"]["failed"], 0);
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
