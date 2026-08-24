use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use scryer_application::{
    AppError, AppResult, ClientJobLocator, DownloadClientBindingRecord, DownloadOrigin,
    DownloadRecord, DownloadRegistryRepository, ObservationResolution, ObservedClientJob,
};
use scryer_domain::download_identity::DownloadId;

use super::{opt_timestamp_string, timestamp_string};
use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

#[derive(Clone)]
pub struct DownloadRegistryStore {
    datastore: StoreDatastore,
}

struct ObservationState {
    first_observed_at: Option<DateTime<Utc>>,
    last_observed_at: Option<DateTime<Utc>>,
}

struct ActiveObservationBinding {
    binding: DownloadClientBindingRecord,
    state: ObservationState,
}

impl DownloadRegistryStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl DownloadRegistryRepository for DownloadRegistryStore {
    async fn resolve_observation(
        &self,
        observation: &ObservedClientJob,
    ) -> AppResult<ObservationResolution> {
        let observation = observation.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "resolve_download_observation", move |tx| {
            let observation = observation.clone();
            Box::pin(async move { resolve_observation_tx(tx, &observation).await })
        })
        .await
    }

    async fn load_download(&self, id: &DownloadId) -> AppResult<Option<DownloadRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT id, origin, created_at, first_observed_at, last_observed_at, terminal_at
             FROM downloads
             WHERE id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .map(download_from_row)
        .transpose()
    }

    async fn load_binding(
        &self,
        id: &DownloadId,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                    native_item_id, created_at, last_seen_at, ended_at
             FROM download_client_bindings
             WHERE download_id = {}",
            &[SqlArg::Text(id.to_string())],
        )
        .await?
        .map(binding_from_row)
        .transpose()
    }

    async fn find_active_binding_by_locator(
        &self,
        locator: &ClientJobLocator,
    ) -> AppResult<Option<DownloadClientBindingRecord>> {
        SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                    native_item_id, created_at, last_seen_at, ended_at
             FROM download_client_bindings
             WHERE ended_at IS NULL
               AND native_item_id IS NOT NULL
               AND COALESCE(client_config_id, '') = {}
               AND LOWER(COALESCE(client_type_snapshot, '')) = {}
               AND native_item_id = {}
             ORDER BY created_at, download_id
             LIMIT 1",
            &[
                SqlArg::Text(locator.client_config_id.clone().unwrap_or_default()),
                SqlArg::Text(locator.client_type.clone()),
                SqlArg::Text(locator.native_item_id.clone()),
            ],
        )
        .await?
        .map(binding_from_row)
        .transpose()
    }

    async fn end_binding(&self, id: &DownloadId) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "end_download_client_binding", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE download_client_bindings
                     SET ended_at = {}
                     WHERE download_id = {}
                       AND ended_at IS NULL",
                    &[SqlArg::Timestamp(Utc::now()), SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }
}

async fn resolve_observation_tx(
    tx: &mut SqlTx<'_>,
    observation: &ObservedClientJob,
) -> AppResult<ObservationResolution> {
    let token_id = observation
        .wire_token
        .as_deref()
        .and_then(DownloadId::from_wire);
    let active_binding = active_observation_binding_by_locator_tx(tx, &observation.locator).await?;

    if let Some(active_binding) = active_binding {
        let ActiveObservationBinding { binding, state } = active_binding;
        if let Some(token_id) = token_id
            && token_id != binding.download_id
        {
            return Err(locator_conflict(
                token_id,
                binding.download_id,
                &observation.locator,
            ));
        }
        touch_observation_if_stale_tx(tx, binding.download_id, &state, &binding, observation)
            .await?;
        return Ok(ObservationResolution::Resolved {
            download_id: binding.download_id,
            newly_foreign: false,
            attached: false,
        });
    }

    if let Some(token_id) = token_id {
        if let Some(state) = observation_state_for_download_tx(tx, token_id).await? {
            let mut attached = false;
            let binding = match binding_for_download_tx(tx, token_id).await? {
                Some(binding) if binding.ended_at.is_none() && binding.native_item_id.is_none() => {
                    attach_unbound_binding_tx(tx, token_id, observation).await?;
                    attached = true;
                    Some(binding)
                }
                Some(binding) if binding_matches_locator(&binding, &observation.locator) => {
                    Some(binding)
                }
                Some(binding) if binding.ended_at.is_none() && binding.native_item_id.is_some() => {
                    return Err(locator_conflict(
                        token_id,
                        binding.download_id,
                        &observation.locator,
                    ));
                }
                Some(binding) => {
                    return Err(AppError::Repository(format!(
                        "cannot attach observed locator {} to ended canonical binding {} for token {}",
                        locator_display(&observation.locator),
                        binding.download_id,
                        token_id
                    )));
                }
                None => {
                    create_bound_binding_tx(tx, token_id, observation).await?;
                    attached = true;
                    None
                }
            };
            if attached {
                touch_observation_tx(tx, token_id, observation).await?;
            } else if let Some(binding) = binding {
                touch_observation_if_stale_tx(tx, token_id, &state, &binding, observation).await?;
            }
            return Ok(ObservationResolution::Resolved {
                download_id: token_id,
                newly_foreign: false,
                attached: true,
            });
        }

        // The writer gate serializes SQLite writers, but re-check immediately
        // before the insert for datastore implementations with concurrent writers.
        if let Some(active_binding) =
            active_observation_binding_by_locator_tx(tx, &observation.locator).await?
        {
            let binding = active_binding.binding;
            return Err(locator_conflict(
                token_id,
                binding.download_id,
                &observation.locator,
            ));
        }
        create_foreign_observation_tx(tx, token_id, observation).await?;
        return Ok(ObservationResolution::Resolved {
            download_id: token_id,
            newly_foreign: true,
            attached: false,
        });
    }

    let candidates = ambiguous_submission_candidates_tx(tx, observation).await?;
    if let [download_id] = candidates.as_slice() {
        attach_unbound_binding_tx(tx, *download_id, observation).await?;
        touch_observation_tx(tx, *download_id, observation).await?;
        return Ok(ObservationResolution::Resolved {
            download_id: *download_id,
            newly_foreign: false,
            attached: true,
        });
    }

    // The later locator-uniqueness migration has not landed yet. Re-check in
    // this transaction before inserting so concurrent first sightings converge.
    if let Some(active_binding) =
        active_observation_binding_by_locator_tx(tx, &observation.locator).await?
    {
        let ActiveObservationBinding { binding, state } = active_binding;
        touch_observation_if_stale_tx(tx, binding.download_id, &state, &binding, observation)
            .await?;
        return Ok(ObservationResolution::Resolved {
            download_id: binding.download_id,
            newly_foreign: false,
            attached: false,
        });
    }

    let download_id = DownloadId::new();
    create_foreign_observation_tx(tx, download_id, observation).await?;
    Ok(ObservationResolution::Resolved {
        download_id,
        newly_foreign: true,
        attached: false,
    })
}

async fn active_observation_binding_by_locator_tx(
    tx: &mut SqlTx<'_>,
    locator: &ClientJobLocator,
) -> AppResult<Option<ActiveObservationBinding>> {
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT b.download_id, b.client_config_id, b.client_type_snapshot, b.client_name_snapshot,
                b.native_item_id, b.created_at, b.last_seen_at, b.ended_at,
                d.first_observed_at, d.last_observed_at
         FROM download_client_bindings b
         JOIN downloads d ON d.id = b.download_id
         WHERE b.ended_at IS NULL
           AND b.native_item_id IS NOT NULL
           AND COALESCE(b.client_config_id, '') = {}
           AND LOWER(COALESCE(b.client_type_snapshot, '')) = {}
           AND b.native_item_id = {}
         ORDER BY b.created_at, b.download_id
         LIMIT 1",
        &[
            SqlArg::Text(locator.client_config_id.clone().unwrap_or_default()),
            SqlArg::Text(locator.client_type.clone()),
            SqlArg::Text(locator.native_item_id.clone()),
        ],
    )
    .await?
    .map(|row| {
        Ok(ActiveObservationBinding {
            state: ObservationState {
                first_observed_at: optional_timestamp_from_row(&row, "first_observed_at")?,
                last_observed_at: optional_timestamp_from_row(&row, "last_observed_at")?,
            },
            binding: binding_from_row(row)?,
        })
    })
    .transpose()
}

async fn binding_for_download_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
) -> AppResult<Option<DownloadClientBindingRecord>> {
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at, last_seen_at, ended_at
         FROM download_client_bindings
         WHERE download_id = {}",
        &[SqlArg::Text(download_id.to_string())],
    )
    .await?
    .map(binding_from_row)
    .transpose()
}

async fn observation_state_for_download_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
) -> AppResult<Option<ObservationState>> {
    SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT first_observed_at, last_observed_at
         FROM downloads
         WHERE id = {}",
        &[SqlArg::Text(download_id.to_string())],
    )
    .await?
    .map(|row| {
        Ok(ObservationState {
            first_observed_at: optional_timestamp_from_row(&row, "first_observed_at")?,
            last_observed_at: optional_timestamp_from_row(&row, "last_observed_at")?,
        })
    })
    .transpose()
}

async fn ambiguous_submission_candidates_tx(
    tx: &mut SqlTx<'_>,
    observation: &ObservedClientJob,
) -> AppResult<Vec<DownloadId>> {
    let Some(observed_name) = normalized_observed_name(observation.observed_name.as_deref()) else {
        return Ok(Vec::new());
    };
    let rows = SqlRuntime::fetch_all(
        SqlExec::Tx(tx),
        "SELECT b.download_id
         FROM download_client_bindings b
         JOIN download_submissions s ON s.id = b.download_id
         WHERE b.ended_at IS NULL
           AND b.native_item_id IS NULL
           AND COALESCE(b.client_config_id, '') = {}
           AND LOWER(COALESCE(b.client_type_snapshot, '')) = {}
           AND s.download_client_item_id IS NULL
           AND LOWER(TRIM(COALESCE(s.source_title, ''))) = {}
         ORDER BY b.created_at, b.download_id
         LIMIT 2",
        &[
            SqlArg::Text(
                observation
                    .locator
                    .client_config_id
                    .clone()
                    .unwrap_or_default(),
            ),
            SqlArg::Text(observation.locator.client_type.clone()),
            SqlArg::Text(observed_name),
        ],
    )
    .await?;
    rows.into_iter()
        .map(|row| download_id_from_column(&row, "download_id"))
        .collect()
}

async fn attach_unbound_binding_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
    observation: &ObservedClientJob,
) -> AppResult<()> {
    let rows_affected = SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE download_client_bindings
         SET client_config_id = COALESCE(client_config_id, {}),
             client_type_snapshot = COALESCE(client_type_snapshot, {}),
             client_name_snapshot = COALESCE(client_name_snapshot, {}),
             native_item_id = {}
         WHERE download_id = {}
           AND native_item_id IS NULL
           AND ended_at IS NULL",
        &[
            SqlArg::OptText(observation.locator.client_config_id.clone()),
            SqlArg::Text(observation.locator.client_type.clone()),
            SqlArg::Text(observation.locator.client_type.clone()),
            SqlArg::Text(observation.locator.native_item_id.clone()),
            SqlArg::Text(download_id.to_string()),
        ],
    )
    .await?;
    if rows_affected == 1 {
        Ok(())
    } else {
        Err(AppError::Repository(format!(
            "canonical download binding {download_id} was no longer an active unbound binding"
        )))
    }
}

async fn create_foreign_observation_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
    observation: &ObservedClientJob,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO downloads (id, origin, created_at, first_observed_at, last_observed_at)
         VALUES ({}, 'foreign_observation', {}, {}, {})",
        &[
            SqlArg::Text(download_id.to_string()),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
        ],
    )
    .await?;
    create_bound_binding_tx(tx, download_id, observation).await
}

async fn create_bound_binding_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
    observation: &ObservedClientJob,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "INSERT INTO download_client_bindings (
            download_id, client_config_id, client_type_snapshot, client_name_snapshot,
            native_item_id, created_at, last_seen_at, ended_at
         ) VALUES ({}, {}, {}, {}, {}, {}, {}, NULL)",
        &[
            SqlArg::Text(download_id.to_string()),
            SqlArg::OptText(observation.locator.client_config_id.clone()),
            SqlArg::Text(observation.locator.client_type.clone()),
            SqlArg::Text(observation.locator.client_type.clone()),
            SqlArg::Text(observation.locator.native_item_id.clone()),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
        ],
    )
    .await?;
    Ok(())
}

async fn touch_observation_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
    observation: &ObservedClientJob,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE downloads
         SET first_observed_at = COALESCE(first_observed_at, {}),
             last_observed_at = CASE
                 WHEN last_observed_at IS NULL OR last_observed_at < {} THEN {}
                 ELSE last_observed_at
             END
         WHERE id = {}",
        &[
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Text(download_id.to_string()),
        ],
    )
    .await?;
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "UPDATE download_client_bindings
         SET last_seen_at = CASE
                 WHEN last_seen_at IS NULL OR last_seen_at < {} THEN {}
                 ELSE last_seen_at
             END
         WHERE download_id = {}",
        &[
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Timestamp(observation.observed_at),
            SqlArg::Text(download_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn touch_observation_if_stale_tx(
    tx: &mut SqlTx<'_>,
    download_id: DownloadId,
    state: &ObservationState,
    binding: &DownloadClientBindingRecord,
    observation: &ObservedClientJob,
) -> AppResult<()> {
    if observation_timestamp_write_required(state, binding, observation.observed_at) {
        touch_observation_tx(tx, download_id, observation).await?;
    }
    Ok(())
}

fn observation_timestamp_write_required(
    state: &ObservationState,
    binding: &DownloadClientBindingRecord,
    observed_at: DateTime<Utc>,
) -> bool {
    state.first_observed_at.is_none()
        || state.last_observed_at.is_none()
        || binding.last_seen_at.is_none_or(|last_seen_at| {
            observed_at.signed_duration_since(last_seen_at) > Duration::seconds(60)
        })
}

fn normalized_observed_name(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn binding_matches_locator(
    binding: &DownloadClientBindingRecord,
    locator: &ClientJobLocator,
) -> bool {
    binding.ended_at.is_none()
        && binding.client_config_id.as_deref().map(str::trim) == locator.client_config_id.as_deref()
        && binding
            .client_type_snapshot
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref()
            == Some(locator.client_type.as_str())
        && binding.native_item_id.as_deref().map(str::trim) == Some(locator.native_item_id.as_str())
}

fn locator_conflict(
    token_id: DownloadId,
    binding_id: DownloadId,
    locator: &ClientJobLocator,
) -> AppError {
    AppError::Repository(format!(
        "canonical download observation conflict: token id {token_id}, binding id {binding_id}, locator {}",
        locator_display(locator)
    ))
}

fn locator_display(locator: &ClientJobLocator) -> String {
    format!(
        "client_config_id={:?}, client_type={:?}, native_item_id={:?}",
        locator.client_config_id, locator.client_type, locator.native_item_id
    )
}

fn download_from_row(row: SqlRow) -> AppResult<DownloadRecord> {
    let id = download_id_from_column(&row, "id")?;
    let origin = match row.text("origin")?.as_str() {
        "scryer_submission" => DownloadOrigin::ScryerSubmission,
        "foreign_observation" => DownloadOrigin::ForeignObservation,
        value => {
            return Err(AppError::Repository(format!(
                "invalid canonical download origin {value:?} for download {id}"
            )));
        }
    };
    Ok(DownloadRecord {
        id,
        origin,
        created_at: timestamp_from_row(&row, "created_at")?,
        first_observed_at: optional_timestamp_from_row(&row, "first_observed_at")?,
        last_observed_at: optional_timestamp_from_row(&row, "last_observed_at")?,
        terminal_at: optional_timestamp_from_row(&row, "terminal_at")?,
    })
}

fn binding_from_row(row: SqlRow) -> AppResult<DownloadClientBindingRecord> {
    Ok(DownloadClientBindingRecord {
        download_id: download_id_from_column(&row, "download_id")?,
        client_config_id: row.opt_text("client_config_id")?,
        client_type_snapshot: row.opt_text("client_type_snapshot")?,
        client_name_snapshot: row.opt_text("client_name_snapshot")?,
        native_item_id: row.opt_text("native_item_id")?,
        created_at: timestamp_from_row(&row, "created_at")?,
        last_seen_at: optional_timestamp_from_row(&row, "last_seen_at")?,
        ended_at: optional_timestamp_from_row(&row, "ended_at")?,
    })
}

fn download_id_from_column(row: &SqlRow, column: &str) -> AppResult<DownloadId> {
    let value = row.text(column)?;
    DownloadId::parse(&value).ok_or_else(|| {
        AppError::Repository(format!(
            "invalid canonical download id {value:?} in {column}"
        ))
    })
}

fn timestamp_from_row(row: &SqlRow, column: &str) -> AppResult<DateTime<Utc>> {
    parse_stored_timestamp(&timestamp_string(row, column)?, column)
}

fn optional_timestamp_from_row(row: &SqlRow, column: &str) -> AppResult<Option<DateTime<Utc>>> {
    opt_timestamp_string(row, column)?
        .map(|value| parse_stored_timestamp(&value, column))
        .transpose()
}

fn parse_stored_timestamp(value: &str, column: &str) -> AppResult<DateTime<Utc>> {
    // 0178's hook writes RFC3339 offsets, while legacy-copied SQLite timestamps
    // use strftime's `...Z` form; RFC3339 parsing deliberately accepts both.
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| {
            AppError::Repository(format!(
                "invalid canonical download timestamp {value:?} in {column}: {error}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    const FIRST_ID: &str = "00000000-0000-4000-8000-000000000001";
    const SECOND_ID: &str = "00000000-0000-4000-8000-000000000002";
    const CREATED_AT: &str = "2026-08-24T12:34:56Z";

    async fn store() -> DownloadRegistryStore {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should open");
        sqlx::query(
            "CREATE TABLE downloads (
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
             CREATE TABLE download_submissions (
                 id TEXT PRIMARY KEY,
                 download_client_id TEXT NOT NULL DEFAULT '',
                 download_client_type TEXT NOT NULL,
                 download_client_item_id TEXT,
                 source_title TEXT,
                 download_id TEXT
             )",
        )
        .execute(&pool)
        .await
        .expect("canonical tables should be created");
        DownloadRegistryStore::new(StoreDatastore::Sqlite {
            pool,
            writer_gate: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    async fn insert_download(
        store: &DownloadRegistryStore,
        id: &str,
        origin: &str,
        first_observed_at: Option<&str>,
    ) {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO downloads (id, origin, created_at, first_observed_at)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(origin.to_string()),
                SqlArg::Text(CREATED_AT.to_string()),
                SqlArg::OptText(first_observed_at.map(str::to_string)),
            ],
        )
        .await
        .expect("download should insert");
    }

    async fn insert_binding(
        store: &DownloadRegistryStore,
        id: &str,
        config_id: Option<&str>,
        native_item_id: Option<&str>,
        ended_at: Option<&str>,
    ) {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_client_bindings (
                download_id, client_config_id, client_type_snapshot, client_name_snapshot,
                native_item_id, created_at, ended_at
             ) VALUES ({}, {}, 'qbittorrent', 'Primary', {}, {}, {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::OptText(config_id.map(str::to_string)),
                SqlArg::OptText(native_item_id.map(str::to_string)),
                SqlArg::Text(CREATED_AT.to_string()),
                SqlArg::OptText(ended_at.map(str::to_string)),
            ],
        )
        .await
        .expect("binding should insert");
    }

    async fn insert_ambiguous_submission(
        store: &DownloadRegistryStore,
        id: &str,
        source_title: &str,
    ) {
        SqlRuntime::execute(
            store.datastore.read_exec(),
            "INSERT INTO download_submissions (
                id, download_client_id, download_client_type, download_client_item_id,
                source_title, download_id
             ) VALUES ({}, 'client-1', 'qbittorrent', NULL, {}, {})",
            &[
                SqlArg::Text(id.to_string()),
                SqlArg::Text(source_title.to_string()),
                SqlArg::Text(DownloadId::parse(id).unwrap().to_wire()),
            ],
        )
        .await
        .expect("ambiguous submission should insert");
    }

    fn observation(
        native_item_id: &str,
        wire_token: Option<&str>,
        observed_name: Option<&str>,
        observed_at: &str,
    ) -> ObservedClientJob {
        ObservedClientJob {
            locator: ClientJobLocator::new(Some("client-1"), "qBittorrent", native_item_id),
            wire_token: wire_token.map(str::to_string),
            observed_name: observed_name.map(str::to_string),
            observed_at: DateTime::parse_from_rfc3339(observed_at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn wire(id: &str) -> String {
        DownloadId::parse(id).unwrap().to_wire()
    }

    #[tokio::test]
    async fn loads_canonical_rows_and_reports_absence() {
        let store = store().await;
        let first_id = DownloadId::parse(FIRST_ID).unwrap();
        assert_eq!(store.load_download(&first_id).await.unwrap(), None);
        assert_eq!(store.load_binding(&first_id).await.unwrap(), None);

        insert_download(
            &store,
            FIRST_ID,
            "scryer_submission",
            Some("2026-08-24T06:34:56-06:00"),
        )
        .await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("job-1"), None).await;

        let download = store
            .load_download(&first_id)
            .await
            .unwrap()
            .expect("download should load");
        assert_eq!(download.origin, DownloadOrigin::ScryerSubmission);
        assert_eq!(download.first_observed_at.unwrap(), download.created_at);
        let binding = store
            .load_binding(&first_id)
            .await
            .unwrap()
            .expect("binding should load");
        assert_eq!(binding.client_name_snapshot.as_deref(), Some("Primary"));
        assert_eq!(binding.native_item_id.as_deref(), Some("job-1"));
    }

    #[tokio::test]
    async fn active_locator_lookup_excludes_ended_and_null_native_items() {
        let store = store().await;
        for id in [FIRST_ID, SECOND_ID, "00000000-0000-4000-8000-000000000003"] {
            insert_download(&store, id, "scryer_submission", None).await;
        }
        insert_binding(
            &store,
            "00000000-0000-4000-8000-000000000003",
            Some("client-1"),
            None,
            None,
        )
        .await;
        insert_binding(
            &store,
            SECOND_ID,
            Some("client-1"),
            Some("job-1"),
            Some(CREATED_AT),
        )
        .await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("job-1"), None).await;

        let found = store
            .find_active_binding_by_locator(&ClientJobLocator::new(
                Some("client-1"),
                "qbittorrent",
                "job-1",
            ))
            .await
            .unwrap()
            .expect("active binding should load");
        assert_eq!(found.download_id, DownloadId::parse(FIRST_ID).unwrap());
    }

    #[tokio::test]
    async fn ending_a_binding_is_idempotent() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "foreign_observation", None).await;
        insert_binding(&store, FIRST_ID, None, Some("job-1"), None).await;

        store.end_binding(&id).await.unwrap();
        let first_end = store.load_binding(&id).await.unwrap().unwrap().ended_at;
        assert!(first_end.is_some());
        store.end_binding(&id).await.unwrap();
        assert_eq!(
            store.load_binding(&id).await.unwrap().unwrap().ended_at,
            first_end
        );
    }

    #[tokio::test]
    async fn known_token_with_matching_locator_throttles_observation_timestamp_writes() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("job-1"), None).await;

        let first = observation(
            "job-1",
            Some(&wire(FIRST_ID)),
            Some("release"),
            "2026-08-24T13:00:00Z",
        );
        assert_eq!(
            store.resolve_observation(&first).await.unwrap(),
            ObservationResolution::Resolved {
                download_id: id,
                newly_foreign: false,
                attached: false,
            }
        );
        let first_download = store.load_download(&id).await.unwrap().unwrap();
        let first_binding = store.load_binding(&id).await.unwrap().unwrap();
        assert_eq!(
            first_download.first_observed_at,
            Some(
                DateTime::parse_from_rfc3339("2026-08-24T13:00:00Z")
                    .unwrap()
                    .into()
            )
        );
        assert_eq!(
            first_download.last_observed_at,
            first_download.first_observed_at
        );
        assert_eq!(first_binding.last_seen_at, first_download.first_observed_at);

        let immediate = observation(
            "job-1",
            Some(&wire(FIRST_ID)),
            Some("release"),
            "2026-08-24T13:00:01Z",
        );
        store.resolve_observation(&immediate).await.unwrap();
        let immediate_download = store.load_download(&id).await.unwrap().unwrap();
        let immediate_binding = store.load_binding(&id).await.unwrap().unwrap();
        assert_eq!(
            immediate_download.last_observed_at,
            first_download.last_observed_at
        );
        assert_eq!(immediate_binding.last_seen_at, first_binding.last_seen_at);

        let later = observation(
            "job-1",
            Some(&wire(FIRST_ID)),
            Some("release"),
            "2026-08-24T13:01:01Z",
        );
        store.resolve_observation(&later).await.unwrap();
        let later_download = store.load_download(&id).await.unwrap().unwrap();
        let later_binding = store.load_binding(&id).await.unwrap().unwrap();
        assert_eq!(
            later_download.first_observed_at,
            first_download.first_observed_at
        );
        assert_eq!(
            later_download.last_observed_at,
            Some(
                DateTime::parse_from_rfc3339("2026-08-24T13:01:01Z")
                    .unwrap()
                    .into()
            )
        );
        assert_eq!(later_binding.last_seen_at, later_download.last_observed_at);
    }

    #[tokio::test]
    async fn known_token_attaches_its_single_unbound_binding() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), None, None).await;

        let resolution = store
            .resolve_observation(&observation(
                "job-1",
                Some(&wire(FIRST_ID)),
                Some("different name"),
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap();

        assert_eq!(
            resolution,
            ObservationResolution::Resolved {
                download_id: id,
                newly_foreign: false,
                attached: true,
            }
        );
        let binding = store.load_binding(&id).await.unwrap().unwrap();
        assert_eq!(binding.native_item_id.as_deref(), Some("job-1"));
        assert_eq!(binding.client_type_snapshot.as_deref(), Some("qbittorrent"));
    }

    #[tokio::test]
    async fn known_token_with_different_active_locator_is_rejected_without_writes() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("other-job"), None).await;
        let before_download = store.load_download(&id).await.unwrap();
        let before_binding = store.load_binding(&id).await.unwrap();

        let error = store
            .resolve_observation(&observation(
                "job-1",
                Some(&wire(FIRST_ID)),
                None,
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap_err();

        assert!(error.to_string().contains(FIRST_ID));
        assert_eq!(store.load_download(&id).await.unwrap(), before_download);
        assert_eq!(store.load_binding(&id).await.unwrap(), before_binding);
    }

    #[tokio::test]
    async fn unknown_valid_token_is_adopted_as_foreign_with_its_exact_id() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();

        assert_eq!(
            store
                .resolve_observation(&observation(
                    "job-1",
                    Some(&wire(FIRST_ID)),
                    Some("release"),
                    "2026-08-24T13:00:00Z",
                ))
                .await
                .unwrap(),
            ObservationResolution::Resolved {
                download_id: id,
                newly_foreign: true,
                attached: false,
            }
        );
        assert_eq!(
            store.load_download(&id).await.unwrap().unwrap().origin,
            DownloadOrigin::ForeignObservation
        );
        assert_eq!(
            store
                .load_binding(&id)
                .await
                .unwrap()
                .unwrap()
                .native_item_id
                .as_deref(),
            Some("job-1")
        );
    }

    #[tokio::test]
    async fn malformed_wire_token_is_treated_as_absent() {
        let store = store().await;

        let ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached,
        } = store
            .resolve_observation(&observation(
                "job-1",
                Some("SABnzbd_nzo_not_a_canonical_token"),
                Some("release"),
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap();

        assert!(newly_foreign);
        assert!(!attached);
        assert!(store.load_download(&download_id).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn no_token_and_no_ambiguous_candidate_creates_a_foreign_download() {
        let store = store().await;

        let ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached,
        } = store
            .resolve_observation(&observation(
                "job-1",
                None,
                Some("release"),
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap();

        assert!(newly_foreign);
        assert!(!attached);
        assert_eq!(
            store
                .load_binding(&download_id)
                .await
                .unwrap()
                .unwrap()
                .native_item_id
                .as_deref(),
            Some("job-1")
        );
    }

    #[tokio::test]
    async fn no_token_uses_the_existing_active_locator() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("job-1"), None).await;

        assert_eq!(
            store
                .resolve_observation(&observation(
                    "job-1",
                    None,
                    Some("release"),
                    "2026-08-24T13:00:00Z",
                ))
                .await
                .unwrap(),
            ObservationResolution::Resolved {
                download_id: id,
                newly_foreign: false,
                attached: false,
            }
        );
    }

    #[tokio::test]
    async fn no_token_attaches_exactly_one_matching_ambiguous_submission() {
        let store = store().await;
        let id = DownloadId::parse(FIRST_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), None, None).await;
        insert_ambiguous_submission(&store, FIRST_ID, "  Paper Lantern  ").await;

        assert_eq!(
            store
                .resolve_observation(&observation(
                    "job-1",
                    None,
                    Some("paper lantern"),
                    "2026-08-24T13:00:00Z",
                ))
                .await
                .unwrap(),
            ObservationResolution::Resolved {
                download_id: id,
                newly_foreign: false,
                attached: true,
            }
        );
        assert_eq!(
            store
                .load_binding(&id)
                .await
                .unwrap()
                .unwrap()
                .native_item_id
                .as_deref(),
            Some("job-1")
        );
    }

    #[tokio::test]
    async fn ambiguous_name_collision_falls_through_to_a_foreign_download() {
        let store = store().await;
        for id in [FIRST_ID, SECOND_ID] {
            insert_download(&store, id, "scryer_submission", None).await;
            insert_binding(&store, id, Some("client-1"), None, None).await;
            insert_ambiguous_submission(&store, id, "Paper Lantern").await;
        }

        let resolution = store
            .resolve_observation(&observation(
                "job-1",
                None,
                Some("paper lantern"),
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap();

        let ObservationResolution::Resolved {
            download_id,
            newly_foreign,
            attached,
        } = resolution;
        assert!(newly_foreign);
        assert!(!attached);
        assert_ne!(download_id, DownloadId::parse(FIRST_ID).unwrap());
        assert_ne!(download_id, DownloadId::parse(SECOND_ID).unwrap());
        assert_eq!(
            store
                .load_binding(&DownloadId::parse(FIRST_ID).unwrap())
                .await
                .unwrap()
                .unwrap()
                .native_item_id,
            None
        );
    }

    #[tokio::test]
    async fn conflicting_token_and_active_locator_is_rejected_without_writes() {
        let store = store().await;
        let first = DownloadId::parse(FIRST_ID).unwrap();
        let second = DownloadId::parse(SECOND_ID).unwrap();
        insert_download(&store, FIRST_ID, "scryer_submission", None).await;
        insert_download(&store, SECOND_ID, "scryer_submission", None).await;
        insert_binding(&store, FIRST_ID, Some("client-1"), Some("job-1"), None).await;
        insert_binding(&store, SECOND_ID, Some("client-1"), Some("other-job"), None).await;
        let before_first = store.load_download(&first).await.unwrap();
        let before_second = store.load_download(&second).await.unwrap();

        let error = store
            .resolve_observation(&observation(
                "job-1",
                Some(&wire(SECOND_ID)),
                None,
                "2026-08-24T13:00:00Z",
            ))
            .await
            .unwrap_err();

        assert!(error.to_string().contains(SECOND_ID));
        assert!(error.to_string().contains(FIRST_ID));
        assert_eq!(store.load_download(&first).await.unwrap(), before_first);
        assert_eq!(store.load_download(&second).await.unwrap(), before_second);
    }

    #[tokio::test]
    async fn concurrent_unseen_locator_resolutions_converge_on_one_foreign_download() {
        let store = store().await;
        let first_store = store.clone();
        let second_store = store.clone();
        let first = tokio::spawn(async move {
            first_store
                .resolve_observation(&observation(
                    "job-1",
                    None,
                    Some("release"),
                    "2026-08-24T13:00:00Z",
                ))
                .await
                .unwrap()
        });
        let second = tokio::spawn(async move {
            second_store
                .resolve_observation(&observation(
                    "job-1",
                    None,
                    Some("release"),
                    "2026-08-24T13:00:01Z",
                ))
                .await
                .unwrap()
        });

        let (first, second) = tokio::join!(first, second);
        let ObservationResolution::Resolved {
            download_id: first_id,
            ..
        } = first.unwrap();
        let ObservationResolution::Resolved {
            download_id: second_id,
            ..
        } = second.unwrap();
        assert_eq!(first_id, second_id);
        assert!(store.load_download(&first_id).await.unwrap().is_some());
    }
}
