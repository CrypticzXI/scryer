use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CollectionUpdate, EpisodeUpdate, LibraryRootDraft,
    PrimaryCollectionSummary, ScopedExternalId,
};
use scryer_domain::{
    AppPermissionMask, CalendarEpisode, Collection, Entitlement, Episode, Id, Library,
    LibraryGrant, LibraryPermissionMask, LibraryRoot, MediaFacet, User,
};
use sqlx::{Row, types::Json};

use crate::catalog_store::{CatalogStore, LibrarySql, ShowSql, TitleDatastoreSql, UserSql};
use crate::queries::{
    show,
    sql_runtime::{SqlTarget, StoreDatastore},
    title::collection_interstitial_column_values,
};

pub type PostgresCatalogStore = CatalogStore<PostgresCatalogSql>;

#[derive(Clone)]
pub struct PostgresCatalogSql {
    pool: sqlx::PgPool,
}

impl PostgresCatalogStore {
    pub fn new(db: &super::PostgresServices) -> Self {
        CatalogStore::from_sql(PostgresCatalogSql::new(db.pool().clone()))
    }
}

impl PostgresCatalogSql {
    fn new(pool: sqlx::PgPool) -> Self {
        Self { pool }
    }
}

impl TitleDatastoreSql for PostgresCatalogSql {
    fn title_datastore(&self) -> StoreDatastore {
        StoreDatastore::Postgres {
            pool: self.pool.clone(),
        }
    }
}

#[async_trait]
impl LibrarySql for PostgresCatalogSql {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        let rows = if let Some(facet) = facet {
            sqlx::query(
                "SELECT id, facet, name, slug, is_default, created_at, updated_at
                   FROM libraries
                  WHERE facet = $1
                  ORDER BY is_default DESC, name ASC",
            )
            .bind(facet.as_str())
            .fetch_all(&self.pool)
            .await
        } else {
            sqlx::query(
                "SELECT id, facet, name, slug, is_default, created_at, updated_at
                   FROM libraries
                  ORDER BY facet, is_default DESC, name ASC",
            )
            .fetch_all(&self.pool)
            .await
        }
        .map_err(repo_err)?;

        let mut libraries = Vec::with_capacity(rows.len());
        for row in rows {
            libraries.push(self.library_from_row(&row).await?);
        }
        Ok(libraries)
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        let row = sqlx::query(
            "SELECT id, facet, name, slug, is_default, created_at, updated_at
               FROM libraries
              WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        match row {
            Some(row) => self.library_from_row(&row).await.map(Some),
            None => Ok(None),
        }
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        let row = sqlx::query(
            "SELECT id, facet, name, slug, is_default, created_at, updated_at
               FROM libraries
              WHERE facet = $1 AND is_default = TRUE
              ORDER BY name ASC
              LIMIT 1",
        )
        .bind(facet.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;

        match row {
            Some(row) => self.library_from_row(&row).await.map(Some),
            None => Ok(None),
        }
    }

    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        let now = Utc::now();
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&library.id)
        .bind(library.facet.as_str())
        .bind(&library.name)
        .bind(&library.slug)
        .bind(library.is_default)
        .bind(library.created_at)
        .bind(library.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        for root in roots {
            sqlx::query(
                "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(Id::new().0)
            .bind(&library.id)
            .bind(&root.path)
            .bind(normalize_path(&root.path))
            .bind(root.is_default)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }

        tx.commit().await.map_err(repo_err)?;
        LibrarySql::get_by_id(self, &library.id)
            .await?
            .ok_or_else(|| AppError::Repository("created library was not found".to_string()))
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let now = Utc::now();
        sqlx::query("UPDATE libraries SET name = $2, slug = $3, updated_at = $4 WHERE id = $1")
            .bind(library_id)
            .bind(name)
            .bind(slug)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        sqlx::query("DELETE FROM library_roots WHERE library_id = $1")
            .bind(library_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        for root in roots {
            sqlx::query(
                "INSERT INTO library_roots (id, library_id, path, normalized_path, is_default, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(Id::new().0)
            .bind(library_id)
            .bind(&root.path)
            .bind(normalize_path(&root.path))
            .bind(root.is_default)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        tx.commit().await.map_err(repo_err)?;
        LibrarySql::get_by_id(self, library_id)
            .await?
            .ok_or_else(|| AppError::Repository("updated library was not found".to_string()))
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM libraries WHERE id = $1 AND is_default = FALSE")
            .bind(library_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(result.rows_affected() > 0)
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        let bits: Option<i64> = sqlx::query_scalar(
            "SELECT permission_mask FROM user_app_permission_masks WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(AppPermissionMask::from_bits_retain(
            bits.unwrap_or_default().max(0) as u64,
        ))
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
             VALUES ($1, $2, $3)
             ON CONFLICT (user_id) DO UPDATE
                SET permission_mask = EXCLUDED.permission_mask,
                    updated_at = EXCLUDED.updated_at",
        )
        .bind(user_id)
        .bind(permissions.bits() as i64)
        .bind(Utc::now())
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        let rows = sqlx::query(
            "SELECT user_id, library_id, permission_mask
               FROM user_library_permission_masks
              WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|row| {
                Ok(LibraryGrant {
                    user_id: row.try_get("user_id")?,
                    library_id: row.try_get("library_id")?,
                    permissions: LibraryPermissionMask::from_bits_retain(
                        row.try_get::<i64, _>("permission_mask")?.max(0) as u64,
                    ),
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(repo_err)
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        let library_ids = grants
            .iter()
            .map(|grant| grant.library_id.clone())
            .collect::<Vec<_>>();
        for grant in &grants {
            sqlx::query(
                "INSERT INTO user_library_permission_masks (user_id, library_id, permission_mask, updated_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (user_id, library_id)
                 DO UPDATE SET
                     permission_mask = EXCLUDED.permission_mask,
                     updated_at = EXCLUDED.updated_at",
            )
            .bind(user_id)
            .bind(&grant.library_id)
            .bind(grant.permissions.bits() as i64)
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        if library_ids.is_empty() {
            sqlx::query("DELETE FROM user_library_permission_masks WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_err)?;
        } else {
            sqlx::query(
                "DELETE FROM user_library_permission_masks
                  WHERE user_id = $1
                    AND NOT (library_id = ANY($2))",
            )
            .bind(user_id)
            .bind(&library_ids)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        sqlx::query_scalar("SELECT library_id FROM titles WHERE id = $1")
            .bind(title_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)
    }
}

impl PostgresCatalogSql {
    async fn library_from_row(&self, row: &sqlx::postgres::PgRow) -> AppResult<Library> {
        let id: String = row.try_get("id").map_err(repo_err)?;
        let roots = self.list_roots_for_library(&id).await?;
        Ok(Library {
            id,
            facet: parse_facet(
                row.try_get::<String, _>("facet")
                    .map_err(repo_err)?
                    .as_str(),
            )?,
            name: row.try_get("name").map_err(repo_err)?,
            slug: row.try_get("slug").map_err(repo_err)?,
            is_default: row.try_get("is_default").map_err(repo_err)?,
            roots,
            created_at: row.try_get("created_at").map_err(repo_err)?,
            updated_at: row.try_get("updated_at").map_err(repo_err)?,
        })
    }

    async fn list_roots_for_library(&self, library_id: &str) -> AppResult<Vec<LibraryRoot>> {
        let rows = sqlx::query(
            "SELECT id, library_id, path, is_default, created_at, updated_at
               FROM library_roots
              WHERE library_id = $1
              ORDER BY is_default DESC, path ASC",
        )
        .bind(library_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.into_iter()
            .map(|row| {
                Ok(LibraryRoot {
                    id: row.try_get("id")?,
                    library_id: row.try_get("library_id")?,
                    path: row.try_get("path")?,
                    is_default: row.try_get("is_default")?,
                    created_at: row.try_get("created_at")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()
            .map_err(repo_err)
    }
}

#[async_trait]
impl UserSql for PostgresCatalogSql {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        self.user_by_clause("username", username).await
    }

    async fn create(&self, user: User) -> AppResult<User> {
        let entitlements = serde_json::to_value(&user.entitlements).map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO users (id, username, entitlements, password_hash)
             VALUES ($1, $2, $3::jsonb, $4)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(entitlements)
        .bind(&user.password_hash)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(user)
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, entitlements, password_hash FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(user_from_row).collect()
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        self.user_by_clause("id", id).await
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let entitlements_json = serde_json::to_value(entitlements).map_err(repo_err)?;
        sqlx::query("UPDATE users SET entitlements = $2::jsonb WHERE id = $1")
            .bind(id)
            .bind(entitlements_json)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        UserSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("updated user was not found".to_string()))
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        sqlx::query("UPDATE users SET password_hash = $2 WHERE id = $1")
            .bind(id)
            .bind(password_hash)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        UserSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("updated user was not found".to_string()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

impl PostgresCatalogSql {
    async fn user_by_clause(&self, column: &str, value: &str) -> AppResult<Option<User>> {
        let sql = format!(
            "SELECT id, username, entitlements, password_hash FROM users WHERE {column} = $1"
        );
        let row = sqlx::query(&sql)
            .bind(value)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(user_from_row).transpose()
    }
}

#[async_trait]
impl ShowSql for PostgresCatalogSql {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        show::list_collections_for_title_query(SqlTarget::Postgres(&self.pool), title_id).await
    }
    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        show::list_collection_external_ids_query(SqlTarget::Postgres(&self.pool), collection_id)
            .await
    }
    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        show::list_collections_for_titles_query(SqlTarget::Postgres(&self.pool), title_ids).await
    }
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        show::get_collection_by_id_query(SqlTarget::Postgres(&self.pool), collection_id).await
    }
    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        show::get_collection_by_ordered_path_query(SqlTarget::Postgres(&self.pool), ordered_path)
            .await
    }
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        let interstitial = collection_interstitial_column_values(&collection)?;
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
              interstitial_movie_anidb_id, interstitial_season_episode, special_movies_json,
              monitored, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
                     $16, $17, $18, $19, $20, $21, $22::jsonb, $23, $24, $25, $26, $27,
                     $28, $29, $30, $31, $32, $33, $34::jsonb, $35, $36, NOW())
             ON CONFLICT (id) DO UPDATE SET
                collection_type = EXCLUDED.collection_type,
                collection_index = EXCLUDED.collection_index,
                label = EXCLUDED.label,
                ordered_path = EXCLUDED.ordered_path,
                narrative_order = EXCLUDED.narrative_order,
                first_episode_number = EXCLUDED.first_episode_number,
                last_episode_number = EXCLUDED.last_episode_number,
                interstitial_tvdb_id = EXCLUDED.interstitial_tvdb_id,
                interstitial_name = EXCLUDED.interstitial_name,
                interstitial_slug = EXCLUDED.interstitial_slug,
                interstitial_year = EXCLUDED.interstitial_year,
                interstitial_content_status = EXCLUDED.interstitial_content_status,
                interstitial_overview = EXCLUDED.interstitial_overview,
                interstitial_poster_url = EXCLUDED.interstitial_poster_url,
                interstitial_language = EXCLUDED.interstitial_language,
                interstitial_runtime_minutes = EXCLUDED.interstitial_runtime_minutes,
                interstitial_sort_title = EXCLUDED.interstitial_sort_title,
                interstitial_imdb_id = EXCLUDED.interstitial_imdb_id,
                interstitial_genres_json = EXCLUDED.interstitial_genres_json,
                interstitial_studio = EXCLUDED.interstitial_studio,
                interstitial_digital_release_date = EXCLUDED.interstitial_digital_release_date,
                interstitial_association_confidence = EXCLUDED.interstitial_association_confidence,
                interstitial_continuity_status = EXCLUDED.interstitial_continuity_status,
                interstitial_movie_form = EXCLUDED.interstitial_movie_form,
                interstitial_confidence = EXCLUDED.interstitial_confidence,
                interstitial_signal_summary = EXCLUDED.interstitial_signal_summary,
                interstitial_placement = EXCLUDED.interstitial_placement,
                interstitial_movie_tmdb_id = EXCLUDED.interstitial_movie_tmdb_id,
                interstitial_movie_mal_id = EXCLUDED.interstitial_movie_mal_id,
                interstitial_movie_anidb_id = EXCLUDED.interstitial_movie_anidb_id,
                interstitial_season_episode = EXCLUDED.interstitial_season_episode,
                special_movies_json = EXCLUDED.special_movies_json,
                monitored = EXCLUDED.monitored,
                updated_at = NOW()",
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
        .bind(interstitial.genres_json.map(Json))
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
        .bind(Json(interstitial.special_movies_json))
        .bind(collection.monitored)
        .bind(collection.created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(collection)
    }
    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let mut collection = self
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound("collection not found".into()))?;
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
        self.create_collection(collection.clone()).await?;
        Ok(collection)
    }
    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        let mut collection = self
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound("collection not found".into()))?;
        collection.interstitial_movie = Some(interstitial_movie);
        self.create_collection(collection.clone()).await?;
        Ok(collection)
    }
    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        let mut collection = self
            .get_collection_by_id(collection_id)
            .await?
            .ok_or_else(|| AppError::NotFound("collection not found".into()))?;
        collection.specials_movies = specials_movies;
        self.create_collection(collection.clone()).await?;
        Ok(collection)
    }
    async fn update_interstitial_season_episode(
        &self,
        collection_id: &str,
        season_episode: Option<String>,
    ) -> AppResult<()> {
        show::update_interstitial_season_episode_query(
            SqlTarget::Postgres(&self.pool),
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
        show::set_collection_episodes_monitored_query(
            SqlTarget::Postgres(&self.pool),
            collection_id,
            monitored,
        )
        .await
    }
    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        show::set_collections_monitored_query(
            SqlTarget::Postgres(&self.pool),
            collection_ids,
            monitored,
        )
        .await
    }
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        show::delete_collection_query(SqlTarget::Postgres(&self.pool), collection_id).await
    }
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        show::delete_collections_for_title_query(SqlTarget::Postgres(&self.pool), title_id).await
    }
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        show::list_episodes_for_collection_query(SqlTarget::Postgres(&self.pool), collection_id)
            .await
    }
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        show::list_episodes_for_title_query(SqlTarget::Postgres(&self.pool), title_id).await
    }
    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        show::list_episode_external_ids_query(SqlTarget::Postgres(&self.pool), episode_id).await
    }
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        show::get_episode_by_id_query(SqlTarget::Postgres(&self.pool), episode_id).await
    }
    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        sqlx::query(
            "INSERT INTO episodes
             (id, title_id, collection_id, episode_type, episode_number, season_number,
              episode_label, title, air_date, duration_seconds, has_multi_audio, has_subtitle,
              is_filler, is_recap, absolute_number, overview, tvdb_id, monitored, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, NOW())
             ON CONFLICT (id) DO UPDATE SET
                collection_id = EXCLUDED.collection_id,
                episode_type = EXCLUDED.episode_type,
                episode_number = EXCLUDED.episode_number,
                season_number = EXCLUDED.season_number,
                episode_label = EXCLUDED.episode_label,
                title = EXCLUDED.title,
                air_date = EXCLUDED.air_date,
                duration_seconds = EXCLUDED.duration_seconds,
                has_multi_audio = EXCLUDED.has_multi_audio,
                has_subtitle = EXCLUDED.has_subtitle,
                is_filler = EXCLUDED.is_filler,
                is_recap = EXCLUDED.is_recap,
                absolute_number = EXCLUDED.absolute_number,
                overview = EXCLUDED.overview,
                tvdb_id = EXCLUDED.tvdb_id,
                monitored = EXCLUDED.monitored,
                updated_at = NOW()",
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
        .bind(episode.has_multi_audio)
        .bind(episode.has_subtitle)
        .bind(episode.is_filler)
        .bind(episode.is_recap)
        .bind(&episode.absolute_number)
        .bind(&episode.overview)
        .bind(&episode.tvdb_id)
        .bind(episode.monitored)
        .bind(episode.created_at)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(episode)
    }
    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        let mut episode = self
            .get_episode_by_id(episode_id)
            .await?
            .ok_or_else(|| AppError::NotFound("episode not found".into()))?;
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
        self.create_episode(episode.clone()).await?;
        Ok(episode)
    }
    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        show::set_episodes_monitored_query(SqlTarget::Postgres(&self.pool), episode_ids, monitored)
            .await
    }
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        show::delete_episode_query(SqlTarget::Postgres(&self.pool), episode_id).await
    }
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        show::delete_episodes_for_title_query(SqlTarget::Postgres(&self.pool), title_id).await
    }
    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let row = sqlx::query(
            "SELECT * FROM episodes
              WHERE title_id = $1 AND season_number = $2 AND episode_number = $3
              LIMIT 1",
        )
        .bind(title_id)
        .bind(season_number)
        .bind(episode_number)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_episode).transpose()
    }
    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        show::find_episode_by_title_and_absolute_number_query(
            SqlTarget::Postgres(&self.pool),
            title_id,
            absolute_number,
        )
        .await
    }
    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        if title_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT DISTINCT ON (title_id) title_id, label, ordered_path
               FROM collections
              WHERE title_id = ANY($1)
              ORDER BY title_id,
                       CASE collection_type WHEN 'season' THEN 0 WHEN 'movie' THEN 1 ELSE 2 END,
                       collection_index ASC",
        )
        .bind(title_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(PrimaryCollectionSummary {
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    label: row.try_get("label").unwrap_or(None),
                    ordered_path: row.try_get("ordered_path").unwrap_or(None),
                })
            })
            .collect()
    }
    async fn list_episodes_in_date_range(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        let rows = sqlx::query(
            "SELECT e.id, e.title_id, t.library_id, l.name AS library_name, l.slug AS library_slug,
                    t.name AS title_name, t.slug AS title_slug, t.facet AS title_facet,
                    e.season_number, e.episode_number, e.title AS episode_title,
                    e.air_date, e.monitored
               FROM episodes e
               JOIN titles t ON t.id = e.title_id
               LEFT JOIN libraries l ON l.id = t.library_id
              WHERE e.air_date >= $1 AND e.air_date <= $2
              ORDER BY e.air_date ASC, t.name ASC, e.season_number ASC, e.episode_number ASC",
        )
        .bind(start_date)
        .bind(end_date)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(CalendarEpisode {
                    id: row.try_get("id").map_err(repo_err)?,
                    title_id: row.try_get("title_id").map_err(repo_err)?,
                    library_id: row.try_get("library_id").map_err(repo_err)?,
                    library_name: row.try_get("library_name").unwrap_or(None),
                    library_slug: row.try_get("library_slug").unwrap_or(None),
                    title_name: row.try_get("title_name").map_err(repo_err)?,
                    title_slug: row.try_get("title_slug").unwrap_or(None),
                    title_facet: row.try_get("title_facet").map_err(repo_err)?,
                    season_number: row.try_get("season_number").unwrap_or(None),
                    episode_number: row.try_get("episode_number").unwrap_or(None),
                    episode_title: row.try_get("episode_title").unwrap_or(None),
                    air_date: row.try_get("air_date").unwrap_or(None),
                    monitored: row.try_get("monitored").unwrap_or(true),
                })
            })
            .collect()
    }
    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        show::replace_anibridge_scoped_external_ids_for_title(
            SqlTarget::Postgres(&self.pool),
            title_id,
            &collection_ids,
            &episode_ids,
        )
        .await
    }
}

fn user_from_row(row: &sqlx::postgres::PgRow) -> AppResult<User> {
    let entitlements_value: serde_json::Value = row.try_get("entitlements").map_err(repo_err)?;
    let entitlements = serde_json::from_value(entitlements_value)
        .map_err(|error| AppError::Repository(error.to_string()))?;
    Ok(User {
        id: row.try_get("id").map_err(repo_err)?,
        username: row.try_get("username").map_err(repo_err)?,
        password_hash: row.try_get("password_hash").map_err(repo_err)?,
        entitlements,
        authorization: Default::default(),
    })
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
}

fn parse_facet(value: &str) -> AppResult<MediaFacet> {
    MediaFacet::parse(value)
        .ok_or_else(|| AppError::Repository(format!("unknown media facet '{value}'")))
}

fn normalize_path(path: &str) -> String {
    path.trim_end_matches('/').to_ascii_lowercase()
}

fn row_timestamp(row: &sqlx::postgres::PgRow, column: &str) -> AppResult<DateTime<Utc>> {
    row.try_get(column).map_err(repo_err)
}

fn row_to_episode(row: &sqlx::postgres::PgRow) -> AppResult<Episode> {
    let episode_type_raw: String = row.try_get("episode_type").map_err(repo_err)?;
    let episode_type = scryer_domain::EpisodeType::parse(&episode_type_raw)
        .ok_or_else(|| AppError::Repository(format!("invalid episode type {episode_type_raw}")))?;
    Ok(Episode {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        collection_id: row.try_get("collection_id").unwrap_or(None),
        episode_type,
        episode_number: row.try_get("episode_number").unwrap_or(None),
        season_number: row.try_get("season_number").unwrap_or(None),
        episode_label: row.try_get("episode_label").unwrap_or(None),
        title: row.try_get("title").unwrap_or(None),
        air_date: row.try_get("air_date").unwrap_or(None),
        duration_seconds: row.try_get("duration_seconds").unwrap_or(None),
        has_multi_audio: row.try_get("has_multi_audio").unwrap_or(false),
        has_subtitle: row.try_get("has_subtitle").unwrap_or(false),
        is_filler: row.try_get("is_filler").unwrap_or(false),
        is_recap: row.try_get("is_recap").unwrap_or(false),
        absolute_number: row.try_get("absolute_number").unwrap_or(None),
        overview: row.try_get("overview").unwrap_or(None),
        tvdb_id: row.try_get("tvdb_id").unwrap_or(None),
        monitored: row.try_get("monitored").unwrap_or(true),
        created_at: row_timestamp(row, "created_at")?,
    })
}
