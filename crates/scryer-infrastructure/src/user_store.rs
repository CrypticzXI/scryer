use async_trait::async_trait;
use scryer_application::{AppError, AppResult, UserRepository};
use scryer_domain::User;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct UserStore {
    datastore: StoreDatastore,
}

impl UserStore {
    pub(crate) fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub fn sqlite(db: &crate::SqliteServices) -> Self {
        Self::new(StoreDatastore::Sqlite {
            pool: db.pool().clone(),
            writer_gate: db.writer_gate(),
        })
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
            "SELECT id, username, password_hash FROM users",
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

async fn load_user_by_username(exec: SqlExec<'_, '_>, username: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash FROM users WHERE username = {}",
        &[SqlArg::Text(username.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<User>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, password_hash FROM users WHERE id = {}",
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
        "INSERT INTO users (id, username, password_hash)
         VALUES ({}, {}, {})",
        &[
            SqlArg::Text(user.id.clone()),
            SqlArg::Text(user.username.clone()),
            SqlArg::OptText(user.password_hash.clone()),
        ],
    )
    .await?;
    Ok(())
}

fn row_to_user(row: &SqlRow) -> AppResult<User> {
    Ok(User {
        id: row.text("id")?,
        username: row.text("username")?,
        password_hash: row.opt_text("password_hash")?,
        authorization: Default::default(),
    })
}
