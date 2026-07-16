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

    // Prune drops the scope entirely.
    store.prune_scope("ep-1").await.unwrap();
    assert!(
        store
            .covered_indexers("ep-1", "series", "fp-b", None)
            .await
            .unwrap()
            .is_empty()
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

#[tokio::test]
async fn prune_orphaned_coverage_is_a_noop_when_entity_tables_are_empty() {
    // Safety guard: a transiently-empty entity/indexer table must never wipe live
    // coverage. Every GC arm is gated on `EXISTS (...)`, so with all tables empty the
    // sweep deletes nothing. (This also smoke-tests the GC statement — wrong table
    // name or bad SQL would error here.)
    let (services, db) = temp_services("scryer_scope_coverage_gc_guard").await;
    let store = ScopeIndexerCoverageStore::new(services.datastore());
    store
        .record_coverage("title:t1", "movie", "idx-1", "fp")
        .await
        .unwrap();
    store
        .record_coverage("episode:e1", "series", "idx-2", "fp")
        .await
        .unwrap();

    store.prune_orphaned_coverage().await.unwrap();

    assert_eq!(
        store
            .covered_indexers("title:t1", "movie", "fp", None)
            .await
            .unwrap(),
        vec!["idx-1".to_string()]
    );
    assert_eq!(
        store
            .covered_indexers("episode:e1", "series", "fp", None)
            .await
            .unwrap(),
        vec!["idx-2".to_string()]
    );

    let _ = std::fs::remove_file(db);
}

#[tokio::test]
async fn prune_orphaned_coverage_drops_dead_indexers_but_keeps_packs() {
    use crate::queries::sql_runtime::{SqlArg, SqlRuntime};

    let (services, db) = temp_services("scryer_scope_coverage_gc").await;
    let store = ScopeIndexerCoverageStore::new(services.datastore());

    // One live indexer exists, activating the indexer-orphan guard.
    SqlRuntime::execute(
        services.datastore().read_exec(),
        "INSERT INTO indexers (id, name, provider_type, base_url, is_enabled, created_at, updated_at)
         VALUES ({}, {}, {}, {}, 1, {}, {})",
        &[
            SqlArg::Text("idx-live".to_string()),
            SqlArg::Text("Live".to_string()),
            SqlArg::Text("newznab".to_string()),
            SqlArg::Text("https://example.invalid".to_string()),
            SqlArg::Text("1970-01-01T00:00:00Z".to_string()),
            SqlArg::Text("1970-01-01T00:00:00Z".to_string()),
        ],
    )
    .await
    .unwrap();

    // Coverage for a live indexer (kept) and a dead indexer (swept), plus a
    // season-pack scope whose coverage must never be swept (hash key, no entity).
    store
        .record_coverage("title:t-live", "movie", "idx-live", "fp")
        .await
        .unwrap();
    store
        .record_coverage("title:t-live", "movie", "idx-dead", "fp")
        .await
        .unwrap();
    store
        .record_coverage("episode_set:packhash", "series", "idx-live", "fp")
        .await
        .unwrap();

    store.prune_orphaned_coverage().await.unwrap();

    // The dead indexer's coverage is swept; the live indexer's survives. (titles is
    // empty, so the title-scope arm is guarded off and never touches title:t-live.)
    assert_eq!(
        store
            .covered_indexers("title:t-live", "movie", "fp", None)
            .await
            .unwrap(),
        vec!["idx-live".to_string()],
        "coverage for a deleted indexer is swept; the live indexer is kept"
    );
    // episode_set: (pack) coverage is never GC'd.
    assert_eq!(
        store
            .covered_indexers("episode_set:packhash", "series", "fp", None)
            .await
            .unwrap(),
        vec!["idx-live".to_string()],
        "episode_set (pack) coverage is never swept"
    );

    let _ = std::fs::remove_file(db);
}
