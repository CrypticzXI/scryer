use crate::library_scan::{
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotSubmitInput,
    DiscoveryExternalIdInput, DiscoverySubjectInput, DiscoveryTitle,
};
use crate::ports::{
    DiscoveryFacetRecord, DiscoveryItemRecord, DiscoveryRawPageRecord,
    DiscoverySubmittedSubjectRecord,
};
use crate::{AppError, AppResult};
use chrono::{DateTime, Utc};
use scryer_domain::{MediaFacet, Title};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryContextDefaults {
    pub(crate) region: String,
    pub(crate) language: String,
    pub(crate) max_items: usize,
    pub(crate) include_owned: bool,
    pub(crate) include_unresolved: bool,
}

impl Default for DiscoveryContextDefaults {
    fn default() -> Self {
        Self {
            region: "US".to_string(),
            language: "eng".to_string(),
            max_items: 5_000,
            include_owned: true,
            include_unresolved: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLibraryContext {
    pub(crate) subjects: Vec<DiscoveryLibrarySubject>,
    pub(crate) fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLibrarySubject {
    pub(crate) title_id: String,
    pub(crate) title_name: String,
    pub(crate) facet: String,
    pub(crate) subject_key: String,
    pub(crate) subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalSubject {
    subject_key: String,
    key: Option<String>,
    kind: String,
    facet: String,
    external_ids: Vec<CanonicalExternalId>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalExternalId {
    source: String,
    value: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalContext<'a> {
    schema_version: u8,
    defaults: &'a DiscoveryContextDefaults,
    subjects: &'a [CanonicalSubject],
}

pub(crate) fn build_discovery_library_context(
    titles: &[Title],
    defaults: DiscoveryContextDefaults,
) -> DiscoveryLibraryContext {
    let mut subjects = titles
        .iter()
        .filter_map(build_discovery_library_subject)
        .collect::<Vec<_>>();

    subjects.sort_by(|left, right| {
        left.subject_key
            .cmp(&right.subject_key)
            .then_with(|| left.canonical.cmp(&right.canonical))
            .then_with(|| left.title_id.cmp(&right.title_id))
    });
    subjects.dedup_by(|left, right| left.subject_key == right.subject_key);

    let canonical_subjects = subjects
        .iter()
        .map(|subject| subject.canonical.clone())
        .collect::<Vec<_>>();

    DiscoveryLibraryContext {
        subjects,
        fingerprint: discovery_context_fingerprint(&defaults, &canonical_subjects),
    }
}

impl DiscoveryLibraryContext {
    pub(crate) fn subject_keys(&self) -> Vec<String> {
        self.subjects
            .iter()
            .map(|subject| subject.subject_key.clone())
            .collect()
    }

    pub(crate) fn snapshot_submit_input(
        &self,
        defaults: &DiscoveryContextDefaults,
    ) -> DiscoveryContextSnapshotSubmitInput {
        DiscoveryContextSnapshotSubmitInput {
            subjects: self
                .subjects
                .iter()
                .map(|subject| subject.subject.clone())
                .collect(),
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            context_fingerprint: Some(self.fingerprint.clone()),
        }
    }

    pub(crate) fn submitted_subject_records(
        &self,
        run_id: &str,
    ) -> AppResult<Vec<DiscoverySubmittedSubjectRecord>> {
        self.subjects
            .iter()
            .map(|subject| {
                let external_ids_json = serde_json::to_string(&subject.subject.external_ids)
                    .map_err(discovery_json_error)?;
                let raw_subject_json =
                    serde_json::to_string(&subject.subject).map_err(discovery_json_error)?;
                Ok(DiscoverySubmittedSubjectRecord {
                    run_id: run_id.to_string(),
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_facet: Some(subject.facet.clone()),
                    title_kind: subject.subject.kind.clone(),
                    display_title: Some(subject.title_name.clone()),
                    external_ids_json,
                    raw_subject_json,
                })
            })
            .collect()
    }
}

fn build_discovery_library_subject(title: &Title) -> Option<DiscoveryLibrarySubject> {
    let external_ids = normalized_supported_external_ids(title);
    if external_ids.is_empty() {
        return None;
    }

    let facet = title.facet.as_str().to_string();
    let kind = discovery_resolver_kind(title);
    let tvdb_id = unique_i32_external_id(&external_ids, "tvdb");
    let tmdb_id = unique_i32_external_id(&external_ids, "tmdb");
    let mal_id = unique_i32_external_id(&external_ids, "mal");
    let anidb_id = unique_i32_external_id(&external_ids, "anidb");
    let subject_key =
        fallback_discovery_subject_key(&kind, &external_ids, tvdb_id, tmdb_id, mal_id, anidb_id);
    let canonical = CanonicalSubject {
        subject_key: subject_key.clone(),
        key: None,
        kind,
        facet: facet.clone(),
        external_ids,
    };

    let subject = DiscoverySubjectInput {
        key: canonical.key.clone(),
        tvdb_id,
        tmdb_id,
        mal_id,
        anidb_id,
        kind: Some(canonical.kind.clone()),
        facet: Some(canonical.facet.clone()),
        external_ids: canonical
            .external_ids
            .iter()
            .map(|external_id| DiscoveryExternalIdInput {
                source: external_id.source.clone(),
                value: external_id.value.clone(),
            })
            .collect(),
    };

    Some(DiscoveryLibrarySubject {
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet,
        subject_key,
        subject,
        canonical,
    })
}

fn normalized_supported_external_ids(title: &Title) -> Vec<CanonicalExternalId> {
    title
        .external_ids
        .iter()
        .filter_map(|external_id| {
            normalize_supported_external_id(&external_id.source, &external_id.value)
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn discovery_resolver_kind(title: &Title) -> String {
    match &title.facet {
        MediaFacet::Anime => "series".to_string(),
        _ => title.facet.as_str().to_string(),
    }
}

fn normalize_supported_external_id(source: &str, value: &str) -> Option<CanonicalExternalId> {
    let source = normalize_supported_external_id_source(source)?;
    let value = parse_positive_external_numeric_id(value)?.to_string();
    Some(CanonicalExternalId { source, value })
}

fn normalize_supported_external_id_source(source: &str) -> Option<String> {
    match source.trim().to_ascii_lowercase().as_str() {
        "tvdb" | "thetvdb" | "tvdb_show" | "tvdb_series" | "tvdb_movie" => Some("tvdb".to_string()),
        "tmdb" | "themoviedb" | "tmdb_tv" | "tmdb_show" | "tmdb_series" | "tmdb_movie" => {
            Some("tmdb".to_string())
        }
        "anidb" => Some("anidb".to_string()),
        "mal" | "myanimelist" => Some("mal".to_string()),
        "anilist" | "anilist_anime" | "anilist:anime" => Some("anilist".to_string()),
        _ => None,
    }
}

fn parse_positive_external_numeric_id(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.rsplit(':').next().unwrap_or(value).trim();
    value.parse::<i64>().ok().filter(|id| *id > 0)
}

fn unique_i32_external_id(external_ids: &[CanonicalExternalId], source: &str) -> Option<i32> {
    let values = external_ids
        .iter()
        .filter(|external_id| external_id.source == source)
        .filter_map(|external_id| external_id.value.parse::<i32>().ok())
        .filter(|id| *id > 0)
        .collect::<BTreeSet<_>>();

    if values.len() == 1 {
        values.into_iter().next()
    } else {
        None
    }
}

fn fallback_discovery_subject_key(
    kind: &str,
    external_ids: &[CanonicalExternalId],
    tvdb_id: Option<i32>,
    tmdb_id: Option<i32>,
    mal_id: Option<i32>,
    anidb_id: Option<i32>,
) -> String {
    if let Some(tvdb_id) = tvdb_id {
        return format!("tvdb:{kind}:{tvdb_id}");
    }
    if let Some(tmdb_id) = tmdb_id {
        return format!("tmdb:{kind}:{tmdb_id}");
    }
    if let Some(mal_id) = mal_id {
        return format!("mal:anime:{mal_id}");
    }
    if let Some(anidb_id) = anidb_id {
        return format!("anidb:anime:{anidb_id}");
    }

    for source in ["tvdb", "tmdb", "anidb", "mal", "anilist"] {
        for external_id in external_ids
            .iter()
            .filter(|external_id| external_id.source == source)
        {
            match source {
                "tvdb" => return format!("tvdb:{kind}:{}", external_id.value),
                "tmdb" => return format!("tmdb:{kind}:{}", external_id.value),
                "mal" => return format!("mal:anime:{}", external_id.value),
                "anidb" => return format!("anidb:anime:{}", external_id.value),
                "anilist" => return format!("anilist:anime:{}", external_id.value),
                _ => {}
            }
        }
    }

    let bytes = serde_json::to_vec(external_ids)
        .expect("discovery subject key fallback input should always serialize");
    format!("local:{}", blake3::hash(&bytes).to_hex())
}

fn discovery_context_fingerprint(
    defaults: &DiscoveryContextDefaults,
    subjects: &[CanonicalSubject],
) -> String {
    let context = CanonicalContext {
        schema_version: 1,
        defaults,
        subjects,
    };
    let bytes = serde_json::to_vec(&context)
        .expect("discovery context fingerprint input should always serialize");
    format!("blake3:{}", blake3::hash(&bytes).to_hex())
}

pub(crate) fn snapshot_raw_page_record(
    run_id: &str,
    page: &DiscoveryContextSnapshotPageResult,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryRawPageRecord> {
    Ok(DiscoveryRawPageRecord {
        run_id: run_id.to_string(),
        payload_kind: "snapshot_page".to_string(),
        page_number: page.page,
        compression: "none".to_string(),
        raw_payload: serde_json::to_string(page).map_err(discovery_json_error)?,
        created_at: now,
    })
}

pub(crate) fn snapshot_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| snapshot_item_record(run_id, base_generation_id, index, item, now))
        .collect()
}

pub(crate) fn snapshot_facet_records(
    run_id: &str,
    pages: &[DiscoveryContextSnapshotPageResult],
) -> AppResult<Vec<DiscoveryFacetRecord>> {
    let mut facets = Vec::new();
    for page in pages {
        for group in &page.facets {
            for value in &group.values {
                let raw_json = serde_json::to_string(&serde_json::json!({
                    "name": group.name,
                    "value": value.value,
                    "count": value.count,
                }))
                .map_err(discovery_json_error)?;
                facets.push(DiscoveryFacetRecord {
                    run_id: run_id.to_string(),
                    facet_name: group.name.clone(),
                    facet_value: value.value.clone(),
                    smg_count: Some(i64::from(value.count)),
                    local_count: None,
                    raw_json,
                });
            }
        }
    }
    Ok(facets)
}

fn snapshot_item_record(
    run_id: &str,
    base_generation_id: &str,
    index: usize,
    item: &DiscoveryTitle,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryItemRecord> {
    Ok(DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: "context_snapshot".to_string(),
        section_id: None,
        target_key: item.target_key.clone(),
        target_kind: item.target_kind.clone(),
        resolved: item.resolved,
        resolved_title_id: non_empty_string(&item.resolved_title_id),
        display_title: item.display_title.clone(),
        original_title: non_empty_string(&item.original_title),
        sort_title: non_empty_string(&item.display_title),
        year: item.year,
        poster_path: non_empty_string(&item.poster_path),
        poster_url: non_empty_string(&item.poster_url),
        background_url: non_empty_string(&item.background_url),
        overview: non_empty_string(&item.overview),
        content_type: non_empty_string(&item.content_type),
        genres_json: discovery_json_array(&item.genres)?,
        rating: item.rating,
        rating_sources_json: discovery_json_array(&item.rating_sources)?,
        status_tags_json: discovery_json_array(&item.status_tags)?,
        source_tags_json: discovery_json_array(&item.source_tags)?,
        sources_json: discovery_json_array(&item.sources)?,
        best_source: non_empty_string(&item.best_source),
        relation_types_json: discovery_json_array(&item.relation_types)?,
        relation_subtypes_json: discovery_json_array(&item.relation_subtypes)?,
        chart_signals_json: discovery_json_array(&item.chart_signals)?,
        provider_signals_json: discovery_json_array(&item.provider_signals)?,
        rank_components_json: discovery_json_array(&item.rank_components)?,
        source_count: Some(item.source_count),
        edge_count: Some(item.edge_count),
        relation_count: Some(item.relation_count),
        source_subject_count: Some(item.source_subject_count),
        rank_score: Some(item.rank_score),
        matched_subject_keys_json: discovery_json_array(&item.matched_subject_keys)?,
        matched_subject_titles_json: discovery_json_array(&item.matched_subject_titles)?,
        matched_subject_count: item.matched_subject_count,
        tmdb_collection_id: item.tmdb_collection_id.map(|id| id.to_string()),
        tmdb_collection_name: non_empty_string(&item.tmdb_collection_name),
        owned_in_input: item.owned_in_input,
        facet_terms_json: discovery_json_array(&item.facet_terms)?,
        context_terms_json: discovery_json_array(&item.context_terms)?,
        change_subject_keys_json: discovery_json_array(&item.change_subject_keys)?,
        removed_subject_keys_json: discovery_json_array(&item.removed_subject_keys)?,
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        raw_json: serde_json::to_string(item).map_err(discovery_json_error)?,
        created_at: now,
        updated_at: now,
    })
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn discovery_json_array<T>(value: &T) -> AppResult<String>
where
    T: Serialize,
{
    serde_json::to_string(value).map_err(discovery_json_error)
}

fn discovery_json_error(error: serde_json::Error) -> AppError {
    AppError::Repository(format!("failed to encode discovery payload JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use scryer_domain::{ExternalId, MediaFacet};

    #[test]
    fn discovery_context_fingerprint_is_stable_across_title_and_external_id_order() {
        let left = build_discovery_library_context(
            &[
                test_title(
                    "series",
                    "The Example Show",
                    MediaFacet::Series,
                    vec![("tmdb_tv", "456"), ("thetvdb", "tvdb:123")],
                ),
                test_title(
                    "anime",
                    "Example Anime",
                    MediaFacet::Anime,
                    vec![("myanimelist", "7"), ("anilist:anime", "9")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );
        let right = build_discovery_library_context(
            &[
                test_title(
                    "anime",
                    "Example Anime",
                    MediaFacet::Anime,
                    vec![("anilist_anime", "9"), ("mal", "7")],
                ),
                test_title(
                    "series",
                    "The Example Show",
                    MediaFacet::Series,
                    vec![("tvdb_series", "123"), ("themoviedb", "456")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(left.fingerprint, right.fingerprint);
        assert_eq!(left.subjects, right.subjects);
    }

    #[test]
    fn discovery_context_only_builds_subjects_with_smg_supported_ids() {
        let mut imdb_only = test_title(
            "imdb-only",
            "IMDb Only",
            MediaFacet::Movie,
            vec![("imdb", "tt0133093")],
        );
        imdb_only.imdb_id = Some("tt0133093".to_string());

        let context = build_discovery_library_context(
            &[
                imdb_only,
                test_title(
                    "unsupported",
                    "Unsupported",
                    MediaFacet::Movie,
                    vec![("otherdb", "100")],
                ),
                test_title(
                    "movie",
                    "The Example Movie",
                    MediaFacet::Movie,
                    vec![("tmdb_movie", "movie:603")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(context.subjects.len(), 1);
        assert_eq!(context.subjects[0].title_id, "movie");
        assert_eq!(context.subjects[0].subject_key, "tmdb:movie:603");
        assert_eq!(context.subjects[0].subject.tmdb_id, Some(603));
        assert_eq!(
            context.subjects[0].subject.external_ids,
            vec![DiscoveryExternalIdInput {
                source: "tmdb".to_string(),
                value: "603".to_string(),
            }]
        );
    }

    #[test]
    fn discovery_context_uses_unique_typed_ids_and_keeps_external_ids() {
        let context = build_discovery_library_context(
            &[test_title(
                "series",
                "Series",
                MediaFacet::Series,
                vec![("tvdb", "10"), ("thetvdb", "11"), ("tmdb", "20")],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(context.subjects[0].subject_key, "tmdb:series:20");
        assert_eq!(subject.tvdb_id, None);
        assert_eq!(subject.tmdb_id, Some(20));
        assert_eq!(subject.kind.as_deref(), Some("series"));
        assert_eq!(subject.facet.as_deref(), Some("series"));
        assert_eq!(
            subject.external_ids,
            vec![
                DiscoveryExternalIdInput {
                    source: "tmdb".to_string(),
                    value: "20".to_string(),
                },
                DiscoveryExternalIdInput {
                    source: "tvdb".to_string(),
                    value: "10".to_string(),
                },
                DiscoveryExternalIdInput {
                    source: "tvdb".to_string(),
                    value: "11".to_string(),
                },
            ]
        );
    }

    #[test]
    fn discovery_context_uses_series_resolver_kind_for_anime_subjects() {
        let context = build_discovery_library_context(
            &[test_title(
                "anime",
                "Anime",
                MediaFacet::Anime,
                vec![("tvdb", "100"), ("mal", "200")],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(context.subjects[0].subject_key, "tvdb:series:100");
        assert_eq!(subject.kind.as_deref(), Some("series"));
        assert_eq!(subject.facet.as_deref(), Some("anime"));
        assert_eq!(subject.tvdb_id, Some(100));
        assert_eq!(subject.mal_id, Some(200));
    }

    #[test]
    fn discovery_context_deduplicates_identical_subjects() {
        let context = build_discovery_library_context(
            &[
                test_title(
                    "library-a",
                    "Movie A",
                    MediaFacet::Movie,
                    vec![("tmdb", "603")],
                ),
                test_title(
                    "library-b",
                    "Movie B",
                    MediaFacet::Movie,
                    vec![("tmdb_movie", "603")],
                ),
            ],
            DiscoveryContextDefaults::default(),
        );

        assert_eq!(context.subjects.len(), 1);
        assert_eq!(context.subjects[0].title_id, "library-a");
    }

    #[test]
    fn discovery_context_fallback_key_uses_external_id_priority_after_ambiguous_typed_ids() {
        let context = build_discovery_library_context(
            &[test_title(
                "anime",
                "Anime",
                MediaFacet::Anime,
                vec![
                    ("mal", "200"),
                    ("myanimelist", "201"),
                    ("anidb", "10"),
                    ("anidb", "11"),
                ],
            )],
            DiscoveryContextDefaults::default(),
        );

        let subject = &context.subjects[0].subject;
        assert_eq!(subject.mal_id, None);
        assert_eq!(subject.anidb_id, None);
        assert_eq!(context.subjects[0].subject_key, "anidb:anime:10");
    }

    fn test_title(
        id: &str,
        name: &str,
        facet: MediaFacet,
        external_ids: Vec<(&str, &str)>,
    ) -> Title {
        Title {
            id: id.to_string(),
            library_id: "library".to_string(),
            name: name.to_string(),
            facet,
            monitored: true,
            tags: Vec::new(),
            external_ids: external_ids
                .into_iter()
                .map(|(source, value)| ExternalId {
                    source: source.to_string(),
                    value: value.to_string(),
                })
                .collect(),
            root_folder_id: "root".to_string(),
            created_by: None,
            created_at: Utc.timestamp_opt(0, 0).unwrap(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
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
            tagged_aliases: Vec::new(),
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }
}
