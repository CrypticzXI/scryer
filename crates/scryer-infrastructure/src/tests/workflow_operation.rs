use super::*;
use crate::workflow::stores::WorkflowOperationStore;
use scryer_application::{
    JobKey, JobRunRecord, JobRunRepository, JobRunStatus, JobTriggerSource, UserRepository,
};
use scryer_domain::User;

fn workflow_operation_store(services: &SqliteServices) -> WorkflowOperationStore {
    WorkflowOperationStore::new(services.datastore())
}

fn test_job_run_record(
    id: &str,
    job_key: JobKey,
    status: JobRunStatus,
    started_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
    actor_user_id: Option<&str>,
) -> JobRunRecord {
    let completed_at = status.is_terminal().then(|| started_at.clone());
    JobRunRecord {
        id: id.to_string(),
        job_key,
        operation_type: format!("{}:test", job_key.as_str()),
        status,
        trigger_source: JobTriggerSource::Manual,
        actor_user_id: actor_user_id.map(str::to_string),
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at,
        completed_at,
        created_at: created_at.clone(),
        updated_at: created_at,
    }
}

async fn seed_user(users: &UserStore, id: &str) {
    users
        .create(User {
            id: id.to_string(),
            username: id.to_string(),
            password_hash: None,
            account_kind: Default::default(),
            authorization: Default::default(),
        })
        .await
        .expect("seed user");
}

#[tokio::test]
async fn job_run_history_orders_by_started_at_not_created_at() {
    let (services, _db) = temp_services("workflow_operation_started_order").await;
    let store = workflow_operation_store(&services);
    let now = Utc::now();

    store
        .create_job_run(&test_job_run_record(
            "started-later",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now,
            now - chrono::Duration::seconds(60),
            None,
        ))
        .await
        .expect("create later-started run");
    store
        .create_job_run(&test_job_run_record(
            "started-earlier",
            JobKey::Housekeeping,
            JobRunStatus::Completed,
            now - chrono::Duration::seconds(60),
            now,
            None,
        ))
        .await
        .expect("create earlier-started run");

    let all_runs = store
        .list_job_runs(None, 10)
        .await
        .expect("list all job runs");
    let job_runs = store
        .list_job_runs(Some(JobKey::Housekeeping), 10)
        .await
        .expect("list per-job runs");

    assert_eq!(all_runs.len(), 2);
    assert_eq!(all_runs[0].started_at, now);
    assert_eq!(all_runs[1].started_at, now - chrono::Duration::seconds(60));
    assert_eq!(job_runs.len(), 2);
    assert_eq!(job_runs[0].started_at, now);
    assert_eq!(job_runs[1].started_at, now - chrono::Duration::seconds(60));
}

#[tokio::test]
async fn actor_job_run_history_remains_actor_scoped_and_started_ordered() {
    let (services, _db) = temp_services("workflow_operation_actor_order").await;
    let store = workflow_operation_store(&services);
    let users = user_store(&services);
    let now = Utc::now();
    seed_user(&users, "actor-a").await;
    seed_user(&users, "actor-b").await;

    store
        .create_job_run(&test_job_run_record(
            "actor-a-later",
            JobKey::TitleDeletion,
            JobRunStatus::Completed,
            now,
            now - chrono::Duration::seconds(120),
            Some("actor-a"),
        ))
        .await
        .expect("create actor a later run");
    store
        .create_job_run(&test_job_run_record(
            "actor-a-earlier",
            JobKey::TitleDeletion,
            JobRunStatus::Completed,
            now - chrono::Duration::seconds(60),
            now,
            Some("actor-a"),
        ))
        .await
        .expect("create actor a earlier run");
    store
        .create_job_run(&test_job_run_record(
            "actor-b-run",
            JobKey::TitleDeletion,
            JobRunStatus::Completed,
            now + chrono::Duration::seconds(60),
            now + chrono::Duration::seconds(60),
            Some("actor-b"),
        ))
        .await
        .expect("create actor b run");

    let actor_runs = store
        .list_job_runs_for_actor(Some(JobKey::TitleDeletion), "actor-a", 10)
        .await
        .expect("list actor job runs");

    assert_eq!(actor_runs.len(), 2);
    assert!(
        actor_runs
            .iter()
            .all(|run| run.actor_user_id.as_deref() == Some("actor-a"))
    );
    assert_eq!(actor_runs[0].started_at, now);
    assert_eq!(
        actor_runs[1].started_at,
        now - chrono::Duration::seconds(60)
    );
}

#[tokio::test]
async fn active_job_runs_order_by_started_at_ascending() {
    let (services, _db) = temp_services("workflow_operation_active_order").await;
    let store = workflow_operation_store(&services);
    let now = Utc::now();

    store
        .create_job_run(&test_job_run_record(
            "active-newer",
            JobKey::Housekeeping,
            JobRunStatus::Running,
            now,
            now,
            None,
        ))
        .await
        .expect("create newer active run");
    store
        .create_job_run(&test_job_run_record(
            "active-older",
            JobKey::LibraryScanMovies,
            JobRunStatus::Queued,
            now - chrono::Duration::seconds(60),
            now + chrono::Duration::seconds(60),
            None,
        ))
        .await
        .expect("create older active run");

    let active_runs = store
        .list_active_job_runs()
        .await
        .expect("list active job runs");

    assert_eq!(active_runs.len(), 2);
    assert_eq!(
        active_runs[0].started_at,
        now - chrono::Duration::seconds(60)
    );
    assert_eq!(active_runs[1].started_at, now);
}

#[tokio::test]
async fn migration_registers_job_run_listing_indexes() {
    let (services, _db) = temp_services("workflow_operation_indexes").await;
    let rows = sqlx::query("PRAGMA index_list('workflow_operations')")
        .fetch_all(&services.pool)
        .await
        .expect("list workflow operation indexes");
    let index_names = rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<HashSet<_>>();

    for expected in [
        "idx_workflow_operations_job_recent_started",
        "idx_workflow_operations_actor_recent_started",
        "idx_workflow_operations_actor_job_started",
        "idx_workflow_operations_active_job_started",
    ] {
        assert!(index_names.contains(expected), "missing index {expected}");
    }
}
