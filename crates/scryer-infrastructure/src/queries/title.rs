use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, EpisodeUpdate,
    PendingTitleHydration, PrimaryCollectionSummary, ScopedExternalId, TitleMetadataUpdate,
};
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, Episode, ExternalId, InterstitialMovieMetadata,
    MediaFacet, Title,
};
use serde_json;
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

use super::common::{parse_utc_datetime, repository_error_from_sqlx};
use crate::title_images::{normalized_base_path_from_env, prefix_local_title_image_path};

const TITLE_COLUMNS: &str = "id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at, \
    year, overview, poster_url, poster_local_path, banner_url, banner_local_path, background_url, background_local_path, \
    sort_title, slug, imdb_id, runtime_minutes, genres, \
    content_status, language, first_aired, network, studio, country, aliases, \
    metadata_language, metadata_fetched_at, min_availability, digital_release_date, folder_path, tagged_aliases_json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TitleReadMode {
    Presentation,
    Matching,
}

fn parse_facet(raw: &str) -> MediaFacet {
    MediaFacet::parse(raw).unwrap_or_default()
}

pub(crate) async fn list_titles_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    query: Option<String>,
) -> AppResult<Vec<Title>> {
    list_titles_query_with_mode(pool, facet, query, TitleReadMode::Presentation).await
}

pub(crate) async fn list_titles_for_libraries_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    library_ids: &[String],
    query: Option<String>,
) -> AppResult<Vec<Title>> {
    if library_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {} FROM titles WHERE library_id IN (",
        TITLE_COLUMNS
    ));
    for (index, library_id) in library_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(library_id);
    }
    builder.push(")");
    if let Some(facet) = facet {
        builder.push(" AND facet = ");
        builder.push_bind(facet.as_str());
    }
    if let Some(search) = query {
        builder.push(" AND LOWER(name) LIKE ");
        builder.push_bind(format!("%{}%", search.to_lowercase()));
    }
    builder.push(" ORDER BY LOWER(name) ASC, id ASC");
    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let base_path = normalized_base_path_from_env();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_title(&row, TitleReadMode::Presentation, &base_path)?);
    }
    Ok(out)
}

pub(crate) async fn list_titles_for_matching_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    query: Option<String>,
) -> AppResult<Vec<Title>> {
    list_titles_query_with_mode(pool, facet, query, TitleReadMode::Matching).await
}

async fn list_titles_query_with_mode(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    query: Option<String>,
    mode: TitleReadMode,
) -> AppResult<Vec<Title>> {
    if matches!(mode, TitleReadMode::Presentation)
        && let Some(search) = query.as_deref()
    {
        return list_titles_via_title_search_query(pool, facet, search, mode).await;
    }

    let mut sql = format!("SELECT {} FROM titles", TITLE_COLUMNS);

    let mut where_clauses = Vec::new();
    if facet.is_some() {
        where_clauses.push("facet = ?");
    }
    if query.is_some() {
        where_clauses.push("LOWER(name) LIKE ?");
    }

    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY LOWER(name) ASC, id ASC");

    let mut statement = sqlx::query(&sql);

    if let Some(selected_facet) = facet {
        statement = statement.bind(selected_facet.as_str());
    }
    if let Some(search) = query {
        statement = statement.bind(format!("%{}%", search.to_lowercase()));
    }

    let rows = statement
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let base_path = normalized_base_path_from_env();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_title(&row, mode, &base_path)?);
    }
    Ok(out)
}

async fn list_titles_via_title_search_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
    query: &str,
    mode: TitleReadMode,
) -> AppResult<Vec<Title>> {
    let Some(search_plan) = super::title_search::build_title_search_plan(facet, query) else {
        return Ok(Vec::new());
    };

    let mut builder = QueryBuilder::<Sqlite>::new("");
    super::title_search::push_ranked_title_matches_cte(&mut builder, &search_plan);
    builder.push(format!(
        "SELECT {} FROM ranked_title_matches
         JOIN titles ON titles.id = ranked_title_matches.title_id
         ORDER BY ranked_title_matches.rank ASC, LOWER(titles.name) ASC, titles.id ASC",
        TITLE_COLUMNS
    ));

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let base_path = normalized_base_path_from_env();
    let mut titles = Vec::with_capacity(rows.len());
    for row in rows {
        titles.push(row_to_title(&row, mode, &base_path)?);
    }

    Ok(titles)
}

pub(crate) async fn clear_metadata_language_for_all_query(pool: &SqlitePool) -> AppResult<u64> {
    let result = sqlx::query(
        "UPDATE titles SET metadata_language = NULL WHERE metadata_language IS NOT NULL",
    )
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(result.rows_affected())
}

pub(crate) async fn get_title_by_id_query(pool: &SqlitePool, id: &str) -> AppResult<Option<Title>> {
    let sql = format!("SELECT {} FROM titles WHERE id = ?", TITLE_COLUMNS);
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_title(
            &row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_title_by_facet_and_slug_query(
    pool: &SqlitePool,
    facet: MediaFacet,
    slug: &str,
) -> AppResult<Option<Title>> {
    let normalized_slug = slug.trim().to_ascii_lowercase();
    if normalized_slug.is_empty() {
        return Ok(None);
    }

    let sql = format!(
        "SELECT {} FROM titles\n         WHERE facet = ?\n           AND LOWER(TRIM(slug)) = ?\n         ORDER BY id ASC\n         LIMIT 2",
        TITLE_COLUMNS,
    );
    let rows = sqlx::query(&sql)
        .bind(facet.as_str())
        .bind(&normalized_slug)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match rows.as_slice() {
        [] => Ok(None),
        [row] => Ok(Some(row_to_title(
            row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        _ => Err(AppError::Validation(format!(
            "multiple titles found for facet `{}` and slug `{normalized_slug}`",
            facet.as_str()
        ))),
    }
}

pub(crate) async fn get_title_by_facet_libraries_and_slug_query(
    pool: &SqlitePool,
    facet: MediaFacet,
    library_ids: &[String],
    slug: &str,
) -> AppResult<Option<Title>> {
    let normalized_slug = slug.trim().to_ascii_lowercase();
    if normalized_slug.is_empty() || library_ids.is_empty() {
        return Ok(None);
    }

    let mut builder = QueryBuilder::<Sqlite>::new(format!(
        "SELECT {} FROM titles
         WHERE facet = ",
        TITLE_COLUMNS,
    ));
    builder.push_bind(facet.as_str());
    builder.push(" AND LOWER(TRIM(slug)) = ");
    builder.push_bind(&normalized_slug);
    builder.push(" AND library_id IN (");
    for (index, library_id) in library_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(library_id);
    }
    builder.push(") ORDER BY id ASC LIMIT 2");

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    match rows.as_slice() {
        [] => Ok(None),
        [row] => Ok(Some(row_to_title(
            row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        _ => Err(AppError::Validation(format!(
            "multiple titles found for slug `{normalized_slug}` in accessible libraries",
        ))),
    }
}

pub(crate) async fn get_title_by_external_id_query(
    pool: &SqlitePool,
    source: &str,
    value: &str,
) -> AppResult<Option<Title>> {
    let row = sqlx::query(
        "SELECT titles.* FROM titles
         JOIN title_external_ids ON title_external_ids.title_id = titles.id
         WHERE LOWER(title_external_ids.source) = LOWER(?)
           AND title_external_ids.external_id = ?
         ORDER BY titles.id ASC
         LIMIT 1",
    )
    .bind(source)
    .bind(value)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_title(
            &row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_titles_by_external_ids_query(
    pool: &SqlitePool,
    source: &str,
    values: &[String],
) -> AppResult<Vec<Title>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new("WITH requested(ordinal, external_id) AS (");
    for (ordinal, value) in values.iter().enumerate() {
        if ordinal > 0 {
            builder.push(" UNION ALL ");
        }
        builder
            .push("SELECT ")
            .push_bind(ordinal as i64)
            .push(", ")
            .push_bind(value);
    }
    builder.push(
        "), requested_title_ids AS (
            SELECT
                requested.ordinal AS ordinal,
                (
                    SELECT title_external_ids.title_id
                    FROM title_external_ids
                    WHERE LOWER(title_external_ids.source) = LOWER(",
    );
    builder.push_bind(source);
    builder.push(
        ")
                      AND title_external_ids.external_id = requested.external_id
                    ORDER BY title_external_ids.title_id ASC
                    LIMIT 1
                ) AS title_id
            FROM requested
        ), deduped AS (
            SELECT MIN(ordinal) AS ordinal, title_id
            FROM requested_title_ids
            WHERE title_id IS NOT NULL
            GROUP BY title_id
        )
        SELECT titles.*",
    );
    builder.push(
        " FROM deduped
          JOIN titles ON titles.id = deduped.title_id
          ORDER BY deduped.ordinal ASC, titles.id ASC",
    );

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let base_path = normalized_base_path_from_env();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_title(&row, TitleReadMode::Presentation, &base_path)?);
    }
    Ok(out)
}

pub(crate) async fn get_title_by_external_id_in_facet_query(
    pool: &SqlitePool,
    facet: MediaFacet,
    source: &str,
    value: &str,
) -> AppResult<Option<Title>> {
    let row = sqlx::query(
        "SELECT titles.*
         FROM titles
         JOIN title_external_ids ON title_external_ids.title_id = titles.id
         WHERE title_external_ids.facet = ?
           AND LOWER(title_external_ids.source) = LOWER(?)
           AND title_external_ids.external_id = ?
         ORDER BY titles.id ASC
         LIMIT 1",
    )
    .bind(facet.as_str())
    .bind(source)
    .bind(value)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_title(
            &row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        None => Ok(None),
    }
}

fn normalized_external_ids(external_ids: &[ExternalId]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for external_id in external_ids {
        let source = external_id.source.trim().to_ascii_lowercase();
        let value = external_id.value.trim().to_string();
        if source.is_empty() || value.is_empty() {
            continue;
        }
        if seen.insert((source.clone(), value.clone())) {
            out.push((source, value));
        }
    }
    out
}

fn normalized_scoped_external_id(
    scoped_id: &ScopedExternalId,
) -> Option<(String, String, String, String)> {
    let scope_id = scoped_id.scope_id.trim();
    let source = scoped_id.source.trim().to_ascii_lowercase();
    let external_id = scoped_id.external_id.trim();
    if scope_id.is_empty() || source.is_empty() || external_id.is_empty() {
        return None;
    }

    let source_scope = scoped_id
        .source_scope
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    Some((
        scope_id.to_string(),
        source,
        external_id.to_string(),
        source_scope,
    ))
}

fn row_to_scoped_external_id(row: &sqlx::sqlite::SqliteRow) -> AppResult<ScopedExternalId> {
    let source_scope: String = row
        .try_get("source_scope")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(ScopedExternalId {
        scope_id: row
            .try_get("scope_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        source: row
            .try_get("source")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        external_id: row
            .try_get("external_id")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        provenance: row
            .try_get("provenance")
            .map_err(|err| AppError::Repository(err.to_string()))?,
        source_scope: if source_scope.trim().is_empty() {
            None
        } else {
            Some(source_scope)
        },
    })
}

async fn list_existing_title_ids_for_external_ids_tx(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: &str,
    external_ids: &[(String, String)],
) -> AppResult<Vec<String>> {
    if external_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT title_id FROM title_external_ids WHERE library_id = ",
    );
    builder.push_bind(library_id);
    builder.push(" AND (");
    for (index, (source, value)) in external_ids.iter().enumerate() {
        if index > 0 {
            builder.push(" OR ");
        }
        builder
            .push("(source = ")
            .push_bind(source)
            .push(" AND external_id = ")
            .push_bind(value)
            .push(")");
    }
    builder.push(")");

    let rows = builder
        .build()
        .fetch_all(&mut **tx)
        .await
        .map_err(repository_error_from_sqlx)?;

    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(
            row.try_get("title_id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        );
    }
    Ok(ids)
}

async fn replace_title_external_ids_projection_tx(
    tx: &mut Transaction<'_, Sqlite>,
    title_id: &str,
    library_id: &str,
    facet: MediaFacet,
    external_ids: &[ExternalId],
) -> AppResult<()> {
    sqlx::query("DELETE FROM title_external_ids WHERE title_id = ?")
        .bind(title_id)
        .execute(&mut **tx)
        .await
        .map_err(repository_error_from_sqlx)?;

    let now = chrono::Utc::now().to_rfc3339();
    for (source, value) in normalized_external_ids(external_ids) {
        sqlx::query(
            "INSERT INTO title_external_ids
             (id, title_id, library_id, facet, source, external_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(scryer_domain::Id::new().0)
        .bind(title_id)
        .bind(library_id)
        .bind(facet.as_str())
        .bind(source)
        .bind(value)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }

    Ok(())
}

async fn get_title_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> AppResult<Option<Title>> {
    let sql = format!("SELECT {} FROM titles WHERE id = ?", TITLE_COLUMNS);
    let row = sqlx::query(&sql)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repository_error_from_sqlx)?;

    match row {
        Some(row) => Ok(Some(row_to_title(
            &row,
            TitleReadMode::Presentation,
            &normalized_base_path_from_env(),
        )?)),
        None => Ok(None),
    }
}

fn row_to_title(
    row: &sqlx::sqlite::SqliteRow,
    mode: TitleReadMode,
    base_path: &str,
) -> AppResult<Title> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let name: String = row
        .try_get("name")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let facet: String = row
        .try_get("facet")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let parsed_facet = parse_facet(&facet);
    let library_id: String = row
        .try_get("library_id")
        .unwrap_or_else(|_| scryer_domain::default_library_id_for_facet(&parsed_facet));
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let tags_json: String = row
        .try_get("tags")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let external_ids_json: String = row
        .try_get("external_ids")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_by: Option<String> = row
        .try_get("created_by")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let tags: Vec<String> =
        serde_json::from_str(&tags_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let external_ids: Vec<ExternalId> = serde_json::from_str(&external_ids_json)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at = parse_utc_datetime(&created_at_raw)?;

    // metadata fields
    let year: Option<i32> = row.try_get("year").unwrap_or(None);
    let overview: Option<String> = row.try_get("overview").unwrap_or(None);
    let poster_url_source: Option<String> = row.try_get("poster_url").unwrap_or(None);
    let poster_local_path: Option<String> = row.try_get("poster_local_path").unwrap_or(None);
    let banner_url_source: Option<String> = row.try_get("banner_url").unwrap_or(None);
    let banner_local_path: Option<String> = row.try_get("banner_local_path").unwrap_or(None);
    let background_url_source: Option<String> = row.try_get("background_url").unwrap_or(None);
    let background_local_path: Option<String> =
        row.try_get("background_local_path").unwrap_or(None);
    let sort_title: Option<String> = row.try_get("sort_title").unwrap_or(None);
    let slug: Option<String> = row.try_get("slug").unwrap_or(None);
    let imdb_id: Option<String> = row.try_get("imdb_id").unwrap_or(None);
    let runtime_minutes: Option<i32> = row.try_get("runtime_minutes").unwrap_or(None);
    let genres_json: String = row.try_get("genres").unwrap_or_else(|_| "[]".to_string());
    let content_status: Option<String> = row.try_get("content_status").unwrap_or(None);
    let language: Option<String> = row.try_get("language").unwrap_or(None);
    let first_aired: Option<String> = row.try_get("first_aired").unwrap_or(None);
    let network: Option<String> = row.try_get("network").unwrap_or(None);
    let studio: Option<String> = row.try_get("studio").unwrap_or(None);
    let country: Option<String> = row.try_get("country").unwrap_or(None);
    let aliases_json: String = row.try_get("aliases").unwrap_or_else(|_| "[]".to_string());
    let metadata_language: Option<String> = row.try_get("metadata_language").unwrap_or(None);
    let metadata_fetched_at_raw: Option<String> =
        row.try_get("metadata_fetched_at").unwrap_or(None);
    let min_availability: Option<String> = row.try_get("min_availability").unwrap_or(None);
    let digital_release_date: Option<String> = row.try_get("digital_release_date").unwrap_or(None);
    let folder_path: Option<String> = row.try_get("folder_path").unwrap_or(None);

    let genres: Vec<String> =
        serde_json::from_str(&genres_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases: Vec<String> =
        serde_json::from_str(&aliases_json).map_err(|err| AppError::Repository(err.to_string()))?;
    let metadata_fetched_at = match metadata_fetched_at_raw {
        Some(raw) => Some(parse_utc_datetime(&raw)?),
        None => None,
    };

    let mut title = Title {
        id,
        library_id,
        name,
        facet: parsed_facet,
        monitored: monitored != 0,
        tags,
        external_ids,
        created_by,
        created_at,
        year,
        overview,
        poster_url: poster_url_source,
        poster_source_url: None,
        banner_url: banner_url_source,
        banner_source_url: None,
        background_url: background_url_source,
        background_source_url: None,
        sort_title,
        slug,
        imdb_id,
        runtime_minutes,
        genres,
        content_status,
        language,
        first_aired,
        network,
        studio,
        country,
        aliases,
        tagged_aliases: {
            let raw: Option<String> = row.try_get("tagged_aliases_json").unwrap_or(None);
            raw.as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default()
        },
        metadata_language,
        metadata_fetched_at,
        min_availability,
        digital_release_date,
        folder_path,
    };

    if mode == TitleReadMode::Presentation {
        if let Some(local_path) = poster_local_path.as_deref() {
            title.poster_source_url = title.poster_url.take();
            title.poster_url = Some(prefix_local_title_image_path(base_path, local_path));
        }
        if let Some(local_path) = banner_local_path.as_deref() {
            title.banner_source_url = title.banner_url.take();
            title.banner_url = Some(prefix_local_title_image_path(base_path, local_path));
        }
        if let Some(local_path) = background_local_path.as_deref() {
            title.background_source_url = title.background_url.take();
            title.background_url = Some(prefix_local_title_image_path(base_path, local_path));
        }
    }

    Ok(title)
}

pub(crate) async fn list_collections_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<Collection>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE title_id = ? ORDER BY collection_index ASC, id ASC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_collection(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_collections_for_titles_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<Collection>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE title_id IN ({placeholders})
         ORDER BY title_id ASC, collection_index ASC, id ASC"
    );

    let mut query = sqlx::query(&sql);
    for title_id in title_ids {
        query = query.bind(title_id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_collection(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_primary_collection_summaries_query(
    pool: &SqlitePool,
    title_ids: &[String],
) -> AppResult<Vec<PrimaryCollectionSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = title_ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT title_id, collection_type, collection_index, label, ordered_path FROM collections \
         WHERE title_id IN ({placeholders}) AND (collection_index = '0' OR collection_type = 'movie')"
    );

    let mut query = sqlx::query(&sql);
    for id in title_ids {
        query = query.bind(id);
    }

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut candidates = rows
        .into_iter()
        .map(|row| {
            let raw_type: String = row.get("collection_type");
            SummaryCandidate {
                title_id: row.get("title_id"),
                collection_type: CollectionType::parse(&raw_type).unwrap_or_default(),
                collection_index: row.get("collection_index"),
                label: row.get("label"),
                ordered_path: row.get("ordered_path"),
            }
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(summary_candidate_sort_key);

    let mut seen = HashSet::new();
    let mut summaries = Vec::new();
    for candidate in candidates {
        if seen.contains(candidate.title_id.as_str()) {
            continue;
        }
        if !summary_candidate_should_include(&candidate) {
            continue;
        }
        seen.insert(candidate.title_id.clone());
        summaries.push(PrimaryCollectionSummary {
            title_id: candidate.title_id,
            label: candidate.label,
            ordered_path: candidate.ordered_path,
        });
    }

    Ok(summaries)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryCandidate {
    title_id: String,
    collection_type: CollectionType,
    collection_index: String,
    label: Option<String>,
    ordered_path: Option<String>,
}

fn summary_candidate_should_include(candidate: &SummaryCandidate) -> bool {
    if candidate.collection_type == CollectionType::Movie {
        return true;
    }
    candidate.collection_index.trim() == "0"
}

fn summary_candidate_sort_key(candidate: &SummaryCandidate) -> (String, bool, bool, u32, String) {
    (
        candidate.title_id.clone(),
        candidate.collection_type != CollectionType::Movie,
        candidate
            .ordered_path
            .as_deref()
            .is_none_or(|path| path.trim().is_empty()),
        candidate
            .collection_index
            .parse::<u32>()
            .unwrap_or(u32::MAX),
        candidate.collection_index.clone(),
    )
}

pub(crate) async fn get_collection_by_id_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE id = ?",
    )
    .bind(collection_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_collection(&row)?)),
        None => Ok(None),
    }
}

async fn get_collection_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections WHERE id = ?",
    )
    .bind(collection_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_collection(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn get_collection_by_ordered_path_query(
    pool: &SqlitePool,
    ordered_path: &str,
) -> AppResult<Option<Collection>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_type, collection_index, label, ordered_path,
                narrative_order, first_episode_number, last_episode_number,
                interstitial_tvdb_id, interstitial_name, interstitial_slug, interstitial_year,
                interstitial_content_status, interstitial_overview, interstitial_poster_url,
                interstitial_language, interstitial_runtime_minutes, interstitial_sort_title,
                interstitial_imdb_id, interstitial_genres_json, interstitial_studio,
                interstitial_digital_release_date, interstitial_association_confidence,
                interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
                interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
                interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
                special_movies_json, monitored, created_at
         FROM collections
         WHERE ordered_path = ?
         ORDER BY id ASC
         LIMIT 1",
    )
    .bind(ordered_path)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_collection(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn create_collection_query(
    pool: &SqlitePool,
    collection: &Collection,
) -> AppResult<Collection> {
    sqlx::query(
        "INSERT INTO collections
         (id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
          first_episode_number, last_episode_number, interstitial_tvdb_id, interstitial_name,
          interstitial_slug, interstitial_year, interstitial_content_status,
          interstitial_overview, interstitial_poster_url, interstitial_language,
          interstitial_runtime_minutes, interstitial_sort_title, interstitial_imdb_id,
          interstitial_genres_json, interstitial_studio, interstitial_digital_release_date,
          interstitial_association_confidence, interstitial_continuity_status,
          interstitial_movie_form, interstitial_confidence, interstitial_signal_summary,
          interstitial_placement, interstitial_movie_tmdb_id, interstitial_movie_mal_id,
          interstitial_movie_anidb_id, interstitial_season_episode, special_movies_json, monitored, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&collection.id)
    .bind(&collection.title_id)
    .bind(collection.collection_type.as_str())
    .bind(&collection.collection_index)
    .bind(&collection.label)
    .bind(&collection.ordered_path)
    .bind(&collection.narrative_order)
    .bind(&collection.first_episode_number)
    .bind(&collection.last_episode_number)
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.tvdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.slug.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.year),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.content_status.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.overview.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.poster_url.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.language.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.runtime_minutes),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.sort_title.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.imdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| serde_json::to_string(&movie.genres).unwrap_or_else(|_| "[]".to_string())),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.studio.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.digital_release_date.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.association_confidence.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.continuity_status.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_form.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.confidence.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.signal_summary.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.placement.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_tmdb_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_mal_id.clone()),
    )
    .bind(
        collection
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.movie_anidb_id.clone()),
    )
    .bind(&collection.interstitial_season_episode)
    .bind(serde_json::to_string(&collection.specials_movies).unwrap_or_else(|_| "[]".to_string()))
    .bind(if collection.monitored { 1_i64 } else { 0_i64 })
    .bind(collection.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(collection.clone())
}

pub(crate) async fn update_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
    update: CollectionUpdate,
) -> AppResult<Collection> {
    let CollectionUpdate {
        collection_type,
        collection_index,
        label,
        ordered_path,
        clear_ordered_path,
        first_episode_number,
        last_episode_number,
        monitored,
    } = update;

    let mut assignments = Vec::new();
    if collection_type.is_some() {
        assignments.push("collection_type = ?");
    }
    if collection_index.is_some() {
        assignments.push("collection_index = ?");
    }
    if label.is_some() {
        assignments.push("label = ?");
    }
    if clear_ordered_path {
        assignments.push("ordered_path = NULL");
    } else if ordered_path.is_some() {
        assignments.push("ordered_path = ?");
    }
    if first_episode_number.is_some() {
        assignments.push("first_episode_number = ?");
    }
    if last_episode_number.is_some() {
        assignments.push("last_episode_number = ?");
    }
    if monitored.is_some() {
        assignments.push("monitored = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one collection field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE collections SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(collection_type) = collection_type {
        statement = statement.bind(collection_type.as_str().to_string());
    }
    if let Some(collection_index) = collection_index {
        statement = statement.bind(collection_index);
    }
    if let Some(label) = label {
        statement = statement.bind(label);
    }
    if !clear_ordered_path && let Some(ordered_path) = ordered_path {
        statement = statement.bind(ordered_path);
    }
    if let Some(first_episode_number) = first_episode_number {
        statement = statement.bind(first_episode_number);
    }
    if let Some(last_episode_number) = last_episode_number {
        statement = statement.bind(last_episode_number);
    }
    if let Some(monitored) = monitored {
        statement = statement.bind(if monitored { 1_i64 } else { 0_i64 });
    }
    statement = statement.bind(collection_id);

    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let result = statement
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    let collection = get_collection_by_id_tx(&mut tx, collection_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(collection)
}

pub(crate) async fn update_collection_interstitial_movie_query(
    pool: &SqlitePool,
    collection_id: &str,
    interstitial_movie: &InterstitialMovieMetadata,
) -> AppResult<Collection> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let result = sqlx::query(
        "UPDATE collections SET
            interstitial_tvdb_id = ?,
            interstitial_name = ?,
            interstitial_slug = ?,
            interstitial_year = ?,
            interstitial_content_status = ?,
            interstitial_overview = ?,
            interstitial_poster_url = ?,
            interstitial_language = ?,
            interstitial_runtime_minutes = ?,
            interstitial_sort_title = ?,
            interstitial_imdb_id = ?,
            interstitial_genres_json = ?,
            interstitial_studio = ?,
            interstitial_digital_release_date = ?,
            interstitial_association_confidence = ?,
            interstitial_continuity_status = ?,
            interstitial_movie_form = ?,
            interstitial_confidence = ?,
            interstitial_signal_summary = ?,
            interstitial_placement = ?,
            interstitial_movie_tmdb_id = ?,
            interstitial_movie_mal_id = ?,
            interstitial_movie_anidb_id = ?
         WHERE id = ?",
    )
    .bind(&interstitial_movie.tvdb_id)
    .bind(&interstitial_movie.name)
    .bind(&interstitial_movie.slug)
    .bind(interstitial_movie.year)
    .bind(&interstitial_movie.content_status)
    .bind(&interstitial_movie.overview)
    .bind(&interstitial_movie.poster_url)
    .bind(&interstitial_movie.language)
    .bind(interstitial_movie.runtime_minutes)
    .bind(&interstitial_movie.sort_title)
    .bind(&interstitial_movie.imdb_id)
    .bind(serde_json::to_string(&interstitial_movie.genres).unwrap_or_else(|_| "[]".to_string()))
    .bind(&interstitial_movie.studio)
    .bind(&interstitial_movie.digital_release_date)
    .bind(&interstitial_movie.association_confidence)
    .bind(&interstitial_movie.continuity_status)
    .bind(&interstitial_movie.movie_form)
    .bind(&interstitial_movie.confidence)
    .bind(&interstitial_movie.signal_summary)
    .bind(&interstitial_movie.placement)
    .bind(&interstitial_movie.movie_tmdb_id)
    .bind(&interstitial_movie.movie_mal_id)
    .bind(&interstitial_movie.movie_anidb_id)
    .bind(collection_id)
    .execute(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    let collection = get_collection_by_id_tx(&mut tx, collection_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(collection)
}

pub(crate) async fn update_collection_specials_movies_query(
    pool: &SqlitePool,
    collection_id: &str,
    specials_movies: &[InterstitialMovieMetadata],
) -> AppResult<Collection> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let result = sqlx::query("UPDATE collections SET special_movies_json = ? WHERE id = ?")
        .bind(serde_json::to_string(specials_movies).unwrap_or_else(|_| "[]".to_string()))
        .bind(collection_id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    let collection = get_collection_by_id_tx(&mut tx, collection_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(collection)
}

pub(crate) async fn list_episodes_for_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<Vec<Episode>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE collection_id = ? ORDER BY episode_number ASC, id ASC",
    )
    .bind(collection_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_episode(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_episodes_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let rows = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE title_id = ? ORDER BY season_number ASC, episode_number ASC, id ASC",
    )
    .bind(title_id)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_episode(&row)?);
    }
    Ok(out)
}

pub(crate) async fn list_collection_external_ids_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = sqlx::query(
        "SELECT collection_id AS scope_id, source, external_id, provenance, source_scope
         FROM collection_external_ids
         WHERE collection_id = ?
         ORDER BY source ASC, external_id ASC, source_scope ASC",
    )
    .bind(collection_id)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    rows.iter().map(row_to_scoped_external_id).collect()
}

pub(crate) async fn list_episode_external_ids_query(
    pool: &SqlitePool,
    episode_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = sqlx::query(
        "SELECT episode_id AS scope_id, source, external_id, provenance, source_scope
         FROM episode_external_ids
         WHERE episode_id = ?
         ORDER BY source ASC, external_id ASC, source_scope ASC",
    )
    .bind(episode_id)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    rows.iter().map(row_to_scoped_external_id).collect()
}

pub(crate) async fn replace_anibridge_scoped_external_ids_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    episode_ids: &[ScopedExternalId],
) -> AppResult<()> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;

    sqlx::query(
        "DELETE FROM collection_external_ids WHERE title_id = ? AND provenance = 'anibridge'",
    )
    .bind(title_id)
    .execute(&mut *tx)
    .await
    .map_err(repository_error_from_sqlx)?;
    sqlx::query("DELETE FROM episode_external_ids WHERE title_id = ? AND provenance = 'anibridge'")
        .bind(title_id)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;

    let now = chrono::Utc::now().to_rfc3339();
    for scoped_id in collection_ids {
        let Some((collection_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        sqlx::query(
            "INSERT OR IGNORE INTO collection_external_ids
             (id, title_id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'anibridge', ?, ?, ?)",
        )
        .bind(scryer_domain::Id::new().0)
        .bind(title_id)
        .bind(collection_id)
        .bind(source)
        .bind(external_id)
        .bind(source_scope)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }

    for scoped_id in episode_ids {
        let Some((episode_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        sqlx::query(
            "INSERT OR IGNORE INTO episode_external_ids
             (id, title_id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, 'anibridge', ?, ?, ?)",
        )
        .bind(scryer_domain::Id::new().0)
        .bind(title_id)
        .bind(episode_id)
        .bind(source)
        .bind(external_id)
        .bind(source_scope)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }

    tx.commit().await.map_err(repository_error_from_sqlx)
}

pub(crate) async fn list_anime_title_ids_missing_anibridge_scoped_external_ids_query(
    pool: &SqlitePool,
    limit: usize,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT t.id
         FROM titles t
         WHERE t.facet = 'anime'
           AND EXISTS (
               SELECT 1 FROM title_external_ids te
               WHERE te.title_id = t.id AND LOWER(te.source) IN ('tvdb', 'tvdb_id')
           )
           AND NOT EXISTS (
               SELECT 1 FROM collection_external_ids ce
               WHERE ce.title_id = t.id AND ce.provenance = 'anibridge'
           )
           AND NOT EXISTS (
               SELECT 1 FROM episode_external_ids ee
               WHERE ee.title_id = t.id AND ee.provenance = 'anibridge'
           )
         ORDER BY COALESCE(t.metadata_fetched_at, ''), t.created_at
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(
            row.try_get("id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        );
    }
    Ok(ids)
}

pub(crate) async fn list_anime_title_ids_missing_title_anidb_external_ids_query(
    pool: &SqlitePool,
    limit: usize,
) -> AppResult<Vec<String>> {
    let rows = sqlx::query(
        "SELECT t.id
         FROM titles t
         WHERE t.facet = 'anime'
           AND EXISTS (
               SELECT 1 FROM title_external_ids te
               WHERE te.title_id = t.id AND LOWER(te.source) IN ('tvdb', 'tvdb_id')
           )
           AND NOT EXISTS (
               SELECT 1 FROM title_external_ids te
               WHERE te.title_id = t.id AND LOWER(te.source) IN ('anidb', 'anidb_id')
           )
         ORDER BY COALESCE(t.metadata_fetched_at, ''), t.created_at
         LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        ids.push(
            row.try_get("id")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        );
    }
    Ok(ids)
}

pub(crate) async fn get_episode_by_id_query(
    pool: &SqlitePool,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE id = ?",
    )
    .bind(episode_id)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

async fn get_episode_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT id, title_id, collection_id, episode_type, episode_number, season_number,
                episode_label, title, air_date, duration_seconds, has_multi_audio,
                has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at
         FROM episodes WHERE id = ?",
    )
    .bind(episode_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn update_episode_query(
    pool: &SqlitePool,
    episode_id: &str,
    update: EpisodeUpdate,
) -> AppResult<Episode> {
    let EpisodeUpdate {
        episode_type,
        episode_number,
        season_number,
        episode_label,
        title,
        air_date,
        duration_seconds,
        has_multi_audio,
        has_subtitle,
        monitored,
        collection_id,
        overview,
        tvdb_id,
    } = update;

    let mut assignments = Vec::new();
    if episode_type.is_some() {
        assignments.push("episode_type = ?");
    }
    if episode_number.is_some() {
        assignments.push("episode_number = ?");
    }
    if season_number.is_some() {
        assignments.push("season_number = ?");
    }
    if episode_label.is_some() {
        assignments.push("episode_label = ?");
    }
    if title.is_some() {
        assignments.push("title = ?");
    }
    if air_date.is_some() {
        assignments.push("air_date = ?");
    }
    if duration_seconds.is_some() {
        assignments.push("duration_seconds = ?");
    }
    if has_multi_audio.is_some() {
        assignments.push("has_multi_audio = ?");
    }
    if has_subtitle.is_some() {
        assignments.push("has_subtitle = ?");
    }
    if monitored.is_some() {
        assignments.push("monitored = ?");
    }
    if collection_id.is_some() {
        assignments.push("collection_id = ?");
    }
    if overview.is_some() {
        assignments.push("overview = ?");
    }
    if tvdb_id.is_some() {
        assignments.push("tvdb_id = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one episode field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE episodes SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(episode_type) = episode_type {
        statement = statement.bind(episode_type.as_str());
    }
    if let Some(episode_number) = episode_number {
        statement = statement.bind(episode_number);
    }
    if let Some(season_number) = season_number {
        statement = statement.bind(season_number);
    }
    if let Some(episode_label) = episode_label {
        statement = statement.bind(episode_label);
    }
    if let Some(title) = title {
        statement = statement.bind(title);
    }
    if let Some(air_date) = air_date {
        statement = statement.bind(air_date);
    }
    if let Some(duration_seconds) = duration_seconds {
        statement = statement.bind(duration_seconds);
    }
    if let Some(has_multi_audio) = has_multi_audio {
        statement = statement.bind(if has_multi_audio { 1_i64 } else { 0_i64 });
    }
    if let Some(has_subtitle) = has_subtitle {
        statement = statement.bind(if has_subtitle { 1_i64 } else { 0_i64 });
    }
    if let Some(monitored) = monitored {
        statement = statement.bind(if monitored { 1_i64 } else { 0_i64 });
    }
    if let Some(collection_id) = collection_id {
        statement = statement.bind(collection_id);
    }
    if let Some(overview) = overview {
        statement = statement.bind(overview);
    }
    if let Some(tvdb_id) = tvdb_id {
        statement = statement.bind(tvdb_id);
    }
    statement = statement.bind(episode_id);

    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let result = statement
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("episode {}", episode_id)));
    }

    let episode = get_episode_by_id_tx(&mut tx, episode_id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(episode)
}

pub(crate) async fn create_episode_query(
    pool: &SqlitePool,
    episode: &Episode,
) -> AppResult<Episode> {
    sqlx::query(
        "INSERT INTO episodes
         (id, title_id, collection_id, episode_type, episode_number, season_number,
          episode_label, title, air_date, duration_seconds, has_multi_audio,
          has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&episode.id)
    .bind(&episode.title_id)
    .bind(&episode.collection_id)
    .bind(episode.episode_type.as_str())
    .bind(&episode.episode_number)
    .bind(&episode.season_number)
    .bind(&episode.episode_label)
    .bind(&episode.title)
    .bind(&episode.air_date)
    .bind(episode.duration_seconds)
    .bind(if episode.has_multi_audio {
        1_i64
    } else {
        0_i64
    })
    .bind(if episode.has_subtitle { 1_i64 } else { 0_i64 })
    .bind(if episode.is_filler { 1_i64 } else { 0_i64 })
    .bind(if episode.is_recap { 1_i64 } else { 0_i64 })
    .bind(&episode.absolute_number)
    .bind(&episode.overview)
    .bind(&episode.tvdb_id)
    .bind(if episode.monitored { 1_i64 } else { 0_i64 })
    .bind(episode.created_at.to_rfc3339())
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(episode.clone())
}

pub(crate) async fn delete_collection_query(
    pool: &SqlitePool,
    collection_id: &str,
) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM collections WHERE id = ?")
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("collection {}", collection_id)));
    }

    Ok(())
}

pub(crate) async fn delete_collections_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM collections WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn delete_episode_query(pool: &SqlitePool, episode_id: &str) -> AppResult<()> {
    let result = sqlx::query("DELETE FROM episodes WHERE id = ?")
        .bind(episode_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("episode {}", episode_id)));
    }

    Ok(())
}

pub(crate) async fn delete_episodes_for_title_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<()> {
    sqlx::query("DELETE FROM episodes WHERE title_id = ?")
        .bind(title_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(())
}

pub(crate) async fn find_episode_by_title_and_numbers_query(
    pool: &SqlitePool,
    title_id: &str,
    season_number: &str,
    episode_number: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number,
                e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds,
                e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number,
                e.overview, e.tvdb_id, e.monitored, e.created_at
         FROM episodes e
         INNER JOIN collections c ON c.id = e.collection_id
         WHERE e.title_id = ?
           AND c.collection_index = ?
           AND e.episode_number = ?
         LIMIT 1",
    )
    .bind(title_id)
    .bind(season_number)
    .bind(episode_number)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn find_episode_by_title_and_absolute_number_query(
    pool: &SqlitePool,
    title_id: &str,
    absolute_number: &str,
) -> AppResult<Option<Episode>> {
    let row = sqlx::query(
        "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number,
                e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds,
                e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number,
                e.overview, e.tvdb_id, e.monitored, e.created_at
         FROM episodes e
         WHERE e.title_id = ?
           AND e.absolute_number = ?
         LIMIT 1",
    )
    .bind(title_id)
    .bind(absolute_number)
    .fetch_optional(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    match row {
        Some(row) => Ok(Some(row_to_episode(&row)?)),
        None => Ok(None),
    }
}

pub(crate) async fn list_episodes_in_date_range_query(
    pool: &SqlitePool,
    start_date: &str,
    end_date: &str,
) -> AppResult<Vec<CalendarEpisode>> {
    let rows = sqlx::query(
        "SELECT e.id, e.title_id, t.library_id, l.name AS library_name, l.slug AS library_slug,
                t.name AS title_name, t.slug AS title_slug, t.facet AS title_facet,
                e.season_number, e.episode_number, e.title AS episode_title,
                e.air_date, e.monitored
         FROM episodes e
         JOIN titles t ON e.title_id = t.id
         LEFT JOIN libraries l ON l.id = t.library_id
         WHERE e.air_date IS NOT NULL AND e.air_date != ''
           AND e.air_date >= ? AND e.air_date <= ?
         ORDER BY e.air_date ASC",
    )
    .bind(start_date)
    .bind(end_date)
    .fetch_all(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(CalendarEpisode {
            id: row.get("id"),
            title_id: row.get("title_id"),
            library_id: row.get("library_id"),
            library_name: row.get("library_name"),
            library_slug: row.get("library_slug"),
            title_name: row.get("title_name"),
            title_slug: row.get("title_slug"),
            title_facet: row.get("title_facet"),
            season_number: row.get("season_number"),
            episode_number: row.get("episode_number"),
            episode_title: row.get("episode_title"),
            air_date: row.get("air_date"),
            monitored: row.get::<i64, _>("monitored") != 0,
        });
    }
    Ok(out)
}

pub(crate) async fn update_interstitial_season_episode_query(
    pool: &SqlitePool,
    collection_id: &str,
    season_episode: Option<&str>,
) -> AppResult<()> {
    sqlx::query("UPDATE collections SET interstitial_season_episode = ? WHERE id = ?")
        .bind(season_episode)
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn set_collection_episodes_monitored_query(
    pool: &SqlitePool,
    collection_id: &str,
    monitored: bool,
) -> AppResult<()> {
    sqlx::query("UPDATE episodes SET monitored = ? WHERE collection_id = ?")
        .bind(if monitored { 1_i64 } else { 0_i64 })
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

fn row_to_collection(row: &sqlx::sqlite::SqliteRow) -> AppResult<Collection> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_type_raw: String = row
        .try_get::<String, _>("collection_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_type = CollectionType::parse(&collection_type_raw).unwrap_or_default();
    let collection_index: String = row
        .try_get("collection_index")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let label: Option<String> = row
        .try_get("label")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let ordered_path: Option<String> = row
        .try_get("ordered_path")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let narrative_order: Option<String> = row.try_get("narrative_order").unwrap_or(None);
    let first_episode_number = optional_text_from_column(row, "first_episode_number")?;
    let last_episode_number = optional_text_from_column(row, "last_episode_number")?;
    let interstitial_movie = row_to_interstitial_movie(row)?;
    let specials_movies = row_to_specials_movies(row)?;
    let interstitial_season_episode: Option<String> =
        row.try_get("interstitial_season_episode").unwrap_or(None);
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(Collection {
        id,
        title_id,
        collection_type,
        collection_index,
        label,
        ordered_path,
        narrative_order,
        first_episode_number,
        last_episode_number,
        interstitial_movie,
        specials_movies,
        interstitial_season_episode,
        monitored: monitored != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
    })
}

fn row_to_interstitial_movie(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<Option<InterstitialMovieMetadata>> {
    let Some(tvdb_id) = row
        .try_get::<Option<String>, _>("interstitial_tvdb_id")
        .unwrap_or(None)
    else {
        return Ok(None);
    };

    let genres_json = row
        .try_get::<Option<String>, _>("interstitial_genres_json")
        .unwrap_or(None);
    let genres = genres_json
        .as_deref()
        .map(serde_json::from_str::<Vec<String>>)
        .transpose()
        .map_err(|err| AppError::Repository(err.to_string()))?
        .unwrap_or_default();

    Ok(Some(InterstitialMovieMetadata {
        tvdb_id,
        name: row.try_get("interstitial_name").unwrap_or_default(),
        slug: row.try_get("interstitial_slug").unwrap_or_default(),
        year: row.try_get("interstitial_year").unwrap_or(None),
        content_status: row
            .try_get("interstitial_content_status")
            .unwrap_or_default(),
        overview: row.try_get("interstitial_overview").unwrap_or_default(),
        poster_url: row.try_get("interstitial_poster_url").unwrap_or_default(),
        language: row.try_get("interstitial_language").unwrap_or_default(),
        runtime_minutes: row
            .try_get("interstitial_runtime_minutes")
            .unwrap_or_default(),
        sort_title: row.try_get("interstitial_sort_title").unwrap_or_default(),
        imdb_id: row.try_get("interstitial_imdb_id").unwrap_or_default(),
        genres,
        studio: row.try_get("interstitial_studio").unwrap_or_default(),
        digital_release_date: row
            .try_get("interstitial_digital_release_date")
            .unwrap_or(None),
        association_confidence: row
            .try_get("interstitial_association_confidence")
            .unwrap_or(None),
        continuity_status: row
            .try_get("interstitial_continuity_status")
            .unwrap_or(None),
        movie_form: row.try_get("interstitial_movie_form").unwrap_or(None),
        confidence: row.try_get("interstitial_confidence").unwrap_or(None),
        signal_summary: row.try_get("interstitial_signal_summary").unwrap_or(None),
        placement: row.try_get("interstitial_placement").unwrap_or(None),
        movie_tmdb_id: row.try_get("interstitial_movie_tmdb_id").unwrap_or(None),
        movie_mal_id: row.try_get("interstitial_movie_mal_id").unwrap_or(None),
        movie_anidb_id: row.try_get("interstitial_movie_anidb_id").unwrap_or(None),
    }))
}

fn row_to_specials_movies(
    row: &sqlx::sqlite::SqliteRow,
) -> AppResult<Vec<InterstitialMovieMetadata>> {
    let raw = row
        .try_get::<Option<String>, _>("special_movies_json")
        .unwrap_or(None)
        .unwrap_or_else(|| "[]".to_string());
    serde_json::from_str(&raw).map_err(|err| AppError::Repository(err.to_string()))
}

fn optional_text_from_column(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> AppResult<Option<String>> {
    match row.try_get::<Option<String>, _>(column) {
        Ok(value) => Ok(value),
        Err(string_err) => match row.try_get::<Option<i64>, _>(column) {
            Ok(value) => Ok(value.map(|value| value.to_string())),
            Err(integer_err) => Err(AppError::Repository(format!(
                "failed decode {column} as optional text: {string_err}; {integer_err}"
            ))),
        },
    }
}

fn row_to_episode(row: &sqlx::sqlite::SqliteRow) -> AppResult<Episode> {
    let id: String = row
        .try_get("id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title_id: String = row
        .try_get("title_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let collection_id: Option<String> = row
        .try_get("collection_id")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_type_raw: String = row
        .try_get::<String, _>("episode_type")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_type = scryer_domain::EpisodeType::parse(&episode_type_raw).unwrap_or_default();
    let episode_number: Option<String> = row
        .try_get("episode_number")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let season_number: Option<String> = row
        .try_get("season_number")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let episode_label: Option<String> = row
        .try_get("episode_label")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let title: Option<String> = row
        .try_get("title")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let air_date: Option<String> = row
        .try_get("air_date")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let duration_seconds: Option<i64> = row
        .try_get("duration_seconds")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let has_multi_audio: i64 = row
        .try_get("has_multi_audio")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let has_subtitle: i64 = row
        .try_get("has_subtitle")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let is_filler: i64 = row.try_get("is_filler").unwrap_or(0);
    let is_recap: i64 = row.try_get("is_recap").unwrap_or(0);
    let absolute_number: Option<String> = row.try_get("absolute_number").unwrap_or(None);
    let overview: Option<String> = row.try_get("overview").unwrap_or(None);
    let tvdb_id: Option<String> = row.try_get("tvdb_id").unwrap_or(None);
    let monitored: i64 = row
        .try_get("monitored")
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let created_at_raw: String = row
        .try_get("created_at")
        .map_err(|err| AppError::Repository(err.to_string()))?;

    Ok(Episode {
        id,
        title_id,
        collection_id,
        episode_type,
        episode_number,
        season_number,
        episode_label,
        title,
        air_date,
        duration_seconds,
        has_multi_audio: has_multi_audio != 0,
        has_subtitle: has_subtitle != 0,
        is_filler: is_filler != 0,
        is_recap: is_recap != 0,
        absolute_number,
        overview,
        tvdb_id,
        monitored: monitored != 0,
        created_at: parse_utc_datetime(&created_at_raw)?,
    })
}

async fn insert_title_row_tx(tx: &mut Transaction<'_, Sqlite>, title: &Title) -> AppResult<()> {
    let tags_json =
        serde_json::to_string(&title.tags).map_err(|err| AppError::Repository(err.to_string()))?;
    let ext_json = serde_json::to_string(&title.external_ids)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let genres_json = serde_json::to_string(&title.genres)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases_json = serde_json::to_string(&title.aliases)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let tagged_aliases_json = serde_json::to_string(&title.tagged_aliases)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let metadata_fetched_at = title.metadata_fetched_at.map(|value| value.to_rfc3339());
    let metadata_hydration_next_attempt_at = if title.metadata_fetched_at.is_none()
        && normalized_external_ids(&title.external_ids)
            .iter()
            .any(|(source, _)| source == "tvdb")
    {
        Some(chrono::Utc::now().to_rfc3339())
    } else {
        None
    };

    sqlx::query(
        "INSERT INTO titles (
            id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at,
            year, overview, poster_url, banner_url, background_url, sort_title, slug, imdb_id,
            runtime_minutes, genres, content_status, language, first_aired, network, studio,
            country, aliases, metadata_language, metadata_fetched_at, min_availability,
            digital_release_date, folder_path, tagged_aliases_json,
            metadata_hydration_next_attempt_at, metadata_hydration_attempt_count
         )
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&title.id)
    .bind(&title.library_id)
    .bind(&title.name)
    .bind(title.facet.as_str())
    .bind(if title.monitored { 1_i64 } else { 0_i64 })
    .bind(&tags_json)
    .bind(&ext_json)
    .bind(&title.created_by)
    .bind(title.created_at.to_rfc3339())
    .bind(title.year)
    .bind(&title.overview)
    .bind(&title.poster_url)
    .bind(&title.banner_url)
    .bind(&title.background_url)
    .bind(&title.sort_title)
    .bind(&title.slug)
    .bind(&title.imdb_id)
    .bind(title.runtime_minutes)
    .bind(&genres_json)
    .bind(&title.content_status)
    .bind(&title.language)
    .bind(&title.first_aired)
    .bind(&title.network)
    .bind(&title.studio)
    .bind(&title.country)
    .bind(&aliases_json)
    .bind(&title.metadata_language)
    .bind(&metadata_fetched_at)
    .bind(&title.min_availability)
    .bind(&title.digital_release_date)
    .bind(&title.folder_path)
    .bind(&tagged_aliases_json)
    .bind(&metadata_hydration_next_attempt_at)
    .bind(0_i64)
    .execute(&mut **tx)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(())
}

pub(crate) async fn create_or_get_existing_title_query(
    pool: &SqlitePool,
    title: &Title,
) -> AppResult<CreateTitleOutcome> {
    let external_ids = normalized_external_ids(&title.external_ids);
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;

    let existing_ids =
        list_existing_title_ids_for_external_ids_tx(&mut tx, &title.library_id, &external_ids)
            .await?;
    if existing_ids.len() > 1 {
        return Err(AppError::Validation(
            "external ids already map to multiple titles".to_string(),
        ));
    }
    if let Some(existing_id) = existing_ids.first() {
        let existing = get_title_by_id_tx(&mut tx, existing_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {}", existing_id)))?;
        tx.rollback().await.map_err(repository_error_from_sqlx)?;
        return Ok(CreateTitleOutcome {
            title: existing,
            reused_existing: true,
        });
    }

    insert_title_row_tx(&mut tx, title).await?;
    super::title_search::replace_title_search_projection_tx(&mut tx, title).await?;

    match replace_title_external_ids_projection_tx(
        &mut tx,
        &title.id,
        &title.library_id,
        title.facet.clone(),
        &title.external_ids,
    )
    .await
    {
        Ok(()) => {
            tx.commit().await.map_err(repository_error_from_sqlx)?;
            Ok(CreateTitleOutcome {
                title: title.clone(),
                reused_existing: false,
            })
        }
        Err(error) => {
            let _ = tx.rollback().await;
            if matches!(&error, AppError::Repository(message) if message.contains("UNIQUE constraint failed"))
            {
                let mut lookup_tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
                let conflict_ids = list_existing_title_ids_for_external_ids_tx(
                    &mut lookup_tx,
                    &title.library_id,
                    &external_ids,
                )
                .await?;
                if conflict_ids.len() == 1 {
                    let existing = get_title_by_id_tx(&mut lookup_tx, &conflict_ids[0])
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("title {}", conflict_ids[0])))?;
                    lookup_tx
                        .rollback()
                        .await
                        .map_err(repository_error_from_sqlx)?;
                    return Ok(CreateTitleOutcome {
                        title: existing,
                        reused_existing: true,
                    });
                }
                if conflict_ids.len() > 1 {
                    lookup_tx
                        .rollback()
                        .await
                        .map_err(repository_error_from_sqlx)?;
                    return Err(AppError::Validation(
                        "external ids already map to multiple titles".to_string(),
                    ));
                }
            }
            Err(error)
        }
    }
}

pub(crate) async fn create_title_query(pool: &SqlitePool, title: &Title) -> AppResult<Title> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    insert_title_row_tx(&mut tx, title).await?;
    super::title_search::replace_title_search_projection_tx(&mut tx, title).await?;
    replace_title_external_ids_projection_tx(
        &mut tx,
        &title.id,
        &title.library_id,
        title.facet.clone(),
        &title.external_ids,
    )
    .await?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(title.clone())
}

pub(crate) async fn list_titles_due_for_hydration_query(
    pool: &SqlitePool,
    limit: usize,
    excluded_facets: &[MediaFacet],
) -> AppResult<Vec<PendingTitleHydration>> {
    let facet_filter = if excluded_facets.is_empty() {
        String::new()
    } else {
        format!(
            " AND facet NOT IN ({})",
            std::iter::repeat_n("?", excluded_facets.len())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let sql = format!(
        "SELECT {columns}, metadata_hydration_attempt_count
         FROM titles
         WHERE metadata_fetched_at IS NULL
           AND metadata_hydration_next_attempt_at IS NOT NULL
           AND metadata_hydration_next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
           {facet_filter}
         ORDER BY metadata_hydration_next_attempt_at ASC, id ASC
         LIMIT ?",
        columns = TITLE_COLUMNS,
        facet_filter = facet_filter,
    );
    let mut query = sqlx::query(&sql);
    for facet in excluded_facets {
        query = query.bind(facet.as_str());
    }
    let rows = query
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let base_path = normalized_base_path_from_env();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(PendingTitleHydration {
            title: row_to_title(&row, TitleReadMode::Presentation, &base_path)?,
            attempt_count: row
                .try_get("metadata_hydration_attempt_count")
                .map_err(|err| AppError::Repository(err.to_string()))?,
        });
    }
    Ok(out)
}

pub(crate) async fn mark_title_metadata_hydration_due_now_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             metadata_hydration_attempt_count = 0
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn schedule_title_metadata_hydration_retry_query(
    pool: &SqlitePool,
    id: &str,
    next_attempt_at: &str,
    attempt_count: i64,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = ?,
             metadata_hydration_attempt_count = ?
         WHERE id = ?",
    )
    .bind(next_attempt_at)
    .bind(attempt_count)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn clear_title_metadata_hydration_retry_state_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<()> {
    sqlx::query(
        "UPDATE titles
         SET metadata_hydration_next_attempt_at = NULL,
             metadata_hydration_attempt_count = 0
         WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn update_title_monitored_query(
    pool: &SqlitePool,
    id: &str,
    monitored: bool,
) -> AppResult<Title> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let result = sqlx::query("UPDATE titles SET monitored = ? WHERE id = ?")
        .bind(if monitored { 1_i64 } else { 0_i64 })
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    let title = get_title_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(title)
}

pub(crate) async fn set_title_folder_path_query(
    pool: &SqlitePool,
    id: &str,
    folder_path: &str,
) -> AppResult<()> {
    sqlx::query("UPDATE titles SET folder_path = ? WHERE id = ?")
        .bind(folder_path)
        .bind(id)
        .execute(pool)
        .await
        .map_err(repository_error_from_sqlx)?;
    Ok(())
}

pub(crate) async fn clear_title_folder_path_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("UPDATE titles SET folder_path = NULL WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(())
}

pub(crate) async fn update_title_metadata_query(
    pool: &SqlitePool,
    id: &str,
    name: Option<String>,
    facet: Option<MediaFacet>,
    tags_json: Option<String>,
) -> AppResult<Title> {
    let mut assignments = Vec::new();
    if name.is_some() {
        assignments.push("name = ?");
    }
    if facet.is_some() {
        assignments.push("facet = ?");
    }
    if tags_json.is_some() {
        assignments.push("tags = ?");
    }

    if assignments.is_empty() {
        return Err(AppError::Validation(
            "at least one title field must be provided".into(),
        ));
    }

    let mut sql = String::from("UPDATE titles SET ");
    sql.push_str(&assignments.join(", "));
    sql.push_str(" WHERE id = ?");

    let mut statement = sqlx::query(&sql);
    if let Some(name) = name {
        let normalized = name.trim();
        if normalized.is_empty() {
            return Err(AppError::Validation("title name cannot be empty".into()));
        }
        statement = statement.bind(normalized.to_string());
    }
    if let Some(facet) = facet {
        statement = statement.bind(facet.as_str());
    }
    if let Some(tags_json) = tags_json {
        statement = statement.bind(tags_json);
    }
    statement = statement.bind(id);

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let result = statement
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    let title = get_title_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
    super::title_search::replace_title_search_projection_tx(&mut tx, &title).await?;
    replace_title_external_ids_projection_tx(
        &mut tx,
        &title.id,
        &title.library_id,
        title.facet.clone(),
        &title.external_ids,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(title)
}

pub(crate) async fn replace_title_match_state_query(
    pool: &SqlitePool,
    id: &str,
    external_ids: Vec<ExternalId>,
    tags: Vec<String>,
) -> AppResult<Title> {
    let external_ids_json = serde_json::to_string(&external_ids)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let tags_json =
        serde_json::to_string(&tags).map_err(|err| AppError::Repository(err.to_string()))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let result = sqlx::query(
        "UPDATE titles SET
            external_ids = ?,
            tags = ?,
            year = NULL,
            overview = NULL,
            poster_url = NULL,
            banner_url = NULL,
            background_url = NULL,
            sort_title = NULL,
            slug = NULL,
            imdb_id = NULL,
            runtime_minutes = NULL,
            genres = '[]',
            content_status = NULL,
            language = NULL,
            first_aired = NULL,
            network = NULL,
            studio = NULL,
            country = NULL,
            aliases = '[]',
            tagged_aliases_json = '[]',
            metadata_language = NULL,
            metadata_fetched_at = NULL,
            metadata_hydration_next_attempt_at = CASE
                WHEN EXISTS (
                    SELECT 1
                    FROM json_each(?) AS external_id
                    WHERE LOWER(TRIM(COALESCE(json_extract(external_id.value, '$.source'), ''))) = 'tvdb'
                      AND TRIM(COALESCE(json_extract(external_id.value, '$.value'), '')) != ''
                )
                THEN strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
                ELSE NULL
            END,
            metadata_hydration_attempt_count = 0,
            digital_release_date = NULL
         WHERE id = ?",
    )
    .bind(&external_ids_json)
    .bind(&tags_json)
    .bind(&external_ids_json)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    let title = get_title_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
    super::title_search::replace_title_search_projection_tx(&mut tx, &title).await?;
    replace_title_external_ids_projection_tx(
        &mut tx,
        &title.id,
        &title.library_id,
        title.facet.clone(),
        &title.external_ids,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(title)
}

pub(crate) async fn update_title_hydrated_metadata_query(
    pool: &SqlitePool,
    id: &str,
    metadata: TitleMetadataUpdate,
) -> AppResult<Title> {
    let genres_json = serde_json::to_string(&metadata.genres)
        .map_err(|err| AppError::Repository(err.to_string()))?;
    let aliases_json = serde_json::to_string(&metadata.aliases)
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    let result = sqlx::query(
        "UPDATE titles SET
            name = COALESCE(NULLIF(?, ''), name),
            year = COALESCE(?, year),
            overview = COALESCE(NULLIF(?, ''), overview),
            poster_url = COALESCE(NULLIF(?, ''), poster_url),
            banner_url = COALESCE(NULLIF(?, ''), banner_url),
            background_url = COALESCE(NULLIF(?, ''), background_url),
            sort_title = COALESCE(NULLIF(?, ''), sort_title),
            slug = COALESCE(NULLIF(?, ''), slug),
            imdb_id = COALESCE(NULLIF(?, ''), imdb_id),
            runtime_minutes = COALESCE(?, runtime_minutes),
            genres = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE genres END,
            content_status = COALESCE(NULLIF(?, ''), content_status),
            language = COALESCE(NULLIF(?, ''), language),
            first_aired = COALESCE(NULLIF(?, ''), first_aired),
            network = COALESCE(NULLIF(?, ''), network),
            studio = COALESCE(NULLIF(?, ''), studio),
            country = COALESCE(NULLIF(?, ''), country),
            aliases = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE aliases END,
            tagged_aliases_json = CASE WHEN NULLIF(?, '[]') IS NOT NULL THEN ? ELSE tagged_aliases_json END,
            metadata_language = COALESCE(NULLIF(?, ''), metadata_language),
            metadata_fetched_at = COALESCE(NULLIF(?, ''), metadata_fetched_at),
            metadata_hydration_next_attempt_at = CASE
                WHEN NULLIF(?, '') IS NOT NULL THEN NULL
                ELSE metadata_hydration_next_attempt_at
            END,
            metadata_hydration_attempt_count = CASE
                WHEN NULLIF(?, '') IS NOT NULL THEN 0
                ELSE metadata_hydration_attempt_count
            END,
            digital_release_date = COALESCE(NULLIF(?, ''), digital_release_date)
         WHERE id = ?",
    )
    .bind(&metadata.name)
    .bind(metadata.year)
    .bind(&metadata.overview)
    .bind(&metadata.poster_url)
    .bind(&metadata.banner_url)
    .bind(&metadata.background_url)
    .bind(&metadata.sort_title)
    .bind(&metadata.slug)
    .bind(&metadata.imdb_id)
    .bind(metadata.runtime_minutes)
    .bind(&genres_json)
    .bind(&genres_json)
    .bind(&metadata.content_status)
    .bind(&metadata.language)
    .bind(&metadata.first_aired)
    .bind(&metadata.network)
    .bind(&metadata.studio)
    .bind(&metadata.country)
    .bind(&aliases_json)
    .bind(&aliases_json)
    .bind(serde_json::to_string(&metadata.tagged_aliases).unwrap_or_else(|_| "[]".to_string()))
    .bind(serde_json::to_string(&metadata.tagged_aliases).unwrap_or_else(|_| "[]".to_string()))
    .bind(&metadata.metadata_language)
    .bind(&metadata.metadata_fetched_at)
    .bind(&metadata.metadata_fetched_at)
    .bind(&metadata.metadata_fetched_at)
    .bind(&metadata.digital_release_date)
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    // Merge extra external IDs (e.g. anime mappings) into the title's external_ids JSON
    if !metadata.extra_external_ids.is_empty() {
        let existing_json: String =
            sqlx::query_scalar("SELECT external_ids FROM titles WHERE id = ?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|err| AppError::Repository(err.to_string()))?;

        let mut existing: Vec<ExternalId> =
            serde_json::from_str(&existing_json).unwrap_or_default();

        for eid in &metadata.extra_external_ids {
            // Replace any existing entry with the same source so that
            // re-hydration converges to a single ID per source (e.g. one
            // "mal" entry instead of one per anime season).
            existing.retain(|e| e.source != eid.source);
            existing.push(eid.clone());
        }

        let merged_json = serde_json::to_string(&existing)
            .map_err(|err| AppError::Repository(err.to_string()))?;

        sqlx::query("UPDATE titles SET external_ids = ? WHERE id = ?")
            .bind(&merged_json)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    // Merge extra tags (e.g. anime metadata) into the title's tags JSON
    if !metadata.extra_tags.is_empty() {
        let existing_json: String = sqlx::query_scalar("SELECT tags FROM titles WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;

        let mut existing: Vec<String> = serde_json::from_str(&existing_json).unwrap_or_default();

        for tag in &metadata.extra_tags {
            if let Some(colon_pos) = tag.rfind(':') {
                let prefix = &tag[..=colon_pos];
                existing.retain(|t| !t.starts_with(prefix));
            }
            existing.push(tag.clone());
        }

        let merged_json = serde_json::to_string(&existing)
            .map_err(|err| AppError::Repository(err.to_string()))?;

        sqlx::query("UPDATE titles SET tags = ? WHERE id = ?")
            .bind(&merged_json)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
    }

    let title = get_title_by_id_tx(&mut tx, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
    super::title_search::replace_title_search_projection_tx(&mut tx, &title).await?;
    replace_title_external_ids_projection_tx(
        &mut tx,
        &title.id,
        &title.library_id,
        title.facet.clone(),
        &title.external_ids,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;
    Ok(title)
}

pub(crate) async fn delete_title_query(pool: &SqlitePool, id: &str) -> AppResult<()> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    super::title_search::delete_title_search_projection_tx(&mut tx, id).await?;

    let result = sqlx::query("DELETE FROM titles WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|err| AppError::Repository(err.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("title {}", id)));
    }

    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{SummaryCandidate, summary_candidate_sort_key};
    use scryer_domain::CollectionType;

    #[test]
    fn movie_collection_wins_over_index_zero_fallback() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Season,
                collection_index: "0".to_string(),
                label: Some("Specials".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/data/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_type, CollectionType::Movie);
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }

    #[test]
    fn movie_collection_with_path_wins_over_pathless_movie_collection() {
        let mut candidates = [
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "2".to_string(),
                label: Some("2160P".to_string()),
                ordered_path: None,
            },
            SummaryCandidate {
                title_id: "title-1".to_string(),
                collection_type: CollectionType::Movie,
                collection_index: "1".to_string(),
                label: Some("1080P".to_string()),
                ordered_path: Some("/data/movies/Movie/Movie.1080P.mkv".to_string()),
            },
        ];
        candidates.sort_by_key(summary_candidate_sort_key);

        assert_eq!(candidates[0].collection_index, "1");
        assert_eq!(candidates[0].label.as_deref(), Some("1080P"));
    }
}
