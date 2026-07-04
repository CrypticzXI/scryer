use std::collections::{BTreeMap, HashSet};

use chrono::Utc;
use scryer_application::AppResult;
use scryer_domain::{CanonicalMediaTag, MediaFacet, Title};

use crate::queries::sql_runtime::{SqlArg, SqlExec, SqlRuntime, SqlTx};

pub(crate) struct CanonicalMediaSubjectInput {
    pub subject_key: String,
    pub subject_key_norm: String,
    pub language: String,
    pub target_kind: String,
    pub title_id: Option<String>,
    pub display_title: String,
    pub year: Option<i32>,
}

pub(crate) fn canonical_subject_input_for_title_with_key(
    title: &Title,
    preferred_subject_key: Option<&str>,
) -> CanonicalMediaSubjectInput {
    let subject_key = preferred_subject_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| title_external_subject_key(title))
        .unwrap_or_else(|| format!("title:{}:{}", title.facet.as_str(), title.id));
    let subject_key_norm = normalize_canonical_subject_key(&subject_key);
    CanonicalMediaSubjectInput {
        subject_key,
        subject_key_norm,
        language: title
            .metadata_language
            .as_deref()
            .filter(|language| !language.trim().is_empty())
            .unwrap_or("en")
            .trim()
            .to_ascii_lowercase(),
        target_kind: title.facet.as_str().to_string(),
        title_id: Some(title.id.clone()),
        display_title: title.name.clone(),
        year: title.year,
    }
}

pub(crate) fn normalize_canonical_subject_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn canonical_subject_id_for(subject_key_norm: &str, language: &str) -> String {
    format!(
        "canonical:{}:{}",
        language.trim().to_ascii_lowercase(),
        normalize_canonical_subject_key(subject_key_norm)
    )
}

pub(crate) async fn upsert_canonical_media_subject_tx(
    tx: &mut SqlTx<'_>,
    input: &CanonicalMediaSubjectInput,
) -> AppResult<String> {
    let subject_key_norm = normalize_canonical_subject_key(&input.subject_key_norm);
    let language = input.language.trim().to_ascii_lowercase();
    let subject_id = canonical_subject_id_for(&subject_key_norm, &language);
    let now = Utc::now();
    tx.execute(
        "INSERT INTO canonical_media_subjects (
            id, subject_key, subject_key_norm, language, target_kind, title_id, display_title, year,
            created_at, updated_at
        ) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})
        ON CONFLICT (subject_key_norm, language) DO UPDATE SET
            subject_key = excluded.subject_key,
            target_kind = CASE
                WHEN excluded.target_kind <> '' THEN excluded.target_kind
                ELSE canonical_media_subjects.target_kind
            END,
            title_id = COALESCE(excluded.title_id, canonical_media_subjects.title_id),
            display_title = CASE
                WHEN excluded.display_title <> '' THEN excluded.display_title
                ELSE canonical_media_subjects.display_title
            END,
            year = COALESCE(excluded.year, canonical_media_subjects.year),
            updated_at = excluded.updated_at",
        &[
            SqlArg::Text(subject_id.clone()),
            SqlArg::Text(input.subject_key.trim().to_string()),
            SqlArg::Text(subject_key_norm),
            SqlArg::Text(language),
            SqlArg::Text(input.target_kind.trim().to_string()),
            SqlArg::OptText(input.title_id.clone()),
            SqlArg::Text(input.display_title.trim().to_string()),
            SqlArg::OptI32(input.year),
            SqlArg::Timestamp(now),
            SqlArg::Timestamp(now),
        ],
    )
    .await?;
    Ok(subject_id)
}

pub(crate) async fn prefer_canonical_media_subject_for_title_tx(
    tx: &mut SqlTx<'_>,
    preferred_subject_id: &str,
    input: &CanonicalMediaSubjectInput,
) -> AppResult<()> {
    let Some(title_id) = input
        .title_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        return Ok(());
    };
    let target_kind = input.target_kind.trim();
    if target_kind.is_empty() {
        return Ok(());
    }

    tx.execute(
        "UPDATE canonical_media_subjects
            SET title_id = NULL,
                updated_at = {}
          WHERE title_id = {}
            AND target_kind = {}
            AND id <> {}",
        &[
            SqlArg::Timestamp(Utc::now()),
            SqlArg::Text(title_id.to_string()),
            SqlArg::Text(target_kind.to_string()),
            SqlArg::Text(preferred_subject_id.to_string()),
        ],
    )
    .await?;
    Ok(())
}

pub(crate) async fn replace_canonical_media_tags_tx(
    tx: &mut SqlTx<'_>,
    subject_id: &str,
    tags: &[CanonicalMediaTag],
) -> AppResult<()> {
    tx.execute(
        "DELETE FROM canonical_media_tags WHERE subject_id = {}",
        &[SqlArg::Text(subject_id.to_string())],
    )
    .await?;

    let mut seen = HashSet::new();
    for (sort_index, tag) in tags.iter().enumerate() {
        let key = tag.key.trim();
        let category = tag.category.trim();
        let name = tag.name.trim();
        if key.is_empty() || category.is_empty() || name.is_empty() || !seen.insert(key.to_string())
        {
            continue;
        }

        tx.execute(
            "INSERT INTO canonical_media_tags (
                subject_id, tag_key, category, name, confidence, is_adult, is_spoiler, sort_index
            ) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
            &[
                SqlArg::Text(subject_id.to_string()),
                SqlArg::Text(key.to_string()),
                SqlArg::Text(category.to_string()),
                SqlArg::Text(name.to_string()),
                SqlArg::OptF64(tag.confidence.filter(|value| value.is_finite())),
                SqlArg::Bool(tag.is_adult),
                SqlArg::Bool(tag.is_spoiler),
                SqlArg::I32(sort_index as i32),
            ],
        )
        .await?;

        insert_tag_values_tx(
            tx,
            "canonical_media_tag_sources",
            "source",
            subject_id,
            key,
            &tag.sources,
        )
        .await?;
        insert_tag_values_tx(
            tx,
            "canonical_media_tag_source_keys",
            "source_tag_key",
            subject_id,
            key,
            &tag.source_tag_keys,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn attach_canonical_tags_to_titles(
    exec: SqlExec<'_, '_>,
    titles: &mut [Title],
) -> AppResult<()> {
    if titles.is_empty() {
        return Ok(());
    }

    let ids = titles
        .iter()
        .map(|title| title.id.clone())
        .collect::<Vec<_>>();
    let tags_by_title = load_canonical_tags_for_title_ids(exec, &ids).await?;
    for title in titles {
        if let Some(tags) = tags_by_title.get(&title.id) {
            title.canonical_tags = tags.clone();
        }
    }
    Ok(())
}

async fn insert_tag_values_tx(
    tx: &mut SqlTx<'_>,
    table: &str,
    column: &str,
    subject_id: &str,
    tag_key: &str,
    values: &[String],
) -> AppResult<()> {
    let mut seen = HashSet::new();
    for (sort_index, value) in values.iter().enumerate() {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        tx.execute(
            &format!(
                "INSERT INTO {table} (subject_id, tag_key, {column}, sort_index)
                 VALUES ({{}}, {{}}, {{}}, {{}})"
            ),
            &[
                SqlArg::Text(subject_id.to_string()),
                SqlArg::Text(tag_key.to_string()),
                SqlArg::Text(value.to_string()),
                SqlArg::I32(sort_index as i32),
            ],
        )
        .await?;
    }
    Ok(())
}

async fn load_canonical_tags_for_title_ids(
    exec: SqlExec<'_, '_>,
    title_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    let placeholders = bind_placeholders(title_ids.len());
    let sql = canonical_tag_select_sql(&format!("s.title_id IN ({placeholders})"));
    let args = title_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_tags_by_title(&rows)
}

pub(crate) async fn load_canonical_tags_for_subject_ids(
    exec: SqlExec<'_, '_>,
    subject_ids: &[String],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    if subject_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let placeholders = bind_placeholders(subject_ids.len());
    let sql = canonical_tag_select_sql(&format!("s.id IN ({placeholders})"));
    let args = subject_ids
        .iter()
        .cloned()
        .map(SqlArg::Text)
        .collect::<Vec<_>>();
    let rows = SqlRuntime::fetch_all(exec, &sql, &args).await?;
    rows_to_tags_by_subject(&rows)
}

fn canonical_tag_select_sql(filter: &str) -> String {
    format!(
        "SELECT
            s.id AS subject_id,
            s.title_id AS title_id,
            t.tag_key AS tag_key,
            t.category AS category,
            t.name AS name,
            t.confidence AS confidence,
            t.is_adult AS is_adult,
            t.is_spoiler AS is_spoiler,
            ts.source AS source,
            tsk.source_tag_key AS source_tag_key
         FROM canonical_media_subjects s
         JOIN canonical_media_tags t ON t.subject_id = s.id
         LEFT JOIN canonical_media_tag_sources ts
            ON ts.subject_id = t.subject_id AND ts.tag_key = t.tag_key
         LEFT JOIN canonical_media_tag_source_keys tsk
            ON tsk.subject_id = t.subject_id AND tsk.tag_key = t.tag_key
         WHERE {filter}
         ORDER BY s.id, t.sort_index, t.category, t.name, ts.sort_index, tsk.sort_index"
    )
}

fn rows_to_tags_by_title(
    rows: &[crate::queries::sql_runtime::SqlRow],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    let mut by_subject = rows_to_tags_by_subject(rows)?;
    let mut by_title = BTreeMap::new();
    for row in rows {
        let Some(title_id) = row.opt_text("title_id")? else {
            continue;
        };
        let subject_id = row.text("subject_id")?;
        if let Some(tags) = by_subject.remove(&subject_id) {
            by_title.insert(title_id, tags);
        }
    }
    Ok(by_title)
}

fn rows_to_tags_by_subject(
    rows: &[crate::queries::sql_runtime::SqlRow],
) -> AppResult<BTreeMap<String, Vec<CanonicalMediaTag>>> {
    #[derive(Default)]
    struct OrderedSubjectTags {
        tag_order: Vec<String>,
        tags: BTreeMap<String, CanonicalMediaTag>,
    }

    let mut tags = BTreeMap::<String, OrderedSubjectTags>::new();
    for row in rows {
        let subject_id = row.text("subject_id")?;
        let tag_key = row.text("tag_key")?;
        let category = row.text("category")?;
        let name = row.text("name")?;
        let confidence = row.opt_f64("confidence")?;
        let is_adult = row.bool("is_adult")?;
        let is_spoiler = row.bool("is_spoiler")?;
        let subject_tags = tags.entry(subject_id).or_default();
        if !subject_tags.tags.contains_key(&tag_key) {
            subject_tags.tag_order.push(tag_key.clone());
            subject_tags.tags.insert(
                tag_key.clone(),
                CanonicalMediaTag {
                    key: tag_key.clone(),
                    category,
                    name,
                    confidence,
                    sources: Vec::new(),
                    source_tag_keys: Vec::new(),
                    is_adult,
                    is_spoiler,
                },
            );
        }
        let tag = subject_tags
            .tags
            .get_mut(&tag_key)
            .expect("canonical tag was inserted before lookup");
        push_unique_opt(&mut tag.sources, row.opt_text("source")?);
        push_unique_opt(&mut tag.source_tag_keys, row.opt_text("source_tag_key")?);
    }
    Ok(tags
        .into_iter()
        .map(|(subject_id, tags)| {
            let values = tags
                .tag_order
                .into_iter()
                .filter_map(|tag_key| tags.tags.get(&tag_key).cloned())
                .collect();
            (subject_id, values)
        })
        .collect())
}

fn push_unique_opt(values: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

fn bind_placeholders(count: usize) -> String {
    (0..count).map(|_| "{}").collect::<Vec<_>>().join(", ")
}

fn title_external_subject_key(title: &Title) -> Option<String> {
    let (facet_kind, source_order): (&str, &[&str]) = match title.facet {
        MediaFacet::Movie => ("movie", &["tmdb", "tvdb", "imdb", "anidb"]),
        MediaFacet::Series => ("series", &["tvdb", "tmdb", "imdb", "anidb"]),
        MediaFacet::Anime => (
            "anime",
            &[
                "anidb",
                "myanimelist",
                "mal",
                "anilist",
                "tvdb",
                "tmdb",
                "imdb",
                "trakt",
            ],
        ),
    };
    for source in source_order.iter().copied() {
        if let Some(value) = title
            .external_ids
            .iter()
            .find(|id| id.source.eq_ignore_ascii_case(source))
            .map(|id| id.value.trim())
            .filter(|value| !value.is_empty())
        {
            return Some(format!(
                "{}:{}:{}",
                source.to_ascii_lowercase(),
                facet_kind,
                value.to_ascii_lowercase()
            ));
        }
    }
    None
}
