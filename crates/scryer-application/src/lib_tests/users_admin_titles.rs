use super::*;

#[tokio::test]
async fn create_user_and_list_users() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(users.iter().any(|entry| entry.username == created.username));
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn create_user_without_permission_grants_allows_manage_users_only_actor() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let created = app
        .create_user(
            &actor,
            "plain-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await
        .expect("create user without grants");

    assert_eq!(created.username, "plain-user");
}

#[tokio::test]
async fn create_user_with_app_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "privileged-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::MANAGE_SYSTEM_SETTINGS,
            Vec::new(),
        )
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn create_user_with_library_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "library-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                permissions: scryer_domain::LibraryPermissionMask::VIEW,
            }],
        )
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn get_user_by_id_returns_created_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "viewer",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let found = app.get_user(&user, &created.id).await.expect("get user");

    assert!(found.is_some());
    let found = found.expect("user should exist");
    assert_eq!(found.id, created.id);
    assert_eq!(found.username, "viewer");
}

#[tokio::test]
async fn create_user_rejects_duplicate_username() {
    let (app, user) = bootstrap();

    let _created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("first create");

    let second = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    assert!(second.is_err());
}

#[tokio::test]
async fn create_user_rejects_recovery_admin_username() {
    let (app, user) = bootstrap();

    let result = app
        .create_user(
            &user,
            "recovery-admin".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await;

    assert!(matches!(result, Err(AppError::Validation(_))));
}

#[tokio::test]
async fn create_user_rejects_anonymous_username() {
    let (app, user) = bootstrap();

    let result = app
        .create_user(
            &user,
            "Anonymous".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await;

    let Err(AppError::Validation(message)) = result else {
        panic!("anonymous username should be reserved");
    };
    assert!(message.contains("anonymous is reserved"));
}

#[tokio::test]
async fn ensure_default_admin_rejects_anonymous_username() {
    let (app, _) = bootstrap();

    let result = app.ensure_default_admin("anonymous", "password123").await;

    let Err(AppError::Validation(message)) = result else {
        panic!("anonymous default admin username should be reserved");
    };
    assert!(message.contains("anonymous is reserved"));
}

#[tokio::test]
async fn delete_title_removes_title_from_catalog() {
    let (app, user) = bootstrap();

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    let titles = app
        .list_titles_unpaged(&user, Some(MediaFacet::Movie), None, None)
        .await
        .expect("list titles");
    assert!(titles.is_empty());
}

#[tokio::test]
async fn start_delete_titles_job_requires_preview_for_disk_delete() {
    let (app, user) = bootstrap();

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Keep Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let result = app
        .start_delete_titles_job(
            &user,
            DeleteTitlesJobRequest {
                items: vec![DeleteTitlesJobItem {
                    title_id: created.id.clone(),
                    preview_fingerprint: None,
                }],
                delete_files_on_disk: true,
                typed_confirmation: None,
            },
        )
        .await;

    assert!(result.is_err());
    assert!(
        app.get_title(&user, &created.id)
            .await
            .expect("get title")
            .is_some()
    );
}

#[tokio::test]
async fn title_deletion_job_runs_are_visible_to_actor_without_system_settings() {
    let job_runs = Arc::new(RecordingJobRunRepo::default());
    let (base_app, admin) = bootstrap();
    let app = base_app.with_test_overrides(|services| services.with_job_runs(job_runs.clone()));
    let manager = create_user_with_permissions(
        &app,
        &admin,
        "title-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create manager");
    let other_manager = create_user_with_permissions(
        &app,
        &admin,
        "other-title-manager",
        "password123",
        vec![TestPermissionPreset::TitleManagement],
    )
    .await
    .expect("create other manager");
    let now = chrono::Utc::now();
    let make_run = |id: &str, job_key: JobKey, actor_user_id: Option<String>| JobRunRecord {
        id: id.to_string(),
        job_key,
        operation_type: format!("{}:test", job_key.as_str()),
        status: JobRunStatus::Completed,
        trigger_source: JobTriggerSource::Manual,
        actor_user_id,
        progress_json: None,
        summary_json: None,
        summary_text: None,
        error_text: None,
        started_at: now,
        completed_at: Some(now),
        created_at: now,
        updated_at: now,
    };

    job_runs
        .seed(make_run(
            "own-title-delete",
            JobKey::TitleDeletion,
            Some(manager.id.clone()),
        ))
        .await;
    job_runs
        .seed(make_run(
            "other-title-delete",
            JobKey::TitleDeletion,
            Some(other_manager.id.clone()),
        ))
        .await;
    job_runs
        .seed(make_run(
            "own-housekeeping",
            JobKey::Housekeeping,
            Some(manager.id.clone()),
        ))
        .await;

    let manager_runs = app
        .list_job_runs(&manager, JobKey::TitleDeletion, 10)
        .await
        .expect("manager can list own title deletion runs");
    assert_eq!(manager_runs.len(), 1);
    assert_eq!(manager_runs[0].id, "own-title-delete");

    let denied = app.list_job_runs(&manager, JobKey::Housekeeping, 10).await;
    assert!(matches!(denied, Err(AppError::Unauthorized(_))));

    let admin_runs = app
        .list_job_runs(&admin, JobKey::TitleDeletion, 10)
        .await
        .expect("admin can list all title deletion runs");
    let admin_run_ids = admin_runs
        .into_iter()
        .map(|run| run.id)
        .collect::<HashSet<_>>();
    assert!(admin_run_ids.contains("own-title-delete"));
    assert!(admin_run_ids.contains("other-title-delete"));
}

#[tokio::test]
async fn delete_title_queues_targeted_cancel_for_active_submission_only() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking_and_queue_commands(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        download_queue_commands.clone(),
    );

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let active_submission = DownloadSubmission {
        title_id: created.id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "queue-active".to_string(),
        source_hint: None,
        source_kind: None,
        source_title: Some(created.name.clone()),
        request_signature: None,
        scope: SubmissionScope::Title,
    };
    let terminal_submission = DownloadSubmission {
        title_id: created.id.clone(),
        purpose: crate::DownloadSubmissionPurpose::Standard,
        facet: "movie".to_string(),
        download_client_id: Some("primary".to_string()),
        download_client_type: "sabnzbd".to_string(),
        download_client_item_id: "queue-imported".to_string(),
        source_hint: None,
        source_kind: None,
        source_title: Some(created.name.clone()),
        request_signature: None,
        scope: SubmissionScope::Title,
    };
    download_submissions
        .record_submission(active_submission.clone())
        .await
        .expect("record active submission");
    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::from_submission(&active_submission),
            "downloading",
        )
        .await
        .expect("track active submission");
    download_submissions
        .record_submission(terminal_submission.clone())
        .await
        .expect("record terminal submission");
    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::from_submission(&terminal_submission),
            "imported",
        )
        .await
        .expect("track terminal submission");

    *download_client.queue_items.lock().await = vec![
        DownloadQueueItem {
            id: "queue-direct".to_string(),
            title_id: Some(created.id.clone()),
            episode_id: None,
            title_name: created.name.clone(),
            facet: Some("movie".to_string()),
            category: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-direct".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-fallback".to_string(),
            title_id: None,
            episode_id: None,
            title_name: created.name.clone(),
            facet: None,
            category: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "sabnzbd".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-active".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-unrelated".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Other".to_string(),
            facet: None,
            category: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "sabnzbd".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            import_transfer_phase: None,
            import_transfer_bytes: None,
            import_transfer_total_bytes: None,
            import_transfer_started_at: None,
            import_transfer_updated_at: None,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-unrelated".to_string(),
            download_id: None,
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
    ];

    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    assert_eq!(*download_client.queue_calls.lock().await, 0);
    assert_eq!(*download_client.history_calls.lock().await, 0);
    assert!(download_client.deleted_items.lock().await.is_empty());
    let queued_commands = download_queue_commands.queued.lock().await.clone();
    assert_eq!(queued_commands.len(), 1);
    assert_eq!(queued_commands[0].client_id.as_deref(), Some("primary"));
    assert_eq!(queued_commands[0].client_type, "sabnzbd");
    assert_eq!(queued_commands[0].download_client_item_id, "queue-active");
    assert!(!queued_commands[0].is_history);
    assert_eq!(
        queued_commands[0].requested_by_user_id.as_deref(),
        Some(user.id.as_str())
    );
    assert_eq!(
        pending_releases.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert_eq!(
        download_submissions.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .all(|entry| entry.title_id != created.id)
    );
}
