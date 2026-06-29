use super::*;
use scryer_application::{
    ExternalImportSetupInstanceApiKeyDraft, ExternalImportSetupSecretDraftInput,
    ExternalImportSetupSecretDraftRepository, ExternalImportSetupSecretInstanceKind,
    ExternalImportSetupSecretOverrideDraft, UserRepository,
};
use scryer_domain::User;

fn set_test_encryption_key(services: &SqliteServices) {
    *services
        .encryption_key_state()
        .write()
        .expect("encryption key lock should not be poisoned") =
        Some(crate::encryption::EncryptionKey::from_bytes([17; 32]));
}

fn secret_draft(label: &str) -> ExternalImportSetupSecretDraftInput {
    ExternalImportSetupSecretDraftInput {
        instance_api_keys: vec![ExternalImportSetupInstanceApiKeyDraft {
            instance_id: format!("sonarr-{label}"),
            kind: ExternalImportSetupSecretInstanceKind::Sonarr,
            api_key: format!("sonarr-key-{label}"),
        }],
        download_client_api_key_overrides: vec![ExternalImportSetupSecretOverrideDraft {
            dedup_key: format!("download-client-api-{label}"),
            secret: format!("download-client-api-key-{label}"),
        }],
        download_client_password_overrides: vec![ExternalImportSetupSecretOverrideDraft {
            dedup_key: format!("download-client-password-{label}"),
            secret: format!("download-client-password-{label}"),
        }],
        indexer_api_key_overrides: vec![ExternalImportSetupSecretOverrideDraft {
            dedup_key: format!("indexer-api-{label}"),
            secret: format!("indexer-api-key-{label}"),
        }],
    }
}

fn duplicate_instance_draft(label: &str) -> ExternalImportSetupSecretDraftInput {
    ExternalImportSetupSecretDraftInput {
        instance_api_keys: vec![
            ExternalImportSetupInstanceApiKeyDraft {
                instance_id: format!("duplicate-{label}"),
                kind: ExternalImportSetupSecretInstanceKind::Sonarr,
                api_key: format!("first-{label}"),
            },
            ExternalImportSetupInstanceApiKeyDraft {
                instance_id: format!("duplicate-{label}"),
                kind: ExternalImportSetupSecretInstanceKind::Radarr,
                api_key: format!("second-{label}"),
            },
        ],
        ..ExternalImportSetupSecretDraftInput::default()
    }
}

async fn create_users(services: &SqliteServices) -> (User, User) {
    let users = user_store(services);
    let owner = users
        .create(User::new_admin("external-import-secret-owner"))
        .await
        .expect("owner user should create");
    let other = users
        .create(User::new_admin("external-import-secret-other"))
        .await
        .expect("other user should create");
    (owner, other)
}

fn draft_store(services: &SqliteServices) -> ExternalImportSetupSecretDraftStore {
    ExternalImportSetupSecretDraftStore::new(services.datastore(), services.encryption_key_state())
}

async fn count_rows(services: &SqliteServices, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
        .fetch_one(&services.pool)
        .await
        .expect("row count should be readable")
}

#[tokio::test]
async fn secret_draft_store_replaces_singleton_and_keeps_secrets_owner_scoped() {
    let (services, _db) = temp_services("external_import_secret_drafts").await;
    set_test_encryption_key(&services);
    let (owner, other) = create_users(&services).await;
    let store = draft_store(&services);

    let first = secret_draft("first");
    let saved = store
        .save_for_owner(&owner.id, first.clone())
        .await
        .expect("owner draft should save");
    assert!(saved.saved);
    assert!(!saved.overwrote_another_user_draft);
    assert_eq!(
        saved.updated_at,
        store
            .status_for_actor(&owner.id)
            .await
            .unwrap()
            .updated_at
            .unwrap()
    );

    let owner_read = store
        .get_for_owner(&owner.id)
        .await
        .expect("owner read should succeed")
        .expect("owner draft should be visible");
    assert_eq!(owner_read.owner_user_id, owner.id);
    assert_eq!(owner_read.secrets, first);
    assert!(
        store
            .get_for_owner(&other.id)
            .await
            .expect("other read should succeed")
            .is_none()
    );

    let other_status = store
        .status_for_actor(&other.id)
        .await
        .expect("status should load");
    assert!(other_status.has_draft);
    assert!(!other_status.owned_by_current_user);
    assert_eq!(other_status.updated_at, Some(saved.updated_at));

    let raw_instance_secret: String = sqlx::query_scalar(
        "SELECT api_key_encrypted
           FROM external_import_setup_instance_api_keys
          WHERE instance_id = 'sonarr-first'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("encrypted instance key should be stored");
    assert_ne!(raw_instance_secret, "sonarr-key-first");
    assert!(raw_instance_secret.starts_with("enc:v1:"));

    let raw_password_secret: String = sqlx::query_scalar(
        "SELECT password_encrypted
           FROM external_import_setup_download_client_password_overrides
          WHERE dedup_key = 'download-client-password-first'",
    )
    .fetch_one(&services.pool)
    .await
    .expect("encrypted download client password should be stored");
    assert_ne!(raw_password_secret, "download-client-password-first");
    assert!(raw_password_secret.starts_with("enc:v1:"));

    let failed_overwrite = store
        .save_for_owner(&other.id, duplicate_instance_draft("failed"))
        .await;
    assert!(failed_overwrite.is_err());
    let owner_after_failed_overwrite = store
        .get_for_owner(&owner.id)
        .await
        .expect("owner read after failed overwrite should succeed")
        .expect("previous owner draft should survive failed overwrite");
    assert_eq!(owner_after_failed_overwrite.secrets, first);

    let second = secret_draft("second");
    let overwritten = store
        .save_for_owner(&other.id, second.clone())
        .await
        .expect("other user should overwrite singleton draft");
    assert!(overwritten.saved);
    assert!(overwritten.overwrote_another_user_draft);

    assert!(
        store
            .get_for_owner(&owner.id)
            .await
            .expect("owner read should succeed")
            .is_none()
    );
    let other_read = store
        .get_for_owner(&other.id)
        .await
        .expect("other read should succeed")
        .expect("other draft should be visible");
    assert_eq!(other_read.owner_user_id, other.id);
    assert_eq!(other_read.secrets, second);
    assert_eq!(
        count_rows(&services, "external_import_setup_secret_drafts").await,
        1
    );
    assert_eq!(
        count_rows(&services, "external_import_setup_instance_api_keys").await,
        1
    );

    assert!(
        !store
            .clear_for_owner(&owner.id)
            .await
            .expect("non-owner clear should be a no-op")
    );
    assert!(
        store
            .status_for_actor(&other.id)
            .await
            .expect("other status should load")
            .owned_by_current_user
    );

    assert!(
        store
            .clear_for_owner(&other.id)
            .await
            .expect("owner clear should delete draft")
    );
    let empty_status = store
        .status_for_actor(&other.id)
        .await
        .expect("empty status should load");
    assert!(!empty_status.has_draft);
    assert_eq!(
        count_rows(&services, "external_import_setup_secret_drafts").await,
        0
    );
    assert_eq!(
        count_rows(
            &services,
            "external_import_setup_download_client_password_overrides"
        )
        .await,
        0
    );
}

#[tokio::test]
async fn secret_draft_store_requires_encryption_key_to_save() {
    let (services, _db) = temp_services("external_import_secret_drafts_no_key").await;
    let (owner, _) = create_users(&services).await;
    let store = draft_store(&services);

    let error = store
        .save_for_owner(&owner.id, secret_draft("missing-key"))
        .await
        .expect_err("save should fail without an encryption key");
    assert!(
        error
            .to_string()
            .contains("external import setup instance API key encryption requires encryption key"),
        "{error}"
    );
    assert_eq!(
        count_rows(&services, "external_import_setup_secret_drafts").await,
        0
    );
}
