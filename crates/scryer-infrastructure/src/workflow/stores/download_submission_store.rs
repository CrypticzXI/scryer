use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppResult, DownloadSourceIdentity, DownloadSubmission, DownloadSubmissionActorSnapshot,
    DownloadSubmissionIdentity, DownloadSubmissionRepository,
};
use scryer_domain::Id;

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, StoreDatastore};

#[derive(Clone)]
pub struct DownloadSubmissionStore {
    datastore: StoreDatastore,
}

impl DownloadSubmissionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

fn download_identity_state_key(
    identity: &DownloadSubmissionIdentity,
    source_identity: Option<&DownloadSourceIdentity>,
) -> Option<String> {
    let download_id = identity
        .download_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)?;

    if download_identity_state_is_global(&download_id) {
        return Some(format!("download:{download_id}"));
    }

    let source_identity = source_identity?;
    let client_type = source_identity.client_type.trim();
    if client_type.is_empty() {
        return None;
    }

    Some(format!(
        "client:{}:{}:download:{}",
        normalize_download_client_id(source_identity.client_id.as_deref()),
        client_type.to_ascii_lowercase(),
        download_id
    ))
}

fn download_identity_state_is_global(download_id: &str) -> bool {
    let value = download_id.trim();
    value.starts_with("scryer-download:") || looks_like_torrent_info_hash(value)
}

fn looks_like_torrent_info_hash(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[async_trait]
impl DownloadSubmissionRepository for DownloadSubmissionStore {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        SqlRuntime::run_in_transaction(&self.datastore, "record_download_submission", move |tx| {
            let submission = submission.clone();
            Box::pin(async move {
                record_download_submission_tx(tx, &submission).await?;
                Ok(())
            })
        })
        .await
    }

    async fn record_submission_with_identity(
        &self,
        submission: DownloadSubmission,
        submission_identity: DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_with_identity",
            move |tx| {
                let submission = submission.clone();
                let submission_identity = submission_identity.clone();
                Box::pin(async move {
                    record_download_submission_with_identity_tx(
                        tx,
                        &submission,
                        &submission_identity,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn record_submission_identity(
        &self,
        identity: &DownloadSourceIdentity,
        submission_identity: &DownloadSubmissionIdentity,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let submission_identity = submission_identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_identity",
            move |tx| {
                let identity = identity.clone();
                let submission_identity = submission_identity.clone();
                Box::pin(async move {
                    record_download_submission_identity_tx(tx, &identity, &submission_identity)
                        .await
                })
            },
        )
        .await
    }

    async fn record_submission_actor_snapshot(
        &self,
        identity: &DownloadSourceIdentity,
        actor: DownloadSubmissionActorSnapshot,
    ) -> AppResult<()> {
        let identity = identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_actor_snapshot",
            move |tx| {
                let identity = identity.clone();
                let actor = actor.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE download_submissions
                         SET actor_kind = {},
                             actor_user_id = {},
                             actor_display_name = {}
                         WHERE download_client_type = {}
                           AND download_client_item_id = {}
                           AND download_client_id = {}",
                        &[
                            SqlArg::Text(actor.kind.as_str().to_string()),
                            SqlArg::OptText(actor.user_id),
                            SqlArg::Text(actor.display_name),
                            SqlArg::Text(identity.client_type),
                            SqlArg::Text(identity.item_id),
                            SqlArg::Text(normalize_download_client_id(
                                identity.client_id.as_deref(),
                            )),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_submission_actor_snapshot(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmissionActorSnapshot>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT actor_kind, actor_user_id, actor_display_name
             FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| download_submission_actor_snapshot_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE download_client_type = {} AND download_client_item_id = {} AND download_client_id = {} LIMIT 1",
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn list_by_download_id(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE download_client_type = {} AND download_client_id = {} AND download_id = {} ORDER BY submitted_at DESC, id DESC",
        );
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(client_type.trim().to_ascii_lowercase()),
                SqlArg::Text(normalize_download_client_id(client_id)),
                SqlArg::Text(download_id.trim().to_string()),
            ],
        )
        .await
    }

    async fn get_submission_identity(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmissionIdentity>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT download_id
             FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| {
            Ok(DownloadSubmissionIdentity {
                download_id: row.opt_text("download_id")?,
            })
        })
        .transpose()
    }

    async fn record_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
        tracked_state: &str,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<()> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(());
        };
        let identity = identity.clone();
        let source_identity = source_identity.cloned();
        let tracked_state = tracked_state.to_string();
        let reason = reason.map(str::to_string);
        let detail = detail.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_identity_tracked_state",
            move |tx| {
                let identity_key = identity_key.clone();
                let identity = identity.clone();
                let source_identity = source_identity.clone();
                let tracked_state = tracked_state.clone();
                let reason = reason.clone();
                let detail = detail.clone();
                Box::pin(async move {
                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_identity_states
                         (id, identity_key, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET download_id = excluded.download_id,
                             client_id = excluded.client_id,
                             client_type = excluded.client_type,
                             download_client_item_id = excluded.download_client_item_id,
                             tracked_state = excluded.tracked_state,
                             reason = excluded.reason,
                             detail = excluded.detail,
                             updated_at = excluded.updated_at",
                        &[
                            SqlArg::Text(Id::new().0),
                            SqlArg::Text(identity_key),
                            SqlArg::OptText(identity.download_id),
                            SqlArg::OptText(source_identity.as_ref().map(|source| {
                                normalize_download_client_id(source.client_id.as_deref())
                            })),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.client_type.clone()),
                            ),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.item_id.clone()),
                            ),
                            SqlArg::Text(tracked_state),
                            SqlArg::OptText(reason),
                            SqlArg::OptText(detail),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_identity_tracked_state(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };

        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state
             FROM download_identity_states
             WHERE identity_key = {}
             LIMIT 1",
            &[SqlArg::Text(identity_key)],
        )
        .await?;
        row.map(|row| row.text("tracked_state")).transpose()
    }

    async fn upsert_identity_tracked_state_returning_previous(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
        tracked_state: &str,
        preserve_previous: &[&str],
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };
        let identity = identity.clone();
        let source_identity = source_identity.cloned();
        let tracked_state = tracked_state.to_string();
        let preserve_previous = preserve_previous
            .iter()
            .map(|state| state.to_string())
            .collect::<Vec<_>>();
        let reason = reason.map(str::to_string);
        let detail = detail.map(str::to_string);
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "upsert_download_identity_tracked_state",
            move |tx| {
                let identity_key = identity_key.clone();
                let identity = identity.clone();
                let source_identity = source_identity.clone();
                let tracked_state = tracked_state.clone();
                let preserve_previous = preserve_previous.clone();
                let reason = reason.clone();
                let detail = detail.clone();
                Box::pin(async move {
                    let previous = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT tracked_state
                         FROM download_identity_states
                         WHERE identity_key = {}
                         LIMIT 1",
                        &[SqlArg::Text(identity_key.clone())],
                    )
                    .await?
                    .map(|row| row.text("tracked_state"))
                    .transpose()?;
                    if let Some(previous) = previous.as_deref().filter(|previous| {
                        preserve_previous
                            .iter()
                            .any(|preserved| preserved == previous)
                    }) {
                        // The read and this early return share the transaction,
                        // so a terminal outcome can never be flipped by a
                        // concurrent ignore.
                        return Ok(Some(previous.to_string()));
                    }
                    let now = Utc::now();
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_identity_states
                         (id, identity_key, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET download_id = excluded.download_id,
                             client_id = excluded.client_id,
                             client_type = excluded.client_type,
                             download_client_item_id = excluded.download_client_item_id,
                             tracked_state = excluded.tracked_state,
                             reason = excluded.reason,
                             detail = excluded.detail,
                             updated_at = excluded.updated_at",
                        &[
                            SqlArg::Text(Id::new().0),
                            SqlArg::Text(identity_key),
                            SqlArg::OptText(identity.download_id),
                            SqlArg::OptText(source_identity.as_ref().map(|source| {
                                normalize_download_client_id(source.client_id.as_deref())
                            })),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.client_type.clone()),
                            ),
                            SqlArg::OptText(
                                source_identity
                                    .as_ref()
                                    .map(|source| source.item_id.clone()),
                            ),
                            SqlArg::Text(tracked_state),
                            SqlArg::OptText(reason),
                            SqlArg::OptText(detail),
                            SqlArg::Timestamp(now),
                            SqlArg::Timestamp(now),
                        ],
                    )
                    .await?;
                    Ok(previous)
                })
            },
        )
        .await
    }

    async fn list_identity_tracked_states_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<(DownloadSourceIdentity, String)>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }

        let mut states = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(client_type = {} AND download_client_item_id = {} AND COALESCE(client_id, '') = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let rows = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!(
                    "SELECT client_id, client_type, download_client_item_id, tracked_state
                     FROM download_identity_states
                     WHERE {clauses}
                     ORDER BY updated_at ASC, id ASC"
                ),
                &args,
            )
            .await?;
            for row in rows {
                states.push((
                    DownloadSourceIdentity::new(
                        row.opt_text("client_id")?.as_deref(),
                        row.text("client_type")?,
                        row.text("download_client_item_id")?,
                    ),
                    row.text("tracked_state")?,
                ));
            }
        }
        Ok(states)
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let chunks = chunk_download_submission_client_items(client_items);
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for chunk in chunks {
            let mut args = Vec::with_capacity(chunk.len() * 3);
            let clauses = chunk
                .iter()
                .map(|identity| {
                    args.push(SqlArg::Text(identity.client_type.clone()));
                    args.push(SqlArg::Text(identity.item_id.clone()));
                    args.push(SqlArg::Text(normalize_download_client_id(
                        identity.client_id.as_deref(),
                    )));
                    "(download_client_type = {} AND download_client_item_id = {} AND download_client_id = {})"
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = download_submission_select_sql(&self.datastore, &format!("WHERE {clauses}"));
            out.extend(fetch_download_submissions(self.datastore.read_exec(), &sql, &args).await?);
        }
        Ok(out)
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        let sql = download_submission_select_sql(&self.datastore, "WHERE title_id = {}");
        fetch_download_submissions(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(title_id.to_string())],
        )
        .await
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
        purpose: scryer_application::DownloadSubmissionPurpose,
        scope: &scryer_application::SubmissionScope,
    ) -> AppResult<Option<DownloadSubmission>> {
        let recent_cutoff = Utc::now() - chrono::Duration::seconds(30);
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {} AND request_signature = {} AND purpose = {} AND COALESCE(tracked_state, '') = '' AND submitted_at >= {} ORDER BY submitted_at DESC, id DESC",
        );
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(request_signature.to_string()),
                SqlArg::Text(purpose.as_str().to_string()),
                SqlArg::Timestamp(recent_cutoff),
            ],
        )
        .await?;
        for row in rows {
            let submission = download_submission_from_row(&row)?;
            if &submission.scope == scope {
                return Ok(Some(submission));
            }
        }
        Ok(None)
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submissions_for_title",
            move |tx| {
                let title_id = title_id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE EXISTS (
                             SELECT 1
                               FROM download_submissions
                              WHERE download_submissions.download_client_id = download_submission_episode_links.download_client_id
                                AND download_submissions.download_client_type = download_submission_episode_links.download_client_type
                                AND download_submissions.download_client_item_id = download_submission_episode_links.download_client_item_id
                                AND download_submissions.title_id = {}
                         )",
                        &[SqlArg::Text(title_id.clone())],
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions WHERE title_id = {}",
                        &[SqlArg::Text(title_id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        let identity = identity.clone();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "delete_download_submission_by_client_item_id",
            move |tx| {
                let identity = identity.clone();
                Box::pin(async move {
                    let normalized_client_id =
                        normalize_download_client_id(identity.client_id.as_deref());
                    let args = [
                        SqlArg::Text(normalized_client_id.clone()),
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
                    ];
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submission_episode_links
                         WHERE download_client_id = {}
                           AND download_client_type = {}
                           AND download_client_item_id = {}",
                        &args,
                    )
                    .await?;
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "DELETE FROM download_submissions
                         WHERE download_client_id = {}
                           AND download_client_type = {}
                           AND download_client_item_id = {}",
                        &args,
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let tracked_state = tracked_state.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_tracked_state", move |tx| {
            let identity = identity.clone();
            let tracked_state = tracked_state.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_submissions
                     (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, purpose, episode_id, collection_id, tracked_state, tracked_state_at)
                     VALUES ({}, '', '', {}, {}, {}, NULL, NULL, NULL, NULL, 'standard', NULL, NULL, {}, {})
                     ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
                     SET tracked_state = excluded.tracked_state,
                         tracked_state_at = excluded.tracked_state_at",
                    &[
                        SqlArg::Text(Id::new().0),
                        SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                        SqlArg::Text(identity.client_type),
                        SqlArg::Text(identity.item_id),
                        SqlArg::Text(tracked_state),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state FROM download_submissions
             WHERE download_client_type = {}
               AND download_client_item_id = {}
               AND download_client_id = {}
             LIMIT 1",
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| row.opt_text("tracked_state"))
            .transpose()
            .map(Option::flatten)
    }
}
