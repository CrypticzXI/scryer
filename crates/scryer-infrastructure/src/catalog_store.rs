use async_trait::async_trait;
use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeUpdate, LibraryRepository, LibraryRootDraft,
    PrimaryCollectionSummary, ScopedExternalId, ShowRepository, UserRepository,
};
use scryer_domain::{
    AppPermissionMask, CalendarEpisode, Collection, Entitlement, Episode, Library, LibraryGrant,
    MediaFacet, User,
};
use std::collections::HashMap;

use crate::SqliteServices;
use crate::queries::{library, show, sql_runtime::SqlTarget, title, user};

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
    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
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
    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()>;
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
        show::list_collections_for_title_query(SqlTarget::Sqlite(&self.pool), title_id).await
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        show::list_collection_external_ids_query(SqlTarget::Sqlite(&self.pool), collection_id).await
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        show::list_collections_for_titles_query(SqlTarget::Sqlite(&self.pool), title_ids).await
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        title::list_primary_collection_summaries_query(&self.pool, title_ids).await
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        show::get_collection_by_id_query(SqlTarget::Sqlite(&self.pool), collection_id).await
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        show::get_collection_by_ordered_path_query(SqlTarget::Sqlite(&self.pool), ordered_path)
            .await
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

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        show::set_collections_monitored_query(
            SqlTarget::Sqlite(&self.pool),
            collection_ids,
            monitored,
        )
        .await
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        self.db.delete_collection(collection_id).await
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        self.db.delete_collections_for_title(title_id).await
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        show::list_episodes_for_collection_query(SqlTarget::Sqlite(&self.pool), collection_id).await
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        show::list_episodes_for_title_query(SqlTarget::Sqlite(&self.pool), title_id).await
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        show::list_episode_external_ids_query(SqlTarget::Sqlite(&self.pool), episode_id).await
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        show::get_episode_by_id_query(SqlTarget::Sqlite(&self.pool), episode_id).await
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.db.create_episode(&episode).await
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        self.db.update_episode(episode_id, update).await
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        show::set_episodes_monitored_query(SqlTarget::Sqlite(&self.pool), episode_ids, monitored)
            .await
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
        show::find_episode_by_title_and_absolute_number_query(
            SqlTarget::Sqlite(&self.pool),
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

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        self.sql
            .set_collections_monitored(collection_ids, monitored)
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

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        self.sql
            .set_episodes_monitored(episode_ids, monitored)
            .await
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
