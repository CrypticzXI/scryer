use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, EpisodeUpdate,
    PendingTitleHydration, PrimaryCollectionSummary, ShowRepository, TitleMetadataUpdate,
    TitleRepository, UserRepository,
};
use scryer_domain::{CalendarEpisode, Collection, Entitlement, Episode, MediaFacet, Title, User};
use std::collections::HashMap;

use crate::SqliteServices;
use crate::queries::{title, user};

#[derive(Clone)]
pub struct SqliteCatalogStore {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteCatalogStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
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

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        title::list_titles_by_external_ids_query(&self.pool, source, values).await
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

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        title::get_title_by_facet_and_slug_query(&self.pool, facet, slug).await
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        title::get_title_by_external_id_query(&self.pool, source, value).await
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        title::get_title_by_external_id_in_facet_query(&self.pool, facet, source, value).await
    }

    async fn create_or_get_existing(&self, title_record: Title) -> AppResult<CreateTitleOutcome> {
        self.db.create_or_get_existing_title(&title_record).await
    }

    async fn create(&self, title_record: Title) -> AppResult<Title> {
        self.db.create_title(&title_record).await
    }

    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        title::list_titles_due_for_hydration_query(&self.pool, limit, excluded_facets).await
    }

    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        self.db.mark_title_metadata_hydration_due_now(id).await
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        self.db
            .schedule_title_metadata_hydration_retry(id, next_attempt_at, attempt_count)
            .await
    }

    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        self.db.clear_title_metadata_hydration_retry_state(id).await
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        self.db.update_title_monitored(id, monitored).await
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
        self.db
            .update_title_metadata(id, name, facet, tags_json)
            .await
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        self.db.update_title_hydrated_metadata(id, metadata).await
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<scryer_domain::ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        self.db
            .replace_title_match_state(id, external_ids, tags)
            .await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.db.delete_title(id).await
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        self.db.set_title_folder_path(id, folder_path).await
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        self.db.clear_title_folder_path(id).await
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        self.db.clear_metadata_language_for_all().await
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
        self.db.create_collection(&collection).await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        self.db.update_collection(collection_id, update).await
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        self.db
            .update_collection_interstitial_movie(collection_id, &interstitial_movie)
            .await
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        self.db
            .update_collection_specials_movies(collection_id, &specials_movies)
            .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        self.db
            .update_interstitial_season_episode(collection_id, season_episode.as_deref())
            .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        self.db
            .set_collection_episodes_monitored(collection_id, monitored)
            .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        self.db.delete_collection(collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_collections_for_title(title_id).await
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
        self.db.create_episode(&episode).await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        self.db.update_episode(episode_id, update).await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        self.db.delete_episode(episode_id).await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_episodes_for_title(title_id).await
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
        self.db.create_user(&user_record).await
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
        self.db
            .update_user_entitlements(id, &entitlements_json)
            .await
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        self.db.update_user_password_hash(id, &password_hash).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.db.delete_user(id).await
    }
}
