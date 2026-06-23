use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use scryer_application::{AppError, AppResult};
use scryer_domain::{
    MediaFacet, default_library_id_for_facet, normalize_library_root_path,
    root_folder_id_for_normalized_path,
};
use sqlx::Row;

const LEGACY_ROOT_FOLDER_TAG_PREFIX: &str = "scryer:root-folder:";
const TEMP_ROOT_ID_PREFIX: &str = "__scryer_0136_tmp__";

#[derive(Clone, Debug)]
struct LibraryRow {
    id: String,
    facet: MediaFacet,
}

#[derive(Clone, Debug)]
struct RootRow {
    id: String,
    library_id: String,
    path: String,
    is_default: bool,
}

#[derive(Clone, Debug)]
struct TitleRow {
    id: String,
    library_id: Option<String>,
    facet: MediaFacet,
    tags_json: String,
}

#[derive(Clone, Debug)]
struct RootDescriptor {
    id: String,
    library_id: String,
    path: String,
    normalized_path: String,
    is_default: bool,
}

#[derive(Clone, Debug)]
struct ExistingRootUpdate {
    old_id: String,
    new_id: String,
    normalized_path: String,
}

#[derive(Clone, Debug)]
struct CreatedRoot {
    id: String,
    library_id: String,
    path: String,
    normalized_path: String,
    is_default: bool,
}

#[derive(Clone, Debug)]
struct TitleAssignment {
    title_id: String,
    root_folder_id: String,
    tags_json: String,
}

#[derive(Clone, Debug)]
struct ParsedTitle {
    id: String,
    library_id: String,
    legacy_root: Option<LegacyRootTag>,
    cleaned_tags: Vec<String>,
}

#[derive(Clone, Debug)]
struct LegacyRootTag {
    path: String,
    normalized_path: String,
}

#[derive(Clone, Debug)]
struct MigrationPlan {
    existing_root_updates: Vec<ExistingRootUpdate>,
    created_roots: Vec<CreatedRoot>,
    title_assignments: Vec<TitleAssignment>,
}

pub(crate) async fn migrate_title_root_folder_ids_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<()> {
    let libraries = sqlite_libraries(tx).await?;
    let roots = sqlite_library_roots(tx).await?;
    let titles = sqlite_titles(tx).await?;
    let plan = build_plan(libraries, roots, titles)?;
    apply_sqlite_plan(tx, plan).await?;
    sqlite_assert_no_orphans(tx).await
}

pub(crate) async fn migrate_title_root_folder_ids_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let libraries = postgres_libraries(tx).await?;
    let roots = postgres_library_roots(tx).await?;
    let titles = postgres_titles(tx).await?;
    let plan = build_plan(libraries, roots, titles)?;
    apply_postgres_plan(tx, plan).await?;
    postgres_assert_no_orphans(tx).await
}

fn build_plan(
    libraries: Vec<LibraryRow>,
    roots: Vec<RootRow>,
    titles: Vec<TitleRow>,
) -> AppResult<MigrationPlan> {
    let libraries_by_id = libraries
        .into_iter()
        .map(|library| (library.id.clone(), library))
        .collect::<HashMap<_, _>>();
    let mut roots_by_library = HashMap::<String, Vec<RootDescriptor>>::new();
    let mut root_by_library_path = HashMap::<(String, String), RootDescriptor>::new();
    let mut root_by_path = HashMap::<String, RootDescriptor>::new();
    let mut existing_root_updates = Vec::new();
    let mut new_root_ids = HashMap::<String, String>::new();
    let mut root_owner_by_normalized_path = HashMap::<String, (String, String)>::new();

    for root in roots {
        let normalized_path = normalize_library_root_path(&root.path);
        if normalized_path.is_empty() {
            return migration_error(format!(
                "library root {} in library {} has an empty path",
                root.id, root.library_id
            ));
        }
        let new_id = root_folder_id_for_normalized_path(&normalized_path);
        if let Some(existing_path) = new_root_ids.insert(new_id.clone(), normalized_path.clone())
            && existing_path != normalized_path
        {
            return migration_error(format!(
                "library root paths {existing_path} and {normalized_path} produced the same deterministic id"
            ));
        }
        if let Some((other_library_id, other_root_id)) =
            root_owner_by_normalized_path.get(&normalized_path)
        {
            return migration_error(format!(
                "library root path {normalized_path} is configured by root {other_root_id} in library {other_library_id} and root {} in library {}; root folder ids are path-derived, so duplicate root paths must be merged before migration",
                root.id, root.library_id
            ));
        }
        root_owner_by_normalized_path.insert(
            normalized_path.clone(),
            (root.library_id.clone(), root.id.clone()),
        );
        let descriptor = RootDescriptor {
            id: new_id.clone(),
            library_id: root.library_id.clone(),
            path: root.path.trim().to_string(),
            normalized_path: normalized_path.clone(),
            is_default: root.is_default,
        };
        root_by_library_path.insert(
            (
                descriptor.library_id.clone(),
                descriptor.normalized_path.clone(),
            ),
            descriptor.clone(),
        );
        root_by_path.insert(descriptor.normalized_path.clone(), descriptor.clone());
        roots_by_library
            .entry(descriptor.library_id.clone())
            .or_default()
            .push(descriptor);
        existing_root_updates.push(ExistingRootUpdate {
            old_id: root.id,
            new_id,
            normalized_path,
        });
    }

    for roots in roots_by_library.values_mut() {
        roots.sort_by(compare_roots_for_default);
    }

    let initial_root_counts = roots_by_library
        .iter()
        .map(|(library_id, roots)| (library_id.clone(), roots.len()))
        .collect::<HashMap<_, _>>();
    let mut created_counts = HashMap::<String, usize>::new();
    let mut created_roots = Vec::new();
    let mut parsed_titles = Vec::new();

    for title in titles {
        let library_id = title
            .library_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_library_id_for_facet(&title.facet));
        let Some(library) = libraries_by_id.get(&library_id) else {
            return migration_error(format!(
                "title {} references missing library {}",
                title.id, library_id
            ));
        };
        if library.facet != title.facet {
            return migration_error(format!(
                "title {} belongs to {} library {} but has {} facet",
                title.id,
                library.facet.as_str(),
                library_id,
                title.facet.as_str()
            ));
        }
        let (legacy_root, cleaned_tags) = parse_title_tags(&title.tags_json);
        parsed_titles.push(ParsedTitle {
            id: title.id,
            library_id,
            legacy_root,
            cleaned_tags,
        });
    }

    let mut title_root_ids = BTreeMap::<String, String>::new();
    for title in parsed_titles
        .iter()
        .filter(|title| title.legacy_root.is_some())
    {
        let legacy_root = title.legacy_root.as_ref().expect("legacy root checked");
        let path_key = (
            title.library_id.clone(),
            legacy_root.normalized_path.clone(),
        );
        if let Some(root) = root_by_library_path.get(&path_key) {
            title_root_ids.insert(title.id.clone(), root.id.clone());
            continue;
        }
        if let Some(other_root) = root_by_path.get(&legacy_root.normalized_path) {
            return migration_error(format!(
                "title {} has legacy root {} in library {}, but that path is configured on library {}",
                title.id, legacy_root.path, title.library_id, other_root.library_id
            ));
        }

        let created_root_id = root_folder_id_for_normalized_path(&legacy_root.normalized_path);
        let is_default = initial_root_counts
            .get(&title.library_id)
            .copied()
            .unwrap_or_default()
            == 0
            && created_counts
                .get(&title.library_id)
                .copied()
                .unwrap_or_default()
                == 0;
        *created_counts.entry(title.library_id.clone()).or_default() += 1;

        let descriptor = RootDescriptor {
            id: created_root_id.clone(),
            library_id: title.library_id.clone(),
            path: legacy_root.normalized_path.clone(),
            normalized_path: legacy_root.normalized_path.clone(),
            is_default,
        };
        root_by_library_path.insert(path_key, descriptor.clone());
        root_by_path.insert(descriptor.normalized_path.clone(), descriptor.clone());
        roots_by_library
            .entry(title.library_id.clone())
            .or_default()
            .push(descriptor.clone());
        roots_by_library
            .get_mut(&title.library_id)
            .expect("created root library entry")
            .sort_by(compare_roots_for_default);
        created_roots.push(CreatedRoot {
            id: descriptor.id.clone(),
            library_id: descriptor.library_id.clone(),
            path: descriptor.path.clone(),
            normalized_path: descriptor.normalized_path.clone(),
            is_default,
        });
        title_root_ids.insert(title.id.clone(), created_root_id);
    }

    for title in parsed_titles
        .iter()
        .filter(|title| title.legacy_root.is_none())
    {
        let Some(root) = roots_by_library
            .get(&title.library_id)
            .and_then(|roots| roots.first())
        else {
            return migration_error(format!(
                "title {} in library {} has no configured root folder and no legacy root-folder tag",
                title.id, title.library_id
            ));
        };
        title_root_ids.insert(title.id.clone(), root.id.clone());
    }

    let mut title_assignments = Vec::new();
    for title in parsed_titles {
        let Some(root_folder_id) = title_root_ids.remove(&title.id) else {
            return migration_error(format!(
                "title {} did not receive a root folder id",
                title.id
            ));
        };
        title_assignments.push(TitleAssignment {
            title_id: title.id,
            root_folder_id,
            tags_json: tags_to_json(&title.cleaned_tags)?,
        });
    }

    Ok(MigrationPlan {
        existing_root_updates,
        created_roots,
        title_assignments,
    })
}

fn parse_title_tags(tags_json: &str) -> (Option<LegacyRootTag>, Vec<String>) {
    let tags = serde_json::from_str::<Vec<String>>(tags_json).unwrap_or_default();
    let mut legacy_root = None;
    let mut cleaned_tags = Vec::new();

    for tag in tags {
        if let Some(path) = tag.strip_prefix(LEGACY_ROOT_FOLDER_TAG_PREFIX) {
            if legacy_root.is_none() {
                let normalized_path = normalize_library_root_path(path);
                if !normalized_path.is_empty() {
                    legacy_root = Some(LegacyRootTag {
                        path: path.trim().to_string(),
                        normalized_path,
                    });
                }
            }
        } else {
            cleaned_tags.push(tag);
        }
    }

    (legacy_root, cleaned_tags)
}

fn compare_roots_for_default(left: &RootDescriptor, right: &RootDescriptor) -> Ordering {
    (
        if left.is_default { 0 } else { 1 },
        left.path.as_str(),
        left.id.as_str(),
    )
        .cmp(&(
            if right.is_default { 0 } else { 1 },
            right.path.as_str(),
            right.id.as_str(),
        ))
}

fn tags_to_json(tags: &[String]) -> AppResult<String> {
    serde_json::to_string(tags).map_err(repo_err)
}

async fn sqlite_libraries(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<LibraryRow>> {
    let rows = sqlx::query("SELECT id, facet FROM libraries ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            let id: String = row.try_get("id").map_err(repo_err)?;
            let facet_text: String = row.try_get("facet").map_err(repo_err)?;
            let facet = MediaFacet::parse(&facet_text).ok_or_else(|| {
                AppError::Repository(format!("unknown library facet '{facet_text}'"))
            })?;
            Ok(LibraryRow { id, facet })
        })
        .collect()
}

async fn postgres_libraries(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<LibraryRow>> {
    let rows = sqlx::query("SELECT id, facet FROM libraries ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            let id: String = row.try_get("id").map_err(repo_err)?;
            let facet_text: String = row.try_get("facet").map_err(repo_err)?;
            let facet = MediaFacet::parse(&facet_text).ok_or_else(|| {
                AppError::Repository(format!("unknown library facet '{facet_text}'"))
            })?;
            Ok(LibraryRow { id, facet })
        })
        .collect()
}

async fn sqlite_library_roots(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query(
        "SELECT id, library_id, path, is_default
           FROM library_roots
          ORDER BY library_id ASC, is_default DESC, path ASC, id ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(RootRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                path: row.try_get("path").map_err(repo_err)?,
                is_default: row.try_get::<i64, _>("is_default").map_err(repo_err)? != 0,
            })
        })
        .collect()
}

async fn postgres_library_roots(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<RootRow>> {
    let rows = sqlx::query(
        "SELECT id, library_id, path, is_default
           FROM library_roots
          ORDER BY library_id ASC, is_default DESC, path ASC, id ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter()
        .map(|row| {
            Ok(RootRow {
                id: row.try_get("id").map_err(repo_err)?,
                library_id: row.try_get("library_id").map_err(repo_err)?,
                path: row.try_get("path").map_err(repo_err)?,
                is_default: row.try_get("is_default").map_err(repo_err)?,
            })
        })
        .collect()
}

async fn sqlite_titles(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query("SELECT id, library_id, facet, tags FROM titles ORDER BY id")
        .fetch_all(&mut **tx)
        .await
        .map_err(repo_err)?;
    rows.into_iter().map(title_row_from_sqlite).collect()
}

async fn postgres_titles(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<Vec<TitleRow>> {
    let rows = sqlx::query(
        "SELECT id, library_id, facet, COALESCE(tags, '[]'::jsonb)::text AS tags_json
           FROM titles
          ORDER BY id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_err)?;
    rows.into_iter().map(title_row_from_postgres).collect()
}

fn title_row_from_sqlite(row: sqlx::sqlite::SqliteRow) -> AppResult<TitleRow> {
    let id: String = row.try_get("id").map_err(repo_err)?;
    let facet_text: String = row.try_get("facet").map_err(repo_err)?;
    let facet = MediaFacet::parse(&facet_text)
        .ok_or_else(|| AppError::Repository(format!("unknown title facet '{facet_text}'")))?;
    Ok(TitleRow {
        id,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        facet,
        tags_json: row
            .try_get::<Option<String>, _>("tags")
            .map_err(repo_err)?
            .unwrap_or_else(|| "[]".to_string()),
    })
}

fn title_row_from_postgres(row: sqlx::postgres::PgRow) -> AppResult<TitleRow> {
    let id: String = row.try_get("id").map_err(repo_err)?;
    let facet_text: String = row.try_get("facet").map_err(repo_err)?;
    let facet = MediaFacet::parse(&facet_text)
        .ok_or_else(|| AppError::Repository(format!("unknown title facet '{facet_text}'")))?;
    Ok(TitleRow {
        id,
        library_id: row.try_get("library_id").map_err(repo_err)?,
        facet,
        tags_json: row.try_get("tags_json").map_err(repo_err)?,
    })
}

async fn apply_sqlite_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    plan: MigrationPlan,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for update in &plan.existing_root_updates {
        sqlx::query("UPDATE library_roots SET id = ?1 WHERE id = ?2")
            .bind(temp_root_id(&update.old_id))
            .bind(&update.old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for update in &plan.existing_root_updates {
        sqlx::query("UPDATE library_roots SET id = ?1, normalized_path = ?2 WHERE id = ?3")
            .bind(&update.new_id)
            .bind(&update.normalized_path)
            .bind(temp_root_id(&update.old_id))
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for root in &plan.created_roots {
        sqlx::query(
            "INSERT INTO library_roots
                (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(&root.id)
        .bind(&root.library_id)
        .bind(&root.path)
        .bind(&root.normalized_path)
        .bind(root.is_default)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    for assignment in &plan.title_assignments {
        sqlx::query("UPDATE titles SET root_folder_id = ?1, tags = ?2 WHERE id = ?3")
            .bind(&assignment.root_folder_id)
            .bind(&assignment.tags_json)
            .bind(&assignment.title_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

async fn apply_postgres_plan(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    plan: MigrationPlan,
) -> AppResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for update in &plan.existing_root_updates {
        sqlx::query("UPDATE library_roots SET id = $1 WHERE id = $2")
            .bind(temp_root_id(&update.old_id))
            .bind(&update.old_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for update in &plan.existing_root_updates {
        sqlx::query("UPDATE library_roots SET id = $1, normalized_path = $2 WHERE id = $3")
            .bind(&update.new_id)
            .bind(&update.normalized_path)
            .bind(temp_root_id(&update.old_id))
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    for root in &plan.created_roots {
        sqlx::query(
            "INSERT INTO library_roots
                (id, library_id, path, normalized_path, is_default, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6::timestamptz, $7::timestamptz)",
        )
        .bind(&root.id)
        .bind(&root.library_id)
        .bind(&root.path)
        .bind(&root.normalized_path)
        .bind(root.is_default)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(repo_err)?;
    }
    for assignment in &plan.title_assignments {
        sqlx::query("UPDATE titles SET root_folder_id = $1, tags = $2::jsonb WHERE id = $3")
            .bind(&assignment.root_folder_id)
            .bind(&assignment.tags_json)
            .bind(&assignment.title_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_err)?;
    }
    Ok(())
}

async fn sqlite_assert_no_orphans(tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>) -> AppResult<()> {
    let missing_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE root_folder_id IS NULL OR trim(root_folder_id) = ''",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    if missing_count > 0 {
        return migration_error(format!(
            "{missing_count} title rows still lack root_folder_id"
        ));
    }

    let orphan_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE NOT EXISTS (
                SELECT 1
                  FROM library_roots roots
                 WHERE roots.id = titles.root_folder_id
                   AND roots.library_id = COALESCE(
                       titles.library_id,
                       CASE titles.facet
                           WHEN 'movie' THEN 'movie_default_library'
                           WHEN 'series' THEN 'series_default_library'
                           WHEN 'anime' THEN 'anime_default_library'
                           ELSE 'movie_default_library'
                       END
                   )
          )",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    if orphan_count > 0 {
        return migration_error(format!(
            "{orphan_count} title root_folder_id values do not reference roots in the title library"
        ));
    }
    Ok(())
}

async fn postgres_assert_no_orphans(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> AppResult<()> {
    let missing_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE root_folder_id IS NULL OR btrim(root_folder_id) = ''",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    if missing_count > 0 {
        return migration_error(format!(
            "{missing_count} title rows still lack root_folder_id"
        ));
    }

    let orphan_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM titles
          WHERE NOT EXISTS (
                SELECT 1
                  FROM library_roots roots
                 WHERE roots.id = titles.root_folder_id
                   AND roots.library_id = COALESCE(
                       titles.library_id,
                       CASE titles.facet
                           WHEN 'movie' THEN 'movie_default_library'
                           WHEN 'series' THEN 'series_default_library'
                           WHEN 'anime' THEN 'anime_default_library'
                           ELSE 'movie_default_library'
                       END
                   )
          )",
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(repo_err)?;
    if orphan_count > 0 {
        return migration_error(format!(
            "{orphan_count} title root_folder_id values do not reference roots in the title library"
        ));
    }
    Ok(())
}

fn temp_root_id(root_id: &str) -> String {
    format!("{TEMP_ROOT_ID_PREFIX}{root_id}")
}

fn migration_error<T>(message: impl Into<String>) -> AppResult<T> {
    Err(AppError::Repository(format!(
        "0136 title root folder id migration failed: {}",
        message.into()
    )))
}

fn repo_err(error: impl std::fmt::Display) -> AppError {
    AppError::Repository(error.to_string())
}
