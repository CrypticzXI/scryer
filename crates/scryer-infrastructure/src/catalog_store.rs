use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, EpisodeUpdate, LibraryRepository,
    LibraryRootDraft, PendingTitleHydration, PrimaryCollectionSummary, ScopedExternalId,
    ShowRepository, TitleMetadataUpdate, TitleRepository, UserRepository,
};
use scryer_domain::{
    AppPermissionMask, CalendarEpisode, Collection, Entitlement, Episode, Library, LibraryGrant,
    MediaFacet, Title, User,
};
use std::collections::HashMap;

use crate::SqliteServices;
use crate::queries::{library, title, user};

#[async_trait]
pub trait TitleSql: Clone + Send + Sync + 'static {
    async fn list(&self, facet: Option<MediaFacet>, query: Option<String>)
    -> AppResult<Vec<Title>>;
    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let titles = self.list(facet, query).await?;
        Ok(titles
            .into_iter()
            .filter(|title| library_ids.iter().any(|id| id == &title.library_id))
            .collect())
    }
    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>>;
    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>>;
    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>>;
    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() || library_ids.is_empty() {
            return Ok(None);
        }

        Ok(self
            .list_for_libraries(Some(facet), library_ids, None)
            .await?
            .into_iter()
            .find(|title| {
                title
                    .slug
                    .as_deref()
                    .is_some_and(|candidate| candidate.trim().eq_ignore_ascii_case(normalized_slug))
            }))
    }
    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>>;
    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>>;
    async fn create_or_get_existing(&self, title_record: Title) -> AppResult<CreateTitleOutcome>;
    async fn create(&self, title_record: Title) -> AppResult<Title>;
    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>>;
    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>>;
    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>>;
    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()>;
    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()>;
    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()>;
    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title>;
    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title>;
    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title>;
    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<scryer_domain::ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title>;
    async fn delete(&self, id: &str) -> AppResult<()>;
    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()>;
    async fn clear_folder_path(&self, id: &str) -> AppResult<()>;
    async fn clear_metadata_language_for_all(&self) -> AppResult<u64>;
}

#[async_trait]
pub trait LibrarySql: Clone + Send + Sync + 'static {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>>;
    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>>;
    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library>;
    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library>;
    async fn delete_library(&self, library_id: &str) -> AppResult<bool>;
    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask>;
    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()>;
    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>>;
    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()>;
    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>>;
}

#[async_trait]
pub trait ShowSql: Clone + Send + Sync + 'static {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>>;
    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>>;
    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>>;
    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>>;
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>>;
    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>>;
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection>;
    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection>;
    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection>;
    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection>;
    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()>;
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()>;
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()>;
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>>;
    async fn list_episode_external_ids(&self, episode_id: &str)
    -> AppResult<Vec<ScopedExternalId>>;
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>>;
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode>;
    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode>;
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()>;
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()>;
    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>>;
    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>>;
    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()>;
}

#[async_trait]
pub trait UserSql: Clone + Send + Sync + 'static {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>>;
    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>>;
    async fn create(&self, user_record: User) -> AppResult<User>;
    async fn list_all(&self) -> AppResult<Vec<User>>;
    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User>;
    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

#[derive(Clone)]
pub struct CatalogStore<S> {
    sql: S,
}

impl<S> CatalogStore<S> {
    pub(crate) fn from_sql(sql: S) -> Self {
        Self { sql }
    }
}

pub type SqliteCatalogStore = CatalogStore<SqliteCatalogSql>;

#[derive(Clone)]
pub struct SqliteCatalogSql {
    db: SqliteServices,
    pool: sqlx::SqlitePool,
}

impl SqliteCatalogStore {
    pub fn new(db: &SqliteServices) -> Self {
        Self::from_sql(SqliteCatalogSql::new(db))
    }
}

impl SqliteCatalogSql {
    fn new(db: &SqliteServices) -> Self {
        Self {
            db: db.clone(),
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl TitleSql for SqliteCatalogSql {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        title::list_titles_query(&self.pool, facet, query).await
    }

    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        title::list_titles_for_libraries_query(&self.pool, facet, library_ids, query).await
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

    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        title::get_title_by_facet_libraries_and_slug_query(&self.pool, facet, library_ids, slug)
            .await
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

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        title::list_anime_title_ids_missing_anibridge_scoped_external_ids_query(&self.pool, limit)
            .await
    }

    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        title::list_anime_title_ids_missing_title_anidb_external_ids_query(&self.pool, limit).await
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
impl LibrarySql for SqliteCatalogSql {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        library::list_libraries_query(&self.pool, facet).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        library::get_library_by_id_query(&self.pool, id).await
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        let expected_id = scryer_domain::default_library_id_for_facet(&facet);
        library::get_library_by_id_query(&self.pool, &expected_id).await
    }

    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        library::create_library_query(&self.pool, library, roots).await
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        library::update_library_query(&self.pool, library_id, name, slug, roots).await
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        library::delete_library_query(&self.pool, library_id).await
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        library::app_permission_mask_for_user_query(&self.pool, user_id).await
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        library::set_app_permission_mask_for_user_query(&self.pool, user_id, permissions).await
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        library::library_permission_masks_for_user_query(&self.pool, user_id).await
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        library::set_library_grants_for_user_query(&self.pool, user_id, grants).await
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        library::title_library_id_query(&self.pool, title_id).await
    }
}

#[async_trait]
impl ShowSql for SqliteCatalogSql {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        title::list_collections_for_title_query(&self.pool, title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        title::list_collection_external_ids_query(&self.pool, collection_id).await
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

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        title::list_episode_external_ids_query(&self.pool, episode_id).await
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

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        self.db
            .replace_anibridge_scoped_external_ids_for_title(
                title_id,
                &collection_ids,
                &episode_ids,
            )
            .await
    }
}

#[async_trait]
impl UserSql for SqliteCatalogSql {
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

#[async_trait]
impl<S: TitleSql> TitleRepository for CatalogStore<S> {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.sql.list(facet, query).await
    }

    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.sql.list_for_libraries(facet, library_ids, query).await
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        self.sql.list_by_external_ids(source, values).await
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.sql.list_for_matching(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        self.sql.get_by_id(id).await
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        self.sql.get_by_facet_and_slug(facet, slug).await
    }

    async fn get_by_facet_libraries_and_slug(
        &self,
        facet: MediaFacet,
        library_ids: &[String],
        slug: &str,
    ) -> AppResult<Option<Title>> {
        self.sql
            .get_by_facet_libraries_and_slug(facet, library_ids, slug)
            .await
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        self.sql.find_by_external_id(source, value).await
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        self.sql
            .find_by_external_id_in_facet(facet, source, value)
            .await
    }

    async fn create_or_get_existing(&self, title_record: Title) -> AppResult<CreateTitleOutcome> {
        self.sql.create_or_get_existing(title_record).await
    }

    async fn create(&self, title_record: Title) -> AppResult<Title> {
        self.sql.create(title_record).await
    }

    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        self.sql
            .list_titles_due_for_hydration(limit, excluded_facets)
            .await
    }

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        self.sql
            .list_anime_title_ids_missing_anibridge_scoped_external_ids(limit)
            .await
    }

    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        self.sql
            .list_anime_title_ids_missing_title_anidb_external_ids(limit)
            .await
    }

    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        self.sql.mark_title_metadata_hydration_due_now(id).await
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        self.sql
            .schedule_title_metadata_hydration_retry(id, next_attempt_at, attempt_count)
            .await
    }

    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        self.sql
            .clear_title_metadata_hydration_retry_state(id)
            .await
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        self.sql.update_monitored(id, monitored).await
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        self.sql.update_metadata(id, name, facet, tags).await
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        self.sql.update_title_hydrated_metadata(id, metadata).await
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<scryer_domain::ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        self.sql.replace_match_state(id, external_ids, tags).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.sql.delete(id).await
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        self.sql.set_folder_path(id, folder_path).await
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        self.sql.clear_folder_path(id).await
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        self.sql.clear_metadata_language_for_all().await
    }
}

#[async_trait]
impl<S: LibrarySql> LibraryRepository for CatalogStore<S> {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        self.sql.list(facet).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        self.sql.get_by_id(id).await
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        self.sql.default_for_facet(facet).await
    }

    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        self.sql.create(library, roots).await
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        self.sql.update(library_id, name, slug, roots).await
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        self.sql.delete_library(library_id).await
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        self.sql.app_permission_mask_for_user(user_id).await
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        self.sql
            .set_app_permission_mask_for_user(user_id, permissions)
            .await
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        self.sql.permission_masks_for_user(user_id).await
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        self.sql.set_grants_for_user(user_id, grants).await
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        self.sql.title_library_id(title_id).await
    }
}

#[async_trait]
impl<S: ShowSql> ShowRepository for CatalogStore<S> {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        self.sql.list_collections_for_title(title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        self.sql.list_collection_external_ids(collection_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        self.sql.list_collections_for_titles(title_ids).await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        self.sql.list_primary_collection_summaries(title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        self.sql.get_collection_by_id(collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        self.sql.get_collection_by_ordered_path(ordered_path).await
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        self.sql.create_collection(collection).await
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        self.sql.update_collection(collection_id, update).await
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        self.sql
            .update_collection_interstitial_movie(collection_id, interstitial_movie)
            .await
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        self.sql
            .update_collection_specials_movies(collection_id, specials_movies)
            .await
    }

    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        self.sql
            .update_interstitial_season_episode(collection_id, season_episode)
            .await
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        self.sql
            .set_collection_episodes_monitored(collection_id, monitored)
            .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        self.sql.delete_collection(collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_collections_for_title(title_id).await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        self.sql.list_episodes_for_collection(collection_id).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        self.sql.list_episodes_for_title(title_id).await
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        self.sql.list_episode_external_ids(episode_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        self.sql.get_episode_by_id(episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.sql.create_episode(episode).await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        self.sql.update_episode(episode_id, update).await
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        self.sql.delete_episode(episode_id).await
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        self.sql.delete_episodes_for_title(title_id).await
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.sql
            .find_episode_by_title_and_numbers(title_id, season_number, episode_number)
            .await
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        self.sql
            .find_episode_by_title_and_absolute_number(title_id, absolute_number)
            .await
    }

    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        self.sql
            .list_episodes_in_date_range(start_date, end_date)
            .await
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        self.sql
            .replace_anibridge_scoped_external_ids_for_title(title_id, collection_ids, episode_ids)
            .await
    }
}

#[async_trait]
impl<S: UserSql> UserRepository for CatalogStore<S> {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        self.sql.get_by_username(username).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        self.sql.get_by_id(id).await
    }

    async fn create(&self, user_record: User) -> AppResult<User> {
        self.sql.create(user_record).await
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        self.sql.list_all().await
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        self.sql.update_entitlements(id, entitlements).await
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        self.sql.update_password_hash(id, password_hash).await
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        self.sql.delete(id).await
    }
}
