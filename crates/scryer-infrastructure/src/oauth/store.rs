use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, OAuthAuthorizationCodeRecord, OAuthConnectedAppRecord,
    OAuthRefreshGrantRecord, OAuthRefreshRotation, OAuthRefreshTokenRecord, OAuthRepository,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct OAuthStore {
    datastore: StoreDatastore,
}

impl OAuthStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl OAuthRepository for OAuthStore {
    async fn create_authorization_code(
        &self,
        record: OAuthAuthorizationCodeRecord,
    ) -> AppResult<OAuthAuthorizationCodeRecord> {
        SqlRuntime::execute(
            self.datastore.read_exec(),
            "INSERT INTO oauth_authorization_codes
                (id, code_hash, client_id, user_id, redirect_uri, scope, code_challenge,
                 code_challenge_method, created_at, expires_at, consumed_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(record.id.clone()),
                SqlArg::Text(record.code_hash.clone()),
                SqlArg::Text(record.client_id.clone()),
                SqlArg::Text(record.user_id.clone()),
                SqlArg::Text(record.redirect_uri.clone()),
                SqlArg::Text(record.scope.clone()),
                SqlArg::Text(record.code_challenge.clone()),
                SqlArg::Text(record.code_challenge_method.clone()),
                SqlArg::Timestamp(record.created_at),
                SqlArg::Timestamp(record.expires_at),
                SqlArg::OptTimestamp(record.consumed_at),
            ],
        )
        .await?;
        Ok(record)
    }

    async fn get_authorization_code(
        &self,
        id: &str,
    ) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
        load_authorization_code(self.datastore.read_exec(), id).await
    }

    async fn consume_authorization_code(
        &self,
        id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE oauth_authorization_codes
                SET consumed_at = {}
              WHERE id = {}
                AND consumed_at IS NULL",
            &[SqlArg::Timestamp(consumed_at), SqlArg::Text(id.to_string())],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn create_refresh_grant(
        &self,
        grant: OAuthRefreshGrantRecord,
        token: OAuthRefreshTokenRecord,
    ) -> AppResult<OAuthRefreshGrantRecord> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_oauth_refresh_grant", move |tx| {
            let grant = grant.clone();
            let token = token.clone();
            Box::pin(async move {
                insert_refresh_grant_tx(tx, &grant).await?;
                insert_refresh_token_tx(tx, &token).await?;
                Ok(grant)
            })
        })
        .await
    }

    async fn get_refresh_token(
        &self,
        id: &str,
    ) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
        load_refresh_token_with_grant(self.datastore.read_exec(), id).await
    }

    async fn rotate_refresh_token(
        &self,
        token_id: &str,
        consumed_at: chrono::DateTime<chrono::Utc>,
        next_token: OAuthRefreshTokenRecord,
    ) -> AppResult<Option<OAuthRefreshRotation>> {
        let token_id = token_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "rotate_oauth_refresh_token", move |tx| {
            let token_id = token_id.clone();
            let next_token = next_token.clone();
            Box::pin(async move {
                let Some((previous_token, grant)) =
                    load_refresh_token_with_grant(SqlExec::Tx(tx), &token_id).await?
                else {
                    return Ok(None);
                };
                if previous_token.consumed_at.is_some()
                    || previous_token.revoked_at.is_some()
                    || grant.revoked_at.is_some()
                {
                    return Ok(None);
                }
                let rows = tx
                    .execute(
                        "UPDATE oauth_refresh_tokens
                            SET consumed_at = {}
                          WHERE id = {}
                            AND consumed_at IS NULL
                            AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(consumed_at),
                            SqlArg::Text(previous_token.id.clone()),
                        ],
                    )
                    .await?;
                if rows == 0 {
                    return Ok(None);
                }
                insert_refresh_token_tx(tx, &next_token).await?;
                tx.execute(
                    "UPDATE oauth_refresh_grants
                        SET updated_at = {},
                            last_used_at = {}
                      WHERE id = {}",
                    &[
                        SqlArg::Timestamp(consumed_at),
                        SqlArg::Timestamp(consumed_at),
                        SqlArg::Text(grant.id.clone()),
                    ],
                )
                .await?;
                let grant = load_refresh_grant_by_id_tx(tx, &grant.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("OAuth grant {}", grant.id)))?;
                Ok(Some(OAuthRefreshRotation {
                    grant,
                    previous_token,
                }))
            })
        })
        .await
    }

    async fn revoke_refresh_grant(
        &self,
        grant_id: &str,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE oauth_refresh_grants
                SET revoked_at = COALESCE(revoked_at, {}),
                    revoked_reason = COALESCE(revoked_reason, {}),
                    updated_at = {}
              WHERE id = {}
                AND user_id = {}
                AND revoked_at IS NULL",
            &[
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(reason.to_string()),
                SqlArg::Timestamp(revoked_at),
                SqlArg::Text(grant_id.to_string()),
                SqlArg::Text(user_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn revoke_refresh_family(
        &self,
        family_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64> {
        let family_id = family_id.to_string();
        let reason = reason.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "revoke_oauth_refresh_family", move |tx| {
            let family_id = family_id.clone();
            let reason = reason.clone();
            Box::pin(async move {
                let grant_rows = tx
                    .execute(
                        "UPDATE oauth_refresh_grants
                            SET revoked_at = COALESCE(revoked_at, {}),
                                revoked_reason = COALESCE(revoked_reason, {}),
                                updated_at = {}
                          WHERE family_id = {}
                            AND revoked_at IS NULL",
                        &[
                            SqlArg::Timestamp(revoked_at),
                            SqlArg::Text(reason.clone()),
                            SqlArg::Timestamp(revoked_at),
                            SqlArg::Text(family_id.clone()),
                        ],
                    )
                    .await?;
                tx.execute(
                    "UPDATE oauth_refresh_tokens
                        SET revoked_at = COALESCE(revoked_at, {})
                      WHERE family_id = {}
                        AND revoked_at IS NULL",
                    &[SqlArg::Timestamp(revoked_at), SqlArg::Text(family_id)],
                )
                .await?;
                Ok(grant_rows)
            })
        })
        .await
    }

    async fn revoke_user_refresh_grants(
        &self,
        user_id: &str,
        revoked_at: chrono::DateTime<chrono::Utc>,
        reason: &str,
    ) -> AppResult<u64> {
        let user_id = user_id.to_string();
        let reason = reason.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "revoke_oauth_user_refresh_grants",
            move |tx| {
                let user_id = user_id.clone();
                let reason = reason.clone();
                Box::pin(async move {
                    let grant_rows = tx
                        .execute(
                            "UPDATE oauth_refresh_grants
                            SET revoked_at = COALESCE(revoked_at, {}),
                                revoked_reason = COALESCE(revoked_reason, {}),
                                updated_at = {}
                          WHERE user_id = {}
                            AND revoked_at IS NULL",
                            &[
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(reason),
                                SqlArg::Timestamp(revoked_at),
                                SqlArg::Text(user_id.clone()),
                            ],
                        )
                        .await?;
                    tx.execute(
                        "UPDATE oauth_refresh_tokens
                        SET revoked_at = COALESCE(revoked_at, {})
                      WHERE grant_id IN (
                            SELECT id FROM oauth_refresh_grants WHERE user_id = {}
                        )
                        AND revoked_at IS NULL",
                        &[SqlArg::Timestamp(revoked_at), SqlArg::Text(user_id)],
                    )
                    .await?;
                    Ok(grant_rows)
                })
            },
        )
        .await
    }

    async fn touch_refresh_grant_last_used(
        &self,
        grant_id: &str,
        client_id: &str,
        used_at: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<bool> {
        let rows = SqlRuntime::execute(
            self.datastore.read_exec(),
            "UPDATE oauth_refresh_grants
                SET updated_at = {},
                    last_used_at = {}
              WHERE id = {}
                AND client_id = {}
                AND revoked_at IS NULL",
            &[
                SqlArg::Timestamp(used_at),
                SqlArg::Timestamp(used_at),
                SqlArg::Text(grant_id.to_string()),
                SqlArg::Text(client_id.to_string()),
            ],
        )
        .await?;
        Ok(rows > 0)
    }

    async fn list_connected_apps(&self, user_id: &str) -> AppResult<Vec<OAuthConnectedAppRecord>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id AS grant_id, client_id, created_at, last_used_at
               FROM oauth_refresh_grants
              WHERE user_id = {}
                AND revoked_at IS NULL
              ORDER BY created_at DESC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_connected_app).collect()
    }
}

async fn load_authorization_code(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<OAuthAuthorizationCodeRecord>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT id, code_hash, client_id, user_id, redirect_uri, scope, code_challenge,
                code_challenge_method, created_at, expires_at, consumed_at
           FROM oauth_authorization_codes
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_authorization_code).transpose()
}

async fn load_refresh_token_with_grant(
    exec: SqlExec<'_, '_>,
    id: &str,
) -> AppResult<Option<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT t.id AS token_id, t.grant_id, t.family_id AS token_family_id, t.token_hash,
                t.created_at AS token_created_at, t.consumed_at, t.revoked_at AS token_revoked_at,
                g.id AS grant_row_id, g.family_id AS grant_family_id, g.user_id, g.client_id,
                g.scope,
                g.auth_session_version, g.created_at AS grant_created_at,
                g.updated_at AS grant_updated_at, g.last_used_at,
                g.revoked_at AS grant_revoked_at, g.revoked_reason
           FROM oauth_refresh_tokens t
           JOIN oauth_refresh_grants g ON g.id = t.grant_id
          WHERE t.id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref()
        .map(row_to_refresh_token_with_grant)
        .transpose()
}

async fn insert_refresh_grant_tx(
    tx: &mut SqlTx<'_>,
    grant: &OAuthRefreshGrantRecord,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO oauth_refresh_grants
            (id, family_id, user_id, client_id, scope, auth_session_version,
             created_at, updated_at, last_used_at, revoked_at, revoked_reason)
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(grant.id.clone()),
            SqlArg::Text(grant.family_id.clone()),
            SqlArg::Text(grant.user_id.clone()),
            SqlArg::Text(grant.client_id.clone()),
            SqlArg::Text(grant.scope.clone()),
            SqlArg::Text(grant.auth_session_version.clone()),
            SqlArg::Timestamp(grant.created_at),
            SqlArg::Timestamp(grant.updated_at),
            SqlArg::OptTimestamp(grant.last_used_at),
            SqlArg::OptTimestamp(grant.revoked_at),
            SqlArg::OptText(grant.revoked_reason.clone()),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_refresh_token_tx(
    tx: &mut SqlTx<'_>,
    token: &OAuthRefreshTokenRecord,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO oauth_refresh_tokens
            (id, grant_id, family_id, token_hash, created_at, consumed_at, revoked_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(token.id.clone()),
            SqlArg::Text(token.grant_id.clone()),
            SqlArg::Text(token.family_id.clone()),
            SqlArg::Text(token.token_hash.clone()),
            SqlArg::Timestamp(token.created_at),
            SqlArg::OptTimestamp(token.consumed_at),
            SqlArg::OptTimestamp(token.revoked_at),
        ],
    )
    .await?;
    Ok(())
}

async fn load_refresh_grant_by_id_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
) -> AppResult<Option<OAuthRefreshGrantRecord>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, family_id, user_id, client_id, scope, auth_session_version, created_at,
                updated_at, last_used_at, revoked_at, revoked_reason
           FROM oauth_refresh_grants
          WHERE id = {}",
        &[SqlArg::Text(id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_refresh_grant).transpose()
}

fn row_to_authorization_code(row: &SqlRow) -> AppResult<OAuthAuthorizationCodeRecord> {
    Ok(OAuthAuthorizationCodeRecord {
        id: row.text("id")?,
        code_hash: row.text("code_hash")?,
        client_id: row.text("client_id")?,
        user_id: row.text("user_id")?,
        redirect_uri: row.text("redirect_uri")?,
        scope: row.text("scope")?,
        code_challenge: row.text("code_challenge")?,
        code_challenge_method: row.text("code_challenge_method")?,
        created_at: row.timestamp("created_at")?,
        expires_at: row.timestamp("expires_at")?,
        consumed_at: row.opt_timestamp("consumed_at")?,
    })
}

fn row_to_refresh_grant(row: &SqlRow) -> AppResult<OAuthRefreshGrantRecord> {
    Ok(OAuthRefreshGrantRecord {
        id: row.text("id")?,
        family_id: row.text("family_id")?,
        user_id: row.text("user_id")?,
        client_id: row.text("client_id")?,
        scope: row.text("scope")?,
        auth_session_version: row.text("auth_session_version")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        revoked_at: row.opt_timestamp("revoked_at")?,
        revoked_reason: row.opt_text("revoked_reason")?,
    })
}

fn row_to_refresh_token_with_grant(
    row: &SqlRow,
) -> AppResult<(OAuthRefreshTokenRecord, OAuthRefreshGrantRecord)> {
    let token = OAuthRefreshTokenRecord {
        id: row.text("token_id")?,
        grant_id: row.text("grant_id")?,
        family_id: row.text("token_family_id")?,
        token_hash: row.text("token_hash")?,
        created_at: row.timestamp("token_created_at")?,
        consumed_at: row.opt_timestamp("consumed_at")?,
        revoked_at: row.opt_timestamp("token_revoked_at")?,
    };
    let grant = OAuthRefreshGrantRecord {
        id: row.text("grant_row_id")?,
        family_id: row.text("grant_family_id")?,
        user_id: row.text("user_id")?,
        client_id: row.text("client_id")?,
        scope: row.text("scope")?,
        auth_session_version: row.text("auth_session_version")?,
        created_at: row.timestamp("grant_created_at")?,
        updated_at: row.timestamp("grant_updated_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
        revoked_at: row.opt_timestamp("grant_revoked_at")?,
        revoked_reason: row.opt_text("revoked_reason")?,
    };
    Ok((token, grant))
}

fn row_to_connected_app(row: &SqlRow) -> AppResult<OAuthConnectedAppRecord> {
    Ok(OAuthConnectedAppRecord {
        grant_id: row.text("grant_id")?,
        client_id: row.text("client_id")?,
        created_at: row.timestamp("created_at")?,
        last_used_at: row.opt_timestamp("last_used_at")?,
    })
}
