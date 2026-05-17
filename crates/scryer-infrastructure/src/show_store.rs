use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeUpdate, PrimaryCollectionSummary,
    ScopedExternalId, ShowRepository,
};
use scryer_domain::{
    CalendarEpisode, Collection, CollectionType, Episode, EpisodeType, Id,
    InterstitialMovieMetadata,
};
use serde_json::Value as JsonValue;
use std::collections::{HashMap, HashSet};

use crate::queries::sql_runtime::{
    SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTarget, SqlTx, StoreDatastore, repo_err,
};

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

const COLLECTION_INSERT_SQL_SQLITE: &str = "INSERT INTO collections (
    id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
    first_episode_number, last_episode_number, interstitial_tvdb_id, interstitial_name,
    interstitial_slug, interstitial_year, interstitial_content_status, interstitial_overview,
    interstitial_poster_url, interstitial_language, interstitial_runtime_minutes,
    interstitial_sort_title, interstitial_imdb_id, interstitial_genres_json,
    interstitial_studio, interstitial_digital_release_date, interstitial_association_confidence,
    interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
    interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
    interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
    special_movies_json, monitored, created_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const COLLECTION_INSERT_SQL_POSTGRES: &str = "INSERT INTO collections (
    id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
    first_episode_number, last_episode_number, interstitial_tvdb_id, interstitial_name,
    interstitial_slug, interstitial_year, interstitial_content_status, interstitial_overview,
    interstitial_poster_url, interstitial_language, interstitial_runtime_minutes,
    interstitial_sort_title, interstitial_imdb_id, interstitial_genres_json,
    interstitial_studio, interstitial_digital_release_date, interstitial_association_confidence,
    interstitial_continuity_status, interstitial_movie_form, interstitial_confidence,
    interstitial_signal_summary, interstitial_placement, interstitial_movie_tmdb_id,
    interstitial_movie_mal_id, interstitial_movie_anidb_id, interstitial_season_episode,
    special_movies_json, monitored, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {},
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const COLLECTION_UPDATE_SQL_SQLITE: &str = "UPDATE collections SET
    title_id = {},
    collection_type = {},
    collection_index = {},
    label = {},
    ordered_path = {},
    narrative_order = {},
    first_episode_number = {},
    last_episode_number = {},
    interstitial_tvdb_id = {},
    interstitial_name = {},
    interstitial_slug = {},
    interstitial_year = {},
    interstitial_content_status = {},
    interstitial_overview = {},
    interstitial_poster_url = {},
    interstitial_language = {},
    interstitial_runtime_minutes = {},
    interstitial_sort_title = {},
    interstitial_imdb_id = {},
    interstitial_genres_json = {},
    interstitial_studio = {},
    interstitial_digital_release_date = {},
    interstitial_association_confidence = {},
    interstitial_continuity_status = {},
    interstitial_movie_form = {},
    interstitial_confidence = {},
    interstitial_signal_summary = {},
    interstitial_placement = {},
    interstitial_movie_tmdb_id = {},
    interstitial_movie_mal_id = {},
    interstitial_movie_anidb_id = {},
    interstitial_season_episode = {},
    special_movies_json = {},
    monitored = {}
WHERE id = {}";

const COLLECTION_UPDATE_SQL_POSTGRES: &str = "UPDATE collections SET
    title_id = {},
    collection_type = {},
    collection_index = {},
    label = {},
    ordered_path = {},
    narrative_order = {},
    first_episode_number = {},
    last_episode_number = {},
    interstitial_tvdb_id = {},
    interstitial_name = {},
    interstitial_slug = {},
    interstitial_year = {},
    interstitial_content_status = {},
    interstitial_overview = {},
    interstitial_poster_url = {},
    interstitial_language = {},
    interstitial_runtime_minutes = {},
    interstitial_sort_title = {},
    interstitial_imdb_id = {},
    interstitial_genres_json = {},
    interstitial_studio = {},
    interstitial_digital_release_date = {},
    interstitial_association_confidence = {},
    interstitial_continuity_status = {},
    interstitial_movie_form = {},
    interstitial_confidence = {},
    interstitial_signal_summary = {},
    interstitial_placement = {},
    interstitial_movie_tmdb_id = {},
    interstitial_movie_mal_id = {},
    interstitial_movie_anidb_id = {},
    interstitial_season_episode = {},
    special_movies_json = {},
    monitored = {},
    updated_at = {}
WHERE id = {}";

const EPISODE_INSERT_SQL_SQLITE: &str = "INSERT INTO episodes (
    id, title_id, collection_id, episode_type, episode_number, season_number,
    episode_label, title, air_date, duration_seconds, has_multi_audio,
    has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id,
    monitored, created_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const EPISODE_INSERT_SQL_POSTGRES: &str = "INSERT INTO episodes (
    id, title_id, collection_id, episode_type, episode_number, season_number,
    episode_label, title, air_date, duration_seconds, has_multi_audio,
    has_subtitle, is_filler, is_recap, absolute_number, overview, tvdb_id,
    monitored, created_at, updated_at
) VALUES (
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}
)";

const EPISODE_UPDATE_SQL_SQLITE: &str = "UPDATE episodes SET
    title_id = {},
    collection_id = {},
    episode_type = {},
    episode_number = {},
    season_number = {},
    episode_label = {},
    title = {},
    air_date = {},
    duration_seconds = {},
    has_multi_audio = {},
    has_subtitle = {},
    is_filler = {},
    is_recap = {},
    absolute_number = {},
    overview = {},
    tvdb_id = {},
    monitored = {}
WHERE id = {}";

const EPISODE_UPDATE_SQL_POSTGRES: &str = "UPDATE episodes SET
    title_id = {},
    collection_id = {},
    episode_type = {},
    episode_number = {},
    season_number = {},
    episode_label = {},
    title = {},
    air_date = {},
    duration_seconds = {},
    has_multi_audio = {},
    has_subtitle = {},
    is_filler = {},
    is_recap = {},
    absolute_number = {},
    overview = {},
    tvdb_id = {},
    monitored = {},
    updated_at = {}
WHERE id = {}";

#[derive(Clone)]
pub struct ShowStore {
    datastore: StoreDatastore,
}

impl ShowStore {
    pub(crate) fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub fn sqlite(db: &crate::SqliteServices) -> Self {
        Self::new(StoreDatastore::Sqlite {
            pool: db.pool().clone(),
            writer_gate: db.writer_gate(),
        })
    }

    fn read_target(&self) -> SqlTarget<'_> {
        match &self.datastore {
            StoreDatastore::Sqlite { pool, .. } => SqlTarget::Sqlite(pool),
            StoreDatastore::Postgres { pool } => SqlTarget::Postgres(pool),
        }
    }
}

#[async_trait]
impl ShowRepository for ShowStore {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        list_collections_for_title_query(self.read_target(), title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        list_collection_external_ids_query(self.read_target(), collection_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        list_collections_for_titles_query(self.read_target(), title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        get_collection_by_id_query(self.read_target(), collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        get_collection_by_ordered_path_query(self.read_target(), ordered_path).await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_collection", move |tx| {
            let collection = collection.clone();
            Box::pin(async move {
                insert_collection_tx(tx, &collection).await?;
                Ok(collection)
            })
        })
        .await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_collection", move |tx| {
            let collection_id = collection_id.clone();
            let update = update.clone();
            Box::pin(async move {
                if collection_update_is_empty(&update) {
                    return Err(AppError::Validation(
                        "at least one collection field must be provided".into(),
                    ));
                }

                let mut collection = load_collection_tx(tx, &collection_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;

                apply_collection_update(&mut collection, update);
                persist_collection_tx(tx, &collection).await?;
                Ok(collection)
            })
        })
        .await
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_collection_interstitial_movie",
            move |tx| {
                let collection_id = collection_id.clone();
                let interstitial_movie = interstitial_movie.clone();
                Box::pin(async move {
                    let mut collection = load_collection_tx(tx, &collection_id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;
                    collection.interstitial_movie = Some(interstitial_movie);
                    persist_collection_tx(tx, &collection).await?;
                    Ok(collection)
                })
            },
        )
        .await
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_collection_specials_movies",
            move |tx| {
                let collection_id = collection_id.clone();
                let specials_movies = specials_movies.clone();
                Box::pin(async move {
                    let mut collection = load_collection_tx(tx, &collection_id)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("collection {collection_id}")))?;
                    collection.specials_movies = specials_movies;
                    persist_collection_tx(tx, &collection).await?;
                    Ok(collection)
                })
            },
        )
        .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "update_interstitial_season_episode",
            move |tx| {
                let collection_id = collection_id.clone();
                let season_episode = season_episode.clone();
                Box::pin(async move {
                    update_interstitial_season_episode_tx(
                        tx,
                        &collection_id,
                        season_episode.as_deref(),
                    )
                    .await
                })
            },
        )
        .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_collection_episodes_monitored",
            move |tx| {
                let collection_id = collection_id.clone();
                Box::pin(async move {
                    set_collection_episodes_monitored_tx(tx, &collection_id, monitored).await
                })
            },
        )
        .await
    }

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let collection_ids = collection_ids.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "set_collections_monitored", move |tx| {
            let collection_ids = collection_ids.clone();
            Box::pin(
                async move { set_collections_monitored_tx(tx, &collection_ids, monitored).await },
            )
        })
        .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let collection_id = collection_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_collection", move |tx| {
            let collection_id = collection_id.clone();
            Box::pin(async move { delete_collection_tx(tx, &collection_id).await })
        })
        .await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_collections_for_title", move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move { delete_collections_for_title_tx(tx, &title_id).await })
        })
        .await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        list_episodes_for_collection_query(self.read_target(), collection_id).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        list_episodes_for_title_query(self.read_target(), title_id).await
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        list_episode_external_ids_query(self.read_target(), episode_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        get_episode_by_id_query(self.read_target(), episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        SqlRuntime::run_in_transaction(&self.datastore, "create_episode", move |tx| {
            let episode = episode.clone();
            Box::pin(async move {
                insert_episode_tx(tx, &episode).await?;
                Ok(episode)
            })
        })
        .await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        let episode_id = episode_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_episode", move |tx| {
            let episode_id = episode_id.clone();
            let update = update.clone();
            Box::pin(async move {
                if episode_update_is_empty(&update) {
                    return Err(AppError::Validation(
                        "at least one episode field must be provided".into(),
                    ));
                }

                let mut episode = load_episode_tx(tx, &episode_id)
                    .await?
                    .ok_or_else(|| AppError::NotFound(format!("episode {episode_id}")))?;

                apply_episode_update(&mut episode, update);
                persist_episode_tx(tx, &episode).await?;
                Ok(episode)
            })
        })
        .await
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let episode_ids = episode_ids.to_vec();
        SqlRuntime::run_in_transaction(&self.datastore, "set_episodes_monitored", move |tx| {
            let episode_ids = episode_ids.clone();
            Box::pin(async move { set_episodes_monitored_tx(tx, &episode_ids, monitored).await })
        })
        .await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let episode_id = episode_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_episode", move |tx| {
            let episode_id = episode_id.clone();
            Box::pin(async move { delete_episode_tx(tx, &episode_id).await })
        })
        .await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_episodes_for_title", move |tx| {
            let title_id = title_id.clone();
            Box::pin(async move { delete_episodes_for_title_tx(tx, &title_id).await })
        })
        .await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        find_episode_by_title_and_numbers_query(
            self.read_target(),
            title_id,
            season_number,
            episode_number,
        )
        .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        find_episode_by_title_and_absolute_number_query(
            self.read_target(),
            title_id,
            absolute_number,
        )
        .await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        list_primary_collection_summaries_query(self.read_target(), title_ids).await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        list_episodes_in_date_range_query(self.read_target(), start_date, end_date).await
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        let title_id = title_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "replace_anibridge_scoped_external_ids_for_title",
            move |tx| {
                let title_id = title_id.clone();
                let collection_ids = collection_ids.clone();
                let episode_ids = episode_ids.clone();
                Box::pin(async move {
                    replace_anibridge_scoped_external_ids_for_title_tx(
                        tx,
                        &title_id,
                        &collection_ids,
                        &episode_ids,
                    )
                    .await
                })
            },
        )
        .await
    }
}

async fn list_collections_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id = {{}} ORDER BY collection_index ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_collection).collect()
}

async fn list_collection_external_ids_query(
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

async fn list_collections_for_titles_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<HashMap<String, Vec<Collection>>> {
    if title_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders = bind_placeholders(title_ids.len());
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE title_id IN ({placeholders}) ORDER BY title_id ASC, collection_index ASC, id ASC"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;

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

async fn list_primary_collection_summaries_query(
    target: SqlTarget<'_>,
    title_ids: &[String],
) -> AppResult<Vec<PrimaryCollectionSummary>> {
    if title_ids.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders = bind_placeholders(title_ids.len());
    let sql = format!(
        "SELECT title_id, collection_type, collection_index, label, ordered_path FROM collections \
         WHERE title_id IN ({placeholders}) AND (collection_index = '0' OR collection_type = 'movie')"
    );
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(SqlExec::Target(target), &sql, &args).await?;

    let mut candidates = rows
        .iter()
        .map(summary_candidate_from_row)
        .collect::<AppResult<Vec<_>>>()?;
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

async fn get_collection_by_id_query(
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

async fn get_collection_by_ordered_path_query(
    target: SqlTarget<'_>,
    ordered_path: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE ordered_path = {{}} ORDER BY id ASC LIMIT 1"
    );
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(ordered_path.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_collection).transpose()
}

async fn list_episodes_for_collection_query(
    target: SqlTarget<'_>,
    collection_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE collection_id = {{}} ORDER BY episode_number ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_episode).collect()
}

async fn list_episodes_for_title_query(
    target: SqlTarget<'_>,
    title_id: &str,
) -> AppResult<Vec<Episode>> {
    let sql = format!(
        "SELECT {EPISODE_COLUMNS} FROM episodes WHERE title_id = {{}} ORDER BY season_number ASC, episode_number ASC, id ASC"
    );
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    rows.iter().map(row_to_episode).collect()
}

async fn list_episode_external_ids_query(
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

async fn get_episode_by_id_query(
    target: SqlTarget<'_>,
    episode_id: &str,
) -> AppResult<Option<Episode>> {
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        &sql,
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn find_episode_by_title_and_numbers_query(
    target: SqlTarget<'_>,
    title_id: &str,
    season_number: &str,
    episode_number: &str,
) -> AppResult<Option<Episode>> {
    let sql = "SELECT e.id, e.title_id, e.collection_id, e.episode_type, e.episode_number, \
               e.season_number, e.episode_label, e.title, e.air_date, e.duration_seconds, \
               e.has_multi_audio, e.has_subtitle, e.is_filler, e.is_recap, e.absolute_number, \
               e.overview, e.tvdb_id, e.monitored, e.created_at \
          FROM episodes e \
          INNER JOIN collections c ON c.id = e.collection_id \
         WHERE e.title_id = {} \
           AND c.collection_index = {} \
           AND e.episode_number = {} \
         LIMIT 1";
    let row = SqlRuntime::fetch_optional(
        SqlExec::Target(target),
        sql,
        &[
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(season_number.to_string()),
            SqlArg::Text(episode_number.to_string()),
        ],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn find_episode_by_title_and_absolute_number_query(
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

async fn list_episodes_in_date_range_query(
    target: SqlTarget<'_>,
    start_date: &str,
    end_date: &str,
) -> AppResult<Vec<CalendarEpisode>> {
    let rows = SqlRuntime::fetch_all(
        SqlExec::Target(target),
        "SELECT e.id, e.title_id, t.library_id, l.name AS library_name, l.slug AS library_slug, \
                t.name AS title_name, t.slug AS title_slug, t.facet AS title_facet, \
                e.season_number, e.episode_number, e.title AS episode_title, \
                e.air_date, e.monitored \
           FROM episodes e \
           JOIN titles t ON e.title_id = t.id \
           LEFT JOIN libraries l ON l.id = t.library_id \
          WHERE e.air_date IS NOT NULL AND e.air_date != '' \
            AND e.air_date >= {} AND e.air_date <= {} \
          ORDER BY e.air_date ASC",
        &[
            SqlArg::Text(start_date.to_string()),
            SqlArg::Text(end_date.to_string()),
        ],
    )
    .await?;
    rows.iter().map(row_to_calendar_episode).collect()
}

async fn load_collection_tx(
    tx: &mut SqlTx<'_>,
    collection_id: &str,
) -> AppResult<Option<Collection>> {
    let sql = format!("SELECT {COLLECTION_COLUMNS} FROM collections WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(collection_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_collection).transpose()
}

async fn insert_collection_tx(tx: &mut SqlTx<'_>, collection: &Collection) -> AppResult<()> {
    let interstitial = collection_interstitial_column_values(collection)?;
    let args = vec![
        SqlArg::Text(collection.id.clone()),
        SqlArg::Text(collection.title_id.clone()),
        SqlArg::Text(collection.collection_type.as_str().to_string()),
        SqlArg::Text(collection.collection_index.clone()),
        SqlArg::OptText(collection.label.clone()),
        SqlArg::OptText(collection.ordered_path.clone()),
        SqlArg::OptText(collection.narrative_order.clone()),
        SqlArg::OptText(collection.first_episode_number.clone()),
        SqlArg::OptText(collection.last_episode_number.clone()),
        SqlArg::OptText(interstitial.tvdb_id),
        SqlArg::OptText(interstitial.name),
        SqlArg::OptText(interstitial.slug),
        SqlArg::OptI32(interstitial.year),
        SqlArg::OptText(interstitial.content_status),
        SqlArg::OptText(interstitial.overview),
        SqlArg::OptText(interstitial.poster_url),
        SqlArg::OptText(interstitial.language),
        SqlArg::OptI64(interstitial.runtime_minutes.map(i64::from)),
        SqlArg::OptText(interstitial.sort_title),
        SqlArg::OptText(interstitial.imdb_id),
        SqlArg::OptJson(interstitial.genres_json),
        SqlArg::OptText(interstitial.studio),
        SqlArg::OptText(interstitial.digital_release_date),
        SqlArg::OptText(interstitial.association_confidence),
        SqlArg::OptText(interstitial.continuity_status),
        SqlArg::OptText(interstitial.movie_form),
        SqlArg::OptText(interstitial.confidence),
        SqlArg::OptText(interstitial.signal_summary),
        SqlArg::OptText(interstitial.placement),
        SqlArg::OptText(interstitial.movie_tmdb_id),
        SqlArg::OptText(interstitial.movie_mal_id),
        SqlArg::OptText(interstitial.movie_anidb_id),
        SqlArg::OptText(collection.interstitial_season_episode.clone()),
        SqlArg::Json(interstitial.special_movies_json),
        SqlArg::Bool(collection.monitored),
        SqlArg::Timestamp(collection.created_at),
    ];

    match tx {
        SqlTx::Sqlite(_) => {
            tx.execute(COLLECTION_INSERT_SQL_SQLITE, &args).await?;
        }
        SqlTx::Postgres(_) => {
            let mut pg_args = args;
            pg_args.push(SqlArg::Timestamp(Utc::now()));
            tx.execute(COLLECTION_INSERT_SQL_POSTGRES, &pg_args).await?;
        }
    }

    Ok(())
}

async fn persist_collection_tx(tx: &mut SqlTx<'_>, collection: &Collection) -> AppResult<()> {
    let interstitial = collection_interstitial_column_values(collection)?;
    let args = vec![
        SqlArg::Text(collection.title_id.clone()),
        SqlArg::Text(collection.collection_type.as_str().to_string()),
        SqlArg::Text(collection.collection_index.clone()),
        SqlArg::OptText(collection.label.clone()),
        SqlArg::OptText(collection.ordered_path.clone()),
        SqlArg::OptText(collection.narrative_order.clone()),
        SqlArg::OptText(collection.first_episode_number.clone()),
        SqlArg::OptText(collection.last_episode_number.clone()),
        SqlArg::OptText(interstitial.tvdb_id),
        SqlArg::OptText(interstitial.name),
        SqlArg::OptText(interstitial.slug),
        SqlArg::OptI32(interstitial.year),
        SqlArg::OptText(interstitial.content_status),
        SqlArg::OptText(interstitial.overview),
        SqlArg::OptText(interstitial.poster_url),
        SqlArg::OptText(interstitial.language),
        SqlArg::OptI64(interstitial.runtime_minutes.map(i64::from)),
        SqlArg::OptText(interstitial.sort_title),
        SqlArg::OptText(interstitial.imdb_id),
        SqlArg::OptJson(interstitial.genres_json),
        SqlArg::OptText(interstitial.studio),
        SqlArg::OptText(interstitial.digital_release_date),
        SqlArg::OptText(interstitial.association_confidence),
        SqlArg::OptText(interstitial.continuity_status),
        SqlArg::OptText(interstitial.movie_form),
        SqlArg::OptText(interstitial.confidence),
        SqlArg::OptText(interstitial.signal_summary),
        SqlArg::OptText(interstitial.placement),
        SqlArg::OptText(interstitial.movie_tmdb_id),
        SqlArg::OptText(interstitial.movie_mal_id),
        SqlArg::OptText(interstitial.movie_anidb_id),
        SqlArg::OptText(collection.interstitial_season_episode.clone()),
        SqlArg::Json(interstitial.special_movies_json),
        SqlArg::Bool(collection.monitored),
    ];

    match tx {
        SqlTx::Sqlite(_) => {
            let mut sqlite_args = args;
            sqlite_args.push(SqlArg::Text(collection.id.clone()));
            tx.execute(COLLECTION_UPDATE_SQL_SQLITE, &sqlite_args)
                .await?;
        }
        SqlTx::Postgres(_) => {
            let mut pg_args = args;
            pg_args.push(SqlArg::Timestamp(Utc::now()));
            pg_args.push(SqlArg::Text(collection.id.clone()));
            tx.execute(COLLECTION_UPDATE_SQL_POSTGRES, &pg_args).await?;
        }
    }

    Ok(())
}

async fn update_interstitial_season_episode_tx(
    tx: &mut SqlTx<'_>,
    collection_id: &str,
    season_episode: Option<&str>,
) -> AppResult<()> {
    tx.execute(
        "UPDATE collections SET interstitial_season_episode = {} WHERE id = {}",
        &[
            SqlArg::OptText(season_episode.map(str::to_string)),
            SqlArg::Text(collection_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn set_collection_episodes_monitored_tx(
    tx: &mut SqlTx<'_>,
    collection_id: &str,
    monitored: bool,
) -> AppResult<()> {
    tx.execute(
        "UPDATE episodes SET monitored = {} WHERE collection_id = {}",
        &[
            SqlArg::Bool(monitored),
            SqlArg::Text(collection_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

async fn set_collections_monitored_tx(
    tx: &mut SqlTx<'_>,
    collection_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if collection_ids.is_empty() {
        return Ok(());
    }

    let placeholders = bind_placeholders(collection_ids.len());
    let sql = format!("UPDATE collections SET monitored = {{}} WHERE id IN ({placeholders})");
    let mut args = vec![SqlArg::Bool(monitored)];
    args.extend(collection_ids.iter().cloned().map(SqlArg::Text));
    tx.execute(&sql, &args).await?;
    Ok(())
}

async fn delete_collection_tx(tx: &mut SqlTx<'_>, collection_id: &str) -> AppResult<()> {
    let rows = tx
        .execute(
            "DELETE FROM collections WHERE id = {}",
            &[SqlArg::Text(collection_id.to_string())],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("collection {collection_id}")));
    }
    Ok(())
}

async fn delete_collections_for_title_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM collections WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn load_episode_tx(tx: &mut SqlTx<'_>, episode_id: &str) -> AppResult<Option<Episode>> {
    let sql = format!("SELECT {EPISODE_COLUMNS} FROM episodes WHERE id = {{}}");
    let row = SqlRuntime::fetch_optional(
        SqlExec::Tx(tx),
        &sql,
        &[SqlArg::Text(episode_id.to_string())],
    )
    .await?;
    row.as_ref().map(row_to_episode).transpose()
}

async fn insert_episode_tx(tx: &mut SqlTx<'_>, episode: &Episode) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(episode.id.clone()),
        SqlArg::Text(episode.title_id.clone()),
        SqlArg::OptText(episode.collection_id.clone()),
        SqlArg::Text(episode.episode_type.as_str().to_string()),
        SqlArg::OptText(episode.episode_number.clone()),
        SqlArg::OptText(episode.season_number.clone()),
        SqlArg::OptText(episode.episode_label.clone()),
        SqlArg::OptText(episode.title.clone()),
        SqlArg::OptText(episode.air_date.clone()),
        SqlArg::OptI64(episode.duration_seconds),
        SqlArg::Bool(episode.has_multi_audio),
        SqlArg::Bool(episode.has_subtitle),
        SqlArg::Bool(episode.is_filler),
        SqlArg::Bool(episode.is_recap),
        SqlArg::OptText(episode.absolute_number.clone()),
        SqlArg::OptText(episode.overview.clone()),
        SqlArg::OptText(episode.tvdb_id.clone()),
        SqlArg::Bool(episode.monitored),
        SqlArg::Timestamp(episode.created_at),
    ];

    match tx {
        SqlTx::Sqlite(_) => {
            tx.execute(EPISODE_INSERT_SQL_SQLITE, &args).await?;
        }
        SqlTx::Postgres(_) => {
            let mut pg_args = args;
            pg_args.push(SqlArg::Timestamp(Utc::now()));
            tx.execute(EPISODE_INSERT_SQL_POSTGRES, &pg_args).await?;
        }
    }

    Ok(())
}

async fn persist_episode_tx(tx: &mut SqlTx<'_>, episode: &Episode) -> AppResult<()> {
    let args = vec![
        SqlArg::Text(episode.title_id.clone()),
        SqlArg::OptText(episode.collection_id.clone()),
        SqlArg::Text(episode.episode_type.as_str().to_string()),
        SqlArg::OptText(episode.episode_number.clone()),
        SqlArg::OptText(episode.season_number.clone()),
        SqlArg::OptText(episode.episode_label.clone()),
        SqlArg::OptText(episode.title.clone()),
        SqlArg::OptText(episode.air_date.clone()),
        SqlArg::OptI64(episode.duration_seconds),
        SqlArg::Bool(episode.has_multi_audio),
        SqlArg::Bool(episode.has_subtitle),
        SqlArg::Bool(episode.is_filler),
        SqlArg::Bool(episode.is_recap),
        SqlArg::OptText(episode.absolute_number.clone()),
        SqlArg::OptText(episode.overview.clone()),
        SqlArg::OptText(episode.tvdb_id.clone()),
        SqlArg::Bool(episode.monitored),
    ];

    match tx {
        SqlTx::Sqlite(_) => {
            let mut sqlite_args = args;
            sqlite_args.push(SqlArg::Text(episode.id.clone()));
            tx.execute(EPISODE_UPDATE_SQL_SQLITE, &sqlite_args).await?;
        }
        SqlTx::Postgres(_) => {
            let mut pg_args = args;
            pg_args.push(SqlArg::Timestamp(Utc::now()));
            pg_args.push(SqlArg::Text(episode.id.clone()));
            tx.execute(EPISODE_UPDATE_SQL_POSTGRES, &pg_args).await?;
        }
    }

    Ok(())
}

async fn set_episodes_monitored_tx(
    tx: &mut SqlTx<'_>,
    episode_ids: &[String],
    monitored: bool,
) -> AppResult<()> {
    if episode_ids.is_empty() {
        return Ok(());
    }

    let placeholders = bind_placeholders(episode_ids.len());
    let sql = format!("UPDATE episodes SET monitored = {{}} WHERE id IN ({placeholders})");
    let mut args = vec![SqlArg::Bool(monitored)];
    args.extend(episode_ids.iter().cloned().map(SqlArg::Text));
    tx.execute(&sql, &args).await?;
    Ok(())
}

async fn delete_episode_tx(tx: &mut SqlTx<'_>, episode_id: &str) -> AppResult<()> {
    let rows = tx
        .execute(
            "DELETE FROM episodes WHERE id = {}",
            &[SqlArg::Text(episode_id.to_string())],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("episode {episode_id}")));
    }
    Ok(())
}

async fn delete_episodes_for_title_tx(tx: &mut SqlTx<'_>, title_id: &str) -> AppResult<()> {
    tx.execute(
        "DELETE FROM episodes WHERE title_id = {}",
        &[SqlArg::Text(title_id.to_string())],
    )
    .await?;
    Ok(())
}

async fn replace_anibridge_scoped_external_ids_for_title_tx(
    tx: &mut SqlTx<'_>,
    title_id: &str,
    collection_ids: &[ScopedExternalId],
    episode_ids: &[ScopedExternalId],
) -> AppResult<()> {
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
    match tx {
        SqlTx::Sqlite(_) => {
            insert_scoped_collection_ids_sqlite(tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_sqlite(tx, title_id, episode_ids, now).await?;
        }
        SqlTx::Postgres(_) => {
            insert_scoped_collection_ids_postgres(tx, title_id, collection_ids, now).await?;
            insert_scoped_episode_ids_postgres(tx, title_id, episode_ids, now).await?;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SummaryCandidate {
    title_id: String,
    collection_type: CollectionType,
    collection_index: String,
    label: Option<String>,
    ordered_path: Option<String>,
}

struct CollectionInterstitialColumnValues {
    tvdb_id: Option<String>,
    name: Option<String>,
    slug: Option<String>,
    year: Option<i32>,
    content_status: Option<String>,
    overview: Option<String>,
    poster_url: Option<String>,
    language: Option<String>,
    runtime_minutes: Option<i32>,
    sort_title: Option<String>,
    imdb_id: Option<String>,
    genres_json: Option<JsonValue>,
    studio: Option<String>,
    digital_release_date: Option<String>,
    association_confidence: Option<String>,
    continuity_status: Option<String>,
    movie_form: Option<String>,
    confidence: Option<String>,
    signal_summary: Option<String>,
    placement: Option<String>,
    movie_tmdb_id: Option<String>,
    movie_mal_id: Option<String>,
    movie_anidb_id: Option<String>,
    special_movies_json: JsonValue,
}

fn collection_update_is_empty(update: &CollectionUpdate) -> bool {
    update.collection_type.is_none()
        && update.collection_index.is_none()
        && update.label.is_none()
        && update.ordered_path.is_none()
        && !update.clear_ordered_path
        && update.first_episode_number.is_none()
        && update.last_episode_number.is_none()
        && update.monitored.is_none()
}

fn apply_collection_update(collection: &mut Collection, update: CollectionUpdate) {
    if let Some(value) = update.collection_type {
        collection.collection_type = value;
    }
    if let Some(value) = update.collection_index {
        collection.collection_index = value;
    }
    if let Some(value) = update.label {
        collection.label = Some(value);
    }
    if update.clear_ordered_path {
        collection.ordered_path = None;
    } else if let Some(value) = update.ordered_path {
        collection.ordered_path = Some(value);
    }
    if let Some(value) = update.first_episode_number {
        collection.first_episode_number = Some(value);
    }
    if let Some(value) = update.last_episode_number {
        collection.last_episode_number = Some(value);
    }
    if let Some(value) = update.monitored {
        collection.monitored = value;
    }
}

fn episode_update_is_empty(update: &EpisodeUpdate) -> bool {
    update.episode_type.is_none()
        && update.episode_number.is_none()
        && update.season_number.is_none()
        && update.episode_label.is_none()
        && update.title.is_none()
        && update.air_date.is_none()
        && update.duration_seconds.is_none()
        && update.has_multi_audio.is_none()
        && update.has_subtitle.is_none()
        && update.monitored.is_none()
        && update.collection_id.is_none()
        && update.overview.is_none()
        && update.tvdb_id.is_none()
}

fn apply_episode_update(episode: &mut Episode, update: EpisodeUpdate) {
    if let Some(value) = update.episode_type {
        episode.episode_type = value;
    }
    if let Some(value) = update.episode_number {
        episode.episode_number = Some(value);
    }
    if let Some(value) = update.season_number {
        episode.season_number = Some(value);
    }
    if let Some(value) = update.episode_label {
        episode.episode_label = Some(value);
    }
    if let Some(value) = update.title {
        episode.title = Some(value);
    }
    if let Some(value) = update.air_date {
        episode.air_date = Some(value);
    }
    if let Some(value) = update.duration_seconds {
        episode.duration_seconds = Some(value);
    }
    if let Some(value) = update.has_multi_audio {
        episode.has_multi_audio = value;
    }
    if let Some(value) = update.has_subtitle {
        episode.has_subtitle = value;
    }
    if let Some(value) = update.monitored {
        episode.monitored = value;
    }
    if let Some(value) = update.collection_id {
        episode.collection_id = Some(value);
    }
    if let Some(value) = update.overview {
        episode.overview = Some(value);
    }
    if let Some(value) = update.tvdb_id {
        episode.tvdb_id = Some(value);
    }
}

fn collection_interstitial_column_values(
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
            .map_err(repo_err)?,
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
        special_movies_json: serde_json::to_value(&collection.specials_movies).map_err(repo_err)?,
    })
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

fn summary_candidate_from_row(row: &SqlRow) -> AppResult<SummaryCandidate> {
    Ok(SummaryCandidate {
        title_id: row.text("title_id")?,
        collection_type: CollectionType::parse(&row.text("collection_type")?).unwrap_or_default(),
        collection_index: row.text("collection_index")?,
        label: row.opt_text("label")?,
        ordered_path: row.opt_text("ordered_path")?,
    })
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

fn bind_placeholders(count: usize) -> String {
    std::iter::repeat_n("{}", count)
        .collect::<Vec<_>>()
        .join(", ")
}

fn row_to_scoped_external_id(row: &SqlRow) -> AppResult<ScopedExternalId> {
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

fn row_to_calendar_episode(row: &SqlRow) -> AppResult<CalendarEpisode> {
    Ok(CalendarEpisode {
        id: row.text("id")?,
        title_id: row.text("title_id")?,
        library_id: row.text("library_id")?,
        library_name: row.opt_text("library_name")?,
        library_slug: row.opt_text("library_slug")?,
        title_name: row.text("title_name")?,
        title_slug: row.opt_text("title_slug")?,
        title_facet: row.text("title_facet")?,
        season_number: row.opt_text("season_number")?,
        episode_number: row.opt_text("episode_number")?,
        episode_title: row.opt_text("episode_title")?,
        air_date: row.opt_text("air_date")?,
        monitored: row.bool("monitored")?,
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
        year: row.opt_i32("interstitial_year")?,
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
