use super::*;

use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, ClientJobLocator, DownloadSubmission, DownloadSubmissionActorSnapshot,
    DownloadSubmissionIdentity, DownloadSubmissionRepository, IdentityTrackedStateTarget, PersistedSeedGoals,
    SeedGoalGrabRecord, SeedGoalResolutionSource,
};
use scryer_domain::{Id, download_identity::DownloadId};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct DownloadSubmissionStore {
    datastore: StoreDatastore,
}

impl DownloadSubmissionStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn find_by_canonical_download_id(
        &self,
        canonical_download_id: &DownloadId,
    ) -> AppResult<Option<DownloadSubmission>> {
        let sql = download_submission_select_sql(&self.datastore, "WHERE id = {} LIMIT 1");
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| download_submission_from_row(&row))
            .transpose()
    }

    async fn active_binding_download_id(
        &self,
        locator: &ClientJobLocator,
    ) -> AppResult<Option<DownloadId>> {
        active_binding_download_id(self.datastore.read_exec(), locator).await
    }

    async fn get_seed_goals_by_canonical_download_id(
        &self,
        canonical_download_id: &DownloadId,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &format!(
                "SELECT {SEED_GOAL_COLUMNS}
                 FROM download_submissions
                 WHERE id = {{}}
                 LIMIT 1"
            ),
            &[SqlArg::Text(canonical_download_id.to_string())],
        )
        .await?;
        row.map(|row| seed_goals_from_row(&row))
            .transpose()
            .map(Option::flatten)
    }
}

async fn active_binding_download_id(
    exec: SqlExec<'_, '_>,
    locator: &ClientJobLocator,
) -> AppResult<Option<DownloadId>> {
    let row = SqlRuntime::fetch_optional(
        exec,
        "SELECT download_id
         FROM download_client_bindings
         WHERE ended_at IS NULL
           AND native_item_id IS NOT NULL
           AND COALESCE(client_config_id, '') = {}
           AND LOWER(COALESCE(client_type_snapshot, '')) = {}
           AND native_item_id = {}
         ORDER BY created_at, download_id
         LIMIT 1",
        &[
            SqlArg::Text(locator.client_id.clone().unwrap_or_default()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.item_id.clone()),
        ],
    )
    .await?;
    row.map(|row| {
        let value = row.text("download_id")?;
        DownloadId::parse(&value).ok_or_else(|| {
            AppError::Repository(format!("active client binding has invalid download id {value:?}"))
        })
    })
    .transpose()
}

async fn active_or_create_binding_download_id_tx(
    tx: &mut SqlTx<'_>,
    locator: &ClientJobLocator,
) -> AppResult<DownloadId> {
    if let Some(download_id) = active_binding_download_id(SqlExec::Tx(tx), locator).await? {
        return Ok(download_id);
    }

    let download_id = DownloadId::new();
    let now = Utc::now();
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO downloads (id, origin, created_at, first_observed_at, last_observed_at)
         VALUES ({}, 'foreign_observation', {}, {}, {})",
        &[
            SqlArg::Text(download_id.to_string()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at, last_seen_at, ended_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, NULL)",
        &[
            SqlArg::Text(download_id.to_string()),
            SqlArg::OptText(locator.client_id.clone()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.item_id.clone()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(download_id)
}

fn canonical_tracked_state_key(canonical_download_id: &DownloadId) -> String {
    format!("download:{canonical_download_id}")
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
        identity: &ClientJobLocator,
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
                    let Some(canonical_download_id) =
                        active_binding_download_id(SqlExec::Tx(tx), &identity).await?
                    else {
                        return Ok(());
                    };
                    record_download_submission_identity_tx(
                        tx,
                        &canonical_download_id,
                        &submission_identity,
                    )
                    .await
                })
            },
        )
        .await
    }

    async fn record_submission_actor_snapshot(
        &self,
        identity: &ClientJobLocator,
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
        identity: &ClientJobLocator,
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
        identity: &ClientJobLocator,
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

    async fn find_by_client_item_id_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<DownloadSubmission>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match self.active_binding_download_id(identity).await? {
                Some(canonical_download_id) => canonical_download_id,
                None => return Ok(None),
            },
        };
        self.find_by_canonical_download_id(&canonical_download_id).await
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

    async fn list_by_download_id_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _client_id: Option<&str>,
        _client_type: &str,
        _download_id: &str,
    ) -> AppResult<Vec<DownloadSubmission>> {
        let Some(canonical_download_id) = canonical_download_id else {
            return Ok(Vec::new());
        };
        Ok(self
            .find_by_canonical_download_id(canonical_download_id)
            .await?
            .into_iter()
            .collect())
    }

    async fn get_submission_identity(
        &self,
        identity: &ClientJobLocator,
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
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
        _tracked_state: &str,
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn record_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
        tracked_state: &str,
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<()> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(());
                    };
                    canonical_download_id
                }
                None => return Ok(()),
            },
        };
        let identity_key = canonical_tracked_state_key(&canonical_download_id);
        let canonical_download_id = canonical_download_id.to_string();
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
                            SqlArg::OptText(Some(canonical_download_id)),
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
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
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
        row.map(|row| row.text("tracked_state")).transpose()
    }

    async fn get_identity_tracked_state_reason(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_reason_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
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
        row.map(|row| row.opt_text("reason"))
            .transpose()
            .map(Option::flatten)
    }

    async fn get_identity_tracked_state_detail(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn get_identity_tracked_state_detail_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        _identity: &DownloadSubmissionIdentity,
        source_identity: Option<&ClientJobLocator>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
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
        row.map(|row| row.opt_text("detail"))
            .transpose()
            .map(Option::flatten)
    }

    async fn upsert_identity_tracked_state_returning_previous(
        &self,
        _identity: &DownloadSubmissionIdentity,
        _source_identity: Option<&ClientJobLocator>,
        _tracked_state: &str,
        _preserve_previous: &[&str],
        _reason: Option<&str>,
        _detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn upsert_identity_tracked_state_for_download_returning_previous(
        &self,
        target: IdentityTrackedStateTarget<'_>,
        tracked_state: &str,
        preserve_previous: &[&str],
        reason: Option<&str>,
        detail: Option<&str>,
    ) -> AppResult<Option<String>> {
        let canonical_download_id = match target.canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match target.source_identity {
                Some(source_identity) => {
                    let Some(canonical_download_id) =
                        self.active_binding_download_id(source_identity).await?
                    else {
                        return Ok(None);
                    };
                    canonical_download_id
                }
                None => return Ok(None),
            },
        };
        let identity_key = canonical_tracked_state_key(&canonical_download_id);
        let canonical_download_id = canonical_download_id.to_string();
        let identity = target.identity.clone();
        let source_identity = target.source_identity.cloned();
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
                    let previous = SqlRuntime::fetch_optional(
                        SqlExec::Tx(tx),
                        "SELECT tracked_state
                         FROM download_identity_states
                         WHERE canonical_download_id = {}
                         ORDER BY updated_at DESC, id DESC
                         LIMIT 1",
                        &[SqlArg::Text(canonical_download_id.clone())],
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
                            SqlArg::OptText(Some(canonical_download_id)),
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
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, String)>> {
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
                    ClientJobLocator::new(
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
        client_items: &[ClientJobLocator],
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

    async fn delete_by_client_item_id(&self, identity: &ClientJobLocator) -> AppResult<()> {
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
        identity: &ClientJobLocator,
        tracked_state: &str,
    ) -> AppResult<()> {
        let identity = identity.clone();
        let tracked_state = tracked_state.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_tracked_state", move |tx| {
            let identity = identity.clone();
            let tracked_state = tracked_state.clone();
            Box::pin(async move {
                let canonical_download_id =
                    active_or_create_binding_download_id_tx(tx, &identity).await?;
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "INSERT INTO download_submissions
                     (id, title_id, facet, download_client_id, download_client_type, download_client_item_id, source_hint, source_kind, source_title, request_signature, purpose, episode_id, collection_id, tracked_state, tracked_state_at)
                     VALUES ({}, '', '', {}, {}, {}, NULL, NULL, NULL, NULL, 'standard', NULL, NULL, {}, {})
                     ON CONFLICT(id) DO UPDATE
                     SET tracked_state = excluded.tracked_state,
                         tracked_state_at = excluded.tracked_state_at",
                    &[
                        SqlArg::Text(canonical_download_id.to_string()),
                        SqlArg::Text(normalize_download_client_id(identity.client_id.as_deref())),
                        SqlArg::Text(identity.client_type.clone()),
                        SqlArg::Text(identity.item_id.clone()),
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
        identity: &ClientJobLocator,
    ) -> AppResult<Option<String>> {
        let Some(canonical_download_id) = self.active_binding_download_id(identity).await? else {
            return Ok(None);
        };
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT tracked_state FROM download_submissions
             WHERE id = {}
             LIMIT 1",
            &[SqlArg::Text(canonical_download_id.to_string())],
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
                         ON CONFLICT(id) DO UPDATE
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
                    Ok(())
                })
            },
        )
        .await
    }

    async fn get_seed_goals(
        &self,
        identity: &ClientJobLocator,
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

    async fn get_seed_goals_for_download(
        &self,
        canonical_download_id: Option<&DownloadId>,
        identity: &ClientJobLocator,
    ) -> AppResult<Option<PersistedSeedGoals>> {
        let canonical_download_id = match canonical_download_id {
            Some(canonical_download_id) => *canonical_download_id,
            None => match self.active_binding_download_id(identity).await? {
                Some(canonical_download_id) => canonical_download_id,
                None => return Ok(None),
            },
        };
        self.get_seed_goals_by_canonical_download_id(&canonical_download_id)
            .await
    }

    async fn list_seed_goals_for_client_items(
        &self,
        client_items: &[ClientJobLocator],
    ) -> AppResult<Vec<(ClientJobLocator, PersistedSeedGoals)>> {
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
                    ClientJobLocator::new(
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
             );
             CREATE TABLE download_identity_states (
                 id TEXT PRIMARY KEY,
                 identity_key TEXT NOT NULL UNIQUE,
                 canonical_download_id TEXT,
                 download_id TEXT,
                 client_id TEXT,
                 client_type TEXT,
                 download_client_item_id TEXT,
                 tracked_state TEXT NOT NULL,
                 reason TEXT,
                 detail TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
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

    fn identity() -> ClientJobLocator {
        ClientJobLocator::new(Some("primary"), "qbittorrent", "job-1")
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
                ClientJobLocator::new(Some("primary"), "qbittorrent", "job-2"),
                // A row with no resolution at all must simply be absent, not a
                // default-valued entry that would read as "no goals resolved".
                ClientJobLocator::new(Some("primary"), "qbittorrent", "job-missing"),
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
    async fn seed_goals_read_by_canonical_download_id() {
        let store = store().await;
        let record = record();
        let canonical_download_id = record.download_id;
        let mut expected = record.goals.clone();
        expected.info_hash = expected.info_hash.map(|hash| hash.to_ascii_lowercase());
        store
            .record_seed_goals(record)
            .await
            .expect("seed goals should persist");

        let loaded = store
            .get_seed_goals_for_download(Some(&canonical_download_id), &identity())
            .await
            .expect("canonical read should succeed");

        assert_eq!(loaded, Some(expected));
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
            .find_by_client_item_id(&ClientJobLocator::new(
                Some("primary"),
                "sabnzbd",
                "SABnzbd_nzo_1",
            ))
            .await
            .expect("stored submission should load")
            .expect("accepted submission should be present");
        assert_eq!(stored.download_id, download_id);
    }

    #[tokio::test]
    async fn download_id_submission_lookup_reads_the_canonical_row() {
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_submissions (
                id, title_id, facet, download_client_id, download_client_type,
                download_client_item_id, download_id
             ) VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(canonical_download_id.to_string()),
                SqlArg::Text("title-canonical".to_string()),
                SqlArg::Text("series".to_string()),
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("nzbget".to_string()),
                SqlArg::Text("canonical-bound-job".to_string()),
                SqlArg::Text("canonical-overloaded-id".to_string()),
            ],
        )
        .await
        .expect("canonical submission fixture should insert");

        let resolved = store
            .list_by_download_id_for_download(
                Some(&canonical_download_id),
                Some("primary"),
                "nzbget",
                "legacy-overloaded-id",
            )
            .await
            .expect("canonical-first lookup should succeed");

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].download_id, canonical_download_id);
        assert_eq!(resolved[0].title_id, "title-canonical");
    }

    #[tokio::test]
    async fn tracked_state_stub_creation_mints_registry_row_and_active_binding() {
        let store = store().await;
        let locator = ClientJobLocator::new(Some("primary"), "nzbget", "unseen-job");

        store
            .update_tracked_state(&locator, "ignored")
            .await
            .expect("tracked-state stub should persist");

        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT d.id, d.origin, b.download_id AS binding_download_id, s.tracked_state
             FROM downloads d
             JOIN download_client_bindings b ON b.download_id = d.id
             JOIN download_submissions s ON s.id = d.id
             WHERE b.ended_at IS NULL
               AND COALESCE(b.client_config_id, '') = {}
               AND b.client_type_snapshot = {}
               AND b.native_item_id = {}",
            &[
                SqlArg::Text("primary".to_string()),
                SqlArg::Text("nzbget".to_string()),
                SqlArg::Text("unseen-job".to_string()),
            ],
        )
        .await
        .expect("registry row should load")
        .expect("registry row should exist");

        assert_eq!(row.text("origin").expect("origin"), "foreign_observation");
        assert_eq!(
            row.text("id").expect("download id"),
            row.text("binding_download_id").expect("binding download id")
        );
        assert_eq!(row.text("tracked_state").expect("tracked state"), "ignored");
    }



    #[tokio::test]
    async fn failed_identity_state_uses_the_canonical_key_and_preserves_compatibility_columns() {
        let store = store().await;
        let canonical_download_id = DownloadId::new();
        let identity = DownloadSubmissionIdentity {
            download_id: Some("legacy-failure-id".to_string()),
        };
        let source_identity =
            ClientJobLocator::new(Some("primary"), "nzbget", "legacy-failed-job");

        store
            .record_identity_tracked_state_for_download(
                Some(&canonical_download_id),
                &identity,
                Some(&source_identity),
                "failed",
                Some("import_gate_rejected"),
                Some("failure detail"),
            )
            .await
            .expect("failed state should persist");

        let row = SqlRuntime::fetch_optional(
            store.datastore.read_exec(),
            "SELECT identity_key, canonical_download_id, download_id
             FROM download_identity_states
             LIMIT 1",
            &[],
        )
        .await
        .expect("failed state row should load")
        .expect("failed state row should exist");

        assert_eq!(
            row.text("identity_key").expect("canonical identity key"),
            format!("download:{canonical_download_id}")
        );
        assert_eq!(
            row.opt_text("canonical_download_id")
                .expect("canonical column"),
            Some(canonical_download_id.to_string())
        );
        assert_eq!(
            row.opt_text("download_id").expect("legacy download id"),
            Some("legacy-failure-id".to_string())
        );
    }
}
