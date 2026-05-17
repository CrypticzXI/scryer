use async_trait::async_trait;
use chrono::Utc;
use scryer_application::{AppError, AppResult, LibraryRepository, LibraryRootDraft};
use scryer_domain::{
    AppPermissionMask, Id, Library, LibraryGrant, LibraryPermissionMask, LibraryRoot, MediaFacet,
    default_library_id_for_facet,
};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRow, SqlRuntime, SqlTx, StoreDatastore};

const LIBRARY_ROOT_COLUMNS: &str = "libraries.id, libraries.facet, libraries.name, libraries.slug,
    libraries.is_default, libraries.created_at, libraries.updated_at,
    library_roots.id AS root_id, library_roots.library_id,
    library_roots.path AS root_path, library_roots.is_default AS root_is_default,
    library_roots.created_at AS root_created_at, library_roots.updated_at AS root_updated_at";

#[derive(Clone)]
pub struct LibraryStore {
    datastore: StoreDatastore,
}

impl LibraryStore {
    pub(crate) fn new(datastore: StoreDatastore) -> Self {
        Self { datastore }
    }

    pub fn sqlite(db: &crate::SqliteServices) -> Self {
        Self::new(StoreDatastore::Sqlite {
            pool: db.pool().clone(),
            writer_gate: db.writer_gate(),
        })
    }
}

#[async_trait]
impl LibraryRepository for LibraryStore {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        list_libraries(self.datastore.read_exec(), facet).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        get_library_by_id(self.datastore.read_exec(), id).await
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        let expected_id = default_library_id_for_facet(&facet);
        self.get_by_id(&expected_id).await
    }

    async fn create(&self, library: Library, roots: Vec<LibraryRootDraft>) -> AppResult<Library> {
        let library_id = library.id.clone();
        SqlRuntime::run_in_transaction(&self.datastore, "create_library", move |tx| {
            let library = library.clone();
            let library_id = library_id.clone();
            let roots = roots.clone();
            Box::pin(async move {
                insert_library_tx(tx, &library, roots).await?;
                load_library_tx(tx, &library_id).await?.ok_or_else(|| {
                    AppError::Repository("created library was not found".to_string())
                })
            })
        })
        .await
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        let library_id = library_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "update_library", move |tx| {
            let library_id = library_id.clone();
            let name = name.clone();
            let slug = slug.clone();
            let roots = roots.clone();
            Box::pin(async move {
                update_library_tx(tx, &library_id, name, slug, roots).await?;
                load_library_tx(tx, &library_id).await?.ok_or_else(|| {
                    AppError::Repository("updated library was not found".to_string())
                })
            })
        })
        .await
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let library_id = library_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "delete_library", move |tx| {
            let library_id = library_id.clone();
            Box::pin(async move {
                tx.execute(
                    "DELETE FROM libraries WHERE id = {}",
                    &[SqlArg::Text(library_id)],
                )
                .await
                .map(|rows| rows > 0)
            })
        })
        .await
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT permission_mask FROM user_app_permission_masks WHERE user_id = {}",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        let mask = row
            .as_ref()
            .map(|row| row.opt_i64("permission_mask"))
            .transpose()?
            .flatten()
            .unwrap_or(0);
        Ok(AppPermissionMask::from_bits_retain(mask_from_db_value(
            mask,
        )))
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(
            &self.datastore,
            "set_app_permission_mask_for_user",
            move |tx| {
                let user_id = user_id.clone();
                Box::pin(async move {
                    tx.execute(
                    "INSERT INTO user_app_permission_masks (user_id, permission_mask, updated_at)
                     VALUES ({}, {}, {})
                     ON CONFLICT(user_id) DO UPDATE SET
                        permission_mask = excluded.permission_mask,
                        updated_at = excluded.updated_at",
                    &[
                        SqlArg::Text(user_id),
                        SqlArg::I64(mask_to_db_value(permissions.bits())),
                        SqlArg::Timestamp(Utc::now()),
                    ],
                )
                .await?;
                    Ok(())
                })
            },
        )
        .await
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        let rows = SqlRuntime::fetch_all(
            self.datastore.read_exec(),
            "SELECT user_id, library_id, permission_mask
             FROM user_library_permission_masks
             WHERE user_id = {}
             ORDER BY library_id ASC",
            &[SqlArg::Text(user_id.to_string())],
        )
        .await?;
        rows.iter().map(row_to_library_grant).collect()
    }

    async fn set_grants_for_user(&self, user_id: &str, grants: Vec<LibraryGrant>) -> AppResult<()> {
        let user_id = user_id.to_string();
        SqlRuntime::run_in_transaction(&self.datastore, "set_library_grants_for_user", move |tx| {
            let user_id = user_id.clone();
            let grants = grants.clone();
            Box::pin(async move { replace_library_grants_tx(tx, &user_id, grants).await })
        })
        .await
    }

    async fn title_library_id(&self, title_id: &str) -> AppResult<Option<String>> {
        let row = SqlRuntime::fetch_optional(
            self.datastore.read_exec(),
            "SELECT library_id FROM titles WHERE id = {}",
            &[SqlArg::Text(title_id.to_string())],
        )
        .await?;
        row.as_ref()
            .map(|row| row.opt_text("library_id"))
            .transpose()
            .map(Option::flatten)
    }
}

async fn list_libraries(
    exec: SqlExec<'_, '_>,
    facet: Option<MediaFacet>,
) -> AppResult<Vec<Library>> {
    let mut sql = format!(
        "SELECT {LIBRARY_ROOT_COLUMNS}
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id"
    );
    let args = if let Some(facet) = facet {
        sql.push_str(" WHERE libraries.facet = {}");
        vec![SqlArg::Text(facet.as_str().to_string())]
    } else {
        Vec::new()
    };
    sql.push_str(
        " ORDER BY libraries.facet ASC, libraries.is_default DESC, LOWER(libraries.name) ASC,
                 libraries.id ASC, library_roots.is_default DESC, library_roots.path ASC",
    );
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_libraries(&rows)
}

async fn get_library_by_id(exec: SqlExec<'_, '_>, id: &str) -> AppResult<Option<Library>> {
    let sql = format!(
        "SELECT {LIBRARY_ROOT_COLUMNS}
         FROM libraries
         LEFT JOIN library_roots ON library_roots.library_id = libraries.id
         WHERE libraries.id = {{}}
         ORDER BY library_roots.is_default DESC, library_roots.path ASC"
    );
    let rows = SqlRuntime::fetch_all(exec, &sql, &[SqlArg::Text(id.to_string())]).await?;
    Ok(rows_to_libraries(&rows)?.into_iter().next())
}

async fn load_library_tx(tx: &mut SqlTx<'_>, id: &str) -> AppResult<Option<Library>> {
    get_library_by_id(SqlExec::Tx(tx), id).await
}

async fn insert_library_tx(
    tx: &mut SqlTx<'_>,
    library: &Library,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<()> {
    tx.execute(
        "INSERT INTO libraries (id, facet, name, slug, is_default, created_at, updated_at)
         VALUES ({}, {}, {}, {}, {}, {}, {})",
        &[
            SqlArg::Text(library.id.clone()),
            SqlArg::Text(library.facet.as_str().to_string()),
            SqlArg::Text(library.name.clone()),
            SqlArg::Text(library.slug.clone()),
            SqlArg::Bool(library.is_default),
            SqlArg::Timestamp(library.created_at),
            SqlArg::Timestamp(library.updated_at),
        ],
    )
    .await?;

    insert_library_roots_tx(tx, &library.id, roots).await
}

async fn update_library_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    name: String,
    slug: String,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<()> {
    let rows = tx
        .execute(
            "UPDATE libraries SET name = {}, slug = {}, updated_at = {} WHERE id = {}",
            &[
                SqlArg::Text(name),
                SqlArg::Text(slug),
                SqlArg::Timestamp(Utc::now()),
                SqlArg::Text(library_id.to_string()),
            ],
        )
        .await?;
    if rows == 0 {
        return Err(AppError::NotFound(format!("library {library_id}")));
    }

    tx.execute(
        "DELETE FROM library_roots WHERE library_id = {}",
        &[SqlArg::Text(library_id.to_string())],
    )
    .await?;
    insert_library_roots_tx(tx, library_id, roots).await
}

async fn insert_library_roots_tx(
    tx: &mut SqlTx<'_>,
    library_id: &str,
    roots: Vec<LibraryRootDraft>,
) -> AppResult<()> {
    let now = Utc::now();
    for root in roots {
        let path = root.path.trim();
        if path.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO library_roots
             (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES ({}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(Id::new().0),
                SqlArg::Text(library_id.to_string()),
                SqlArg::Text(path.to_string()),
                SqlArg::Text(normalize_root_path(path)),
                SqlArg::Bool(root.is_default),
                SqlArg::Timestamp(now),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn replace_library_grants_tx(
    tx: &mut SqlTx<'_>,
    user_id: &str,
    grants: Vec<LibraryGrant>,
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM user_library_permission_masks WHERE user_id = {}",
        &[SqlArg::Text(user_id.to_string())],
    )
    .await?;
    let now = Utc::now();
    for grant in grants {
        if grant.permissions.is_empty() {
            continue;
        }
        tx.execute(
            "INSERT INTO user_library_permission_masks
             (user_id, library_id, permission_mask, updated_at)
             VALUES ({}, {}, {}, {})",
            &[
                SqlArg::Text(user_id.to_string()),
                SqlArg::Text(grant.library_id),
                SqlArg::I64(mask_to_db_value(grant.permissions.bits())),
                SqlArg::Timestamp(now),
            ],
        )
        .await?;
    }
    Ok(())
}

fn rows_to_libraries(rows: &[SqlRow]) -> AppResult<Vec<Library>> {
    let mut libraries = Vec::<Library>::new();
    for row in rows {
        let library_id = row.text("id")?;
        if libraries
            .last()
            .is_none_or(|library| library.id != library_id)
        {
            libraries.push(row_to_library(row, library_id.clone())?);
        }
        if row.opt_text("root_id")?.is_some()
            && let Some(library) = libraries.last_mut()
        {
            library.roots.push(row_to_root(row)?);
        }
    }
    Ok(libraries)
}

fn row_to_library(row: &SqlRow, id: String) -> AppResult<Library> {
    let facet = MediaFacet::parse(&row.text("facet")?).unwrap_or_default();
    Ok(Library {
        id,
        facet,
        name: row.text("name")?,
        slug: row.text("slug")?,
        is_default: row.bool("is_default")?,
        roots: Vec::new(),
        created_at: row.timestamp("created_at")?,
        updated_at: row.timestamp("updated_at")?,
    })
}

fn row_to_root(row: &SqlRow) -> AppResult<LibraryRoot> {
    Ok(LibraryRoot {
        id: row.text("root_id")?,
        library_id: row.text("library_id")?,
        path: row.text("root_path")?,
        is_default: row.bool("root_is_default")?,
        created_at: row.timestamp("root_created_at")?,
        updated_at: row.timestamp("root_updated_at")?,
    })
}

fn row_to_library_grant(row: &SqlRow) -> AppResult<LibraryGrant> {
    Ok(LibraryGrant {
        user_id: row.text("user_id")?,
        library_id: row.text("library_id")?,
        permissions: LibraryPermissionMask::from_bits_retain(mask_from_db_value(
            row.i64("permission_mask")?,
        )),
    })
}

fn normalize_root_path(path: &str) -> String {
    path.trim().trim_end_matches('/').to_ascii_lowercase()
}

fn mask_to_db_value(mask: u64) -> i64 {
    mask as i64
}

fn mask_from_db_value(mask: i64) -> u64 {
    mask as u64
}
