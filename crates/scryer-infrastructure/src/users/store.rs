use async_trait::async_trait;
use scryer_application::{AppError, AppResult, UserExternalAccountRepository, UserRepository};
use scryer_domain::{
    AppPermissionMask, ExternalAccountProvider, ExternalAccountStatus, LibraryGrant, User,
    UserAccountKind, UserExternalAccount,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct UserStore {
    datastore: StoreDatastore,
}

impl UserStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl UserRepository for UserStore {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        load_user_by_username(self.datastore.read_exec(), username).await
    }

    async fn create(&self, user: User) -> AppResult<User> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_user", move |tx| {
            let user = user.clone();
            Box::pin(async move {
                insert_user_tx(tx, &user).await?;
                Ok(user)
            })
        })
        .await
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, username, password_hash, account_kind FROM users",
            &[],
        )
        .await?;
        rows.iter().map(row_to_user).collect()
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        load_user_by_id(self.datastore.read_exec(), id).await
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_user_password_hash", move |tx| {
            let id = id.clone();
            let password_hash = password_hash.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE users SET password_hash = {} WHERE id = {}",
                        &[SqlArg::Text(password_hash), SqlArg::Text(id.clone())],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("user {id}")));
                }
                load_user_by_id_tx(tx, &id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("user {id}")))
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_user", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "DELETE FROM users WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("user {id}")));
                }
                Ok(())
            })
        })
        .await
    }
}

#[async_trait]
impl UserExternalAccountRepository for UserStore {
    async fn create(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_user_external_account", move |tx| {
            let account = account.clone();
            Box::pin(async move {
                insert_external_account_tx(tx, &account).await?;
                load_external_account_by_id_tx(tx, &account.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("external account {}", account.id)))
            })
        })
        .await
    }

    async fn list_by_user_id(&self, user_id: &str) -> AppResult<Vec<UserExternalAccount>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE user_id = {}
              ORDER BY provider, username",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_external_account).collect()
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<UserExternalAccount>> {
        load_external_account_by_id(self.datastore.read_exec(), id).await
    }

    async fn get_by_provider_identity(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
        external_user_id: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE provider = {} AND connection_id = {} AND external_user_id = {}",
            &[
                SqlArg::Text(provider.as_str().to_string()),
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Text(external_user_id.to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_external_account).transpose()
    }

    async fn get_pending_claim_by_provider_username(
        &self,
        provider: ExternalAccountProvider,
        connection_id: &str,
        username: &str,
    ) -> AppResult<Option<UserExternalAccount>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, user_id, provider, connection_id, external_user_id, username,
                    display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
               FROM user_external_accounts
              WHERE provider = {}
                AND connection_id = {}
                AND status = 'pending_claim'
                AND external_user_id IS NULL
                AND LOWER(username) = LOWER({})",
            &[
                SqlArg::Text(provider.as_str().to_string()),
                SqlArg::Text(connection_id.to_string()),
                SqlArg::Text(username.trim().to_string()),
            ],
        )
        .await?;
        row.as_ref().map(row_to_external_account).transpose()
    }

    async fn update(&self, account: UserExternalAccount) -> AppResult<UserExternalAccount> {
        SqlRuntime::run_in_transaction(&self.datastore, "update_user_external_account", move |tx| {
            let account = account.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE user_external_accounts
                            SET user_id = {},
                                provider = {},
                                connection_id = {},
                                external_user_id = {},
                                username = {},
                                display_name = {},
                                avatar_url = {},
                                status = {},
                                verified_at = {},
                                last_login_at = {},
                                updated_at = {}
                           WHERE id = {}",
                        &[
                            SqlArg::Text(account.user_id.clone()),
                            SqlArg::Text(account.provider.as_str().to_string()),
                            SqlArg::Text(account.connection_id.clone()),
                            SqlArg::OptText(account.external_user_id.clone()),
                            SqlArg::Text(account.username.clone()),
                            SqlArg::OptText(account.display_name.clone()),
                            SqlArg::OptText(account.avatar_url.clone()),
                            SqlArg::Text(account.status.as_str().to_string()),
                            SqlArg::OptTimestamp(account.verified_at),
                            SqlArg::OptTimestamp(account.last_login_at),
                            SqlArg::Timestamp(account.updated_at),
                            SqlArg::Text(account.id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!(
                        "external account {}",
                        account.id
                    )));
                }
                load_external_account_by_id_tx(tx, &account.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("external account {}", account.id)))
            })
        })
        .await
    }

    async fn create_auto_added_user_with_account(
        &self,
        user: User,
        app_permissions: AppPermissionMask,
        library_grants: Vec<LibraryGrant>,
        account: UserExternalAccount,
    ) -> AppResult<(User, UserExternalAccount)> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_auto_added_user_with_account",
            move |tx| {
                let user = user.clone();
                let account = account.clone();
                let library_grants = library_grants.clone();
                Box::pin(async move {
                    insert_user_tx(tx, &user).await?;
                    upsert_app_permission_mask_tx(tx, &user.id, app_permissions).await?;
                    replace_library_grants_tx(tx, &user.id, &library_grants).await?;
                    insert_external_account_tx(tx, &account).await?;
                    let account = load_external_account_by_id_tx(tx, &account.id)
                        .await?
                        .ok_or_else(|| {
                            AppError::NotFound(format!("external account {}", account.id))
                        })?;
                    Ok((user, account))
                })
            },
        )
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_user_external_account", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "DELETE FROM user_external_accounts WHERE id = {}",
                        &[SqlArg::Text(id.clone())],
                    )
                    .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("external account {id}")));
                }
                Ok(())
            })
        })
        .await
    }
}

async fn load_user_by_username(exec: SqlExec<'_, '_>, username: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash, account_kind FROM users WHERE username = {}",
        &[SqlArg::Text(username.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash, account_kind FROM users WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<User>> {
    load_user_by_id(SqlExec::Tx(tx), id).await
}

async fn insert_user_tx(tx: &mut SqlTx<'_>, user: &User) -> AppResult<()> {
    tx.execute(
        "INSERT INTO users (id, username, password_hash, account_kind)
         VALUES ({}, {}, {}, {})",
        &[
            SqlArg::Text(user.id.clone()),
            SqlArg::Text(user.username.clone()),
            SqlArg::OptText(user.password_hash.clone()),
            SqlArg::Text(user.account_kind.as_str().to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn upsert_app_permission_mask_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    permissions: AppPermissionMask,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
         VALUES ({}, {}, {})
         ON CONFLICT(user_id) DO UPDATE SET
            permission_mask = excluded.permission_mask,
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(user_id.to_string()),
            SqlArg::I64(permissions.bits() as i64),
            SqlArg::Timestamp(chrono::Utc::now()),
        ],
    )
    .await?;
    Ok(())
}

async fn replace_library_grants_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    grants: &[LibraryGrant],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM user_library_permission_masks WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    for grant in grants.iter().filter(|grant| !grant.permissions.is_empty()) {
        tx.execute(
            "INSERT INTO user_library_permission_masks
             (user_id, library_id, permission_mask, updated_at)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(user_id.to_string()),
                SqlArg::Text(grant.library_id.clone()),
                SqlArg::I64(grant.permissions.bits() as i64),
                SqlArg::Timestamp(chrono::Utc::now()),
            ],
        )
        .await?;
    }
    Ok(())
}

fn row_to_user(row: &SqlRow) -> AppResult<User> {
    Ok(User {
        id: row.text("id")?,
        username: row.text("username")?,
        password_hash: row.opt_text("password_hash")?,
        account_kind: UserAccountKind::parse(&row.text("account_kind")?).ok_or_else(|| {
            AppError::Repository(format!(
                "invalid user account kind for user {}",
                row.text("id").unwrap_or_else(|_| "<unknown>".to_string())
            ))
        })?,
        authorization: Default::default(),
    })
}

async fn load_external_account_by_id(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, user_id, provider, connection_id, external_user_id, username,
                display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
           FROM user_external_accounts
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_external_account).transpose()
}

async fn load_external_account_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<UserExternalAccount>> {
    load_external_account_by_id(SqlExec::Tx(tx), id).await
}

async fn insert_external_account_tx(
    tx: &mut SqlTx<'_>,
    account: &UserExternalAccount,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO user_external_accounts (
             id, user_id, provider, connection_id, external_user_id, username,
             display_name, avatar_url, status, verified_at, last_login_at, created_at, updated_at
          )
          VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(account.id.clone()),
            SqlArg::Text(account.user_id.clone()),
            SqlArg::Text(account.provider.as_str().to_string()),
            SqlArg::Text(account.connection_id.clone()),
            SqlArg::OptText(account.external_user_id.clone()),
            SqlArg::Text(account.username.clone()),
            SqlArg::OptText(account.display_name.clone()),
            SqlArg::OptText(account.avatar_url.clone()),
            SqlArg::Text(account.status.as_str().to_string()),
            SqlArg::OptTimestamp(account.verified_at),
            SqlArg::OptTimestamp(account.last_login_at),
            SqlArg::Timestamp(account.created_at),
            SqlArg::Timestamp(account.updated_at),
        ],
    )
    .await?;
    Ok(())
}

fn row_to_external_account(row: &SqlRow) -> AppResult<UserExternalAccount> {
    let provider = ExternalAccountProvider::parse(&row.text("provider")?)
        .ok_or_else(|| AppError::Repository("invalid external account provider".into()))?;
    let status = ExternalAccountStatus::parse(&row.text("status")?)
        .ok_or_else(|| AppError::Repository("invalid external account status".into()))?;
    Ok(UserExternalAccount {
        id: row.text("id")?,
        user_id: row.text("user_id")?,
        provider,
        connection_id: row.text("connection_id")?,
        external_user_id: row.opt_text("external_user_id")?,
        username: row.text("username")?,
        display_name: row.opt_text("display_name")?,
        avatar_url: row.opt_text("avatar_url")?,
        status,
        verified_at: row.opt_timestamp("verified_at")?,
        last_login_at: row.opt_timestamp("last_login_at")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::Utc;
    use scryer_application::{UserExternalAccountRepository, UserRepository};
    use scryer_domain::{AppPermissionMask, LibraryPermissionMask, UserAuthorization};
    use sqlx::sqlite::SqlitePoolOptions;
    use tokio::sync::Mutex;

    use super::*;

    async fn test_store() -> UserStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create sqlite pool");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("enable foreign keys");
        sqlx::query(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY NOT NULL,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT,
                account_kind TEXT NOT NULL DEFAULT 'local'
            )",
        )
        .execute(&pool)
        .await
        .expect("create users table");
        sqlx::query(
            "CREATE TABLE user_external_accounts (
                id TEXT PRIMARY KEY NOT NULL,
                user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                provider TEXT NOT NULL,
                connection_id TEXT NOT NULL,
                external_user_id TEXT,
                username TEXT NOT NULL,
                display_name TEXT,
                avatar_url TEXT,
                status TEXT NOT NULL DEFAULT 'pending_claim',
                verified_at TEXT,
                last_login_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )",
        )
        .execute(&pool)
        .await
        .expect("create user_external_accounts table");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_provider_identity
               ON user_external_accounts(provider, connection_id, external_user_id)",
        )
        .execute(&pool)
        .await
        .expect("create provider identity index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_pending_username
               ON user_external_accounts(provider, connection_id, LOWER(username))
               WHERE status = 'pending_claim' AND external_user_id IS NULL",
        )
        .execute(&pool)
        .await
        .expect("create pending username index");
        sqlx::query(
            "CREATE UNIQUE INDEX idx_user_external_accounts_user_provider_connection
               ON user_external_accounts(user_id, provider, connection_id)",
        )
        .execute(&pool)
        .await
        .expect("create user provider connection index");

        UserStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(Mutex::new(())),
        })
    }

    fn test_user(id: &str) -> User {
        User {
            id: id.to_string(),
            username: format!("{id}_name"),
            password_hash: Some("hash".to_string()),
            account_kind: Default::default(),
            authorization: UserAuthorization {
                app: AppPermissionMask::NONE,
                libraries: HashMap::new(),
                default_library: LibraryPermissionMask::NONE,
                loaded: true,
            },
        }
    }

    fn test_account(
        id: &str,
        user_id: &str,
        provider: ExternalAccountProvider,
        connection_id: &str,
        external_user_id: &str,
    ) -> UserExternalAccount {
        let now = Utc::now();
        UserExternalAccount {
            id: id.to_string(),
            user_id: user_id.to_string(),
            provider,
            connection_id: connection_id.to_string(),
            external_user_id: Some(external_user_id.to_string()),
            username: format!("{external_user_id}_name"),
            display_name: None,
            avatar_url: None,
            status: ExternalAccountStatus::PendingClaim,
            verified_at: None,
            last_login_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn external_account_provider_identity_is_unique() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create first user");
        UserRepository::create(&store, test_user("user_b"))
            .await
            .expect("create second user");

        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        let duplicate = UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_b",
                "user_b",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn pending_external_account_username_is_unique_for_connection() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create first user");
        UserRepository::create(&store, test_user("user_b"))
            .await
            .expect("create second user");

        let mut first = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        first.external_user_id = None;
        first.username = "JellyUser".to_string();
        UserExternalAccountRepository::create(&store, first)
            .await
            .expect("create pending account");

        let mut duplicate = test_account(
            "account_b",
            "user_b",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_2",
        );
        duplicate.external_user_id = None;
        duplicate.username = "jellyuser".to_string();
        let duplicate = UserExternalAccountRepository::create(&store, duplicate).await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn pending_external_account_can_be_found_by_username() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        let mut account = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        account.external_user_id = None;
        account.username = "JellyUser".to_string();
        UserExternalAccountRepository::create(&store, account)
            .await
            .expect("create pending account");

        let found = UserExternalAccountRepository::get_pending_claim_by_provider_username(
            &store,
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "jellyuser",
        )
        .await
        .expect("lookup pending account")
        .expect("pending account exists");

        assert_eq!(found.id, "account_a");
        assert_eq!(found.external_user_id, None);
    }

    #[tokio::test]
    async fn external_account_user_provider_connection_is_unique() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");

        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Plex,
                "plex_main",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        let duplicate = UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_b",
                "user_a",
                ExternalAccountProvider::Plex,
                "plex_main",
                "external_2",
            ),
        )
        .await;

        assert!(duplicate.is_err());
    }

    #[tokio::test]
    async fn external_account_status_transition_persists() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");
        let mut new_account = test_account(
            "account_a",
            "user_a",
            ExternalAccountProvider::Jellyfin,
            "server_1",
            "external_1",
        );
        let initial_login_at = Utc::now();
        new_account.status = ExternalAccountStatus::Active;
        new_account.verified_at = Some(initial_login_at);
        new_account.last_login_at = Some(initial_login_at);
        let mut account = UserExternalAccountRepository::create(&store, new_account)
            .await
            .expect("create account");
        assert_eq!(account.last_login_at, Some(initial_login_at));

        let listed = UserExternalAccountRepository::list_by_user_id(&store, "user_a")
            .await
            .expect("list accounts");
        assert_eq!(listed[0].last_login_at, Some(initial_login_at));

        account.status = ExternalAccountStatus::Active;
        let now = Utc::now();
        account.verified_at = Some(now);
        account.last_login_at = Some(now);
        let updated = UserExternalAccountRepository::update(&store, account)
            .await
            .expect("update account");

        assert_eq!(updated.status, ExternalAccountStatus::Active);
        assert!(updated.verified_at.is_some());
        assert!(updated.last_login_at.is_some());
    }

    #[tokio::test]
    async fn deleting_user_cascades_external_accounts() {
        let store = test_store().await;
        UserRepository::create(&store, test_user("user_a"))
            .await
            .expect("create user");
        UserExternalAccountRepository::create(
            &store,
            test_account(
                "account_a",
                "user_a",
                ExternalAccountProvider::Jellyfin,
                "server_1",
                "external_1",
            ),
        )
        .await
        .expect("create account");

        UserRepository::delete(&store, "user_a")
            .await
            .expect("delete user");

        let remaining = UserExternalAccountRepository::list_by_user_id(&store, "user_a")
            .await
            .expect("list accounts");
        assert!(remaining.is_empty());
    }
}
