use std::collections::HashMap;
use std::path::Path;

use scryer_domain::{Collection, ExternalId, MediaFacet, NewTitle, Title};

use crate::library_discovery::derive_movie_probe_path;
use crate::library_scan::MetadataSearchItem;

fn normalize_title_key(name: &str) -> String {
    crate::title_matching::canonical_lookup_key(name)
}

fn index_movie_title(
    title: &Title,
    index: usize,
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    existing_titles_by_imdb_id: &mut HashMap<String, usize>,
    existing_titles_by_tmdb_id: &mut HashMap<String, usize>,
) {
    existing_titles_by_name.insert(normalize_title_key(&title.name), index);
    for alias in &title.aliases {
        existing_titles_by_name.insert(normalize_title_key(alias), index);
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
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
) {
    existing_titles_by_name.insert(normalize_title_key(&title.name), index);
    for external_id in &title.external_ids {
        if external_id.source.eq_ignore_ascii_case("tvdb") {
            existing_titles_by_tvdb_id.insert(external_id.value.clone(), index);
        }
    }
}

pub(crate) fn append_movie_title(
    existing_titles: &mut Vec<Title>,
    existing_titles_by_name: &mut HashMap<String, usize>,
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
    existing_titles_by_name: &mut HashMap<String, usize>,
    existing_titles_by_tvdb_id: &mut HashMap<String, usize>,
    title: Title,
) -> usize {
    let index = existing_titles.len();
    existing_titles.push(title);
    index_series_title(
        &existing_titles[index],
        index,
        existing_titles_by_name,
        existing_titles_by_tvdb_id,
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
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        existing_titles_by_folder_path.insert(folder_path.to_string(), index);
    }
}

pub(crate) fn update_movie_probe_path_index(
    existing_titles_by_probe_path: &mut HashMap<String, usize>,
    root: &Path,
    file_path: &str,
    index: usize,
) {
    if let Some(parent) = Path::new(file_path).parent()
        && parent != root
    {
        existing_titles_by_probe_path.insert(parent.to_string_lossy().to_string(), index);
    } else {
        existing_titles_by_probe_path.insert(file_path.to_string(), index);
    }
}

pub(crate) type MovieTitleIndexes = (
    HashMap<String, usize>,
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

pub(crate) fn build_series_title_indexes(
    existing_titles: &[Title],
) -> (HashMap<String, usize>, HashMap<String, usize>) {
    let mut existing_titles_by_name = HashMap::new();
    let mut existing_titles_by_tvdb_id = HashMap::new();

    for (index, title) in existing_titles.iter().enumerate() {
        index_series_title(
            title,
            index,
            &mut existing_titles_by_name,
            &mut existing_titles_by_tvdb_id,
        );
    }

    (existing_titles_by_name, existing_titles_by_tvdb_id)
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
        if let Some(probe_path) = derive_movie_probe_path(root, title, &collections) {
            existing_titles_by_probe_path.insert(probe_path.to_string_lossy().to_string(), index);
        }
    }

    existing_titles_by_probe_path
}

pub(crate) fn find_existing_title_index_for_metadata_match(
    selected: &MetadataSearchItem,
    existing_titles_by_name: &HashMap<String, usize>,
    existing_titles_by_tvdb_id: &HashMap<String, usize>,
) -> Option<usize> {
    let key = normalize_title_key(&selected.name);
    existing_titles_by_tvdb_id
        .get(&selected.tvdb_id)
        .copied()
        .or_else(|| existing_titles_by_name.get(&key).copied())
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
