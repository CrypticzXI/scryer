use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ExternalImportSetupInstanceApiKeyDraft, ExternalImportSetupSecretDraft,
    ExternalImportSetupSecretDraftInput, ExternalImportSetupSecretDraftRepository,
    ExternalImportSetupSecretDraftSaveResult, ExternalImportSetupSecretDraftStatus,
    ExternalImportSetupSecretInstanceKind, ExternalImportSetupSecretOverrideDraft,
};

use crate::config_store::{current_encryption_key, decrypt_value, encrypt_value};
use crate::encryption::EncryptionKey;
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

const DRAFT_KEY: &str = "active";

#[derive(Clone)]
pub struct ExternalImportSetupSecretDraftStore {
    datastore: StoreDatastore,
    encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl ExternalImportSetupSecretDraftStore {
    pub fn new(
        datastore: StoreDatastore,
        encryption_key: Arc<RwLock<Option<EncryptionKey>>>,
    ) -> Self {
        Self {
            datastore,
            encryption_key,
        }
    }

    fn encryption_key(&self) -> AppResult<Option<EncryptionKey>> {
        current_encryption_key(&self.encryption_key)
    }
}

#[async_trait]
impl ExternalImportSetupSecretDraftRepository for ExternalImportSetupSecretDraftStore {
    async fn get_for_owner(
        &self,
        owner_user_id: &str,
    ) -> AppResult<Option<ExternalImportSetupSecretDraft>> {
        let encryption_key = self.encryption_key()?;
        fetch_draft_for_owner(
            self.datastore.read_exec(),
            owner_user_id,
            encryption_key.as_ref(),
        )
        .await
    }

    async fn status_for_actor(
        &self,
        actor_user_id: &str,
    ) -> AppResult<ExternalImportSetupSecretDraftStatus> {
        let Some(parent) = fetch_parent(self.datastore.read_exec()).await? else {
            return Ok(ExternalImportSetupSecretDraftStatus {
                has_draft: false,
                owned_by_current_user: false,
                updated_at: None,
            });
        };

        Ok(ExternalImportSetupSecretDraftStatus {
            has_draft: true,
            owned_by_current_user: parent.owner_user_id == actor_user_id,
            updated_at: Some(parent.updated_at),
        })
    }

    async fn save_for_owner(
        &self,
        owner_user_id: &str,
        draft: ExternalImportSetupSecretDraftInput,
    ) -> AppResult<ExternalImportSetupSecretDraftSaveResult> {
        let encryption_key = self.encryption_key()?;
        let encrypted = EncryptedDraft::from_draft(draft, encryption_key.as_ref())?;
        let owner_user_id = owner_user_id.to_string();
        let updated_at = Utc::now();

        SqlRuntime::run_in_transaction(
            &self.datastore,
            "save_external_import_setup_secret_draft",
            move |tx| {
                let encrypted = encrypted.clone();
                let owner_user_id = owner_user_id.clone();
                Box::pin(async move {
                    lock_secret_draft_singleton(tx).await?;
                    let deleted_owner = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "DELETE FROM external_import_setup_secret_drafts
                          WHERE draft_key = {}
                          RETURNING owner_user_id",
                        &[SqlArg::Text(DRAFT_KEY.to_string())],
                    )
                    .await?
                    .map(|row| row.text("owner_user_id"))
                    .transpose()?;
                    let overwrote_another_user_draft = deleted_owner
                        .as_deref()
                        .is_some_and(|existing| existing != owner_user_id);

                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO external_import_setup_secret_drafts
                            (draft_key, owner_user_id, created_at, updated_at)
                         VALUES ({}, {}, {}, {})",
                        &[
                            SqlArg::Text(DRAFT_KEY.to_string()),
                            SqlArg::Text(owner_user_id),
                            SqlArg::Timestamp(updated_at),
                            SqlArg::Timestamp(updated_at),
                        ],
                    )
                    .await?;
                    insert_encrypted_draft(tx, encrypted, updated_at).await?;

                    Ok(ExternalImportSetupSecretDraftSaveResult {
                        saved: true,
                        overwrote_another_user_draft,
                        updated_at,
                    })
                })
            },
        )
        .await
    }

    async fn clear_for_owner(&self, owner_user_id: &str) -> AppResult<bool> {
        let owner_user_id = owner_user_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "clear_external_import_setup_secret_draft",
            move |tx| {
                let owner_user_id = owner_user_id.clone();
                Box::pin(async move {
                    let rows = SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM external_import_setup_secret_drafts
                          WHERE draft_key = {} AND owner_user_id = {}",
                        &[
                            SqlArg::Text(DRAFT_KEY.to_string()),
                            SqlArg::Text(owner_user_id),
                        ],
                    )
                    .await?;
                    Ok(rows > 0)
                })
            },
        )
        .await
    }
}

struct DraftParent {
    owner_user_id: String,
    updated_at: chrono::DateTime<Utc>,
}

#[derive(Clone)]
struct EncryptedInstanceApiKey {
    instance_id: String,
    kind: ExternalImportSetupSecretInstanceKind,
    api_key_encrypted: String,
    position: i32,
}

#[derive(Clone)]
struct EncryptedSecretOverride {
    dedup_key: String,
    secret_encrypted: String,
    position: i32,
}

#[derive(Clone)]
struct EncryptedDraft {
    instance_api_keys: Vec<EncryptedInstanceApiKey>,
    download_client_api_key_overrides: Vec<EncryptedSecretOverride>,
    download_client_password_overrides: Vec<EncryptedSecretOverride>,
    indexer_api_key_overrides: Vec<EncryptedSecretOverride>,
}

impl EncryptedDraft {
    fn from_draft(
        draft: ExternalImportSetupSecretDraftInput,
        encryption_key: Option<&EncryptionKey>,
    ) -> AppResult<Self> {
        Ok(Self {
            instance_api_keys: draft
                .instance_api_keys
                .into_iter()
                .enumerate()
                .map(|(position, entry)| {
                    Ok(EncryptedInstanceApiKey {
                        instance_id: entry.instance_id,
                        kind: entry.kind,
                        api_key_encrypted: encrypt_value(
                            encryption_key,
                            &entry.api_key,
                            "external import setup instance API key",
                            true,
                        )?,
                        position: saturating_i32(position),
                    })
                })
                .collect::<AppResult<Vec<_>>>()?,
            download_client_api_key_overrides: encrypt_overrides(
                draft.download_client_api_key_overrides,
                encryption_key,
                "external import setup download client API key override",
            )?,
            download_client_password_overrides: encrypt_overrides(
                draft.download_client_password_overrides,
                encryption_key,
                "external import setup download client password override",
            )?,
            indexer_api_key_overrides: encrypt_overrides(
                draft.indexer_api_key_overrides,
                encryption_key,
                "external import setup indexer API key override",
            )?,
        })
    }
}

fn encrypt_overrides(
    entries: Vec<ExternalImportSetupSecretOverrideDraft>,
    encryption_key: Option<&EncryptionKey>,
    label: &str,
) -> AppResult<Vec<EncryptedSecretOverride>> {
    entries
        .into_iter()
        .enumerate()
        .map(|(position, entry)| {
            Ok(EncryptedSecretOverride {
                dedup_key: entry.dedup_key,
                secret_encrypted: encrypt_value(encryption_key, &entry.secret, label, true)?,
                position: saturating_i32(position),
            })
        })
        .collect()
}

async fn fetch_parent(exec: SqlExec<'_, '_>) -> AppResult<Option<DraftParent>> {
    SqlRuntime::fetch_optional(
        exec,
        "SELECT owner_user_id, updated_at
           FROM external_import_setup_secret_drafts
          WHERE draft_key = {}",
        &[SqlArg::Text(DRAFT_KEY.to_string())],
    )
    .await?
    .map(|row| {
        Ok(DraftParent {
            owner_user_id: row.text("owner_user_id")?,
            updated_at: row.timestamp("updated_at")?,
        })
    })
    .transpose()
}

async fn fetch_draft_for_owner(
    exec: SqlExec<'_, '_>,
    owner_user_id: &str,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<Option<ExternalImportSetupSecretDraft>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT 0 AS row_sort,
                'draft' AS entry_kind,
                d.owner_user_id,
                d.updated_at,
                0 AS position,
                NULL AS instance_id,
                NULL AS kind,
                NULL AS dedup_key,
                NULL AS secret_encrypted
           FROM external_import_setup_secret_drafts d
          WHERE d.draft_key = {} AND d.owner_user_id = {}
         UNION ALL
         SELECT 1 AS row_sort,
                'instance_api_key' AS entry_kind,
                d.owner_user_id,
                d.updated_at,
                i.position,
                i.instance_id,
                i.kind,
                NULL AS dedup_key,
                i.api_key_encrypted AS secret_encrypted
           FROM external_import_setup_secret_drafts d
           JOIN external_import_setup_instance_api_keys i ON i.draft_key = d.draft_key
          WHERE d.draft_key = {} AND d.owner_user_id = {}
         UNION ALL
         SELECT 2 AS row_sort,
                'download_client_api_key_override' AS entry_kind,
                d.owner_user_id,
                d.updated_at,
                o.position,
                NULL AS instance_id,
                NULL AS kind,
                o.dedup_key,
                o.api_key_encrypted AS secret_encrypted
           FROM external_import_setup_secret_drafts d
           JOIN external_import_setup_download_client_api_key_overrides o ON o.draft_key = d.draft_key
          WHERE d.draft_key = {} AND d.owner_user_id = {}
         UNION ALL
         SELECT 3 AS row_sort,
                'download_client_password_override' AS entry_kind,
                d.owner_user_id,
                d.updated_at,
                o.position,
                NULL AS instance_id,
                NULL AS kind,
                o.dedup_key,
                o.password_encrypted AS secret_encrypted
           FROM external_import_setup_secret_drafts d
           JOIN external_import_setup_download_client_password_overrides o ON o.draft_key = d.draft_key
          WHERE d.draft_key = {} AND d.owner_user_id = {}
         UNION ALL
         SELECT 4 AS row_sort,
                'indexer_api_key_override' AS entry_kind,
                d.owner_user_id,
                d.updated_at,
                o.position,
                NULL AS instance_id,
                NULL AS kind,
                o.dedup_key,
                o.api_key_encrypted AS secret_encrypted
           FROM external_import_setup_secret_drafts d
           JOIN external_import_setup_indexer_api_key_overrides o ON o.draft_key = d.draft_key
          WHERE d.draft_key = {} AND d.owner_user_id = {}
          ORDER BY row_sort ASC, position ASC, instance_id ASC, dedup_key ASC",
        &[
            SqlArg::Text(DRAFT_KEY.to_string()),
            SqlArg::Text(owner_user_id.to_string()),
            SqlArg::Text(DRAFT_KEY.to_string()),
            SqlArg::Text(owner_user_id.to_string()),
            SqlArg::Text(DRAFT_KEY.to_string()),
            SqlArg::Text(owner_user_id.to_string()),
            SqlArg::Text(DRAFT_KEY.to_string()),
            SqlArg::Text(owner_user_id.to_string()),
            SqlArg::Text(DRAFT_KEY.to_string()),
            SqlArg::Text(owner_user_id.to_string()),
        ],
    )
    .await?;

    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let mut draft = ExternalImportSetupSecretDraft {
        owner_user_id: first.text("owner_user_id")?,
        updated_at: first.timestamp("updated_at")?,
        secrets: ExternalImportSetupSecretDraftInput::default(),
    };

    for row in rows {
        match row.text("entry_kind")?.as_str() {
            "draft" => {}
            "instance_api_key" => {
                draft
                    .secrets
                    .instance_api_keys
                    .push(row_to_instance_api_key(&row, encryption_key)?);
            }
            "download_client_api_key_override" => {
                draft
                    .secrets
                    .download_client_api_key_overrides
                    .push(row_to_secret_override(
                        &row,
                        encryption_key,
                        "external import setup download client API key override",
                    )?);
            }
            "download_client_password_override" => {
                draft
                    .secrets
                    .download_client_password_overrides
                    .push(row_to_secret_override(
                        &row,
                        encryption_key,
                        "external import setup download client password override",
                    )?);
            }
            "indexer_api_key_override" => {
                draft
                    .secrets
                    .indexer_api_key_overrides
                    .push(row_to_secret_override(
                        &row,
                        encryption_key,
                        "external import setup indexer API key override",
                    )?);
            }
            entry_kind => {
                return Err(AppError::Repository(format!(
                    "invalid external import setup secret draft row kind {entry_kind}"
                )));
            }
        }
    }

    Ok(Some(draft))
}

fn row_to_instance_api_key(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
) -> AppResult<ExternalImportSetupInstanceApiKeyDraft> {
    let kind_raw = row.text("kind")?;
    let kind = ExternalImportSetupSecretInstanceKind::parse(&kind_raw).ok_or_else(|| {
        AppError::Repository(format!(
            "invalid external import setup instance kind {kind_raw}"
        ))
    })?;
    Ok(ExternalImportSetupInstanceApiKeyDraft {
        instance_id: row.text("instance_id")?,
        kind,
        api_key: decrypt_value(
            encryption_key,
            row.text("secret_encrypted")?,
            "external import setup instance API key",
            true,
        )?,
    })
}

fn row_to_secret_override(
    row: &SqlRow,
    encryption_key: Option<&EncryptionKey>,
    label: &str,
) -> AppResult<ExternalImportSetupSecretOverrideDraft> {
    Ok(ExternalImportSetupSecretOverrideDraft {
        dedup_key: row.text("dedup_key")?,
        secret: decrypt_value(encryption_key, row.text("secret_encrypted")?, label, true)?,
    })
}

async fn lock_secret_draft_singleton(tx: &mut SqlTx<'_>) -> AppResult<()> {
    match tx {
        SqlTx::Sqlite(_) => Ok(()),
        SqlTx::Postgres(_) => SqlRuntime::execute(
            SqlExec::Tx(tx),
            "LOCK TABLE external_import_setup_secret_drafts IN EXCLUSIVE MODE",
            &[],
        )
        .await
        .map(|_| ()),
    }
}

async fn insert_encrypted_draft(
    tx: &mut SqlTx<'_>,
    draft: EncryptedDraft,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for entry in draft.instance_api_keys {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO external_import_setup_instance_api_keys
                (draft_key, instance_id, kind, api_key_encrypted, position, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(DRAFT_KEY.to_string()),
                SqlArg::Text(entry.instance_id),
                SqlArg::Text(entry.kind.as_str().to_string()),
                SqlArg::Text(entry.api_key_encrypted),
                SqlArg::I32(entry.position),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    insert_secret_overrides(
        tx,
        "external_import_setup_download_client_api_key_overrides",
        "api_key_encrypted",
        draft.download_client_api_key_overrides,
        now,
    )
    .await?;
    insert_secret_overrides(
        tx,
        "external_import_setup_download_client_password_overrides",
        "password_encrypted",
        draft.download_client_password_overrides,
        now,
    )
    .await?;
    insert_secret_overrides(
        tx,
        "external_import_setup_indexer_api_key_overrides",
        "api_key_encrypted",
        draft.indexer_api_key_overrides,
        now,
    )
    .await
}

async fn insert_secret_overrides(
    tx: &mut crate::queries::sql_runtime::SqlTx<'_>,
    table: &'static str,
    secret_column: &'static str,
    entries: Vec<EncryptedSecretOverride>,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO {table}
            (draft_key, dedup_key, {secret_column}, position, created_at, updated_at)
         VALUES ({{}}, {{}}, {{}}, {{}}, {{}}, {{}})"
    );
    for entry in entries {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            &sql,
            &[
                SqlArg::Text(DRAFT_KEY.to_string()),
                SqlArg::Text(entry.dedup_key),
                SqlArg::Text(entry.secret_encrypted),
                SqlArg::I32(entry.position),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

fn saturating_i32(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
