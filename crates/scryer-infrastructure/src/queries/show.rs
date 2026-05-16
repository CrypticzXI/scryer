use std::collections::HashMap;

use chrono::Utc;
use scryer_application::{AppError, AppResult, ScopedExternalId};
use scryer_domain::{
    Collection, CollectionType, Episode, EpisodeType, Id, InterstitialMovieMetadata,
};
use sqlx::{Postgres, QueryBuilder, Sqlite};

use super::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTarget, SqlTx, repo_err};

const COLLECTION_COLUMNS: &str = "id, title_id, collection_type, collection_index, label, ordered_path, \
    narrative_order, first_episode_number, last_episode_number, interstitial_tvdb_id, interstitial_name, \
    interstitial_slug, interstitial_year, interstitial_content_status, interstitial_overview, \
    interstitial_poster_url, interstitial_language, interstitial_runtime_minutes, interstitial_sort_title, \
    interstitial_imdb_id, interstitial_genres_json, interstitial_studio, interstitial_digital_release_date, \
    interstitial_association_confidence, interstitial_continuity_status, interstitial_movie_form, \
    interstitial_confidence, interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id, \
    interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode, special_movies_json, \
    monitored, created_at";

const EPISODE_COLUMNS: &str = "id, title_id, collection_id, episode_type, episode_number, season_number, \
    episode_label, title, air_date, duration_seconds, has_multi_audio, has_subtitle, is_filler, is_recap, \
    absolute_number, overview, tvdb_id, monitored, created_at";

pub(crate) async fn list_collections_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id = {{}} ORDER BY collection_index ASC, id ASC"
    );
    let rows =
        SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &[SqlArg::Text(title_id.to_string())])
            .await?;
    rows.iter().map(row_to_collection).collect()
}

pub(crate) async fn list_collection_external_ids_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT collection_id AS scope_id, source, external_id, provenance, source_scope \
         FROM collection_external_ids WHERE collection_id = {} \
         ORDER BY source ASC, external_id ASC, source_scope ASC",
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_scoped_external_id).collect()
}

pub(crate) async fn list_collections_for_titles_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<HashMap<String, Vec<Collection>>> {
    if title_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let rows = match target {
        SqlTarget::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new(format!(
                "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id IN ("
            ));
            {
                let mut separated = builder.separated(", ");
                for title_id in title_ids {
                    separated.push_bind(title_id);
                }
            }
            builder.push(") ORDER BY title_id ASC, collection_index ASC, id ASC");
            builder
                .build()
                .fetch_all(pool)
                .await
                .map(|rows| rows.into_iter().map(SqlRow::Sqlite).collect::<Vec<_>>())
                .map_err(repo_err)?
        }
        SqlTarget::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new(format!(
                "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id IN ("
            ));
            {
                let mut separated = builder.separated(", ");
                for title_id in title_ids {
                    separated.push_bind(title_id);
                }
            }
            builder.push(") ORDER BY title_id ASC, collection_index ASC, id ASC");
            builder
                .build()
                .fetch_all(pool)
                .await
                .map(|rows| rows.into_iter().map(SqlRow::Postgres).collect::<Vec<_>>())
                .map_err(repo_err)?
        }
    };

    let mut grouped = HashMap::<String, Vec<Collection>>::new();
    for row in &rows {
        let collection = row_to_collection(row)?;
        grouped
            .entry(collection.title_id.clone())
            .or_default()
            .push(collection);
    }

    Ok(grouped)
}

pub(crate) async fn get_collection_by_id_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
        .await?;
    row.as_ref().map(row_to_collection).transpose()
}

pub(crate) async fn get_collection_by_ordered_path_query(
    target: SqlTarget<'_>,
    ordered_path: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE ordered_path = {{}} ORDER BY id ASC LIMIT 1"
    );
    let row =
        SqlRuntime::fetch_optional(
            SqlExec::Target(target),
            &sql,
            &[SqlArg::Text(ordered_path.to_string())],
        )
        .await?;
    row.as_ref().map(row_to_collection).transpose()
}

pub(crate) async fn update_interstitial_season_episode_query(
    target: SqlTarget<'_>,
    collection_id: &str,
    season_episode: Option<&str>,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Target(target),
        "UPDATE collections SET interstitial_season_episode = {} WHERE id = {}",
        &[
            SqlArg::OptText(season_episode.map(str::to_string)),
            SqlArg::Text(collection_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub(crate) async fn set_collection_episodes_monitored_query(
    target: SqlTarget<'_>,
    collection_id: &str,
    monitored: bool,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Target(target),
        "UPDATE episodes SET monitored = {} WHERE collection_id = {}",
        &[
            SqlArg::Bool(monitored),
            SqlArg::Text(collection_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub(crate) async fn set_collections_monitored_query(
    target: SqlTarget<'_>,
    collection_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if collection_ids.is_empty() {
        return Ok(());
    }

    match target {
        SqlTarget::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new("UPDATE collections SET monitored = ");
            builder.push_bind(if monitored { 1_i64 } else { 0_i64 });
            builder.push(" WHERE id IN (");
            {
                let mut separated = builder.separated(", ");
                for collection_id in collection_ids {
                    separated.push_bind(collection_id);
                }
            }
            builder.push(")");
            builder.build().execute(pool).await.map_err(repo_err)?;
        }
        SqlTarget::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new("UPDATE collections SET monitored = ");
            builder.push_bind(monitored);
            builder.push(" WHERE id IN (");
            {
                let mut separated = builder.separated(", ");
                for collection_id in collection_ids {
                    separated.push_bind(collection_id);
                }
            }
            builder.push(")");
            builder.build().execute(pool).await.map_err(repo_err)?;
        }
    }

    Ok(())
}

pub(crate) async fn delete_collection_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<()> {
    let rows = SqlRuntime::execute(
        SqlExec::Target(target),
        "DELETE FROM collections WHERE id = {}",
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("collection {collection_id}")));
    }
    Ok(())
}

pub(crate) async fn delete_collections_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Target(target),
        "DELETE FROM collections WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

pub(crate) async fn list_episodes_for_collection_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE collection_id = {{}} ORDER BY episode_number ASC, id ASC"
    );
    let rows =
        SqlRuntime::fetch_all(
            SqlExec::Target(target),
            &sql,
            &[SqlArg::Text(collection_id.to_string())],
        )
        .await?;
    rows.iter().map(row_to_episode).collect()
}

pub(crate) async fn list_episodes_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE title_id = {{}} ORDER BY season_number ASC, episode_number ASC, id ASC"
    );
    let rows =
        SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &[SqlArg::Text(title_id.to_string())])
            .await?;
    rows.iter().map(row_to_episode).collect()
}

pub(crate) async fn list_episode_external_ids_query(
    target: SqlTarget<'_>,
    episode_id: &str,
) -> AppResult<Vec<ScopedExternalId>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT episode_id AS scope_id, source, external_id, provenance, source_scope \
         FROM episode_external_ids WHERE episode_id = {} \
         ORDER BY source ASC, external_id ASC, source_scope ASC",
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_scoped_external_id).collect()
}

pub(crate) async fn get_episode_by_id_query(
    target: SqlTarget<'_>,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id = {{}}");
    let row =
        SqlRuntime::fetch_optional(
            SqlExec::Target(target),
            &sql,
            &[SqlArg::Text(episode_id.to_string())],
        )
        .await?;
    row.as_ref().map(row_to_episode).transpose()
}

pub(crate) async fn set_episodes_monitored_query(
    target: SqlTarget<'_>,
    episode_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if episode_ids.is_empty() {
        return Ok(());
    }

    match target {
        SqlTarget::Sqlite(pool) => {
            let mut builder = QueryBuilder::<Sqlite>::new("UPDATE episodes SET monitored = ");
            builder.push_bind(if monitored { 1_i64 } else { 0_i64 });
            builder.push(" WHERE id IN (");
            {
                let mut separated = builder.separated(", ");
                for episode_id in episode_ids {
                    separated.push_bind(episode_id);
                }
            }
            builder.push(")");
            builder.build().execute(pool).await.map_err(repo_err)?;
        }
        SqlTarget::Postgres(pool) => {
            let mut builder = QueryBuilder::<Postgres>::new("UPDATE episodes SET monitored = ");
            builder.push_bind(monitored);
            builder.push(" WHERE id IN (");
            {
                let mut separated = builder.separated(", ");
                for episode_id in episode_ids {
                    separated.push_bind(episode_id);
                }
            }
            builder.push(")");
            builder.build().execute(pool).await.map_err(repo_err)?;
        }
    }

    Ok(())
}

pub(crate) async fn delete_episode_query(target: SqlTarget<'_>, episode_id: &str) -> AppResult<()> {
    let rows = SqlRuntime::execute(
        SqlExec::Target(target),
        "DELETE FROM episodes WHERE id = {}",
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("episode {episode_id}")));
    }
    Ok(())
}

pub(crate) async fn delete_episodes_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<()> {
    SqlRuntime::execute(
        SqlExec::Target(target),
        "DELETE FROM episodes WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

pub(crate) async fn find_episode_by_title_and_absolute_number_query(
    target: SqlTarget<'_>,
    title_id: &str,
    absolute_number: &str,
) -> AppResult<Option<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE title_id = {{}} AND absolute_number = {{}} LIMIT 1"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(absolute_number.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

pub(crate) async fn replace_anibridge_scoped_external_ids_for_title(
    target: SqlTarget<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    episode_ids: &[ScopedExternalId],
) -> AppResult<()> {
    let mut tx = SqlRuntime::begin(target).await?;
    tx.execute(
        "DELETE FROM collection_external_ids WHERE title_id = {} AND provenance = 'anibridge'",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    tx.execute(
        "DELETE FROM episode_external_ids WHERE title_id = {} AND provenance = 'anibridge'",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;

    let now = Utc::now();
    match &mut tx {
        SqlTx::Sqlite(_) => {
            insert_scoped_collection_ids_sqlite(&mut tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_sqlite(&mut tx, title_id, episode_ids, now).await?;
        }
        SqlTx::Postgres(_) => {
            insert_scoped_collection_ids_postgres(&mut tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_postgres(&mut tx, title_id, episode_ids, now).await?;
        }
    }

    tx.commit().await
}

pub(crate) fn normalized_scoped_external_id(
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

pub(crate) fn row_to_scoped_external_id(row: &SqlRow) -> AppResult<ScopedExternalId> {
    let source_scope = row.opt_text("source_scope")?.unwrap_or_default();
    Ok(ScopedExternalId {
        scope_id: row.text("scope_id")?,
        source: row.text("source")?,
        external_id: row.text("external_id")?,
        provenance: row.text("provenance")?,
        source_scope: if source_scope.trim().is_empty() {
            None
        } else {
            Some(source_scope)
        },
    })
}

fn row_to_collection(row: &SqlRow) -> AppResult<Collection> {
    let collection_type = CollectionType::parse(&row.text("collection_type")?).unwrap_or_default();
    Ok(Collection {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        collection_type,
        collection_index: row.text("collection_index")?,
        label: row.opt_text("label")?,
        ordered_path: row.opt_text("ordered_path")?,
        narrative_order: row.opt_text("narrative_order")?,
        first_episode_number: row.opt_text("first_episode_number")?,
        last_episode_number: row.opt_text("last_episode_number")?,
        interstitial_movie: row_to_interstitial_movie(row)?,
        specials_movies: row_to_specials_movies(row)?,
        interstitial_season_episode: row.opt_text("interstitial_season_episode")?,
        monitored: row.bool("monitored")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_episode(row: &SqlRow) -> AppResult<Episode> {
    let episode_type = EpisodeType::parse(&row.text("episode_type")?).unwrap_or_default();
    Ok(Episode {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        collection_id: row.opt_text("collection_id")?,
        episode_type,
        episode_number: row.opt_text("episode_number")?,
        season_number: row.opt_text("season_number")?,
        episode_label: row.opt_text("episode_label")?,
        title: row.opt_text("title")?,
        air_date: row.opt_text("air_date")?,
        duration_seconds: row.opt_i64("duration_seconds")?,
        has_multi_audio: row.bool("has_multi_audio")?,
        has_subtitle: row.bool("has_subtitle")?,
        is_filler: row.opt_bool("is_filler")?.unwrap_or(false),
        is_recap: row.opt_bool("is_recap")?.unwrap_or(false),
        absolute_number: row.opt_text("absolute_number")?,
        overview: row.opt_text("overview")?,
        tvdb_id: row.opt_text("tvdb_id")?,
        monitored: row.bool("monitored")?,
        created_at: row.timestamp("created_at")?,
    })
}

fn row_to_interstitial_movie(row: &SqlRow) -> AppResult<Option<InterstitialMovieMetadata>> {
    let Some(tvdb_id) = row.opt_text("interstitial_tvdb_id")? else {
        return Ok(None);
    };

    let genres = row
        .opt_json("interstitial_genres_json")?
        .map(serde_json::from_value::<Vec<String>>)
        .transpose()
        .map_err(repo_err)?
        .unwrap_or_default();

    Ok(Some(InterstitialMovieMetadata {
        tvdb_id,
        name: row.opt_text("interstitial_name")?.unwrap_or_default(),
        slug: row.opt_text("interstitial_slug")?.unwrap_or_default(),
        year: row.opt_i64("interstitial_year")?.map(|value| value as i32),
        content_status: row
            .opt_text("interstitial_content_status")?
            .unwrap_or_default(),
        overview: row.opt_text("interstitial_overview")?.unwrap_or_default(),
        poster_url: row.opt_text("interstitial_poster_url")?.unwrap_or_default(),
        language: row.opt_text("interstitial_language")?.unwrap_or_default(),
        runtime_minutes: row
            .opt_i64("interstitial_runtime_minutes")?
            .unwrap_or_default() as i32,
        sort_title: row.opt_text("interstitial_sort_title")?.unwrap_or_default(),
        imdb_id: row.opt_text("interstitial_imdb_id")?.unwrap_or_default(),
        genres,
        studio: row.opt_text("interstitial_studio")?.unwrap_or_default(),
        digital_release_date: row.opt_text("interstitial_digital_release_date")?,
        association_confidence: row.opt_text("interstitial_association_confidence")?,
        continuity_status: row.opt_text("interstitial_continuity_status")?,
        movie_form: row.opt_text("interstitial_movie_form")?,
        confidence: row.opt_text("interstitial_confidence")?,
        signal_summary: row.opt_text("interstitial_signal_summary")?,
        placement: row.opt_text("interstitial_placement")?,
        movie_tmdb_id: row.opt_text("interstitial_movie_tmdb_id")?,
        movie_mal_id: row.opt_text("interstitial_movie_mal_id")?,
        movie_anidb_id: row.opt_text("interstitial_movie_anidb_id")?,
    }))
}

fn row_to_specials_movies(row: &SqlRow) -> AppResult<Vec<InterstitialMovieMetadata>> {
    match row.opt_json("special_movies_json")? {
        Some(value) => serde_json::from_value(value).map_err(repo_err),
        None => Ok(Vec::new()),
    }
}

async fn insert_scoped_collection_ids_sqlite(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in collection_ids {
        let Some((collection_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT OR IGNORE INTO collection_external_ids \
             (id, title_id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(collection_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_episode_ids_sqlite(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    episode_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in episode_ids {
        let Some((episode_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT OR IGNORE INTO episode_external_ids \
             (id, title_id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(episode_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_collection_ids_postgres(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in collection_ids {
        let Some((collection_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT INTO collection_external_ids \
             (id, title_id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {}) ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(collection_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn insert_scoped_episode_ids_postgres(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    episode_ids: &[ScopedExternalId],
    now: chrono::DateTime<Utc>,
) -> AppResult<()> {
    for scoped_id in episode_ids {
        let Some((episode_id, source, external_id, source_scope)) =
            normalized_scoped_external_id(scoped_id)
        else {
            continue;
        };
        tx.execute(
            "INSERT INTO episode_external_ids \
             (id, title_id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, 'anibridge', {}, {}, {}) ON CONFLICT DO NOTHING",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(title_id.to_string()),
                SqlArg::Text(episode_id),
                SqlArg::Text(source),
                SqlArg::Text(external_id),
                SqlArg::Text(source_scope),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}
