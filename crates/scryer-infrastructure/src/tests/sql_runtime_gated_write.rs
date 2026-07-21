use super::*;
use crate::queries::sql_runtime::{SqlArg, SqlRuntime};

/// End-to-end proof that `execute_write` routes single-statement writes through
/// the sqlite writer gate + transaction machinery:
///
/// 1. while the gate is held a gated write cannot touch the database, and it
///    lands once the gate is released; and
/// 2. a placeholder/argument arity mismatch surfaces as an immediate error
///    instead of spinning in the sqlite busy-retry loop.
///
/// Both checks share one in-memory services handle: `sqlite://:memory:` opens a
/// shared-cache database, so a second concurrent opener in the same test binary
/// would collide on the schema — a single handle keeps the test isolated.
#[tokio::test]
async fn execute_write_is_gated_and_rejects_arity_mismatch() {
    let services = SqliteServices::new("sqlite://:memory:")
        .await
        .expect("in-memory services should initialize");

    // Scratch table created directly through the pool (no gated writer is
    // contending yet, so the read executor is fine here).
    sqlx::query("CREATE TABLE gated_write_probe (id TEXT PRIMARY KEY, note TEXT NOT NULL)")
        .execute(services.pool())
        .await
        .expect("scratch table should create");

    let datastore = services.datastore();

    // --- Gate blocks the write; releasing it lets the write land -------------

    // The datastore shares the exact same writer-gate `Arc<Mutex>` as the
    // services handle, so holding `writer_gate()` blocks any `execute_write`.
    let gate = services.writer_gate();
    let guard = gate.lock().await;

    let write_datastore = datastore.clone();
    let mut write_task = tokio::spawn(async move {
        SqlRuntime::execute_write(
            &write_datastore,
            "insert_gated_write_probe",
            "INSERT INTO gated_write_probe (id, note) VALUES ({}, {})",
            &[
                SqlArg::Text("row-1".to_string()),
                SqlArg::Text("hello".to_string()),
            ],
        )
        .await
    });

    // With the gate held the spawned write cannot make progress: the timeout
    // must expire with the task still pending.
    assert!(
        timeout(Duration::from_millis(250), &mut write_task)
            .await
            .is_err(),
        "execute_write must not complete while the writer gate is held"
    );

    // ...and nothing was persisted while the gate was held.
    let count_while_held: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM gated_write_probe")
        .fetch_one(services.pool())
        .await
        .expect("probe count should read");
    assert_eq!(
        count_while_held, 0,
        "no row should be persisted while the writer gate is held"
    );

    // Release the gate; the write now completes and reports one affected row.
    drop(guard);
    let rows_affected = write_task
        .await
        .expect("spawned write task should join")
        .expect("execute_write should succeed once the gate is free");
    assert_eq!(
        rows_affected, 1,
        "execute_write should affect exactly one row"
    );

    let note: String = sqlx::query_scalar("SELECT note FROM gated_write_probe WHERE id = ?")
        .bind("row-1")
        .fetch_one(services.pool())
        .await
        .expect("row should be present after the gate is released");
    assert_eq!(note, "hello");

    // --- Arity mismatch fails fast rather than retrying forever --------------

    // Two `{}` placeholders but only one bound argument. The `timeout` guards
    // against a regression that would retry the mismatch instead of failing; the
    // mismatch is a non-transient `Repository` error rejected before any SQL is
    // executed.
    let outcome = timeout(
        Duration::from_secs(5),
        SqlRuntime::execute_write(
            &datastore,
            "arity_mismatch_probe",
            "INSERT INTO gated_write_probe (id, note) VALUES ({}, {})",
            &[SqlArg::Text("only-one".to_string())],
        ),
    )
    .await
    .expect("execute_write must return promptly instead of retrying forever");

    match outcome {
        Err(AppError::Repository(message)) => assert!(
            message.contains("placeholder mismatch"),
            "expected a placeholder mismatch error, got: {message}"
        ),
        other => panic!("expected a Repository placeholder-mismatch error, got {other:?}"),
    }
}
