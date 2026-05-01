use scryer_application::{AppError, AppResult};
use scryer_domain::{Entitlement, User};
use serde_json;
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

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

fn parse_stored_entitlements(raw: &str) -> AppResult<(Vec<Entitlement>, bool)> {
    let tokens: Vec<String> =
        serde_json::from_str(raw).map_err(|err| AppError::Repository(err.to_string()))?;
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

pub(crate) async fn create_user_query(pool: &SqlitePool, user: &User) -> AppResult<User> {
    let entitlements_json = serde_json::to_string(&user.entitlements)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    sqlx::query(
        "INSERT INTO users (id, username, entitlements, password_hash) VALUES (?, ?, ?, ?)",
    )
    .bind(&user.id)
    .bind(&user.username)
    .bind(&entitlements_json)
    .bind(&user.password_hash)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(user.clone())
}

pub(crate) async fn get_user_by_id_query(pool: &SqlitePool, id: &str) -> AppResult<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some((id, username, entitlements_raw, password_hash)) => {
            let (entitlements, changed) = parse_stored_entitlements(&entitlements_raw)?;
            if changed {
                let entitlements_json = serde_json::to_string(&entitlements)
                    .map_err(|err| AppError::Repository(err.to_string()))?;
                return update_user_entitlements_query(pool, &id, &entitlements_json)
                    .await
                    .map(Some);
            }
            Ok(Some(User {
                id,
                username,
                password_hash,
                entitlements,
            }))
        }
        None => Ok(None),
    }
}

async fn get_user_by_id_tx(tx: &mut Transaction<'_, Sqlite>, id: &str) -> AppResult<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some((id, username, entitlements_raw, password_hash)) => {
            let (entitlements, _) = parse_stored_entitlements(&entitlements_raw)?;
            Ok(Some(User {
                id,
                username,
                password_hash,
                entitlements,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn get_user_by_username_query(
    pool: &SqlitePool,
    username: &str,
) -> AppResult<Option<User>> {
    let row = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some((id, username, entitlements_raw, password_hash)) => {
            let (entitlements, changed) = parse_stored_entitlements(&entitlements_raw)?;
            if changed {
                let entitlements_json = serde_json::to_string(&entitlements)
                    .map_err(|err| AppError::Repository(err.to_string()))?;
                return update_user_entitlements_query(pool, &id, &entitlements_json)
                    .await
                    .map(Some);
            }
            Ok(Some(User {
                id,
                username,
                password_hash,
                entitlements,
            }))
        }
        None => Ok(None),
    }
}

pub(crate) async fn list_users_query(pool: &SqlitePool) -> AppResult<Vec<User>> {
    let rows = sqlx::query_as::<_, (String, String, String, Option<String>)>(
        "SELECT id, username, entitlements, password_hash FROM users",
    )
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut users = Vec::with_capacity(rows.len());
    for (id, username, entitlements_json, password_hash) in rows {
        let (entitlements, changed) = parse_stored_entitlements(&entitlements_json)?;
        if changed {
            let next_entitlements_json = serde_json::to_string(&entitlements)
                .map_err(|err| AppError::Repository(err.to_string()))?;
            users.push(update_user_entitlements_query(pool, &id, &next_entitlements_json).await?);
            continue;
        }
        users.push(User {
            id,
            username,
            password_hash,
            entitlements,
        });
    }

    Ok(users)
}

pub(crate) async fn update_user_entitlements_query(
    pool: &SqlitePool,
    id: &str,
    entitlements_json: &str,
) -> AppResult<User> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let result = sqlx::query("UPDATE users SET entitlements = ? WHERE id = ?")
        .bind(entitlements_json)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    let user = get_user_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(user)
}

pub(crate) async fn update_user_password_query(
    pool: &SqlitePool,
    id: &str,
    password_hash: &str,
) -> AppResult<User> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let result = sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(password_hash)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    let user = get_user_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(user)
}

pub(crate) async fn delete_user_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("user {}", id)));
    }

    Ok(())
}
