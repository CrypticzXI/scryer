use scryer_application::{AppError, AppResult, LibraryRootDraft};
use scryer_domain::{AppPermissionMask, Library, LibraryGrant, LibraryRoot, MediaFacet};
use sqlx::{Row, SqlitePool};

use super::common::{parse_utc_datetime, repository_error_from_sqlx};

fn normalize_root_path(path: &str) -> String {
    path.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn mask_to_db_value(mask: u64) -> i64 {
    mask as i64
}

fn mask_from_db_value(mask: i64) -> u64 {
    mask as u64
}

fn row_to_root(row: &sqlx::sqlite::SqliteRow) -> AppResult<LibraryRoot> {
    Ok(LibraryRoot {
        id: row.try_get("root_id").map_err(repository_error_from_sqlx)?,
        library_id: row
            .try_get("library_id")
            .map_err(repository_error_from_sqlx)?,
        path: row
            .try_get("root_path")
            .map_err(repository_error_from_sqlx)?,
        is_default: row
            .try_get::<i64, _>("root_is_default")
            .map_err(repository_error_from_sqlx)?
            != 0,
        created_at: parse_utc_datetime(
            &row.try_get::<String, _>("root_created_at")
                .map_err(repository_error_from_sqlx)?,
        )?,
        updated_at: parse_utc_datetime(
            &row.try_get::<String, _>("root_updated_at")
                .map_err(repository_error_from_sqlx)?,
        )?,
    })
}

fn rows_to_libraries(rows: Vec<sqlx::sqlite::SqliteRow>) -> AppResult<Vec<Library>> {
    let mut libraries = Vec::<Library>::new();
    for row in rows {
        let library_id: String = row.try_get("id").map_err(repository_error_from_sqlx)?;
        let root_id: Option<String> = row.try_get("root_id").unwrap_or(None);
        if libraries
            .last()
            .is_none_or(|library| library.id != library_id)
        {
            let facet: String = row.try_get("facet").map_err(repository_error_from_sqlx)?;
            libraries.push(Library {
                id: library_id.clone(),
                facet: MediaFacet::parse(&facet).unwrap_or_default(),
                name: row.try_get("name").map_err(repository_error_from_sqlx)?,
                slug: row.try_get("slug").map_err(repository_error_from_sqlx)?,
                is_default: row
                    .try_get::<i64, _>("is_default")
                    .map_err(repository_error_from_sqlx)?
                    != 0,
                roots: Vec::new(),
                created_at: parse_utc_datetime(
                    &row.try_get::<String, _>("created_at")
                        .map_err(repository_error_from_sqlx)?,
                )?,
                updated_at: parse_utc_datetime(
                    &row.try_get::<String, _>("updated_at")
                        .map_err(repository_error_from_sqlx)?,
                )?,
            });
        }
        if root_id.is_some()
            && let Some(library) = libraries.last_mut()
        {
            library.roots.push(row_to_root(&row)?);
        }
    }
    Ok(libraries)
}

pub(crate) async fn list_libraries_query(
    pool: &SqlitePool,
    facet: Option<MediaFacet>,
) -> AppResult<Vec<Library>> {
    let mut sql = String::from(
        "SELECT libraries.id, libraries.facet, libraries.name, libraries.slug,
                libraries.is_default, libraries.created_at, libraries.updated_at,
                library_roots.id AS root_id, library_roots.library_id,
                library_roots.path AS root_path, library_roots.is_default AS root_is_default,
                library_roots.created_at AS root_created_at,
                library_roots.updated_at AS root_updated_at
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id",
    );
    if facet.is_some() {
        sql.push_str(" WHERE libraries.facet = ?");
    }
    sql.push_str(
        " ORDER BY libraries.facet ASC, libraries.is_default DESC, LOWER(libraries.name) ASC,
                 libraries.id ASC, library_roots.is_default DESC, library_roots.path ASC",
    );
    let mut query = sqlx::query(&sql);
    if let Some(facet) = facet {
        query = query.bind(facet.as_str());
    }
    let rows = query
        .fetch_all(pool)
        .await
        .map_err(repository_error_from_sqlx)?;
    rows_to_libraries(rows)
}

pub(crate) async fn get_library_by_id_query(
    pool: &SqlitePool,
    id: &str,
) -> AppResult<Option<Library>> {
    let rows = sqlx::query(
        "SELECT libraries.id, libraries.facet, libraries.name, libraries.slug,
                libraries.is_default, libraries.created_at, libraries.updated_at,
                library_roots.id AS root_id, library_roots.library_id,
                library_roots.path AS root_path, library_roots.is_default AS root_is_default,
                library_roots.created_at AS root_created_at,
                library_roots.updated_at AS root_updated_at
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id
         WHERE libraries.id = ?
         ORDER BY library_roots.is_default DESC, library_roots.path ASC",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;

    Ok(rows_to_libraries(rows)?.into_iter().next())
}

pub(crate) async fn create_library_query(
    pool: &SqlitePool,
    library: Library,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<Library> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    sqlx::query(
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&library.id)
    .bind(library.facet.as_str())
    .bind(&library.name)
    .bind(&library.slug)
    .bind(if library.is_default { 1_i64 } else { 0_i64 })
    .bind(library.created_at.to_rfc3339())
    .bind(library.updated_at.to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(repository_error_from_sqlx)?;

    let now = chrono::Utc::now().to_rfc3339();
    for root in roots {
        let path = root.path.trim();
        if path.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(scryer_domain::Id::new().0)
        .bind(&library.id)
        .bind(path)
        .bind(normalize_root_path(path))
        .bind(if root.is_default { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }

    tx.commit().await.map_err(repository_error_from_sqlx)?;
    get_library_by_id_query(pool, &library.id)
        .await?
        .ok_or_else(|| AppError::Repository("created library was not found".into()))
}

pub(crate) async fn update_library_query(
    pool: &SqlitePool,
    library_id: &str,
    name: String,
    slug: String,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<Library> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    let now = chrono::Utc::now().to_rfc3339();
    let rows_affected =
        sqlx::query("UPDATE libraries SET name = ?, slug = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(slug)
            .bind(&now)
            .bind(library_id)
            .execute(&mut *tx)
            .await
            .map_err(repository_error_from_sqlx)?
            .rows_affected();
    if rows_affected == 0 {
        return Err(AppError::NotFound(format!("library {library_id}")));
    }

    sqlx::query("DELETE FROM library_roots WHERE library_id = ?")
        .bind(library_id)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;

    for root in roots {
        let path = root.path.trim();
        if path.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(scryer_domain::Id::new().0)
        .bind(library_id)
        .bind(path)
        .bind(normalize_root_path(path))
        .bind(if root.is_default { 1_i64 } else { 0_i64 })
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }

    tx.commit().await.map_err(repository_error_from_sqlx)?;
    get_library_by_id_query(pool, library_id)
        .await?
        .ok_or_else(|| AppError::Repository("updated library was not found".into()))
}

pub(crate) async fn delete_empty_library_query(
    pool: &SqlitePool,
    library_id: &str,
) -> AppResult<bool> {
    let title_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM titles WHERE library_id = ?")
        .bind(library_id)
        .fetch_one(pool)
        .await
        .map_err(repository_error_from_sqlx)?;
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_scan_unmatched_items
         WHERE library_id = ? AND status = 'pending'",
    )
    .bind(library_id)
    .fetch_one(pool)
    .await
    .map_err(repository_error_from_sqlx)?;
    if title_count > 0 || pending_count > 0 {
        return Ok(false);
    }

    let rows_affected = sqlx::query("DELETE FROM libraries WHERE id = ?")
        .bind(library_id)
        .execute(pool)
        .await
        .map_err(repository_error_from_sqlx)?
        .rows_affected();
    Ok(rows_affected > 0)
}

pub(crate) async fn app_permission_mask_for_user_query(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<AppPermissionMask> {
    let mask = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT permission_mask FROM user_app_permission_masks WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(repository_error_from_sqlx)?
    .flatten()
    .unwrap_or(0);
    Ok(AppPermissionMask::from_bits_retain(mask_from_db_value(
        mask,
    )))
}

pub(crate) async fn set_app_permission_mask_for_user_query(
    pool: &SqlitePool,
    user_id: &str,
    permissions: AppPermissionMask,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
            permission_mask = excluded.permission_mask,
            updated_at = excluded.updated_at",
    )
    .bind(user_id)
    .bind(mask_to_db_value(permissions.bits()))
    .bind(now)
    .execute(pool)
    .await
    .map_err(repository_error_from_sqlx)?;
    Ok(())
}

pub(crate) async fn library_permission_masks_for_user_query(
    pool: &SqlitePool,
    user_id: &str,
) -> AppResult<Vec<LibraryGrant>> {
    let rows = sqlx::query(
        "SELECT user_id, library_id, permission_mask
         FROM user_library_permission_masks
         WHERE user_id = ?
         ORDER BY library_id ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(repository_error_from_sqlx)?;
    let mut grants = Vec::with_capacity(rows.len());
    for row in rows {
        let mask: i64 = row
            .try_get("permission_mask")
            .map_err(repository_error_from_sqlx)?;
        grants.push(LibraryGrant {
            user_id: row.try_get("user_id").map_err(repository_error_from_sqlx)?,
            library_id: row
                .try_get("library_id")
                .map_err(repository_error_from_sqlx)?,
            permissions: scryer_domain::LibraryPermissionMask::from_bits_retain(
                mask_from_db_value(mask),
            ),
        });
    }
    Ok(grants)
}

pub(crate) async fn set_library_grants_for_user_query(
    pool: &SqlitePool,
    user_id: &str,
    grants: Vec<LibraryGrant>,
) -> AppResult<()> {
    let mut tx = pool.begin().await.map_err(repository_error_from_sqlx)?;
    sqlx::query("DELETE FROM user_library_permission_masks WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    let now = chrono::Utc::now().to_rfc3339();
    for grant in grants {
        if grant.permissions.is_empty() {
            continue;
        }
        sqlx::query(
            "INSERT INTO user_library_permission_masks
             (user_id, library_id, permission_mask, updated_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(&grant.library_id)
        .bind(mask_to_db_value(grant.permissions.bits()))
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repository_error_from_sqlx)?;
    }
    tx.commit().await.map_err(repository_error_from_sqlx)?;
    Ok(())
}

pub(crate) async fn title_library_id_query(
    pool: &SqlitePool,
    title_id: &str,
) -> AppResult<Option<String>> {
    let row = sqlx::query("SELECT library_id FROM titles WHERE id = ?")
        .bind(title_id)
        .fetch_optional(pool)
        .await
        .map_err(repository_error_from_sqlx)?;
    Ok(row.and_then(|row| row.try_get("library_id").ok()))
}
