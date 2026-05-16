use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, CreateTitleOutcome, PendingTitleHydration, TitleMetadataUpdate,
    TitleRepository,
    persisted_records::PersistedTitleReadMode,
};
use scryer_domain::{ExternalId, MediaFacet, Title};
use serde_json::Value as JsonValue;

use crate::queries::{
    common::parse_utc_datetime,
    sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx, StoreDatastore},
    title::{
        TITLE_COLUMNS, apply_title_metadata_update, decode_optional_runtime_title_row,
        decode_runtime_title_rows, normalize_title_query, normalized_external_ids,
        title_has_tvdb_external_id,
    },
    title_search::{
        delete_title_search_projection_tx, normalize_title_search_text,
        replace_title_search_projection_pg_tx, replace_title_search_projection_tx,
    },
};

const TITLE_INSERT_SQL: &str = "INSERT INTO titles (
    id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at,
    year, overview, poster_url, banner_url, background_url, sort_title, slug, imdb_id,
    runtime_minutes, genres, content_status, language, first_aired, network, studio,
    country, aliases, metadata_language, metadata_fetched_at, min_availability,
    digital_release_date, folder_path, tagged_aliases_json,
    metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}
)";

const TITLE_UPSERT_SQL: &str = "INSERT INTO titles (
    id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at,
    year, overview, poster_url, banner_url, background_url, sort_title, slug, imdb_id,
    runtime_minutes, genres, content_status, language, first_aired, network, studio,
    country, aliases, metadata_language, metadata_fetched_at, min_availability,
    digital_release_date, folder_path, tagged_aliases_json,
    metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}
)
ON CONFLICT (id) DO UPDATE SET
    library_id = excluded.library_id,
    name = excluded.name,
    facet = excluded.facet,
    monitored = excluded.monitored,
    tags = excluded.tags,
    external_ids = excluded.external_ids,
    created_by = excluded.created_by,
    created_at = excluded.created_at,
    year = excluded.year,
    overview = excluded.overview,
    poster_url = excluded.poster_url,
    banner_url = excluded.banner_url,
    background_url = excluded.background_url,
    sort_title = excluded.sort_title,
    slug = excluded.slug,
    imdb_id = excluded.imdb_id,
    runtime_minutes = excluded.runtime_minutes,
    genres = excluded.genres,
    content_status = excluded.content_status,
    language = excluded.language,
    first_aired = excluded.first_aired,
    network = excluded.network,
    studio = excluded.studio,
    country = excluded.country,
    aliases = excluded.aliases,
    metadata_language = excluded.metadata_language,
    metadata_fetched_at = excluded.metadata_fetched_at,
    min_availability = excluded.min_availability,
    digital_release_date = excluded.digital_release_date,
    folder_path = excluded.folder_path,
    tagged_aliases_json = excluded.tagged_aliases_json,
    metadata_hydration_next_attempt_at = CASE
        WHEN excluded.metadata_fetched_at IS NOT NULL THEN NULL
        ELSE COALESCE(titles.metadata_hydration_next_attempt_at, excluded.metadata_hydration_next_attempt_at)
    END,
    metadata_hydration_attempt_count = CASE
        WHEN excluded.metadata_fetched_at IS NOT NULL THEN 0
        ELSE titles.metadata_hydration_attempt_count
    END";

#[derive(Clone)]
pub struct TitleStore {
    datastore: StoreDatastore,
}

impl TitleStore {
    pub fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    async fn list_internal(
        &self,
        facet: Option<MediaFacet>,
        library_ids: Option<&[String]>,
        query: Option<String>,
        mode: PersistedTitleReadMode,
        include_external_ids: bool,
    ) -> AppResult<Vec<Title>> {
        let query = normalize_title_query(query);

        let (sql, args) = if let Some(query) = query {
            build_ranked_title_list_sql(facet, library_ids, &query)
        } else {
            build_plain_title_list_sql(facet, library_ids)
        };

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        decode_runtime_title_rows(&rows, mode, include_external_ids)
    }

    async fn get_by_id_internal(
        &self,
        id: &str,
        include_external_ids: bool,
    ) -> AppResult<Option<Title>> {
        let sql = format!("SELECT {TITLE_COLUMNS} FROM titles WHERE id = {{}}");
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[SqlArg::Text(id.to_string())],
        )
        .await?;

        decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            include_external_ids,
        )
    }
}

#[async_trait]
impl TitleRepository for TitleStore {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            None,
            query,
            PersistedTitleReadMode::Presentation,
            true,
        )
        .await
    }

    async fn list_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            None,
            query,
            PersistedTitleReadMode::Presentation,
            false,
        )
        .await
    }

    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            Some(library_ids),
            query,
            PersistedTitleReadMode::Presentation,
            true,
        )
        .await
    }

    async fn list_for_libraries_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(
            facet,
            Some(library_ids),
            query,
            PersistedTitleReadMode::Presentation,
            false,
        )
        .await
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let normalized_source = source.trim().to_ascii_lowercase();
        let values = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if normalized_source.is_empty() || values.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders = std::iter::repeat_n("{}", values.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT DISTINCT {TITLE_COLUMNS}
               FROM titles
               JOIN title_external_ids ON title_external_ids.title_id = titles.id
              WHERE LOWER(title_external_ids.source) = {{}}
                AND title_external_ids.external_id IN ({placeholders})
              ORDER BY LOWER(name), id"
        );
        let mut args = vec![SqlArg::Text(normalized_source)];
        args.extend(values.into_iter().map(SqlArg::Text));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        decode_runtime_title_rows(&rows, PersistedTitleReadMode::Presentation, true)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_internal(facet, None, query, PersistedTitleReadMode::Matching, true)
            .await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id_internal(id, true).await
    }

    async fn get_by_id_without_external_ids(&self, id: &str) -> AppResult<Option<Title>> {
        self.get_by_id_internal(id, false).await
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE facet = {{}}
                AND LOWER(slug) = LOWER({{}})
              ORDER BY LOWER(name), id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(slug.to_string()),
            ],
        )
        .await?;
        decode_optional_runtime_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        if library_ids.is_empty() {
            return Ok(None);
        }

        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE facet = {{}}
                AND LOWER(slug) = LOWER({{}})
                AND library_id IN ({placeholders})
              ORDER BY LOWER(name), id
              LIMIT 1"
        );
        let mut args = vec![
            SqlArg::Text(facet.as_str().to_string()),
            SqlArg::Text(slug.to_string()),
        ];
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));

        let row = SqlRuntime::fetch_optional(self.datastore.read_exec(), &sql, &args).await?;
        decode_optional_runtime_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let sql = format!(
            "SELECT DISTINCT {TITLE_COLUMNS}
               FROM titles
               JOIN title_external_ids ON title_external_ids.title_id = titles.id
              WHERE LOWER(title_external_ids.source) = {{}}
                AND title_external_ids.external_id = {{}}
              ORDER BY LOWER(name), id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(source.trim().to_ascii_lowercase()),
                SqlArg::Text(value.trim().to_string()),
            ],
        )
        .await?;
        decode_optional_runtime_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let sql = format!(
            "SELECT DISTINCT {TITLE_COLUMNS}
               FROM titles
               JOIN title_external_ids ON title_external_ids.title_id = titles.id
              WHERE titles.facet = {{}}
                AND LOWER(title_external_ids.source) = {{}}
                AND title_external_ids.external_id = {{}}
              ORDER BY LOWER(name), id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            &sql,
            &[
                SqlArg::Text(facet.as_str().to_string()),
                SqlArg::Text(source.trim().to_ascii_lowercase()),
                SqlArg::Text(value.trim().to_string()),
            ],
        )
        .await?;
        decode_optional_runtime_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "create_or_get_existing_title",
            move |tx| {
                let title = title.clone();
                Box::pin(async move {
                    if let Some(existing) = find_existing_title_for_create_tx(tx, &title).await? {
                        return Ok(CreateTitleOutcome {
                            title: existing,
                            reused_existing: true,
                        });
                    }

                    create_title_tx(tx, &title).await?;
                    Ok(CreateTitleOutcome {
                        title,
                        reused_existing: false,
                    })
                })
            },
        )
        .await
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_title", move |tx| {
            let title = title.clone();
            Box::pin(async move {
                create_title_tx(tx, &title).await?;
                Ok(title)
            })
        })
        .await
    }

    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        let mut sql = format!(
            "SELECT {TITLE_COLUMNS}, metadata_hydration_attempt_count
               FROM titles
              WHERE metadata_fetched_at IS NULL
                AND metadata_hydration_next_attempt_at IS NOT NULL
                AND metadata_hydration_next_attempt_at <= {{}}"
        );
        let mut args = vec![SqlArg::Timestamp(Utc::now())];
        if !excluded_facets.is_empty() {
            let placeholders = std::iter::repeat_n("{}", excluded_facets.len())
                .collect::<Vec<_>>()
                .join(", ");
            sql.push_str(&format!(" AND facet NOT IN ({placeholders})"));
            args.extend(
                excluded_facets
                    .iter()
                    .map(|facet| SqlArg::Text(facet.as_str().to_string())),
            );
        }
        sql.push_str(" ORDER BY metadata_hydration_next_attempt_at ASC, id ASC LIMIT {}");
        args.push(SqlArg::I64(limit as i64));

        let rows = SqlRuntime::fetch_all(self.datastore.read_exec(), &sql, &args).await?;
        rows.into_iter()
            .map(|row| {
                Ok(PendingTitleHydration {
                    title: decode_optional_runtime_title_row(
                        Some(&row),
                        PersistedTitleReadMode::Presentation,
                        true,
                    )?
                    .ok_or_else(|| {
                        AppError::Repository(
                            "title hydration query returned an empty row projection".to_string(),
                        )
                    })?,
                    attempt_count: row.i64("metadata_hydration_attempt_count")?,
                })
            })
            .collect()
    }

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id
               FROM titles
              WHERE facet = 'anime'
                AND EXISTS (
                    SELECT 1
                      FROM title_external_ids
                     WHERE title_external_ids.title_id = titles.id
                       AND LOWER(title_external_ids.source) IN ('tvdb', 'tvdb_id')
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM collection_external_ids
                     WHERE collection_external_ids.title_id = titles.id
                       AND collection_external_ids.provenance = 'anibridge'
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM episode_external_ids
                     WHERE episode_external_ids.title_id = titles.id
                       AND episode_external_ids.provenance = 'anibridge'
                )
              ORDER BY
                CASE WHEN metadata_fetched_at IS NULL THEN 0 ELSE 1 END,
                metadata_fetched_at,
                created_at
              LIMIT {}",
            &[SqlArg::I64(limit as i64)],
        )
        .await?;
        rows.into_iter().map(|row| row.text("id")).collect()
    }

    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT id
               FROM titles
              WHERE facet = 'anime'
                AND EXISTS (
                    SELECT 1
                      FROM title_external_ids
                     WHERE title_external_ids.title_id = titles.id
                       AND LOWER(title_external_ids.source) IN ('tvdb', 'tvdb_id')
                )
                AND NOT EXISTS (
                    SELECT 1
                      FROM title_external_ids
                     WHERE title_external_ids.title_id = titles.id
                       AND LOWER(title_external_ids.source) IN ('anidb', 'anidb_id')
                )
              ORDER BY
                CASE WHEN metadata_fetched_at IS NULL THEN 0 ELSE 1 END,
                metadata_fetched_at,
                created_at
              LIMIT {}",
            &[SqlArg::I64(limit as i64)],
        )
        .await?;
        rows.into_iter().map(|row| row.text("id")).collect()
    }

    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "mark_title_metadata_hydration_due_now",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = {},
                                metadata_hydration_attempt_count = 0
                          WHERE id = {}",
                        &[SqlArg::Timestamp(Utc::now()), SqlArg::Text(id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        let id = id.to_string();
        let next_attempt_at = parse_utc_datetime(next_attempt_at)?;
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "schedule_title_metadata_hydration_retry",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = {},
                                metadata_hydration_attempt_count = {}
                          WHERE id = {}",
                        &[
                            SqlArg::Timestamp(next_attempt_at),
                            SqlArg::I64(attempt_count),
                            SqlArg::Text(id),
                        ],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "clear_title_metadata_hydration_retry_state",
            move |tx| {
                let id = id.clone();
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_hydration_next_attempt_at = NULL,
                                metadata_hydration_attempt_count = 0
                          WHERE id = {}",
                        &[SqlArg::Text(id)],
                    )
                    .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_title_monitored", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                let mut title = load_title_tx(tx, &id, true)
                    .await?
                    .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
                title.monitored = monitored;
                persist_title_tx(tx, &title).await?;
                Ok(title)
            })
        })
        .await
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_title_metadata", move |tx| {
            let id = id.clone();
            let name = name.clone();
            let facet = facet.clone();
            let tags = tags.clone();
            Box::pin(async move {
                let mut title = load_title_tx(tx, &id, true)
                    .await?
                    .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
                if let Some(name) = name {
                    title.name = name;
                }
                if let Some(facet) = facet {
                    title.facet = facet;
                }
                if let Some(tags) = tags {
                    title.tags = tags;
                }
                persist_title_tx(tx, &title).await?;
                Ok(title)
            })
        })
        .await
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_title_hydrated_metadata",
            move |tx| {
                let id = id.clone();
                let metadata = metadata.clone();
                Box::pin(async move {
                    let mut title = load_title_tx(tx, &id, true)
                        .await?
                        .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
                    apply_title_metadata_update(&mut title, metadata)?;
                    persist_title_tx(tx, &title).await?;
                    Ok(title)
                })
            },
        )
        .await
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "replace_title_match_state", move |tx| {
            let id = id.clone();
            let external_ids = external_ids.clone();
            let tags = tags.clone();
            Box::pin(async move {
                let mut title = load_title_tx(tx, &id, true)
                    .await?
                    .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
                title.external_ids = external_ids;
                title.tags = tags;
                title.year = None;
                title.overview = None;
                title.poster_url = None;
                title.poster_source_url = None;
                title.banner_url = None;
                title.banner_source_url = None;
                title.background_url = None;
                title.background_source_url = None;
                title.sort_title = None;
                title.slug = None;
                title.imdb_id = None;
                title.runtime_minutes = None;
                title.genres.clear();
                title.content_status = None;
                title.language = None;
                title.first_aired = None;
                title.network = None;
                title.studio = None;
                title.country = None;
                title.aliases.clear();
                title.tagged_aliases.clear();
                title.metadata_language = None;
                title.metadata_fetched_at = None;
                title.digital_release_date = None;
                persist_title_tx(tx, &title).await?;
                Ok(title)
            })
        })
        .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_title", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                delete_title_search_projection_sql_tx(tx, &id).await?;
                let rows = SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "DELETE FROM titles WHERE id = {}",
                    &[SqlArg::Text(id.clone())],
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::NotFound(format!("title {id}")));
                }
                Ok(())
            })
        })
        .await
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        let id = id.to_string();
        let folder_path = folder_path.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_title_folder_path", move |tx| {
            let id = id.clone();
            let folder_path = folder_path.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE titles SET folder_path = {} WHERE id = {}",
                    &[SqlArg::Text(folder_path), SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "clear_title_folder_path", move |tx| {
            let id = id.clone();
            Box::pin(async move {
                SqlRuntime::execute(
                    SqlExec::Tx(tx),
                    "UPDATE titles SET folder_path = NULL WHERE id = {}",
                    &[SqlArg::Text(id)],
                )
                .await?;
                Ok(())
            })
        })
        .await
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "clear_metadata_language_for_all",
            move |tx| {
                Box::pin(async move {
                    SqlRuntime::execute(
                        SqlExec::Tx(tx),
                        "UPDATE titles
                            SET metadata_language = NULL
                          WHERE metadata_language IS NOT NULL",
                        &[],
                    )
                    .await
                })
            },
        )
        .await
    }
}

fn build_plain_title_list_sql(
    facet: Option<MediaFacet>,
    library_ids: Option<&[String]>,
) -> (String, Vec<SqlArg>) {
    let mut sql = format!("SELECT {TITLE_COLUMNS} FROM titles");
    let mut where_clauses = Vec::new();
    let mut args = Vec::new();

    if let Some(facet) = facet {
        where_clauses.push("facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }

    if let Some(library_ids) = library_ids
        && !library_ids.is_empty()
    {
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        where_clauses.push(Box::leak(format!("library_id IN ({placeholders})").into_boxed_str()));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY LOWER(name), id");
    (sql, args)
}

fn build_ranked_title_list_sql(
    facet: Option<MediaFacet>,
    library_ids: Option<&[String]>,
    query: &str,
) -> (String, Vec<SqlArg>) {
    let normalized = normalize_title_search_text(query);
    let mut sql = format!(
        "SELECT {TITLE_COLUMNS}
           FROM titles
           JOIN (
                SELECT title_id,
                       MIN(
                           CASE
                               WHEN normalized_term = {{}} THEN 0
                               WHEN normalized_term LIKE {{}} THEN 1000
                               WHEN normalized_term LIKE {{}} THEN 2000
                               ELSE 3000
                           END + weight
                       ) AS rank
                  FROM title_search_terms"
    );
    let mut where_clauses = Vec::new();
    let mut args = vec![
        SqlArg::Text(normalized.clone()),
        SqlArg::Text(format!("{normalized}%")),
        SqlArg::Text(format!("%{normalized}%")),
    ];

    if let Some(facet) = facet {
        where_clauses.push("facet = {}");
        args.push(SqlArg::Text(facet.as_str().to_string()));
    }
    where_clauses.push("normalized_term LIKE {}");
    args.push(SqlArg::Text(format!("%{normalized}%")));

    sql.push_str(" WHERE ");
    sql.push_str(&where_clauses.join(" AND "));
    sql.push_str(
        " GROUP BY title_id
           ) ranked_titles ON ranked_titles.title_id = titles.id",
    );

    if let Some(library_ids) = library_ids
        && !library_ids.is_empty()
    {
        let placeholders = std::iter::repeat_n("{}", library_ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str(&format!(" WHERE library_id IN ({placeholders})"));
        args.extend(library_ids.iter().cloned().map(SqlArg::Text));
    }

    sql.push_str(" ORDER BY ranked_titles.rank, LOWER(name), id");
    (sql, args)
}

async fn load_title_tx(
    tx: &mut SqlTx<'_>,
    id: &str,
    include_external_ids: bool,
) -> AppResult<Option<Title>> {
    let sql = format!("SELECT {TITLE_COLUMNS} FROM titles WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(SqlExec::Tx(tx), &sql, &[SqlArg::Text(id.to_string())])
        .await?;
    decode_optional_runtime_title_row(
        row.as_ref(),
        PersistedTitleReadMode::Presentation,
        include_external_ids,
    )
}

async fn find_existing_title_for_create_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
) -> AppResult<Option<Title>> {
    let external_ids = normalized_external_ids(&title.external_ids);
    if !external_ids.is_empty() {
        let mut sql =
            "SELECT DISTINCT title_id FROM title_external_ids WHERE library_id = {}".to_string();
        let mut args = vec![SqlArg::Text(title.library_id.clone())];
        sql.push_str(" AND (");
        for (index, (source, value)) in external_ids.iter().enumerate() {
            if index > 0 {
                sql.push_str(" OR ");
            }
            sql.push_str("(LOWER(source) = LOWER({}) AND external_id = {})");
            args.push(SqlArg::Text(source.clone()));
            args.push(SqlArg::Text(value.clone()));
        }
        sql.push(')');

        let rows = SqlRuntime::fetch_all(SqlExec::Tx(tx), &sql, &args).await?;
        let title_ids = rows
            .into_iter()
            .map(|row| row.text("title_id"))
            .collect::<AppResult<Vec<_>>>()?;
        match title_ids.as_slice() {
            [] => {}
            [existing_id] => return load_title_tx(tx, existing_id, true).await,
            _ => {
                return Err(AppError::Validation(
                    "external ids already map to multiple titles".to_string(),
                ));
            }
        }
    }

    if let Some(slug) = title.slug.as_deref() {
        let sql = format!(
            "SELECT {TITLE_COLUMNS}
               FROM titles
              WHERE library_id = {{}}
                AND facet = {{}}
                AND LOWER(slug) = LOWER({{}})
              ORDER BY LOWER(name), id
              LIMIT 1"
        );
        let row = SqlRuntime::fetch_optional(
            SqlExec::Tx(tx),
            &sql,
            &[
                SqlArg::Text(title.library_id.clone()),
                SqlArg::Text(title.facet.as_str().to_string()),
                SqlArg::Text(slug.to_string()),
            ],
        )
        .await?;
        return decode_optional_runtime_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            true,
        );
    }

    Ok(None)
}

async fn create_title_tx(tx: &mut SqlTx<'_>, title: &Title) -> AppResult<()> {
    let args = title_write_args(title, scheduled_hydration_attempt(title));
    SqlRuntime::execute(SqlExec::Tx(tx), TITLE_INSERT_SQL, &args).await?;
    replace_title_search_projection_sql_tx(tx, title).await?;
    replace_title_external_ids_projection_sql_tx(tx, title).await?;
    Ok(())
}

async fn persist_title_tx(tx: &mut SqlTx<'_>, title: &Title) -> AppResult<()> {
    let args = title_write_args(title, scheduled_hydration_attempt(title));
    SqlRuntime::execute(SqlExec::Tx(tx), TITLE_UPSERT_SQL, &args).await?;
    replace_title_search_projection_sql_tx(tx, title).await?;
    replace_title_external_ids_projection_sql_tx(tx, title).await?;
    Ok(())
}

fn scheduled_hydration_attempt(title: &Title) -> Option<chrono::DateTime<Utc>> {
    if title.metadata_fetched_at.is_none() && title_has_tvdb_external_id(title) {
        Some(Utc::now())
    } else {
        None
    }
}

fn title_write_args(
    title: &Title,
    metadata_hydration_next_attempt_at: Option<chrono::DateTime<Utc>>,
) -> Vec<SqlArg> {
    vec![
        SqlArg::Text(title.id.clone()),
        SqlArg::Text(title.library_id.clone()),
        SqlArg::Text(title.name.clone()),
        SqlArg::Text(title.facet.as_str().to_string()),
        SqlArg::Bool(title.monitored),
        SqlArg::Json(serde_json::to_value(&title.tags).unwrap_or(JsonValue::Array(Vec::new()))),
        SqlArg::Json(
            serde_json::to_value(&title.external_ids).unwrap_or(JsonValue::Array(Vec::new())),
        ),
        SqlArg::OptText(title.created_by.clone()),
        SqlArg::Timestamp(title.created_at),
        SqlArg::OptI64(title.year.map(i64::from)),
        SqlArg::OptText(title.overview.clone()),
        SqlArg::OptText(title.poster_url.clone()),
        SqlArg::OptText(title.banner_url.clone()),
        SqlArg::OptText(title.background_url.clone()),
        SqlArg::OptText(title.sort_title.clone()),
        SqlArg::OptText(title.slug.clone()),
        SqlArg::OptText(title.imdb_id.clone()),
        SqlArg::OptI64(title.runtime_minutes.map(i64::from)),
        SqlArg::Json(serde_json::to_value(&title.genres).unwrap_or(JsonValue::Array(Vec::new()))),
        SqlArg::OptText(title.content_status.clone()),
        SqlArg::OptText(title.language.clone()),
        SqlArg::OptText(title.first_aired.clone()),
        SqlArg::OptText(title.network.clone()),
        SqlArg::OptText(title.studio.clone()),
        SqlArg::OptText(title.country.clone()),
        SqlArg::Json(serde_json::to_value(&title.aliases).unwrap_or(JsonValue::Array(Vec::new()))),
        SqlArg::OptText(title.metadata_language.clone()),
        SqlArg::OptTimestamp(title.metadata_fetched_at),
        SqlArg::OptText(title.min_availability.clone()),
        SqlArg::OptText(title.digital_release_date.clone()),
        SqlArg::OptText(title.folder_path.clone()),
        SqlArg::Json(
            serde_json::to_value(&title.tagged_aliases).unwrap_or(JsonValue::Array(Vec::new())),
        ),
        SqlArg::OptTimestamp(metadata_hydration_next_attempt_at),
        SqlArg::I64(0),
    ]
}

async fn replace_title_external_ids_projection_sql_tx(
    tx: &mut SqlTx<'_>,
    title: &Title,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Tx(tx),
        "DELETE FROM title_external_ids WHERE title_id = {}",
        &[SqlArg::Text(title.id.clone())],
    )
    .await?;

    let now = Utc::now();
    for (source, value) in normalized_external_ids(&title.external_ids) {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "INSERT INTO title_external_ids
             (id, title_id, library_id, facet, source, external_id, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(scryer_domain::Id::new().0),
                SqlArg::Text(title.id.clone()),
                SqlArg::Text(title.library_id.clone()),
                SqlArg::Text(title.facet.as_str().to_string()),
                SqlArg::Text(source),
                SqlArg::Text(value),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }

    Ok(())
}

async fn replace_title_search_projection_sql_tx(tx: &mut SqlTx<'_>, title: &Title) -> AppResult<()> {
    if let Some(sqlite_tx) = tx.sqlite() {
        replace_title_search_projection_tx(sqlite_tx, title).await
    } else if let Some(pg_tx) = tx.postgres() {
        replace_title_search_projection_pg_tx(pg_tx, title).await
    } else {
        Err(AppError::Repository(
            "unsupported transaction backend for title search projection".to_string(),
        ))
    }
}

async fn delete_title_search_projection_sql_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    if let Some(sqlite_tx) = tx.sqlite() {
        delete_title_search_projection_tx(sqlite_tx, title_id).await
    } else {
        SqlRuntime::execute(
            SqlExec::Tx(tx),
            "DELETE FROM title_search_terms WHERE title_id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        Ok(())
    }
}
