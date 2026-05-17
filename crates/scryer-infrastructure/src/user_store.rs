use async_trait::async_trait;
use scryer_application::{AppError, AppResult, UserRepository};
use scryer_domain::{Entitlement, User};
use serde_json::Value as JsonValue;
use std::collections::HashSet;

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
        let user = load_user_by_username(self.datastore.read_exec(), username).await?;
        self.repair_if_needed(user).await
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
            "SELECT id, username, entitlements, password_hash FROM users",
            &[],
        )
        .await?;

        let mut users = Vec::with_capacity(rows.len());
        for row in &rows {
            let decoded = row_to_user(row)?;
            if decoded.changed {
                users.push(
                    self.update_entitlements(&decoded.user.id, decoded.user.entitlements)
                        .await?,
                );
            } else {
                users.push(decoded.user);
            }
        }
        Ok(users)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        let user = load_user_by_id(self.datastore.read_exec(), id).await?;
        self.repair_if_needed(user).await
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_user_entitlements", move |tx| {
            let id = id.clone();
            let entitlements = entitlements.clone();
            Box::pin(async move {
                let rows = tx
                    .execute(
                        "UPDATE users SET entitlements = {} WHERE id = {}",
                        &[
                            SqlArg::Json(entitlements_json(&entitlements)?),
                            SqlArg::Text(id.clone()),
                        ],
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

impl UserStore {
    async fn repair_if_needed(&self, decoded: Option<DecodedUser>) -> AppResult<Option<User>> {
        match decoded {
            Some(decoded) if decoded.changed => self
                .update_entitlements(&decoded.user.id, decoded.user.entitlements)
                .await
                .map(Some),
            Some(decoded) => Ok(Some(decoded.user)),
            None => Ok(None),
        }
    }
}

async fn load_user_by_username(
    exec: SqlExec<'_, '_>,
    username: &str,
) -> AppResult<Option<DecodedUser>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, entitlements, password_hash FROM users WHERE username = {}",
        &[SqlArg::Text(username.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<DecodedUser>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, username, entitlements, password_hash FROM users WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_user).transpose()
}

async fn load_user_by_id_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<User>> {
    load_user_by_id(SqlExec::Tx(tx), id)
        .await
        .map(|user| user.map(|decoded| decoded.user))
}

async fn insert_user_tx(tx: &mut SqlTx<'_>, user: &User) -> AppResult<()> {
    tx.execute(
        "INSERT INTO users (id, username, entitlements, password_hash)
         VALUES ({}, {}, {}, {})",
        &[
            SqlArg::Text(user.id.clone()),
            SqlArg::Text(user.username.clone()),
            SqlArg::Json(entitlements_json(&user.entitlements)?),
            SqlArg::OptText(user.password_hash.clone()),
        ],
    )
    .await?;
    Ok(())
}

struct DecodedUser {
    user: User,
    changed: bool,
}

fn row_to_user(row: &SqlRow) -> AppResult<DecodedUser> {
    let entitlements_value = row
        .opt_json("entitlements")?
        .ok_or_else(|| AppError::Repository("user entitlements were null".to_string()))?;
    let (entitlements, changed) = parse_stored_entitlements(entitlements_value)?;
    Ok(DecodedUser {
        user: User {
            id: row.text("id")?,
            username: row.text("username")?,
            password_hash: row.opt_text("password_hash")?,
            entitlements,
            authorization: Default::default(),
        },
        changed,
    })
}

fn entitlements_json(entitlements: &[Entitlement]) -> AppResult<JsonValue> {
    serde_json::to_value(entitlements).map_err(|err| AppError::Repository(err.to_string()))
}

fn canonical_stored_entitlement_token(entitlement: &Entitlement) -> &'static str {
    match entitlement {
        Entitlement::ViewCatalog => "ViewCatalog",
        Entitlement::ManageTitle => "ManageTitle",
        Entitlement::ManageUsers => "ManageUsers",
        Entitlement::ManageConfig => "ManageConfig",
    }
}

fn parse_stored_entitlement_token(raw: &str) -> Option<Entitlement> {
    match raw.trim().to_lowercase().replace(['-', ' '], "_").as_str() {
        "viewcatalog" | "view_catalog" => Some(Entitlement::ViewCatalog),
        "monitortitle" | "monitor_title" => Some(Entitlement::ManageTitle),
        "managetitle" | "manage_title" => Some(Entitlement::ManageTitle),
        "triggeractions" | "trigger_actions" => Some(Entitlement::ManageTitle),
        "manageusers" | "manage_users" => Some(Entitlement::ManageUsers),
        "manageconfig" | "manage_config" => Some(Entitlement::ManageConfig),
        "viewhistory" | "view_history" => Some(Entitlement::ManageTitle),
        _ => None,
    }
}

fn parse_stored_entitlements(raw: JsonValue) -> AppResult<(Vec<Entitlement>, bool)> {
    let tokens: Vec<String> =
        serde_json::from_value(raw).map_err(|err| AppError::Repository(err.to_string()))?;
    let mut seen = HashSet::new();
    let mut entitlements = Vec::with_capacity(tokens.len());
    let mut changed = false;

    for token in tokens {
        let entitlement = parse_stored_entitlement_token(&token).ok_or_else(|| {
            AppError::Repository(format!("unknown stored entitlement token: {token}"))
        })?;
        if canonical_stored_entitlement_token(&entitlement) != token {
            changed = true;
        }
        if seen.insert(entitlement.clone()) {
            entitlements.push(entitlement);
        } else {
            changed = true;
        }
    }

    Ok((entitlements, changed))
}
