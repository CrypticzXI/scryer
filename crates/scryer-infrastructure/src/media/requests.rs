use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, MediaRequestQuery, MediaRequestRepository, NewMediaRequest,
};
use scryer_domain::{
    ExternalId, MediaFacet, MediaRequest, MediaRequestRequester, MediaRequestStatus,
    NewDomainEvent, User,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};
use crate::workflow::stores::append_domain_event_tx;

#[derive(Clone)]
pub struct MediaRequestStore {
    datastore: StoreDatastore,
}

impl MediaRequestStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }
}

#[async_trait]
impl MediaRequestRepository for MediaRequestStore {
    async fn submit(
        &self,
        request: NewMediaRequest,
        requester: &User,
        submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequest> {
        let requester = requester.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "submit_media_request", move |tx| {
            let request = request.clone();
            let requester = requester.clone();
            let submitted_event = submitted_event.clone();
            Box::pin(async move {
                let now = Utc::now();
                insert_media_request_tx(tx, &request, now).await?;
                insert_media_request_external_ids_tx(
                    tx,
                    &request.id,
                    &request.library_id,
                    &request.external_ids,
                    now,
                )
                .await?;
                insert_media_request_requester_tx(tx, &request.id, &requester.id, now).await?;
                append_domain_event_tx(tx, submitted_event).await?;

                load_media_request_tx(tx, &request.id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("media request {}", request.id)))
            })
        })
        .await
    }

    async fn list(&self, query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>> {
        if matches!(&query.library_ids, Some(library_ids) if library_ids.is_empty()) {
            return Ok(Vec::new());
        }

        let (sql, args) = build_media_request_list_sql(&query);
        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        let mut requests = Vec::with_capacity(rows.len());
        for row in rows {
            let mut request = row_to_media_request(&row)?;
            request.external_ids =
                load_media_request_external_ids(self.datastore.read_exec(), &request.id).await?;
            request.requesters =
                load_media_request_requesters(self.datastore.read_exec(), &request.id).await?;
            requests.push(request);
        }
        Ok(requests)
    }
}

async fn insert_media_request_tx(
    tx: &mut SqlTx<'_>,
    request: &NewMediaRequest,
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO media_requests (
            id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
            poster_url, year, overview, runtime_minutes, language, content_status,
            created_by_user_id, created_at, updated_at
        ) VALUES (
            {}, {}, {}, {}, {}, {}, {}, {},
            {}, {}, {}, {}, {}, {},
            {}, {}, {}
        )",
        &[
            SqlArg::Text(request.id.clone()),
            SqlArg::Text(request.library_id.clone()),
            SqlArg::Text(request.facet.as_str().to_string()),
            SqlArg::Text(MediaRequestStatus::Pending.as_str().to_string()),
            SqlArg::Text(request.identity_fingerprint.clone()),
            SqlArg::Text(request.title.clone()),
            SqlArg::OptText(request.sort_title.clone()),
            SqlArg::OptText(request.slug.clone()),
            SqlArg::OptText(request.poster_url.clone()),
            SqlArg::OptI32(request.year),
            SqlArg::OptText(request.overview.clone()),
            SqlArg::OptI32(request.runtime_minutes),
            SqlArg::OptText(request.language.clone()),
            SqlArg::OptText(request.content_status.clone()),
            SqlArg::Text(request.created_by_user_id.clone()),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(())
}

async fn insert_media_request_external_ids_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    library_id: &str,
    external_ids: &[ExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for external_id in external_ids {
        tx.execute(
            "INSERT INTO media_request_external_ids (
                request_id, library_id, source, external_id, created_at
            ) VALUES ({}, {}, {}, {}, {})
            ON CONFLICT (request_id, source, external_id) DO NOTHING",
            &[
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(external_id.source.clone()),
                SqlArg::Text(external_id.value.clone()),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_media_request_requester_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
    user_id: &str,
    requested_at: chrono::DateTime<Utc>,
) -> AppResult<bool> {
    let rows = tx
        .execute(
            "INSERT INTO media_request_requesters (request_id, user_id, requested_at)
             VALUES ({}, {}, {})
             ON CONFLICT (request_id, user_id) DO NOTHING",
            &[
                SqlArg::Text(request_id.to_string()),
                SqlArg::Text(user_id.to_string()),
                SqlArg::Timestamp(requested_at),
            ],
        )
        .await?;
    Ok(rows > 0)
}

async fn load_media_request_tx(
    tx: &mut SqlTx<'_>,
    request_id: &str,
) -> AppResult<Option<MediaRequest>> {
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        "SELECT id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
                poster_url, year, overview, runtime_minutes, language, content_status,
                created_by_user_id, created_at, updated_at
           FROM media_requests
          WHERE id = {}",
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let mut request = row_to_media_request(&row)?;
    request.external_ids = load_media_request_external_ids(SqlExec::Tx(tx), request_id).await?;
    request.requesters = load_media_request_requesters(SqlExec::Tx(tx), request_id).await?;
    Ok(Some(request))
}

async fn load_media_request_external_ids(
    exec: SqlExec<'_, '_>,
    request_id: &str,
) -> AppResult<Vec<ExternalId>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT source, external_id
           FROM media_request_external_ids
          WHERE request_id = {}
          ORDER BY source, external_id",
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ExternalId {
                source: row.text("source")?,
                value: row.text("external_id")?,
            })
        })
        .collect()
}

async fn load_media_request_requesters(
    exec: SqlExec<'_, '_>,
    request_id: &str,
) -> AppResult<Vec<MediaRequestRequester>> {
    let rows = SqlRuntime::fetch_all(
        exec,
        "SELECT mrr.user_id, users.username, mrr.requested_at
           FROM media_request_requesters mrr
           JOIN users ON users.id = mrr.user_id
          WHERE mrr.request_id = {}
          ORDER BY mrr.requested_at ASC, users.username ASC",
        &[SqlArg::Text(request_id.to_string())],
    )
    .await?;
    rows.iter()
        .map(|row| {
            Ok(MediaRequestRequester {
                user_id: row.text("user_id")?,
                username: row.text("username")?,
                requested_at: row.timestamp("requested_at")?,
            })
        })
        .collect()
}

fn build_media_request_list_sql(query: &MediaRequestQuery) -> (String, Vec<SqlArg>) {
    let mut sql = String::from(
        "SELECT id, library_id, facet, status, identity_fingerprint, title, sort_title, slug,
                poster_url, year, overview, runtime_minutes, language, content_status,
                created_by_user_id, created_at, updated_at
           FROM media_requests
          WHERE 1 = 1",
    );
    let mut args = Vec::new();

    if let Some(facet) = &query.facet {
        sql.push_str(" AND facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    if let Some(status) = query.status {
        sql.push_str(" AND status = {}");
        args.push(SqlArg::Text(status.as_str().to_string()));
    }

    if let Some(library_ids) = &query.library_ids {
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" AND library_id IN ({placeholders})"));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    }

    sql.push_str(" ORDER BY updated_at DESC, created_at DESC");
    (sql, args)
}

fn row_to_media_request(row: &SqlRow) -> AppResult<MediaRequest> {
    let facet_raw = row.text("facet")?;
    let facet = MediaFacet::parse(&facet_raw)
        .ok_or_else(|| AppError::Repository(format!("unknown media request facet {facet_raw}")))?;
    let status_raw = row.text("status")?;
    let status = MediaRequestStatus::parse(&status_raw).ok_or_else(|| {
        AppError::Repository(format!("unknown media request status {status_raw}"))
    })?;

    Ok(MediaRequest {
        id: row.text("id")?,
        library_id: row.text("library_id")?,
        facet,
        status,
        identity_fingerprint: row.text("identity_fingerprint")?,
        title: row.text("title")?,
        sort_title: row.opt_text("sort_title")?,
        slug: row.opt_text("slug")?,
        poster_url: row.opt_text("poster_url")?,
        year: row.opt_i32("year")?,
        overview: row.opt_text("overview")?,
        runtime_minutes: row.opt_i32("runtime_minutes")?,
        language: row.opt_text("language")?,
        content_status: row.opt_text("content_status")?,
        external_ids: Vec::new(),
        requesters: Vec::new(),
        created_by_user_id: row.text("created_by_user_id")?,
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}
