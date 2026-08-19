use std::collections::HashMap;
use std::path::Path;

use scryer_domain::{Collection, ExternalId, MediaFacet, NewTitle, Title};

use crate::library_discovery::derive_movie_probe_path;
use crate::library_scan::MetadataSearchItem;
use crate::stored_paths::{path_to_stored_string, stored_path_to_path_buf};

fn normalize_title_key(name: &str) -> String {
    crate::title_matching::canonical_lookup_key(name)
}

/// Every existing title (and alias) under one normalized name key. A name is
/// not an identity — remakes and originals share it — so a lookup by name must
/// see ALL same-name titles and pick by year; a single slot per key hid every
/// title but the last inserted (#148: a remake's folder could only ever be
/// compared with the original's title, or the other way round).
pub(crate) type TitleNameIndex = HashMap<String, Vec<usize>>;

fn index_title_name(existing_titles_by_name: &mut TitleNameIndex, name: &str, index: usize) {
    let indexes = existing_titles_by_name
        .entry(normalize_title_key(name))
        .or_default();
    if !indexes.contains(&index) {
        indexes.push(index);
    }
}

/// Whether a scanned/selected year is compatible with a catalog title's year:
/// equal, or unknown on either side. Two known, different years are two
/// different titles that merely share a name.
pub(crate) fn title_year_compatible(title: &Title, year: Option<u32>) -> bool {
    match (title.year, year) {
        (Some(title_year), Some(year)) => title_year >= 0 && title_year as u32 == year,
        _ => true,
    }
}

fn index_movie_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    index_title_name(existing_titles_by_name, &title.name, index);
    for alias in &title.aliases {
        index_title_name(existing_titles_by_name, alias, index);
    }
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
        {
            existing_titles_by_imdb_id.insert(imdb_id, index);
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            existing_titles_by_tmdb_id.insert(external_id.value.clone(), index);
        }
    }
}

fn index_series_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    index_title_name(existing_titles_by_name, &title.name, index);
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        } else if external_id.source.eq_ignore_ascii_case("imdb")
            && let Some(imdb_id) = crate::normalize::normalize_imdb_id(&external_id.value)
        {
            existing_titles_by_imdb_id.insert(imdb_id, index);
        } else if external_id.source.eq_ignore_ascii_case("tmdb") {
            existing_titles_by_tmdb_id.insert(external_id.value.clone(), index);
        }
    }
}

pub(crate) fn append_movie_title(
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
) -> usize {
    let index = existing_titles.len();
    existing_titles.push(title);
    index_movie_title(
        &existing_titles[index],
        index,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    );
    index
}

pub(crate) fn append_series_title(
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut TitleNameIndex,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
    title: Title,
) -> usize {
    let index = existing_titles.len();
    existing_titles.push(title);
    index_series_title(
        &existing_titles[index],
        index,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    );
    index
}

pub(crate) fn update_series_title_folder_path_index(
    existing_titles_by_folder_path: &mut HashMap<String, usize>,
    title: &Title,
    index: usize,
) {
    if let Some(folder_path) = title
        .folder_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .and_then(crate::stored_paths::folder_path_identity_key)
    {
        existing_titles_by_folder_path.insert(folder_path, index);
    }
}

pub(crate) fn update_movie_probe_path_index(
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    root: &Path,
    file_path: &str,
    index: usize,
) {
    let file_path_buf = stored_path_to_path_buf(file_path);
    if let Some(parent) = file_path_buf.parent()
        && parent != root
    {
        existing_titles_by_probe_path.insert(path_to_stored_string(parent), index);
    } else {
        existing_titles_by_probe_path.insert(file_path.to_string(), index);
    }
}

pub(crate) type MovieTitleIndexes = (
    TitleNameIndex,
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
);

pub(crate) fn build_movie_title_indexes(existing_titles: &[Title]) -> MovieTitleIndexes {
    let mut existing_titles_by_name = HashMap::new();
    let mut existing_titles_by_tvdb_id = HashMap::new();
    let mut existing_titles_by_imdb_id = HashMap::new();
    let mut existing_titles_by_tmdb_id = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        index_movie_title(
            title,
            index,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
        );
    }

    (
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
}

pub(crate) type SeriesTitleIndexes = (
    TitleNameIndex,
    HashMap<String, usize>,
    HashMap<String, usize>,
    HashMap<String, usize>,
);

pub(crate) fn build_series_title_indexes(existing_titles: &[Title]) -> SeriesTitleIndexes {
    let mut existing_titles_by_name = HashMap::new();
    let mut existing_titles_by_tvdb_id = HashMap::new();
    let mut existing_titles_by_imdb_id = HashMap::new();
    let mut existing_titles_by_tmdb_id = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        index_series_title(
            title,
            index,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
            &mut existing_titles_by_imdb_id,
            &mut existing_titles_by_tmdb_id,
        );
    }

    (
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
        existing_titles_by_imdb_id,
        existing_titles_by_tmdb_id,
    )
}

pub(crate) fn build_series_title_folder_path_index(
    existing_titles: &[Title],
) -> HashMap<String, usize> {
    let mut existing_titles_by_folder_path = HashMap::new();
    for (index, title) in existing_titles.iter().enumerate() {
        update_series_title_folder_path_index(&mut existing_titles_by_folder_path, title, index);
    }
    existing_titles_by_folder_path
}

pub(crate) fn build_movie_probe_path_indexes(
    root: &Path,
    existing_titles: &[Title],
    collections_by_title: &HashMap<String, Vec<Collection>>,
) -> HashMap<String, usize> {
    let mut existing_titles_by_probe_path = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        let collections = collections_by_title
            .get(&title.id)
            .cloned()
            .unwrap_or_default();
        if let Some(probe_path) = derive_movie_probe_path(root, title, &collections)
            && let Some(key) =
                crate::stored_paths::folder_path_identity_key(&path_to_stored_string(&probe_path))
        {
            existing_titles_by_probe_path.insert(key, index);
        }
    }

    existing_titles_by_probe_path
}

/// The existing catalog title a metadata match refers to, if any: the same
/// canonical (tvdb) id, else a same-name title whose year agrees with the
/// match. A same-name title with a *different* year is a different film (the
/// remake vs the original) and must not absorb the match — that misbinding is
/// what left an original's folder refused as "already owns another folder"
/// against the remake's title (#148).
pub(crate) fn find_existing_title_index_for_metadata_match(
    selected: &MetadataSearchItem,
    existing_titles: &[Title],
    existing_titles_by_name: &TitleNameIndex,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    if let Some(&index) = existing_titles_by_tvdb_id.get(&selected.tvdb_id) {
        return Some(index);
    }
    let key = normalize_title_key(&selected.name);
    let selected_year = selected.year.and_then(|year| u32::try_from(year).ok());
    existing_titles_by_name
        .get(&key)?
        .iter()
        .copied()
        .find(|index| {
            existing_titles
                .get(*index)
                .is_some_and(|title| title_year_compatible(title, selected_year))
        })
}

pub(crate) fn build_new_title_from_metadata_match(
    facet: &MediaFacet,
    selected: &MetadataSearchItem,
) -> NewTitle {
    NewTitle {
        name: selected.name.clone(),
        facet: facet.clone(),
        monitored: false,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".into(),
            value: selected.tvdb_id.clone(),
        }],
        min_availability: None,
        year: selected.year,
        ..Default::default()
    }
}
