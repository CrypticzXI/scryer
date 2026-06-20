use super::*;

#[tokio::test]
async fn create_backup_requires_password_argument() {
    let ctx = TestContext::new().await;

    let body = schema_exec(
        &ctx,
        r#"
        mutation CreateBackup {
          createBackup(input: {}) {
            filename
          }
        }
        "#,
        None,
    )
    .await;

    let errors = body["errors"].as_array().expect("graphql errors");
    assert!(
        errors
            .first()
            .and_then(|error| error["message"].as_str())
            .is_some_and(|message| message.contains("password")),
        "expected missing password error: {body}"
    );
    assert!(backup_dir_is_empty(&ctx));
}

#[tokio::test]
async fn create_backup_rejects_blank_passwords_without_queuing_backup() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");

    for password_literal in ["\"\"", "\"   \""] {
        let body = schema_exec(
            &ctx,
            &format!(
                r#"
                mutation CreateBackup {{
                  createBackup(input: {{ password: {password_literal} }}) {{
                    filename
                  }}
                }}
                "#
            ),
            Some(admin.clone()),
        )
        .await;

        let errors = body["errors"].as_array().expect("graphql errors");
        assert!(
            errors
                .first()
                .and_then(|error| error["message"].as_str())
                .is_some_and(|message| message.contains("backup password is required")),
            "expected blank password validation error: {body}"
        );
        assert!(backup_dir_is_empty(&ctx));
    }
}

#[tokio::test]
async fn prepare_backup_download_returns_signed_url_for_ready_backup() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");
    write_backup_fixture(
        &ctx,
        BackupInfo {
            filename: "backup_20260515_abcd1234.tar.zst".to_string(),
            size_bytes: 42,
            created_at: Utc::now().to_rfc3339(),
            format_version: "scryer-backup-bundle-v2".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: Some("0122".to_string()),
            encrypted: false,
            row_counts: BTreeMap::from([("settings_definitions".to_string(), 1)]),
            trigger: BackupTrigger::Manual,
            status: BackupStatus::Ready,
            error_message: None,
        },
        b"ready-backup",
    );

    let body = schema_exec(
        &ctx,
        r#"
        mutation PrepareBackupDownload {
          prepareBackupDownload(input: { filename: "backup_20260515_abcd1234.tar.zst" }) {
            downloadUrl
            downloadAuthorizationToken
            expiresAt
          }
        }
        "#,
        Some(admin),
    )
    .await;

    assert_no_errors(&body);
    let download_url = body["data"]["prepareBackupDownload"]["downloadUrl"]
        .as_str()
        .expect("download url should be present");
    assert_eq!(
        download_url,
        "/backups/backup_20260515_abcd1234.tar.zst/download"
    );
    let token = body["data"]["prepareBackupDownload"]["downloadAuthorizationToken"]
        .as_str()
        .expect("download authorization token should be present");
    assert!(!token.trim().is_empty());
    assert!(!download_url.contains("ticket="));
    assert!(
        body["data"]["prepareBackupDownload"]["expiresAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

#[tokio::test]
async fn prepare_backup_download_percent_encodes_reserved_filename_characters() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");
    let filename = "backup 2026 #%?.tar.zst";
    write_backup_fixture(
        &ctx,
        BackupInfo {
            filename: filename.to_string(),
            size_bytes: 42,
            created_at: Utc::now().to_rfc3339(),
            format_version: "scryer-backup-bundle-v2".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: Some("0122".to_string()),
            encrypted: false,
            row_counts: BTreeMap::from([("settings_definitions".to_string(), 1)]),
            trigger: BackupTrigger::Manual,
            status: BackupStatus::Ready,
            error_message: None,
        },
        b"ready-backup-reserved",
    );
    let filename_literal = serde_json::to_string(filename).expect("serialize filename");
    let query = format!(
        r#"
        mutation PrepareBackupDownload {{
          prepareBackupDownload(input: {{ filename: {filename_literal} }}) {{
            downloadUrl
            downloadAuthorizationToken
            expiresAt
          }}
        }}
        "#
    );

    let body = schema_exec(&ctx, &query, Some(admin)).await;

    assert_no_errors(&body);
    let download_url = body["data"]["prepareBackupDownload"]["downloadUrl"]
        .as_str()
        .expect("download url should be present");
    assert_eq!(
        download_url, "/backups/backup%202026%20%23%25%3F.tar.zst/download",
        "expected percent-encoded path segment without query ticket"
    );
    assert!(
        body["data"]["prepareBackupDownload"]["downloadAuthorizationToken"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
}

#[tokio::test]
async fn prepare_backup_download_requires_manage_system_settings() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");
    let viewer = ctx
        .app
        .create_user(
            &admin,
            "backup_viewer".to_string(),
            "password123".to_string(),
            scryer_domain::AppPermissionMask::NONE,
            vec![],
        )
        .await
        .expect("create limited user");
    let viewer = ctx
        .app
        .attach_user_authorization(viewer)
        .await
        .expect("viewer authorization");

    let body = schema_exec(
        &ctx,
        r#"
        mutation PrepareBackupDownload {
          prepareBackupDownload(input: { filename: "backup_20260515_abcd1234.tar.zst" }) {
            downloadUrl
            expiresAt
          }
        }
        "#,
        Some(viewer),
    )
    .await;

    assert!(
        body["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected authorization error: {body}"
    );
}

#[tokio::test]
async fn prepare_backup_download_rejects_missing_file_and_non_ready_backup() {
    let ctx = TestContext::new().await;
    let admin = ctx
        .app
        .attach_user_authorization(
            ctx.app
                .find_or_create_default_user()
                .await
                .expect("default user should exist"),
        )
        .await
        .expect("default user authorization");

    write_backup_fixture(
        &ctx,
        BackupInfo {
            filename: "backup_20260515_missing.tar.zst".to_string(),
            size_bytes: 42,
            created_at: Utc::now().to_rfc3339(),
            format_version: "scryer-backup-bundle-v2".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: Some("0122".to_string()),
            encrypted: false,
            row_counts: BTreeMap::new(),
            trigger: BackupTrigger::Manual,
            status: BackupStatus::Ready,
            error_message: None,
        },
        b"missing-backup",
    );
    std::fs::remove_file(ctx.app.backup_dir().join("backup_20260515_missing.tar.zst"))
        .expect("remove bundle file");

    let missing_file = schema_exec(
        &ctx,
        r#"
        mutation PrepareBackupDownload {
          prepareBackupDownload(input: { filename: "backup_20260515_missing.tar.zst" }) {
            downloadUrl
            expiresAt
          }
        }
        "#,
        Some(admin.clone()),
    )
    .await;
    assert!(
        missing_file["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected missing bundle error: {missing_file}"
    );

    write_backup_fixture(
        &ctx,
        BackupInfo {
            filename: "backup_20260515_creating.tar.zst".to_string(),
            size_bytes: 42,
            created_at: Utc::now().to_rfc3339(),
            format_version: "scryer-backup-bundle-v2".to_string(),
            source_scryer_version: "0.15.0".to_string(),
            source_engine: "sqlite".to_string(),
            source_migration_key: Some("0122".to_string()),
            encrypted: false,
            row_counts: BTreeMap::new(),
            trigger: BackupTrigger::Manual,
            status: BackupStatus::Creating,
            error_message: None,
        },
        b"creating-backup",
    );

    let creating = schema_exec(
        &ctx,
        r#"
        mutation PrepareBackupDownload {
          prepareBackupDownload(input: { filename: "backup_20260515_creating.tar.zst" }) {
            downloadUrl
            expiresAt
          }
        }
        "#,
        Some(admin),
    )
    .await;
    assert!(
        creating["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected non-ready backup error: {creating}"
    );
}
