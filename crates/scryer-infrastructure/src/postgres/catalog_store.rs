use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use scryer_application::{
    AppError, AppResult, CollectionUpdate, CreateTitleOutcome, EpisodeUpdate, LibraryRootDraft,
    PendingTitleHydration, PrimaryCollectionSummary, ScopedExternalId, TitleMetadataUpdate,
};
use scryer_domain::{
    AppPermissionMask, CalendarEpisode, Collection, Entitlement, Episode, ExternalId, Id, Library,
    LibraryGrant, LibraryPermissionMask, LibraryRoot, MediaFacet, TaggedAlias, Title, User,
};
use serde_json::Value;
use sqlx::Row;

use crate::catalog_store::{CatalogStore, LibrarySql, ShowSql, TitleSql, UserSql};
use crate::postgres::timestamp::parse_rfc3339_timestamp;
use crate::queries::title_search::{self, TitleSearchProjectionSource};

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
                let projection = title_record_projection("t.record_json", include_external_ids);
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
                let projection = title_record_projection("record_json", include_external_ids);
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
                let projection = title_record_projection("t.record_json", include_external_ids);
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
                let projection = title_record_projection("record_json", include_external_ids);
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

        rows.iter().map(title_from_record_row).collect()
    }

    async fn get_by_id_internal(
        &self,
        id: &str,
        include_external_ids: bool,
    ) -> AppResult<Option<Title>> {
        let projection = title_record_projection("record_json", include_external_ids);
        let row = sqlx::query(&format!("SELECT {projection} FROM titles WHERE id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(title_from_record_row).transpose()
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
        let record_json = serde_json::to_value(title).map_err(repo_err)?;
        let search_source = TitleSearchProjectionSource::from(title);
        let search_terms = title_search::build_title_search_terms(&search_source);
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "INSERT INTO titles (
                id, library_id, facet, name, slug, tags, external_ids, metadata_json,
                record_json, folder_path, monitored, created_at, updated_at
             )
             VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb, $9::jsonb, $10, $11, $12, $13)
             ON CONFLICT (id) DO UPDATE SET
                library_id = EXCLUDED.library_id,
                facet = EXCLUDED.facet,
                name = EXCLUDED.name,
                slug = EXCLUDED.slug,
                tags = EXCLUDED.tags,
                external_ids = EXCLUDED.external_ids,
                metadata_json = EXCLUDED.metadata_json,
                record_json = EXCLUDED.record_json,
                folder_path = EXCLUDED.folder_path,
                monitored = EXCLUDED.monitored,
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&title.id)
        .bind(&title.library_id)
        .bind(title.facet.as_str())
        .bind(&title.name)
        .bind(&title.slug)
        .bind(tags)
        .bind(external_ids)
        .bind(record_json.clone())
        .bind(record_json)
        .bind(&title.folder_path)
        .bind(title.monitored)
        .bind(title.created_at)
        .bind(Utc::now())
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;

        sqlx::query("DELETE FROM title_search_terms WHERE title_id = $1")
            .bind(&title.id)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;

        for term in search_terms {
            sqlx::query(
                "INSERT INTO title_search_terms
                 (title_id, facet, term_kind, raw_term, normalized_term, weight, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT (title_id, term_kind, normalized_term) DO UPDATE SET
                    raw_term = EXCLUDED.raw_term,
                    weight = EXCLUDED.weight,
                    updated_at = EXCLUDED.updated_at",
            )
            .bind(&title.id)
            .bind(title.facet.as_str())
            .bind(term.term_kind)
            .bind(&term.raw_term)
            .bind(&term.normalized_term)
            .bind(term.weight)
            .bind(Utc::now())
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }

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
                sqlx::query(
                    "SELECT t.record_json
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
                      ORDER BY search.rank, lower(t.name), t.id",
                )
                .bind(facet.as_str())
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (Some(facet), None) => {
                sqlx::query(
                    "SELECT record_json
                   FROM titles
                  WHERE facet = $1
                  ORDER BY lower(name), id",
                )
                .bind(facet.as_str())
                .fetch_all(&self.pool)
                .await
            }
            (None, Some(query)) => {
                let normalized = title_search::normalize_title_search_text(query);
                let pattern = format!("%{normalized}%");
                sqlx::query(
                    "SELECT t.record_json
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
                      ORDER BY search.rank, lower(t.name), t.id",
                )
                .bind(pattern)
                .bind(normalized)
                .fetch_all(&self.pool)
                .await
            }
            (None, None) => {
                sqlx::query(
                    "SELECT record_json
                   FROM titles
                  ORDER BY lower(name), id",
                )
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(repo_err)?;

        rows.iter().map(title_from_record_row).collect()
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT record_json
               FROM titles
              WHERE EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $1
                       AND external_id->>'value' = ANY($2)
              )
              ORDER BY lower(name), id",
        )
        .bind(source)
        .bind(values)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(title_from_record_row).collect()
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
        TitleSql::list(self, facet, query).await
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
        let row = sqlx::query(
            "SELECT record_json
               FROM titles
              WHERE facet = $1 AND lower(slug) = lower($2)
              ORDER BY lower(name), id
              LIMIT 1",
        )
        .bind(facet.as_str())
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(title_from_record_row).transpose()
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

        let row = sqlx::query(
            "SELECT record_json
               FROM titles
              WHERE facet = $1
                AND lower(slug) = lower($2)
                AND library_id = ANY($3)
              ORDER BY lower(name), id
              LIMIT 1",
        )
        .bind(facet.as_str())
        .bind(slug)
        .bind(library_ids)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(title_from_record_row).transpose()
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let row = sqlx::query(
            "SELECT record_json
               FROM titles
              WHERE EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $1
                       AND external_id->>'value' = $2
              )
              ORDER BY lower(name), id
              LIMIT 1",
        )
        .bind(source)
        .bind(value)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(title_from_record_row).transpose()
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let row = sqlx::query(
            "SELECT record_json
               FROM titles
              WHERE facet = $1
                AND EXISTS (
                    SELECT 1
                      FROM jsonb_array_elements(external_ids) AS external_id
                     WHERE external_id->>'source' = $2
                       AND external_id->>'value' = $3
                )
              ORDER BY lower(name), id
              LIMIT 1",
        )
        .bind(facet.as_str())
        .bind(source)
        .bind(value)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(title_from_record_row).transpose()
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
        let mut builder = sqlx::QueryBuilder::<sqlx::Postgres>::new(
            "SELECT record_json, metadata_hydration_attempt_count
               FROM titles
              WHERE NULLIF(record_json ->> 'metadata_fetched_at', '') IS NULL
                AND metadata_hydration_next_attempt_at IS NOT NULL
                AND metadata_hydration_next_attempt_at <= NOW()",
        );
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
        rows.iter()
            .map(|row| {
                Ok(PendingTitleHydration {
                    title: title_from_record_row(row)?,
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
              ORDER BY COALESCE(record_json ->> 'metadata_fetched_at', ''), created_at
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
              ORDER BY COALESCE(record_json ->> 'metadata_fetched_at', ''), created_at
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
        if let Some(name) = metadata.name {
            title.name = name;
        }
        title.year = metadata.year;
        title.overview = metadata.overview;
        title.poster_url = metadata.poster_url;
        title.banner_url = metadata.banner_url;
        title.background_url = metadata.background_url;
        title.sort_title = metadata.sort_title;
        title.slug = metadata.slug;
        title.imdb_id = metadata.imdb_id;
        title.runtime_minutes = metadata.runtime_minutes;
        title.genres = metadata.genres;
        title.content_status = metadata.content_status;
        title.language = metadata.language;
        title.first_aired = metadata.first_aired;
        title.network = metadata.network;
        title.studio = metadata.studio;
        title.country = metadata.country;
        title.aliases = metadata.aliases;
        title.tagged_aliases = metadata.tagged_aliases;
        title.metadata_language = metadata.metadata_language;
        title.metadata_fetched_at = parse_optional_datetime(metadata.metadata_fetched_at)?;
        title.digital_release_date = metadata.digital_release_date;
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
        let rows = sqlx::query(
            "SELECT * FROM collections WHERE title_id = $1 ORDER BY collection_index, id",
        )
        .bind(title_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_collection).collect()
    }
    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let rows = sqlx::query(
            "SELECT collection_id AS scope_id, source, external_id, provenance, source_scope
               FROM collection_external_ids
              WHERE collection_id = $1
              ORDER BY source, external_id",
        )
        .bind(collection_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(ScopedExternalId {
                    scope_id: row.try_get("scope_id").map_err(repo_err)?,
                    source: row.try_get("source").map_err(repo_err)?,
                    external_id: row.try_get("external_id").map_err(repo_err)?,
                    provenance: row
                        .try_get("provenance")
                        .unwrap_or_else(|_| "metadata".into()),
                    source_scope: row.try_get("source_scope").unwrap_or(None),
                })
            })
            .collect()
    }
    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        if title_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows = sqlx::query("SELECT * FROM collections WHERE title_id = ANY($1) ORDER BY title_id, collection_index, id")
            .bind(title_ids)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        let mut grouped: HashMap<String, Vec<Collection>> = HashMap::new();
        for row in &rows {
            let collection = row_to_collection(row)?;
            grouped
                .entry(collection.title_id.clone())
                .or_default()
                .push(collection);
        }
        Ok(grouped)
    }
    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let row = sqlx::query("SELECT * FROM collections WHERE id = $1")
            .bind(collection_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_collection).transpose()
    }
    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        let row = sqlx::query("SELECT * FROM collections WHERE ordered_path = $1 LIMIT 1")
            .bind(ordered_path)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_collection).transpose()
    }
    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        sqlx::query(
            "INSERT INTO collections
             (id, title_id, collection_type, collection_index, label, ordered_path, narrative_order,
              first_episode_number, last_episode_number, interstitial_movie_json,
              specials_movies_json, interstitial_season_episode, monitored, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::jsonb, $11::jsonb, $12, $13, $14, NOW())
             ON CONFLICT (id) DO UPDATE SET
                collection_type = EXCLUDED.collection_type,
                collection_index = EXCLUDED.collection_index,
                label = EXCLUDED.label,
                ordered_path = EXCLUDED.ordered_path,
                narrative_order = EXCLUDED.narrative_order,
                first_episode_number = EXCLUDED.first_episode_number,
                last_episode_number = EXCLUDED.last_episode_number,
                interstitial_movie_json = EXCLUDED.interstitial_movie_json,
                specials_movies_json = EXCLUDED.specials_movies_json,
                interstitial_season_episode = EXCLUDED.interstitial_season_episode,
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
        .bind(
            collection
                .interstitial_movie
                .as_ref()
                .map(serde_json::to_value)
                .transpose()
                .map_err(repo_err)?,
        )
        .bind(serde_json::to_value(&collection.specials_movies).map_err(repo_err)?)
        .bind(&collection.interstitial_season_episode)
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
        sqlx::query("UPDATE collections SET interstitial_season_episode = $2, updated_at = NOW() WHERE id = $1")
            .bind(collection_id)
            .bind(season_episode)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        sqlx::query(
            "UPDATE episodes SET monitored = $2, updated_at = NOW() WHERE collection_id = $1",
        )
        .bind(collection_id)
        .bind(monitored)
        .execute(&self.pool)
        .await
        .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM collections WHERE id = $1")
            .bind(collection_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM collections WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let rows = sqlx::query("SELECT * FROM episodes WHERE collection_id = $1 ORDER BY season_number, episode_number, id")
            .bind(collection_id)
            .fetch_all(&self.pool)
            .await
            .map_err(repo_err)?;
        rows.iter().map(row_to_episode).collect()
    }
    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let rows = sqlx::query(
            "SELECT * FROM episodes WHERE title_id = $1 ORDER BY season_number, episode_number, id",
        )
        .bind(title_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter().map(row_to_episode).collect()
    }
    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let rows = sqlx::query(
            "SELECT episode_id AS scope_id, source, external_id, provenance, source_scope
               FROM episode_external_ids
              WHERE episode_id = $1
              ORDER BY source, external_id",
        )
        .bind(episode_id)
        .fetch_all(&self.pool)
        .await
        .map_err(repo_err)?;
        rows.iter()
            .map(|row| {
                Ok(ScopedExternalId {
                    scope_id: row.try_get("scope_id").map_err(repo_err)?,
                    source: row.try_get("source").map_err(repo_err)?,
                    external_id: row.try_get("external_id").map_err(repo_err)?,
                    provenance: row
                        .try_get("provenance")
                        .unwrap_or_else(|_| "metadata".into()),
                    source_scope: row.try_get("source_scope").unwrap_or(None),
                })
            })
            .collect()
    }
    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let row = sqlx::query("SELECT * FROM episodes WHERE id = $1")
            .bind(episode_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(repo_err)?;
        row.as_ref().map(row_to_episode).transpose()
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
    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM episodes WHERE id = $1")
            .bind(episode_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
    }
    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM episodes WHERE title_id = $1")
            .bind(title_id)
            .execute(&self.pool)
            .await
            .map_err(repo_err)?;
        Ok(())
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
        let row = sqlx::query(
            "SELECT * FROM episodes
              WHERE title_id = $1 AND absolute_number = $2
              LIMIT 1",
        )
        .bind(title_id)
        .bind(absolute_number)
        .fetch_optional(&self.pool)
        .await
        .map_err(repo_err)?;
        row.as_ref().map(row_to_episode).transpose()
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
        let mut tx = self.pool.begin().await.map_err(repo_err)?;
        sqlx::query(
            "DELETE FROM collection_external_ids
              WHERE collection_id IN (SELECT id FROM collections WHERE title_id = $1)
                AND source = 'anibridge'",
        )
        .bind(title_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        sqlx::query(
            "DELETE FROM episode_external_ids
              WHERE episode_id IN (SELECT id FROM episodes WHERE title_id = $1)
                AND source = 'anibridge'",
        )
        .bind(title_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_err)?;
        for external_id in collection_ids {
            sqlx::query(
                "INSERT INTO collection_external_ids
                 (id, collection_id, source, external_id, provenance, source_scope, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())",
            )
            .bind(Id::new().0)
            .bind(&external_id.scope_id)
            .bind(&external_id.source)
            .bind(&external_id.external_id)
            .bind(&external_id.provenance)
            .bind(&external_id.source_scope)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        for external_id in episode_ids {
            sqlx::query(
                "INSERT INTO episode_external_ids
                 (id, episode_id, source, external_id, provenance, source_scope, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())",
            )
            .bind(Id::new().0)
            .bind(&external_id.scope_id)
            .bind(&external_id.source)
            .bind(&external_id.external_id)
            .bind(&external_id.provenance)
            .bind(&external_id.source_scope)
            .execute(&mut *tx)
            .await
            .map_err(repo_err)?;
        }
        tx.commit().await.map_err(repo_err)?;
        Ok(())
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

fn title_from_record_row(row: &sqlx::postgres::PgRow) -> AppResult<Title> {
    let value: Value = row.try_get("record_json").map_err(repo_err)?;
    serde_json::from_value(value).map_err(repo_err)
}

fn title_record_projection(column: &str, include_external_ids: bool) -> String {
    if include_external_ids {
        format!("{column} AS record_json")
    } else {
        format!("jsonb_set({column}, '{{external_ids}}', '[]'::jsonb, true) AS record_json")
    }
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

fn merge_external_ids(existing: &mut Vec<ExternalId>, additions: Vec<ExternalId>) {
    for external_id in additions {
        if !existing.iter().any(|candidate| {
            candidate.source == external_id.source && candidate.value == external_id.value
        }) {
            existing.push(external_id);
        }
    }
}

fn merge_tags(existing: &mut Vec<String>, additions: Vec<String>) {
    for tag in additions {
        if !existing.iter().any(|candidate| candidate == &tag) {
            existing.push(tag);
        }
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

fn row_to_collection(row: &sqlx::postgres::PgRow) -> AppResult<Collection> {
    let collection_type_raw: String = row.try_get("collection_type").map_err(repo_err)?;
    let collection_type =
        scryer_domain::CollectionType::parse(&collection_type_raw).ok_or_else(|| {
            AppError::Repository(format!("invalid collection type {collection_type_raw}"))
        })?;
    let interstitial_movie = row
        .try_get::<Option<Value>, _>("interstitial_movie_json")
        .ok()
        .flatten()
        .and_then(json_value_unless_null)
        .map(serde_json::from_value)
        .transpose()
        .map_err(repo_err)?;
    let specials_movies = row
        .try_get::<Option<Value>, _>("specials_movies_json")
        .ok()
        .flatten()
        .and_then(json_value_unless_null)
        .map(serde_json::from_value)
        .transpose()
        .map_err(repo_err)?
        .unwrap_or_default();
    Ok(Collection {
        id: row.try_get("id").map_err(repo_err)?,
        title_id: row.try_get("title_id").map_err(repo_err)?,
        collection_type,
        collection_index: row.try_get("collection_index").map_err(repo_err)?,
        label: row.try_get("label").unwrap_or(None),
        ordered_path: row.try_get("ordered_path").unwrap_or(None),
        narrative_order: row.try_get("narrative_order").unwrap_or(None),
        first_episode_number: row.try_get("first_episode_number").unwrap_or(None),
        last_episode_number: row.try_get("last_episode_number").unwrap_or(None),
        interstitial_movie,
        specials_movies,
        interstitial_season_episode: row.try_get("interstitial_season_episode").unwrap_or(None),
        monitored: row.try_get("monitored").unwrap_or(true),
        created_at: row_timestamp(row, "created_at")?,
    })
}

fn json_value_unless_null(value: Value) -> Option<Value> {
    (!value.is_null()).then_some(value)
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
