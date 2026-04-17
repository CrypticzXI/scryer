use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeUpdate, PrimaryCollectionSummary, ShowRepository,
    TitleMetadataUpdate, TitleRepository, UserRepository,
};
use scryer_domain::{CalendarEpisode, Collection, Entitlement, Episode, MediaFacet, Title, User};
use std::collections::HashMap;

use crate::SqliteServices;
use crate::queries::{title, user};

#[derive(Clone)]
pub struct SqliteCatalogStore {
    pool: sqlx::SqlitePool,
}

impl SqliteCatalogStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl TitleRepository for SqliteCatalogStore {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        title::list_titles_query(&self.pool, facet, query).await
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        title::list_titles_for_matching_query(&self.pool, facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        title::get_title_by_id_query(&self.pool, id).await
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        title::get_title_by_external_id_query(&self.pool, source, value).await
    }

    async fn create(&self, title_record: Title) -> AppResult<Title> {
        title::create_title_query(&self.pool, &title_record).await
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        title::update_title_monitored_query(&self.pool, id, monitored).await
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let tags_json = match tags {
            Some(tags) => Some(
                serde_json::to_string(&tags)
                    .map_err(|err| AppError::Repository(err.to_string()))?,
            ),
            None => None,
        };
        title::update_title_metadata_query(&self.pool, id, name, facet, tags_json).await
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        title::update_title_hydrated_metadata_query(&self.pool, id, metadata).await
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<scryer_domain::ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        title::replace_title_match_state_query(&self.pool, id, external_ids, tags).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        title::delete_title_query(&self.pool, id).await
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        title::set_title_folder_path_query(&self.pool, id, folder_path).await
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        title::clear_title_folder_path_query(&self.pool, id).await
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        title::clear_metadata_language_for_all_query(&self.pool).await
    }
}

#[async_trait]
impl ShowRepository for SqliteCatalogStore {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        title::list_collections_for_title_query(&self.pool, title_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        let collections = title::list_collections_for_titles_query(&self.pool, title_ids).await?;
        let mut grouped = HashMap::<String, Vec<Collection>>::new();
        for collection in collections {
            grouped
                .entry(collection.title_id.clone())
                .or_default()
                .push(collection);
        }
        Ok(grouped)
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        title::list_primary_collection_summaries_query(&self.pool, title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        title::get_collection_by_id_query(&self.pool, collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        title::get_collection_by_ordered_path_query(&self.pool, ordered_path).await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        title::create_collection_query(&self.pool, &collection).await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        title::update_collection_query(&self.pool, collection_id, update).await
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        title::update_collection_interstitial_movie_query(
            &self.pool,
            collection_id,
            &interstitial_movie,
        )
        .await
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        title::update_collection_specials_movies_query(&self.pool, collection_id, &specials_movies)
            .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        title::update_interstitial_season_episode_query(
            &self.pool,
            collection_id,
            season_episode.as_deref(),
        )
        .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        title::set_collection_episodes_monitored_query(&self.pool, collection_id, monitored).await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        title::delete_collection_query(&self.pool, collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        title::delete_collections_for_title_query(&self.pool, title_id).await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        title::list_episodes_for_collection_query(&self.pool, collection_id).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        title::list_episodes_for_title_query(&self.pool, title_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        title::get_episode_by_id_query(&self.pool, episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        title::create_episode_query(&self.pool, &episode).await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        title::update_episode_query(&self.pool, episode_id, update).await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        title::delete_episode_query(&self.pool, episode_id).await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        title::delete_episodes_for_title_query(&self.pool, title_id).await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        title::find_episode_by_title_and_numbers_query(
            &self.pool,
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
        title::find_episode_by_title_and_absolute_number_query(
            &self.pool,
            title_id,
            absolute_number,
        )
        .await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        title::list_episodes_in_date_range_query(&self.pool, start_date, end_date).await
    }
}

#[async_trait]
impl UserRepository for SqliteCatalogStore {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        user::get_user_by_username_query(&self.pool, username).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        user::get_user_by_id_query(&self.pool, id).await
    }

    async fn create(&self, user_record: User) -> AppResult<User> {
        user::create_user_query(&self.pool, &user_record).await
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        user::list_users_query(&self.pool).await
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let entitlements_json = serde_json::to_string(&entitlements)
            .map_err(|err| AppError::Repository(err.to_string()))?;
        user::update_user_entitlements_query(&self.pool, id, &entitlements_json).await
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        user::update_user_password_query(&self.pool, id, &password_hash).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        user::delete_user_query(&self.pool, id).await
    }
}
