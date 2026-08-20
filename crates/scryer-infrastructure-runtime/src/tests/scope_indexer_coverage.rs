use super::*;
use crate::indexers::scope_indexer_coverage_store::ScopeIndexerCoverageStore;
use scryer_application::ScopeIndexerCoverageRepository;

#[tokio::test]
async fn coverage_records_and_reads_by_fingerprint() {
    let (services, db) = temp_services("scryer_scope_coverage").await;
    let store = ScopeIndexerCoverageStore::new(services.datastore());

    // No coverage initially.
    assert!(
        store
            .covered_indexers("ep-1", "series", "fp-a", None)
            .await
            .unwrap()
            .is_empty()
    );

    // Record two indexers under fingerprint fp-a.
    store
        .record_coverage("ep-1", "series", "idx-1", "fp-a")
        .await
        .unwrap();
    store
        .record_coverage("ep-1", "series", "idx-2", "fp-a")
        .await
        .unwrap();

    let mut covered = store
        .covered_indexers("ep-1", "series", "fp-a", None)
        .await
        .unwrap();
    covered.sort();
    assert_eq!(covered, vec!["idx-1".to_string(), "idx-2".to_string()]);

    let rows = store
        .list_coverage_for_scope_keys(&["ep-1".to_string()])
        .await
        .expect("coverage rows should decode");
    assert_eq!(rows.len(), 2);
    for row in rows {
        chrono::DateTime::parse_from_rfc3339(&row.searched_at)
            .expect("coverage timestamps should use portable RFC 3339 encoding");
    }

    // A different fingerprint reads as uncovered (stale coverage).
    assert!(
        store
            .covered_indexers("ep-1", "series", "fp-b", None)
            .await
            .unwrap()
            .is_empty()
    );

    // Re-search idx-1 under fp-b overwrites its row (upsert on the PK): it now
    // counts for fp-b and no longer for fp-a.
    store
        .record_coverage("ep-1", "series", "idx-1", "fp-b")
        .await
        .unwrap();
    assert_eq!(
        store
            .covered_indexers("ep-1", "series", "fp-b", None)
            .await
            .unwrap(),
        vec!["idx-1".to_string()]
    );
    assert_eq!(
        store
            .covered_indexers("ep-1", "series", "fp-a", None)
            .await
            .unwrap(),
        vec!["idx-2".to_string()]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn coverage_stale_before_excludes_old_rows() {
    let (services, db) = temp_services("scryer_scope_coverage_stale").await;
    let store = ScopeIndexerCoverageStore::new(services.datastore());
    store
        .record_coverage("movie-1", "movie", "idx-1", "fp")
        .await
        .unwrap();

    // stale_before in the future ⇒ the just-written row is "too old" ⇒ excluded
    // (this is the optional slow re-converge backstop treating it as uncovered).
    let future = chrono::Utc::now() + chrono::Duration::hours(1);
    assert!(
        store
            .covered_indexers("movie-1", "movie", "fp", Some(future))
            .await
            .unwrap()
            .is_empty()
    );

    // stale_before in the past ⇒ the row still counts.
    let past = chrono::Utc::now() - chrono::Duration::hours(1);
    assert_eq!(
        store
            .covered_indexers("movie-1", "movie", "fp", Some(past))
            .await
            .unwrap(),
        vec!["idx-1".to_string()]
    );

    let _ = std::fs::remove_file(db);
}
