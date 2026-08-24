use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppResult, DownloadSourceIdentity, DownloadSubmission, DownloadSubmissionActorSnapshot,
    DownloadSubmissionIdentity, DownloadSubmissionRepository, PersistedSeedGoals,
    SeedGoalGrabRecord, SeedGoalResolutionSource,
};
use scryer_domain::{Id, download_identity::DownloadId};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, StoreDatastore};

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

fn canonical_download_id_for_tracked_state(
    canonical_download_id: Option<&DownloadId>,
    identity: &DownloadSubmissionIdentity,
) -> Option<String> {
    canonical_download_id.map(ToString::to_string).or_else(|| {
        identity
            .download_id
            .as_deref()
            .and_then(DownloadId::from_wire)
            .map(|download_id| download_id.to_string())
    })
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

    async fn record_ambiguous_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_ambiguous_download_submission",
            move |tx| {
                let submission = submission.clone();
                Box::pin(
                    async move { record_ambiguous_download_submission_tx(tx, &submission).await },
                )
            },
        )
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
            "WHERE download_client_type = {} AND download_client_id = {} AND download_id = {} AND download_client_item_id IS NOT NULL ORDER BY submitted_at DESC, id DESC",
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
        self.record_identity_tracked_state_for_download(
            None,
            identity,
            source_identity,
            tracked_state,
            reason,
            detail,
        )
        .await
    }

    async fn record_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
        tracked_state: &str,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<()> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(());
        };
        let canonical_download_id =
            canonical_download_id_for_tracked_state(canonical_download_id, identity);
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
                let canonical_download_id = canonical_download_id.clone();
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
                         (id, identity_key, canonical_download_id, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET canonical_download_id = excluded.canonical_download_id,
                             download_id = excluded.download_id,
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
                            SqlArg::OptText(canonical_download_id),
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

    async fn get_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(canonical_download_id) = canonical_download_id else {
            return self
                .get_identity_tracked_state(identity, source_identity)
                .await;
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        if let Some(row) = row {
            return row.text("tracked_state").map(Some);
        }
        self.get_identity_tracked_state(identity, source_identity)
            .await
    }

    async fn get_identity_tracked_state_reason(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };

        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT reason
             FROM download_identity_states
             WHERE identity_key = {}
             LIMIT 1",
            &[SqlArg::Text(identity_key)],
        )
        .await?;
        row.map(|row| row.opt_text("reason"))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_identity_tracked_state_reason_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(canonical_download_id) = canonical_download_id else {
            return self
                .get_identity_tracked_state_reason(identity, source_identity)
                .await;
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT reason
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        if let Some(row) = row {
            return row.opt_text("reason");
        }
        self.get_identity_tracked_state_reason(identity, source_identity)
            .await
    }

    async fn get_identity_tracked_state_detail(
        &self,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(identity_key) = download_identity_state_key(identity, source_identity) else {
            return Ok(None);
        };

        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT detail
             FROM download_identity_states
             WHERE identity_key = {}
             LIMIT 1",
            &[SqlArg::Text(identity_key)],
        )
        .await?;
        row.map(|row| row.opt_text("detail"))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_identity_tracked_state_detail_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&DownloadSourceIdentity>,
    ) -> AppResult<Option<String>> {
        let Some(canonical_download_id) = canonical_download_id else {
            return self
                .get_identity_tracked_state_detail(identity, source_identity)
                .await;
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT detail
             FROM download_identity_states
             WHERE canonical_download_id = {}
             ORDER BY updated_at DESC, id DESC
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        if let Some(row) = row {
            return row.opt_text("detail");
        }
        self.get_identity_tracked_state_detail(identity, source_identity)
            .await
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
        self.upsert_identity_tracked_state_for_download_returning_previous(
            None,
            identity,
            source_identity,
            tracked_state,
            preserve_previous,
            reason,
            detail,
        )
        .await
    }

    async fn upsert_identity_tracked_state_for_download_returning_previous(
        &self,
        canonical_download_id: Option<&DownloadId>,
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
        let canonical_download_id =
            canonical_download_id_for_tracked_state(canonical_download_id, identity);
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
                let canonical_download_id = canonical_download_id.clone();
                let identity = identity.clone();
                let source_identity = source_identity.clone();
                let tracked_state = tracked_state.clone();
                let preserve_previous = preserve_previous.clone();
                let reason = reason.clone();
                let detail = detail.clone();
                Box::pin(async move {
                    let previous = if let Some(canonical_download_id) = &canonical_download_id {
                        SqlRuntime::fetch_optional(
                            SqlExec::Tx(tx),
                            "SELECT tracked_state
                             FROM download_identity_states
                             WHERE canonical_download_id = {}
                             ORDER BY updated_at DESC, id DESC
                             LIMIT 1",
                            &[SqlArg::Text(canonical_download_id.clone())],
                        )
                        .await?
                    } else {
                        SqlRuntime::fetch_optional(
                            SqlExec::Tx(tx),
                            "SELECT tracked_state
                             FROM download_identity_states
                             WHERE identity_key = {}
                             LIMIT 1",
                            &[SqlArg::Text(identity_key.clone())],
                        )
                        .await?
                    }
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
                         (id, identity_key, canonical_download_id, download_id,
                          client_id, client_type, download_client_item_id,
                          tracked_state, reason, detail, created_at, updated_at)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(identity_key) DO UPDATE
                         SET canonical_download_id = excluded.canonical_download_id,
                             download_id = excluded.download_id,
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
                            SqlArg::OptText(canonical_download_id),
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
        let sql = download_submission_select_sql(
            &self.datastore,
            "WHERE title_id = {} AND download_client_item_id IS NOT NULL",
        );
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
            "WHERE title_id = {} AND request_signature = {} AND purpose = {} AND download_client_item_id IS NOT NULL AND COALESCE(tracked_state, '') = '' AND submitted_at >= {} ORDER BY submitted_at DESC, id DESC",
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
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
                        SqlArg::Text(tracked_state),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                sync_canonical_download_rows_tx(tx, &identity).await?;
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

    /// Upsert, not update: the download-client choke point resolves and records
    /// the goals as soon as the client accepts the torrent, which happens
    /// before the acquisition layer records the submission itself. The insert
    /// carries the title/facet/purpose the router already knows, so the row is
    /// never a bare orphan stub, and `record_download_submission_tx` later
    /// conflict-updates the remaining columns without touching the seed ones.
    async fn record_seed_goals(&self, record: SeedGoalGrabRecord) -> AppResult<()> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "record_download_submission_seed_goals",
            move |tx| {
                let record = record.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "INSERT INTO download_submissions
                         (id, title_id, facet, download_client_id, download_client_type,
                          download_client_item_id, purpose, seeding_profile_id, seed_goal_ratio,
                          seed_goal_seconds, seed_never_remove, seed_goal_met_action,
                          seed_post_import_tracking, seed_goal_source, seed_info_hash)
                         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})
                         ON CONFLICT(download_client_id, download_client_type, download_client_item_id) DO UPDATE
                         SET seeding_profile_id = excluded.seeding_profile_id,
                             seed_goal_ratio = excluded.seed_goal_ratio,
                             seed_goal_seconds = excluded.seed_goal_seconds,
                             seed_never_remove = excluded.seed_never_remove,
                             seed_goal_met_action = excluded.seed_goal_met_action,
                             seed_post_import_tracking = excluded.seed_post_import_tracking,
                             seed_goal_source = excluded.seed_goal_source,
                             seed_info_hash = excluded.seed_info_hash",
                        &[
                            SqlArg::Text(record.download_id.to_string()),
                            SqlArg::Text(record.title_id.clone()),
                            SqlArg::Text(record.facet.clone()),
                            SqlArg::Text(normalize_download_client_id(
                                record.client_id.as_deref(),
                            )),
                            SqlArg::Text(record.client_type.clone()),
                            SqlArg::Text(record.client_item_id.clone()),
                            SqlArg::Text(record.purpose.as_str().to_string()),
                            SqlArg::OptText(record.goals.seeding_profile_id.clone()),
                            SqlArg::OptF64(record.goals.seed_goal_ratio),
                            SqlArg::OptI64(record.goals.seed_goal_seconds),
                            SqlArg::OptBool(Some(record.goals.never_remove)),
                            SqlArg::OptText(
                                record
                                    .goals
                                    .goal_met_action
                                    .map(|action| action.as_str().to_string()),
                            ),
                            SqlArg::OptText(Some(
                                record.goals.post_import_tracking.as_str().to_string(),
                            )),
                            SqlArg::OptText(Some(
                                record.goals.resolution_source.as_str().to_string(),
                            )),
                            SqlArg::OptText(normalized_info_hash(
                                record.goals.info_hash.as_deref(),
                            )),
                        ],
                    )
                    .await?;
                    sync_canonical_download_rows_tx(
                        tx,
                        &DownloadSourceIdentity::new(
                            record.client_id.as_deref(),
                            &record.client_type,
                            &record.client_item_id,
                        ),
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_seed_goals(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE download_client_type = {{}}
                   AND download_client_item_id = {{}}
                   AND download_client_id = {{}}
                 LIMIT 1"
            ),
            &[
                SqlArg::Text(identity.client_type.clone()),
                SqlArg::Text(identity.item_id.clone()),
                SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
            ],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }

    async fn list_seed_goals_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<(DownloadSourceIdentity, PersistedSeedGoals)>> {
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
            let rows = SqlRuntime::fetch_all(
                self.datastore.read_exec(),
                &format!(
                    "SELECT download_client_id, download_client_type, download_client_item_id, \
                     {SEED_GOAL_COLUMNS}
                     FROM download_submissions
                     WHERE {clauses}"
                ),
                &args,
            )
            .await?;
            for row in rows {
                let Some(goals) = seed_goals_from_row(&row)? else {
                    continue;
                };
                let client_id = row.opt_text("download_client_id")?;
                out.push((
                    DownloadSourceIdentity::new(
                        client_id.as_deref(),
                        &row.text("download_client_type")?,
                        &row.text("download_client_item_id")?,
                    ),
                    goals,
                ));
            }
        }
        Ok(out)
    }

    async fn find_seed_goals_by_info_hash(
        &self,
        info_hash: &str,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let Some(info_hash) = normalized_info_hash(Some(info_hash)) else {
            return Ok(None);
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE seed_info_hash = {{}}
                 ORDER BY submitted_at DESC
                 LIMIT 1"
            ),
            &[SqlArg::Text(info_hash)],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }
}

const SEED_GOAL_COLUMNS: &str = "seeding_profile_id, seed_goal_ratio, seed_goal_seconds, \
     seed_never_remove, seed_goal_met_action, seed_post_import_tracking, seed_goal_source, \
     seed_info_hash";

/// Info hashes are compared case-insensitively across clients; store and look
/// them up in one canonical form.
fn normalized_info_hash(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

/// `None` when the row predates any seeding resolution (or the grab resolved to
/// no profile at all), so callers can tell "not evaluated" from "evaluated to
/// nothing".
fn seed_goals_from_row(row: &SqlRow) -> AppResult<Option<PersistedSeedGoals>> {
    let Some(source) = row
        .opt_text("seed_goal_source")?
        .as_deref()
        .and_then(SeedGoalResolutionSource::parse)
    else {
        return Ok(None);
    };
    if source == SeedGoalResolutionSource::None {
        return Ok(None);
    }
    Ok(Some(PersistedSeedGoals {
        seeding_profile_id: row.opt_text("seeding_profile_id")?,
        seed_goal_ratio: row.opt_f64("seed_goal_ratio")?,
        seed_goal_seconds: row.opt_i64("seed_goal_seconds")?,
        never_remove: row.opt_bool("seed_never_remove")?.unwrap_or(false),
        goal_met_action: row
            .opt_text("seed_goal_met_action")?
            .as_deref()
            .and_then(scryer_domain::SeedGoalMetAction::parse),
        // Absent (rows written before migration 0166) or unparseable reads as
        // `Park`: Scryer keeps managing the torrent, which is the direction
        // that cannot lose a seeding obligation.
        post_import_tracking: row
            .opt_text("seed_post_import_tracking")?
            .as_deref()
            .and_then(scryer_domain::PostImportTracking::parse)
            .unwrap_or_default(),
        resolution_source: source,
        info_hash: row.opt_text("seed_info_hash")?,
    }))
}

#[cfg(test)]
mod seed_goal_tests {
    use std::sync::Arc;

    use scryer_application::{DownloadSubmissionPurpose, SubmissionScope};
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn store() -> DownloadSubmissionStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE download_submissions (
                 id TEXT PRIMARY KEY,
                 title_id TEXT NOT NULL,
                 facet TEXT NOT NULL,
                 download_client_id TEXT NOT NULL DEFAULT '',
                 download_client_type TEXT NOT NULL,
                 download_client_item_id TEXT,
                 source_title TEXT,
                 release_size_bytes INTEGER,
                 submitted_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                 collection_id TEXT,
                 tracked_state TEXT,
                 tracked_state_at TEXT,
                 source_hint TEXT,
                 source_provider_id TEXT,
                 source_provider_name TEXT,
                 source_kind TEXT,
                 request_signature TEXT,
                 episode_id TEXT,
                 download_id TEXT,
                 purpose TEXT NOT NULL DEFAULT 'standard',
                 series_movie_link_id TEXT,
                 actor_kind TEXT,
                 actor_user_id TEXT,
                 actor_display_name TEXT,
                 UNIQUE(download_client_id, download_client_type, download_client_item_id)
             )",
        )
        .execute(&pool)
        .await
        .expect("submission table should be created");
        sqlx::query(
            "CREATE TABLE download_submission_episode_links (
                 download_client_id TEXT NOT NULL,
                 download_client_type TEXT NOT NULL,
                 download_client_item_id TEXT NOT NULL,
                 episode_id TEXT NOT NULL,
                 PRIMARY KEY (download_client_id, download_client_type, download_client_item_id, episode_id)
             )",
        )
        .execute(&pool)
        .await
        .expect("episode link table should be created");
        sqlx::query(
            "CREATE TABLE download_clients (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL
             );
             CREATE TABLE downloads (
                 id TEXT PRIMARY KEY,
                 origin TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 first_observed_at TEXT,
                 last_observed_at TEXT,
                 terminal_at TEXT
             );
             CREATE TABLE download_client_bindings (
                 download_id TEXT PRIMARY KEY,
                 client_config_id TEXT,
                 client_type_snapshot TEXT,
                 client_name_snapshot TEXT,
                 native_item_id TEXT,
                 created_at TEXT NOT NULL,
                 last_seen_at TEXT,
                 ended_at TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("canonical download fixture tables should be created");
        for statement in include_str!(
            "../../../../scryer/src/db/migrations/0164_download_submission_seed_goals.sql"
        )
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("seed goal migration should apply");
        }
        // 0166 also touches `seeding_profiles`, which this fixture does not
        // create; apply only its download-submission statements.
        for statement in include_str!(
            "../../../../scryer/src/db/migrations/0166_seeding_profile_post_import_tracking.sql"
        )
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty() && statement.contains("download_submissions"))
        {
            sqlx::query(statement)
                .execute(&pool)
                .await
                .expect("post-import tracking migration should apply");
        }
        DownloadSubmissionStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    fn identity() -> DownloadSourceIdentity {
        DownloadSourceIdentity::new(Some("primary"), "qbittorrent", "job-1")
    }

    fn record() -> SeedGoalGrabRecord {
        SeedGoalGrabRecord {
            download_id: scryer_domain::download_identity::DownloadId::new(),
            client_id: Some("primary".to_string()),
            client_type: "qbittorrent".to_string(),
            client_item_id: "job-1".to_string(),
            title_id: "title-1".to_string(),
            facet: "series".to_string(),
            purpose: DownloadSubmissionPurpose::Standard,
            goals: PersistedSeedGoals {
                seeding_profile_id: Some("profile-1".to_string()),
                seed_goal_ratio: Some(2.5),
                seed_goal_seconds: Some(7200),
                never_remove: true,
                goal_met_action: Some(scryer_domain::SeedGoalMetAction::StopSeeding),
                post_import_tracking: scryer_domain::PostImportTracking::HandOff,
                resolution_source: SeedGoalResolutionSource::Indexer,
                info_hash: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01".to_string()),
            },
        }
    }

    #[tokio::test]
    async fn seed_goals_load_in_one_batch_for_the_queue_projection() {
        let store = store().await;
        store
            .record_seed_goals(record())
            .await
            .expect("seed goals should persist");
        let mut second = record();
        second.client_item_id = "job-2".to_string();
        second.goals.seed_goal_ratio = Some(1.25);
        second.goals.info_hash = None;
        store
            .record_seed_goals(second)
            .await
            .expect("seed goals should persist");

        let loaded = store
            .list_seed_goals_for_client_items(&[
                identity(),
                DownloadSourceIdentity::new(Some("primary"), "qbittorrent", "job-2"),
                // A row with no resolution at all must simply be absent, not a
                // default-valued entry that would read as "no goals resolved".
                DownloadSourceIdentity::new(Some("primary"), "qbittorrent", "job-missing"),
            ])
            .await
            .expect("batch read should succeed");

        let mut by_item = loaded
            .into_iter()
            .map(|(identity, goals)| (identity.item_id, goals))
            .collect::<Vec<_>>();
        by_item.sort_by(|left, right| left.0.cmp(&right.0));
        assert_eq!(by_item.len(), 2);
        assert_eq!(by_item[0].0, "job-1");
        assert_eq!(by_item[0].1.seed_goal_ratio, Some(2.5));
        assert!(by_item[0].1.never_remove);
        assert_eq!(by_item[1].0, "job-2");
        assert_eq!(by_item[1].1.seed_goal_ratio, Some(1.25));
    }

    #[tokio::test]
    async fn seed_goals_round_trip_by_client_identity_and_info_hash() {
        let store = store().await;
        store
            .record_seed_goals(record())
            .await
            .expect("seed goals should persist");

        let loaded = store
            .get_seed_goals(&identity())
            .await
            .expect("read should succeed")
            .expect("goals should be present");
        assert_eq!(loaded.seeding_profile_id.as_deref(), Some("profile-1"));
        assert_eq!(loaded.seed_goal_ratio, Some(2.5));
        assert_eq!(loaded.seed_goal_seconds, Some(7200));
        assert!(loaded.never_remove);
        assert_eq!(
            loaded.goal_met_action,
            Some(scryer_domain::SeedGoalMetAction::StopSeeding)
        );
        assert_eq!(
            loaded.post_import_tracking,
            scryer_domain::PostImportTracking::HandOff
        );
        assert_eq!(loaded.resolution_source, SeedGoalResolutionSource::Indexer);
        // Stored lowercased so an observed hash from any client matches.
        assert_eq!(
            loaded.info_hash.as_deref(),
            Some("abcdef0123456789abcdef0123456789abcdef01")
        );

        let by_hash = store
            .find_seed_goals_by_info_hash("AbCdEf0123456789ABCDEF0123456789abcdef01")
            .await
            .expect("read should succeed")
            .expect("goals should be found by info hash");
        assert_eq!(by_hash, loaded);
    }

    #[tokio::test]
    async fn rows_without_a_resolution_read_back_as_none() {
        let store = store().await;
        store
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                scope: SubmissionScope::Title,
                title_id: "title-1".to_string(),
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "qbittorrent".to_string(),
                download_client_item_id: "job-1".to_string(),
                source_hint: None,
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: None,
                release_size_bytes: None,
                request_signature: None,
                purpose: DownloadSubmissionPurpose::Standard,
            })
            .await
            .expect("submission should record");

        assert_eq!(
            store
                .get_seed_goals(&identity())
                .await
                .expect("read should succeed"),
            None
        );
        assert_eq!(
            store
                .find_seed_goals_by_info_hash("abcdef0123456789abcdef0123456789abcdef01")
                .await
                .expect("read should succeed"),
            None
        );
    }

    /// Rows frozen before migration 0166 carry a NULL tracking mode. They must
    /// read back as `Park` — Scryer keeps managing them — because the other
    /// direction would silently stop tracking torrents that were grabbed under
    /// a profile that never offered the choice.
    #[tokio::test]
    async fn a_row_written_before_the_tracking_column_reads_back_as_park() {
        let store = store().await;
        store
            .record_seed_goals(record())
            .await
            .expect("seed goals should persist");
        SqlRuntime::run_in_transaction(&store.datastore, "clear_post_import_tracking", |tx| {
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE download_submissions SET seed_post_import_tracking = NULL",
                    &[],
                )
                .await?;
                Ok(())
            })
        })
        .await
        .expect("column should clear");

        let loaded = store
            .get_seed_goals(&identity())
            .await
            .expect("read should succeed")
            .expect("goals should be present");
        assert_eq!(
            loaded.post_import_tracking,
            scryer_domain::PostImportTracking::Park
        );
    }

    /// The router writes the goals before the acquisition layer records the
    /// submission, so the later insert must fill the row in without clobbering
    /// what the choke point already froze onto it.
    #[tokio::test]
    async fn recording_the_submission_afterwards_preserves_the_seed_goals() {
        let store = store().await;
        let record = record();
        let seeded_download_id = record.download_id;
        store
            .record_seed_goals(record)
            .await
            .expect("seed goals should persist");
        let stub = store
            .find_by_client_item_id(&identity())
            .await
            .expect("stub should be readable")
            .expect("record_seed_goals should create an externally visible row");
        assert_eq!(stub.title_id, "title-1");
        assert_eq!(stub.download_client_id.as_deref(), Some("primary"));
        assert_eq!(stub.download_client_type, "qbittorrent");
        assert_eq!(stub.download_client_item_id, "job-1");
        assert_eq!(stub.download_id, seeded_download_id);
        store
            .record_submission(DownloadSubmission {
                download_id: scryer_domain::download_identity::DownloadId::new(),
                scope: SubmissionScope::Episode {
                    episode_id: "episode-1".to_string(),
                },
                title_id: "title-1".to_string(),
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "qbittorrent".to_string(),
                download_client_item_id: "job-1".to_string(),
                source_hint: Some("https://example.invalid/release.torrent".to_string()),
                source_provider_id: None,
                source_provider_name: None,
                source_kind: None,
                source_title: Some("Test Release".to_string()),
                release_size_bytes: None,
                request_signature: Some("sig".to_string()),
                purpose: DownloadSubmissionPurpose::Standard,
            })
            .await
            .expect("submission should record");

        let loaded = store
            .get_seed_goals(&identity())
            .await
            .expect("read should succeed")
            .expect("goals should survive the submission upsert");
        assert_eq!(loaded.seed_goal_ratio, Some(2.5));

        let submission = store
            .find_by_client_item_id(&identity())
            .await
            .expect("read should succeed")
            .expect("submission should be present");
        assert_eq!(
            submission.scope,
            SubmissionScope::Episode {
                episode_id: "episode-1".to_string()
            }
        );
        assert_eq!(submission.source_title.as_deref(), Some("Test Release"));
        assert_eq!(submission.download_id, seeded_download_id);
    }

    fn ambiguous_submission(
        download_id: scryer_domain::download_identity::DownloadId,
    ) -> DownloadSubmission {
        DownloadSubmission {
            download_id,
            scope: SubmissionScope::Title,
            title_id: "title-ambiguous".to_string(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            // `record_ambiguous_submission` deliberately persists NULL here.
            download_client_item_id: String::new(),
            source_hint: Some("https://indexer.invalid/release.nzb".to_string()),
            source_provider_id: Some("indexer-1".to_string()),
            source_provider_name: Some("Indexer".to_string()),
            source_kind: Some(scryer_application::DownloadSourceKind::NzbUrl),
            source_title: Some("Ambiguous Release".to_string()),
            release_size_bytes: Some(123),
            request_signature: Some("ambiguous-signature".to_string()),
            purpose: DownloadSubmissionPurpose::Standard,
        }
    }

    #[tokio::test]
    async fn ambiguous_submissions_are_unbound_durable_rows_and_hidden_from_legacy_readers() {
        let store = store().await;
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_clients (id, name) VALUES ({}, {})",
            &[
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("Primary SAB".to_string()),
            ],
        )
        .await
        .expect("client fixture should insert");
        let first = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000001",
        )
        .expect("fixed UUID should parse");
        let second = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000002",
        )
        .expect("fixed UUID should parse");

        store
            .record_ambiguous_submission(ambiguous_submission(first))
            .await
            .expect("first ambiguous mutation should persist");
        store
            .record_ambiguous_submission(ambiguous_submission(second))
            .await
            .expect("a later ambiguous mutation must coexist");

        let rows = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id, download_client_item_id FROM download_submissions
             WHERE title_id = {} ORDER BY id",
            &[SqlArg::Text("title-ambiguous".to_string())],
        )
        .await
        .expect("ambiguous rows should be readable directly");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].text("id").expect("first id"), first.to_string());
        assert_eq!(rows[1].text("id").expect("second id"), second.to_string());
        assert!(rows.iter().all(|row| {
            row.opt_text("download_client_item_id")
                .expect("nullable item id")
                .is_none()
        }));

        let bindings = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT download_id, client_config_id, client_type_snapshot,
                    client_name_snapshot, native_item_id, ended_at
             FROM download_client_bindings ORDER BY download_id",
            &[],
        )
        .await
        .expect("unbound bindings should persist with the rows");
        assert_eq!(bindings.len(), 2);
        for binding in &bindings {
            assert_eq!(
                binding.opt_text("client_config_id").expect("config"),
                Some("primary".to_string())
            );
            assert_eq!(
                binding.opt_text("client_type_snapshot").expect("type"),
                Some("sabnzbd".to_string())
            );
            assert_eq!(
                binding.opt_text("client_name_snapshot").expect("name"),
                Some("Primary SAB".to_string())
            );
            assert_eq!(binding.opt_text("native_item_id").expect("native id"), None);
            assert_eq!(binding.opt_text("ended_at").expect("ended at"), None);
        }
        let downloads = SqlRuntime::fetch_all(
            store.datastore.read_exec(),
            "SELECT id, origin FROM downloads ORDER BY id",
            &[],
        )
        .await
        .expect("canonical downloads should persist with the rows");
        assert_eq!(downloads.len(), 2);
        assert!(
            downloads
                .iter()
                .all(|row| { row.text("origin").expect("origin") == "scryer_submission" })
        );

        assert!(
            store
                .list_for_title("title-ambiguous")
                .await
                .expect("legacy title reader should succeed")
                .is_empty()
        );
        assert!(
            store
                .find_by_title_and_request_signature(
                    "title-ambiguous",
                    "ambiguous-signature",
                    DownloadSubmissionPurpose::Standard,
                    &SubmissionScope::Title,
                )
                .await
                .expect("legacy dedupe reader should succeed")
                .is_none()
        );
    }

    #[tokio::test]
    async fn accepted_sab_style_submission_keeps_its_preallocated_row_id() {
        let store = store().await;
        let download_id = scryer_domain::download_identity::DownloadId::parse(
            "00000000-0000-4000-8000-000000000003",
        )
        .expect("fixed UUID should parse");
        let mut submission = ambiguous_submission(download_id);
        submission.download_client_item_id = "SABnzbd_nzo_1".to_string();
        store
            .record_submission_with_identity(
                submission,
                DownloadSubmissionIdentity {
                    download_id: Some("SABnzbd_nzo_1".to_string()),
                },
            )
            .await
            .expect("accepted SAB-style submission should persist");
        let stored = store
            .find_by_client_item_id(&DownloadSourceIdentity::new(
                Some("primary"),
                "sabnzbd",
                "SABnzbd_nzo_1",
            ))
            .await
            .expect("stored submission should load")
            .expect("accepted submission should be present");
        assert_eq!(stored.download_id, download_id);
    }
}
