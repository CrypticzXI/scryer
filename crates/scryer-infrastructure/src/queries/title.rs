use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeUpdate, PrimaryCollectionSummary,
};
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, Episode, InterstitialMovieMetadata,
};
use serde_json;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

use super::common::{parse_utc_datetime, repository_error_from_sqlx};

pub(crate) const TITLE_COLUMNS: &str = "id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at, \
    year, overview, poster_url, poster_local_path, banner_url, banner_local_path, background_url, background_local_path, \
    sort_title, slug, imdb_id, runtime_minutes, genres, \
    content_status, language, first_aired, network, studio, country, aliases, \
    metadata_language, metadata_fetched_at, min_availability, digital_release_date, folder_path, tagged_aliases_json";

pub(crate) struct CollectionInterstitialColumnValues {
    pub(crate) tvdb_id: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) slug: Option<String>,
    pub(crate) year: Option<i32>,
    pub(crate) content_status: Option<String>,
    pub(crate) overview: Option<String>,
    pub(crate) poster_url: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) sort_title: Option<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) genres_json: Option<serde_json::Value>,
    pub(crate) studio: Option<String>,
    pub(crate) digital_release_date: Option<String>,
    pub(crate) association_confidence: Option<String>,
    pub(crate) continuity_status: Option<String>,
    pub(crate) movie_form: Option<String>,
    pub(crate) confidence: Option<String>,
    pub(crate) signal_summary: Option<String>,
    pub(crate) placement: Option<String>,
    pub(crate) movie_tmdb_id: Option<String>,
    pub(crate) movie_mal_id: Option<String>,
    pub(crate) movie_anidb_id: Option<String>,
    pub(crate) special_movies_json: serde_json::Value,
}

pub(crate) fn collection_interstitial_column_values(
    collection: &Collection,
) -> AppResult<CollectionInterstitialColumnValues> {
    let movie = collection.interstitial_movie.as_ref();
    Ok(CollectionInterstitialColumnValues {
        tvdb_id: movie.map(|value| value.tvdb_id.clone()),
        name: movie.map(|value| value.name.clone()),
        slug: movie.map(|value| value.slug.clone()),
        year: movie.and_then(|value| value.year),
        content_status: movie.map(|value| value.content_status.clone()),
        overview: movie.map(|value| value.overview.clone()),
        poster_url: movie.map(|value| value.poster_url.clone()),
        language: movie.map(|value| value.language.clone()),
        runtime_minutes: movie.map(|value| value.runtime_minutes),
        sort_title: movie.map(|value| value.sort_title.clone()),
        imdb_id: movie.map(|value| value.imdb_id.clone()),
        genres_json: movie
            .map(|value| serde_json::to_value(&value.genres))
            .transpose()
            .map_err(|err| AppError::Repository(err.to_string()))?,
        studio: movie.map(|value| value.studio.clone()),
        digital_release_date: movie.and_then(|value| value.digital_release_date.clone()),
        association_confidence: movie.and_then(|value| value.association_confidence.clone()),
        continuity_status: movie.and_then(|value| value.continuity_status.clone()),
        movie_form: movie.and_then(|value| value.movie_form.clone()),
        confidence: movie.and_then(|value| value.confidence.clone()),
        signal_summary: movie.and_then(|value| value.signal_summary.clone()),
        placement: movie.and_then(|value| value.placement.clone()),
        movie_tmdb_id: movie.and_then(|value| value.movie_tmdb_id.clone()),
        movie_mal_id: movie.and_then(|value| value.movie_mal_id.clone()),
        movie_anidb_id: movie.and_then(|value| value.movie_anidb_id.clone()),
        special_movies_json: serde_json::to_value(&collection.specials_movies)
            .map_err(|err| AppError::Repository(err.to_string()))?,
    })
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

pub(crate) async fn create_collection_query(
    pool: &SqlitePool,
    collection: &Collection,
) -> AppResult<Collection> {
    let interstitial = collection_interstitial_column_values(collection)?;
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
    .bind(interstitial.tvdb_id)
    .bind(interstitial.name)
    .bind(interstitial.slug)
    .bind(interstitial.year)
    .bind(interstitial.content_status)
    .bind(interstitial.overview)
    .bind(interstitial.poster_url)
    .bind(interstitial.language)
    .bind(interstitial.runtime_minutes)
    .bind(interstitial.sort_title)
    .bind(interstitial.imdb_id)
    .bind(interstitial.genres_json.map(|value| value.to_string()))
    .bind(interstitial.studio)
    .bind(interstitial.digital_release_date)
    .bind(interstitial.association_confidence)
    .bind(interstitial.continuity_status)
    .bind(interstitial.movie_form)
    .bind(interstitial.confidence)
    .bind(interstitial.signal_summary)
    .bind(interstitial.placement)
    .bind(interstitial.movie_tmdb_id)
    .bind(interstitial.movie_mal_id)
    .bind(interstitial.movie_anidb_id)
    .bind(&collection.interstitial_season_episode)
    .bind(interstitial.special_movies_json.to_string())
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
