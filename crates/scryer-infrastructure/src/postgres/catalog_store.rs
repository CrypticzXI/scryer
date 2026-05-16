use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, EpisodeUpdate, LibraryRootDraft,
    PendingTitleHydration, PrimaryCollectionSummary, ScopedExternalId, TitleMetadataUpdate,
    persisted_records::{
        PersistedTitleDecodeOptions, PersistedTitleReadMode, finalize_persisted_title,
    },
};
use scryer_domain::{
    AppPermissionMask, CalendarEpisode, Collection, Entitlement, Episode, ExternalId, Id, Library,
    LibraryGrant, LibraryPermissionMask, LibraryRoot, MediaFacet, TaggedAlias, Title, User,
};
use serde_json::Value;
use sqlx::{Row, types::Json};

use crate::catalog_store::{CatalogStore, LibrarySql, ShowSql, TitleSql, UserSql};
use crate::postgres::timestamp::parse_rfc3339_timestamp;
use crate::queries::{
    show,
    sql_runtime::SqlTarget,
    title::collection_interstitial_column_values,
    title_search::{self, replace_title_search_projection_pg_tx},
};
use crate::title_images::normalized_base_path_from_env;

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

    async fn list_for_libraries_internal(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
        include_external_ids: bool,
    ) -> AppResult<Vec<Title>> {
        if library_ids.is_empty() {
            return Ok(Vec::new());
        }

        let query = normalized_query(query);
        let rows = match (facet, query.as_deref()) {
            (Some(facet), Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles t
                       JOIN (
                            SELECT title_id,
                                   MIN(
                                     CASE
                                       WHEN normalized_term = $3 THEN 0
                                       WHEN normalized_term LIKE $3 || '%' THEN 1000
                                       WHEN normalized_term LIKE '%' || $3 || '%' THEN 2000
                                       ELSE 3000
                                     END + weight
                                   ) AS rank
                              FROM title_search_terms
                             WHERE facet = $1
                               AND normalized_term ILIKE $2
                             GROUP BY title_id
                       ) search ON search.title_id = t.id
                      WHERE t.library_id = ANY($4)
                      ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(facet.as_str())
                .bind(pattern)
                .bind(normalized)
                .bind(library_ids)
                .fetch_all(&self.pool)
                .await
            }
            (Some(facet), None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles
                      WHERE facet = $1
                        AND library_id = ANY($2)
                      ORDER BY lower(name), id"
                ))
                .bind(facet.as_str())
                .bind(library_ids)
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles t
                       JOIN (
                            SELECT title_id,
                                   MIN(
                                     CASE
                                       WHEN normalized_term = $2 THEN 0
                                       WHEN normalized_term LIKE $2 || '%' THEN 1000
                                       WHEN normalized_term LIKE '%' || $2 || '%' THEN 2000
                                       ELSE 3000
                                     END + weight
                                   ) AS rank
                              FROM title_search_terms
                             WHERE normalized_term ILIKE $1
                             GROUP BY title_id
                       ) search ON search.title_id = t.id
                      WHERE t.library_id = ANY($3)
                      ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(pattern)
                .bind(normalized)
                .bind(library_ids)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles
                      WHERE library_id = ANY($1)
                      ORDER BY lower(name), id"
                ))
                .bind(library_ids)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(repo_err)?;

        decode_title_rows(
            &rows,
            PersistedTitleReadMode::Presentation,
            include_external_ids,
        )
    }

    async fn get_by_id_internal(
        &self,
        id: &str,
        include_external_ids: bool,
    ) -> AppResult<Option<Title>> {
        let projection = title_columns(None);
        let row = sqlx::query(&format!("SELECT {projection} FROM titles WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        decode_optional_title_row(
            row.as_ref(),
            PersistedTitleReadMode::Presentation,
            include_external_ids,
        )
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
        .map_err(|error| AppError::Repository(error.to_string()))?;

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
        .map_err(|error| AppError::Repository(error.to_string()))?;

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
        .map_err(|error| AppError::Repository(error.to_string()))?;

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
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        let now = Utc::now();
        sqlx::query("UPDATE libraries SET name = $2, slug = $3, updated_at = $4 WHERE id = $1")
            .bind(library_id)
            .bind(name)
            .bind(slug)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query("DELETE FROM library_roots WHERE library_id = $1")
            .bind(library_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        LibrarySql::get_by_id(self, library_id)
            .await?
            .ok_or_else(|| AppError::Repository("updated library was not found".to_string()))
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let result = sqlx::query("DELETE FROM libraries WHERE id = $1 AND is_default = FALSE")
            .bind(library_id)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(result.rows_affected() > 0)
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        let bits: Option<i64> = sqlx::query_scalar(
            "SELECT permission_mask FROM user_app_permission_masks WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
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
        .map_err(|error| AppError::Repository(error.to_string()))?;
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
        .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        if library_ids.is_empty() {
            sqlx::query("DELETE FROM user_library_permission_masks WHERE user_id = $1")
                .bind(user_id)
                .execute(&mut *tx)
                .await
                .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))?;
        }
        tx.commit()
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(())
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        sqlx::query_scalar("SELECT library_id FROM titles WHERE id = $1")
            .bind(title_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))
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
        .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))
    }
}

#[async_trait]
impl UserSql for PostgresCatalogSql {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        self.user_by_clause("username", username).await
    }

    async fn create(&self, user: User) -> AppResult<User> {
        let entitlements = serde_json::to_value(&user.entitlements)
            .map_err(|error| AppError::Repository(error.to_string()))?;
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
        .map_err(|error| AppError::Repository(error.to_string()))?;
        Ok(user)
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        let rows = sqlx::query(
            "SELECT id, username, entitlements, password_hash FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|error| AppError::Repository(error.to_string()))?;
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
        let entitlements_json = serde_json::to_value(entitlements)
            .map_err(|error| AppError::Repository(error.to_string()))?;
        sqlx::query("UPDATE users SET entitlements = $2::jsonb WHERE id = $1")
            .bind(id)
            .bind(entitlements_json)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))?;
        UserSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("updated user was not found".to_string()))
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|error| AppError::Repository(error.to_string()))?;
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
            .map_err(|error| AppError::Repository(error.to_string()))?;
        row.as_ref().map(user_from_row).transpose()
    }

    async fn upsert_title_record(&self, title: &Title) -> AppResult<()> {
        let tags = serde_json::to_value(&title.tags).map_err(repo_err)?;
        let external_ids = serde_json::to_value(&title.external_ids).map_err(repo_err)?;
        let genres = serde_json::to_value(&title.genres).map_err(repo_err)?;
        let aliases = serde_json::to_value(&title.aliases).map_err(repo_err)?;
        let tagged_aliases = serde_json::to_value(&title.tagged_aliases).map_err(repo_err)?;
        let metadata_hydration_next_attempt_at =
            if title.metadata_fetched_at.is_none() && title_has_tvdb_external_id(title) {
                Some(Utc::now())
            } else {
                None
            };
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO titles (
                id, library_id, name, facet, monitored, tags, external_ids, created_by, created_at,
                year, overview, poster_url, banner_url, background_url, sort_title, slug, imdb_id,
                runtime_minutes, genres, content_status, language, first_aired, network, studio,
                country, aliases, metadata_language, metadata_fetched_at, min_availability,
                digital_release_date, folder_path, tagged_aliases_json,
                metadata_hydration_next_attempt_at, metadata_hydration_attempt_count, updated_at
             )
             VALUES (
                $1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8, $9,
                $10, $11, $12, $13, $14, $15, $16, $17,
                $18, $19::jsonb, $20, $21, $22, $23, $24,
                $25, $26::jsonb, $27, $28, $29,
                $30, $31, $32::jsonb, $33, $34, $35
             )
             ON CONFLICT (id) DO UPDATE SET
                library_id = EXCLUDED.library_id,
                name = EXCLUDED.name,
                facet = EXCLUDED.facet,
                monitored = EXCLUDED.monitored,
                tags = EXCLUDED.tags,
                external_ids = EXCLUDED.external_ids,
                created_by = EXCLUDED.created_by,
                created_at = EXCLUDED.created_at,
                year = EXCLUDED.year,
                overview = EXCLUDED.overview,
                poster_url = EXCLUDED.poster_url,
                banner_url = EXCLUDED.banner_url,
                background_url = EXCLUDED.background_url,
                sort_title = EXCLUDED.sort_title,
                slug = EXCLUDED.slug,
                imdb_id = EXCLUDED.imdb_id,
                runtime_minutes = EXCLUDED.runtime_minutes,
                genres = EXCLUDED.genres,
                content_status = EXCLUDED.content_status,
                language = EXCLUDED.language,
                first_aired = EXCLUDED.first_aired,
                network = EXCLUDED.network,
                studio = EXCLUDED.studio,
                country = EXCLUDED.country,
                aliases = EXCLUDED.aliases,
                metadata_language = EXCLUDED.metadata_language,
                metadata_fetched_at = EXCLUDED.metadata_fetched_at,
                min_availability = EXCLUDED.min_availability,
                digital_release_date = EXCLUDED.digital_release_date,
                folder_path = EXCLUDED.folder_path,
                tagged_aliases_json = EXCLUDED.tagged_aliases_json,
                metadata_hydration_next_attempt_at = CASE
                    WHEN EXCLUDED.metadata_fetched_at IS NOT NULL THEN NULL
                    ELSE COALESCE(titles.metadata_hydration_next_attempt_at, EXCLUDED.metadata_hydration_next_attempt_at)
                END,
                metadata_hydration_attempt_count = CASE
                    WHEN EXCLUDED.metadata_fetched_at IS NOT NULL THEN 0
                    ELSE titles.metadata_hydration_attempt_count
                END,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&title.id)
        .bind(&title.library_id)
        .bind(&title.name)
        .bind(title.facet.as_str())
        .bind(title.monitored)
        .bind(tags)
        .bind(external_ids)
        .bind(&title.created_by)
        .bind(title.created_at)
        .bind(title.year)
        .bind(&title.overview)
        .bind(&title.poster_url)
        .bind(&title.banner_url)
        .bind(&title.background_url)
        .bind(&title.sort_title)
        .bind(&title.slug)
        .bind(&title.imdb_id)
        .bind(title.runtime_minutes)
        .bind(genres)
        .bind(&title.content_status)
        .bind(&title.language)
        .bind(&title.first_aired)
        .bind(&title.network)
        .bind(&title.studio)
        .bind(&title.country)
        .bind(aliases)
        .bind(&title.metadata_language)
        .bind(title.metadata_fetched_at)
        .bind(&title.min_availability)
        .bind(&title.digital_release_date)
        .bind(&title.folder_path)
        .bind(tagged_aliases)
        .bind(metadata_hydration_next_attempt_at)
        .bind(0_i64)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        replace_title_search_projection_pg_tx(&mut tx, title).await?;

        tx.commit().await.map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl TitleSql for PostgresCatalogSql {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let query = normalized_query(query);
        let rows = match (facet, query.as_deref()) {
            (Some(facet), Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles t
                       JOIN (
                            SELECT title_id,
                                   MIN(
                                     CASE
                                       WHEN normalized_term = $3 THEN 0
                                       WHEN normalized_term LIKE $3 || '%' THEN 1000
                                       WHEN normalized_term LIKE '%' || $3 || '%' THEN 2000
                                       ELSE 3000
                                     END + weight
                                   ) AS rank
                              FROM title_search_terms
                             WHERE facet = $1
                               AND normalized_term ILIKE $2
                             GROUP BY title_id
                       ) search ON search.title_id = t.id
                      ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(facet.as_str())
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (Some(facet), None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                   FROM titles
                  WHERE facet = $1
                  ORDER BY lower(name), id"
                ))
                .bind(facet.as_str())
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                       FROM titles t
                       JOIN (
                            SELECT title_id,
                                   MIN(
                                     CASE
                                       WHEN normalized_term = $2 THEN 0
                                       WHEN normalized_term LIKE $2 || '%' THEN 1000
                                       WHEN normalized_term LIKE '%' || $2 || '%' THEN 2000
                                       ELSE 3000
                                     END + weight
                                   ) AS rank
                             FROM title_search_terms
                             WHERE normalized_term ILIKE $1
                             GROUP BY title_id
                       ) search ON search.title_id = t.id
                      ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                   FROM titles
                  ORDER BY lower(name), id"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(repo_err)?;

        decode_title_rows(&rows, PersistedTitleReadMode::Presentation, true)
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let projection = title_columns(None);
        let rows = sqlx::query(&format!(
            "SELECT {projection}
               FROM titles
              WHERE EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $1
                       AND external_id->>'value' = ANY($2)
              )
              ORDER BY lower(name), id"
        ))
        .bind(source)
        .bind(values)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        decode_title_rows(&rows, PersistedTitleReadMode::Presentation, true)
    }

    async fn list_for_libraries(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_for_libraries_internal(facet, library_ids, query, true)
            .await
    }

    async fn list_for_libraries_without_external_ids(
        &self,
        facet: Option<MediaFacet>,
        library_ids: &[String],
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list_for_libraries_internal(facet, library_ids, query, false)
            .await
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let query = normalized_query(query);
        let rows = match (facet, query.as_deref()) {
            (Some(facet), Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                           FROM titles t
                           JOIN (
                                SELECT title_id,
                                       MIN(
                                         CASE
                                           WHEN normalized_term = $3 THEN 0
                                           WHEN normalized_term LIKE $3 || '%' THEN 1000
                                           WHEN normalized_term LIKE '%' || $3 || '%' THEN 2000
                                           ELSE 3000
                                         END + weight
                                       ) AS rank
                                  FROM title_search_terms
                                 WHERE facet = $1
                                   AND normalized_term ILIKE $2
                                 GROUP BY title_id
                           ) search ON search.title_id = t.id
                          ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(facet.as_str())
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (Some(facet), None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                           FROM titles
                          WHERE facet = $1
                          ORDER BY lower(name), id"
                ))
                .bind(facet.as_str())
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                let projection = title_columns(Some("t"));
                sqlx::query(&format!(
                    "SELECT {projection}
                           FROM titles t
                           JOIN (
                                SELECT title_id,
                                       MIN(
                                         CASE
                                           WHEN normalized_term = $2 THEN 0
                                           WHEN normalized_term LIKE $2 || '%' THEN 1000
                                           WHEN normalized_term LIKE '%' || $2 || '%' THEN 2000
                                           ELSE 3000
                                         END + weight
                                       ) AS rank
                                  FROM title_search_terms
                                 WHERE normalized_term ILIKE $1
                                 GROUP BY title_id
                           ) search ON search.title_id = t.id
                          ORDER BY search.rank, lower(t.name), t.id"
                ))
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                let projection = title_columns(None);
                sqlx::query(&format!(
                    "SELECT {projection}
                           FROM titles
                          ORDER BY lower(name), id"
                ))
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(repo_err)?;

        decode_title_rows(&rows, PersistedTitleReadMode::Matching, true)
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
        let projection = title_columns(None);
        let row = sqlx::query(&format!(
            "SELECT {projection}
               FROM titles
              WHERE facet = $1 AND lower(slug) = lower($2)
              ORDER BY lower(name), id
              LIMIT 1"
        ))
        .bind(facet.as_str())
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        decode_optional_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
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

        let projection = title_columns(None);
        let row = sqlx::query(&format!(
            "SELECT {projection}
               FROM titles
              WHERE facet = $1
                AND lower(slug) = lower($2)
                AND library_id = ANY($3)
              ORDER BY lower(name), id
              LIMIT 1"
        ))
        .bind(facet.as_str())
        .bind(slug)
        .bind(library_ids)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        decode_optional_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let projection = title_columns(None);
        let row = sqlx::query(&format!(
            "SELECT {projection}
               FROM titles
              WHERE EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $1
                       AND external_id->>'value' = $2
              )
              ORDER BY lower(name), id
              LIMIT 1"
        ))
        .bind(source)
        .bind(value)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        decode_optional_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let projection = title_columns(None);
        let row = sqlx::query(&format!(
            "SELECT {projection}
               FROM titles
              WHERE facet = $1
                AND EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $2
                       AND external_id->>'value' = $3
                )
              ORDER BY lower(name), id
              LIMIT 1"
        ))
        .bind(facet.as_str())
        .bind(source)
        .bind(value)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        decode_optional_title_row(row.as_ref(), PersistedTitleReadMode::Presentation, true)
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        for external_id in &title.external_ids {
            if let Some(existing) = self
                .find_by_external_id_in_facet(
                    title.facet.clone(),
                    &external_id.source,
                    &external_id.value,
                )
                .await?
            {
                return Ok(CreateTitleOutcome {
                    title: existing,
                    reused_existing: true,
                });
            }
        }
        if let Some(slug) = title.slug.as_deref()
            && let Some(existing) = self
                .get_by_facet_and_slug(title.facet.clone(), slug)
                .await?
        {
            return Ok(CreateTitleOutcome {
                title: existing,
                reused_existing: true,
            });
        }
        self.upsert_title_record(&title).await?;
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.upsert_title_record(&title).await?;
        Ok(title)
    }

    async fn list_titles_due_for_hydration(
        &self,
        limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        let projection = title_columns(None);
        let mut builder = sqlx::QueryBuilder::<sqlx::Postgres>::new(format!(
            "SELECT {projection}, metadata_hydration_attempt_count
               FROM titles
              WHERE metadata_fetched_at IS NULL
                AND metadata_hydration_next_attempt_at IS NOT NULL
                AND metadata_hydration_next_attempt_at <= NOW()"
        ));
        if !excluded_facets.is_empty() {
            let facets = excluded_facets
                .iter()
                .map(MediaFacet::as_str)
                .collect::<Vec<_>>();
            builder.push(" AND facet != ALL(");
            builder.push_bind(facets);
            builder.push(")");
        }
        builder.push(" ORDER BY metadata_hydration_next_attempt_at ASC, id ASC LIMIT ");
        builder.push_bind(limit as i64);
        let rows = builder
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        let base_path = normalized_base_path_from_env();
        rows.iter()
            .map(|row| {
                Ok(PendingTitleHydration {
                    title: title_from_row(
                        row,
                        PersistedTitleReadMode::Presentation,
                        true,
                        &base_path,
                    )?,
                    attempt_count: row
                        .try_get("metadata_hydration_attempt_count")
                        .map_err(repo_err)?,
                })
            })
            .collect()
    }
    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            "SELECT id FROM titles
              WHERE facet = 'anime'
                AND EXISTS (
                    SELECT 1 FROM jsonb_array_elements(external_ids) external_id
                     WHERE LOWER(external_id ->> 'source') IN ('tvdb', 'tvdb_id')
                )
                AND NOT EXISTS (
                    SELECT 1 FROM collection_external_ids cei
                      JOIN collections c ON c.id = cei.collection_id
                     WHERE c.title_id = titles.id
                       AND cei.provenance = 'anibridge'
                )
                AND NOT EXISTS (
                    SELECT 1 FROM episode_external_ids eei
                      JOIN episodes e ON e.id = eei.episode_id
                     WHERE e.title_id = titles.id
                       AND eei.provenance = 'anibridge'
                )
              ORDER BY metadata_fetched_at NULLS FIRST, created_at
              LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| row.try_get("id").map_err(repo_err))
            .collect()
    }

    async fn list_anime_title_ids_missing_title_anidb_external_ids(
        &self,
        limit: usize,
    ) -> AppResult<Vec<String>> {
        let rows = sqlx::query(
            "SELECT id FROM titles
              WHERE facet = 'anime'
                AND EXISTS (
                    SELECT 1 FROM jsonb_array_elements(external_ids) external_id
                     WHERE LOWER(external_id ->> 'source') IN ('tvdb', 'tvdb_id')
                )
                AND NOT EXISTS (
                    SELECT 1 FROM jsonb_array_elements(external_ids) external_id
                     WHERE LOWER(external_id ->> 'source') IN ('anidb', 'anidb_id')
                )
              ORDER BY metadata_fetched_at NULLS FIRST, created_at
              LIMIT $1",
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| row.try_get("id").map_err(repo_err))
            .collect()
    }

    async fn mark_title_metadata_hydration_due_now(&self, id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE titles
                SET metadata_hydration_next_attempt_at = NOW(),
                    metadata_hydration_attempt_count = 0
              WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn schedule_title_metadata_hydration_retry(
        &self,
        id: &str,
        next_attempt_at: &str,
        attempt_count: i64,
    ) -> AppResult<()> {
        let next_attempt_at =
            parse_rfc3339_timestamp(next_attempt_at, "titles.metadata_hydration_next_attempt_at")?;
        sqlx::query(
            "UPDATE titles
                SET metadata_hydration_next_attempt_at = $2::timestamptz,
                    metadata_hydration_attempt_count = $3
              WHERE id = $1",
        )
        .bind(id)
        .bind(next_attempt_at)
        .bind(attempt_count)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn clear_title_metadata_hydration_retry_state(&self, id: &str) -> AppResult<()> {
        sqlx::query(
            "UPDATE titles
                SET metadata_hydration_next_attempt_at = NULL,
                    metadata_hydration_attempt_count = 0
              WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let mut title = TitleSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
        title.monitored = monitored;
        self.upsert_title_record(&title).await?;
        Ok(title)
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let mut title = TitleSql::get_by_id(self, id)
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
        self.upsert_title_record(&title).await?;
        Ok(title)
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let mut title = TitleSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
        if let Some(name) = metadata.name.filter(|value| !value.is_empty()) {
            title.name = name;
        }
        if metadata.year.is_some() {
            title.year = metadata.year;
        }
        merge_optional_text(&mut title.overview, metadata.overview);
        merge_optional_text(&mut title.poster_url, metadata.poster_url);
        merge_optional_text(&mut title.banner_url, metadata.banner_url);
        merge_optional_text(&mut title.background_url, metadata.background_url);
        merge_optional_text(&mut title.sort_title, metadata.sort_title);
        merge_optional_text(&mut title.slug, metadata.slug);
        merge_optional_text(&mut title.imdb_id, metadata.imdb_id);
        if metadata.runtime_minutes.is_some() {
            title.runtime_minutes = metadata.runtime_minutes;
        }
        if !metadata.genres.is_empty() {
            title.genres = metadata.genres;
        }
        merge_optional_text(&mut title.content_status, metadata.content_status);
        merge_optional_text(&mut title.language, metadata.language);
        merge_optional_text(&mut title.first_aired, metadata.first_aired);
        merge_optional_text(&mut title.network, metadata.network);
        merge_optional_text(&mut title.studio, metadata.studio);
        merge_optional_text(&mut title.country, metadata.country);
        if !metadata.aliases.is_empty() {
            title.aliases = metadata.aliases;
        }
        if !metadata.tagged_aliases.is_empty() {
            title.tagged_aliases = metadata.tagged_aliases;
        }
        merge_optional_text(&mut title.metadata_language, metadata.metadata_language);
        let metadata_fetched_at = parse_optional_datetime(metadata.metadata_fetched_at)?;
        if metadata_fetched_at.is_some() {
            title.metadata_fetched_at = metadata_fetched_at;
        }
        merge_optional_text(
            &mut title.digital_release_date,
            metadata.digital_release_date,
        );
        merge_external_ids(&mut title.external_ids, metadata.extra_external_ids);
        merge_tags(&mut title.tags, metadata.extra_tags);
        self.upsert_title_record(&title).await?;
        Ok(title)
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let mut title = TitleSql::get_by_id(self, id)
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
        self.upsert_title_record(&title).await?;
        Ok(title)
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM titles WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        let mut title = TitleSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
        title.folder_path = Some(folder_path.to_string());
        self.upsert_title_record(&title).await?;
        Ok(())
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        let mut title = TitleSql::get_by_id(self, id)
            .await?
            .ok_or_else(|| AppError::Repository("title was not found".to_string()))?;
        title.folder_path = None;
        self.upsert_title_record(&title).await?;
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        let mut titles = TitleSql::list(self, None, None).await?;
        let count = titles
            .iter()
            .filter(|title| title.metadata_language.is_some())
            .count() as u64;
        for title in &mut titles {
            title.metadata_language = None;
            self.upsert_title_record(title).await?;
        }
        Ok(count)
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

const TITLE_COLUMN_NAMES: &[&str] = &[
    "id",
    "library_id",
    "name",
    "facet",
    "monitored",
    "tags",
    "external_ids",
    "created_by",
    "created_at",
    "year",
    "overview",
    "poster_url",
    "poster_local_path",
    "banner_url",
    "banner_local_path",
    "background_url",
    "background_local_path",
    "sort_title",
    "slug",
    "imdb_id",
    "runtime_minutes",
    "genres",
    "content_status",
    "language",
    "first_aired",
    "network",
    "studio",
    "country",
    "aliases",
    "metadata_language",
    "metadata_fetched_at",
    "min_availability",
    "digital_release_date",
    "folder_path",
    "tagged_aliases_json",
];

fn title_columns(alias: Option<&str>) -> String {
    TITLE_COLUMN_NAMES
        .iter()
        .map(|column| match alias {
            Some(alias) => format!("{alias}.{column} AS {column}"),
            None => (*column).to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn title_from_row(
    row: &sqlx::postgres::PgRow,
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
    base_path: &str,
) -> AppResult<Title> {
    let facet: String = row.try_get("facet").map_err(repo_err)?;
    let poster_local_path: Option<String> = row.try_get("poster_local_path").unwrap_or(None);
    let banner_local_path: Option<String> = row.try_get("banner_local_path").unwrap_or(None);
    let background_local_path: Option<String> =
        row.try_get("background_local_path").unwrap_or(None);

    let title = Title {
        id: row.try_get("id").map_err(repo_err)?,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        name: row.try_get("name").map_err(repo_err)?,
        facet: parse_facet(&facet)?,
        monitored: row.try_get("monitored").map_err(repo_err)?,
        tags: json_column(row, "tags")?,
        external_ids: json_column(row, "external_ids")?,
        created_by: row.try_get("created_by").unwrap_or(None),
        created_at: row.try_get("created_at").map_err(repo_err)?,
        year: row.try_get("year").unwrap_or(None),
        overview: row.try_get("overview").unwrap_or(None),
        poster_url: row.try_get("poster_url").unwrap_or(None),
        poster_source_url: None,
        banner_url: row.try_get("banner_url").unwrap_or(None),
        banner_source_url: None,
        background_url: row.try_get("background_url").unwrap_or(None),
        background_source_url: None,
        sort_title: row.try_get("sort_title").unwrap_or(None),
        slug: row.try_get("slug").unwrap_or(None),
        imdb_id: row.try_get("imdb_id").unwrap_or(None),
        runtime_minutes: row.try_get("runtime_minutes").unwrap_or(None),
        genres: json_column(row, "genres")?,
        content_status: row.try_get("content_status").unwrap_or(None),
        language: row.try_get("language").unwrap_or(None),
        first_aired: row.try_get("first_aired").unwrap_or(None),
        network: row.try_get("network").unwrap_or(None),
        studio: row.try_get("studio").unwrap_or(None),
        country: row.try_get("country").unwrap_or(None),
        aliases: json_column(row, "aliases")?,
        tagged_aliases: json_column(row, "tagged_aliases_json")?,
        metadata_language: row.try_get("metadata_language").unwrap_or(None),
        metadata_fetched_at: row.try_get("metadata_fetched_at").unwrap_or(None),
        min_availability: row.try_get("min_availability").unwrap_or(None),
        digital_release_date: row.try_get("digital_release_date").unwrap_or(None),
        folder_path: row.try_get("folder_path").unwrap_or(None),
    };

    Ok(finalize_persisted_title(
        title,
        PersistedTitleDecodeOptions {
            mode,
            include_external_ids,
            base_path,
            poster_local_path: poster_local_path.as_deref(),
            banner_local_path: banner_local_path.as_deref(),
            background_local_path: background_local_path.as_deref(),
        },
    ))
}

fn json_column<T>(row: &sqlx::postgres::PgRow, column: &str) -> AppResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let value: Value = row.try_get(column).map_err(repo_err)?;
    serde_json::from_value(value).map_err(repo_err)
}

fn decode_title_rows(
    rows: &[sqlx::postgres::PgRow],
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
) -> AppResult<Vec<Title>> {
    let base_path = normalized_base_path_from_env();
    rows.iter()
        .map(|row| title_from_row(row, mode, include_external_ids, &base_path))
        .collect()
}

fn decode_optional_title_row(
    row: Option<&sqlx::postgres::PgRow>,
    mode: PersistedTitleReadMode,
    include_external_ids: bool,
) -> AppResult<Option<Title>> {
    let base_path = normalized_base_path_from_env();
    row.map(|row| title_from_row(row, mode, include_external_ids, &base_path))
        .transpose()
}

fn normalized_query(query: Option<String>) -> Option<String> {
    query
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_optional_datetime(value: Option<String>) -> AppResult<Option<DateTime<Utc>>> {
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| parsed.with_timezone(&Utc))
                .map_err(|error| {
                    AppError::Validation(format!(
                        "invalid title metadata timestamp '{value}': {error}"
                    ))
                })
        })
        .transpose()
}

fn title_has_tvdb_external_id(title: &Title) -> bool {
    title.external_ids.iter().any(|external_id| {
        external_id.source.trim().eq_ignore_ascii_case("tvdb")
            && !external_id.value.trim().is_empty()
    })
}

fn merge_optional_text(target: &mut Option<String>, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        *target = Some(value);
    }
}

fn merge_external_ids(existing: &mut Vec<ExternalId>, additions: Vec<ExternalId>) {
    for external_id in additions {
        existing.retain(|candidate| candidate.source != external_id.source);
        existing.push(external_id);
    }
}

fn merge_tags(existing: &mut Vec<String>, additions: Vec<String>) {
    for tag in additions {
        if let Some(colon_pos) = tag.rfind(':') {
            let prefix = &tag[..=colon_pos];
            existing.retain(|candidate| !candidate.starts_with(prefix));
        }
        existing.push(tag);
    }
}

fn parse_facet(value: &str) -> AppResult<MediaFacet> {
    MediaFacet::parse(value)
        .ok_or_else(|| AppError::Repository(format!("unknown media facet '{value}'")))
}

fn normalize_path(path: &str) -> String {
    path.trim_end_matches('/').to_ascii_lowercase()
}

fn repo_err(error: impl ToString) -> AppError {
    AppError::Repository(error.to_string())
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

#[allow(dead_code)]
fn _empty_title(id: String, name: String, facet: MediaFacet, created_at: DateTime<Utc>) -> Title {
    Title {
        id,
        library_id: scryer_domain::default_library_id_for_facet(&facet),
        name,
        facet,
        monitored: true,
        tags: Vec::new(),
        external_ids: Vec::new(),
        created_by: None,
        created_at,
        year: None,
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: Vec::new(),
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: Vec::new(),
        tagged_aliases: Vec::<TaggedAlias>::new(),
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}
