use crate::library_scan::{
    DiscoveryContextChangeType, DiscoveryContextChangedSubjectInput, DiscoveryContextChangesInput,
    DiscoveryContextChangesResult, DiscoveryContextSnapshotPageResult,
    DiscoveryContextSnapshotSubmitInput, DiscoveryDashboardResult, DiscoveryDashboardSection,
    DiscoveryExternalIdInput, DiscoveryPublicFeedInput, DiscoverySubjectInput, DiscoveryTitle,
};
use crate::ports::{
    DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryFacetRecord, DiscoveryHomeQuery, DiscoveryHomeResult,
    DiscoveryItemRecord, DiscoveryItemsQuery, DiscoveryItemsResult,
    DiscoveryPendingContextChangeRecord, DiscoveryRawPageRecord, DiscoverySectionRecord,
    DiscoverySectionResult, DiscoverySubmittedSubjectRecord, DiscoverySyncStatus,
};
use crate::{AppError, AppResult, AppUseCase};
use chrono::{DateTime, Utc};
use scryer_domain::{
    DomainEvent, DomainEventPayload, DomainExternalIds, LibraryPermission, MediaFacet, Title,
    TitleContextSnapshot, User,
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

pub(crate) const DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS: usize = 250;

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

impl DiscoveryContextDefaults {
    pub(crate) fn public_feed_input(&self) -> DiscoveryPublicFeedInput {
        DiscoveryPublicFeedInput {
            region: self.region.clone(),
            language: self.language.clone(),
            section_types: Vec::new(),
            limit_per_section: 25,
            include_unresolved: self.include_unresolved,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoverySubjectParts {
    facet: String,
    subject_key: String,
    subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

impl AppUseCase {
    pub async fn discovery_home(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        let can_view_personalized = self.discovery_actor_can_view_personalized(actor).await?;
        let status = self
            .load_discovery_sync_status_for_visibility(can_view_personalized)
            .await?;
        let limit = discovery_section_limit(query.limit_per_section);
        let include_unresolved = query.include_unresolved;

        let mut public_sections = Vec::new();
        if query.include_public {
            if let Some(public_run_id) = status.state.last_public_feed_generation_id.as_deref() {
                let sections = self
                    .services
                    .library
                    .discovery
                    .list_discovery_sections(public_run_id, Some("public"))
                    .await?;
                let public_items = self
                    .services
                    .library
                    .discovery
                    .list_discovery_items_for_generation(public_run_id)
                    .await?;
                public_sections =
                    public_section_results(sections, public_items, include_unresolved, limit);
            }
        }

        let mut personalized_sections = Vec::new();
        let mut complete_collection = None;
        let mut facets = Vec::new();
        if can_view_personalized && query.include_personalized {
            if let Some(context_run_id) = status.state.last_success_generation_id.as_deref() {
                let mut personalized_items = self
                    .services
                    .library
                    .discovery
                    .list_discovery_items_for_generation(context_run_id)
                    .await?;
                let submitted_subjects = self
                    .services
                    .library
                    .discovery
                    .list_discovery_submitted_subjects(context_run_id)
                    .await?;
                resolve_discovery_matched_subjects(&mut personalized_items, &submitted_subjects)?;
                complete_collection =
                    complete_collection_section(&personalized_items, include_unresolved, limit);
                personalized_sections =
                    personalized_section_results(&personalized_items, include_unresolved, limit);
                let facet_counts = local_facet_counts(&personalized_items, include_unresolved);
                facets = self
                    .services
                    .library
                    .discovery
                    .list_discovery_facets(context_run_id)
                    .await?
                    .into_iter()
                    .map(|mut facet| {
                        facet.local_count = Some(local_count_for_facet(&facet_counts, &facet));
                        facet
                    })
                    .collect();
            }
        }

        Ok(DiscoveryHomeResult {
            status,
            public_sections,
            personalized_sections,
            complete_collection,
            facets,
            can_view_personalized,
        })
    }

    pub async fn discovery_items(
        &self,
        actor: &User,
        query: DiscoveryItemsQuery,
    ) -> AppResult<DiscoveryItemsResult> {
        let can_view_personalized = self.discovery_actor_can_view_personalized(actor).await?;
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let mut items = Vec::new();

        if can_view_personalized {
            if let Some(context_run_id) = state.last_success_generation_id.as_deref() {
                let mut personalized_items = self
                    .services
                    .library
                    .discovery
                    .list_discovery_items_for_generation(context_run_id)
                    .await?;
                let submitted_subjects = self
                    .services
                    .library
                    .discovery
                    .list_discovery_submitted_subjects(context_run_id)
                    .await?;
                resolve_discovery_matched_subjects(&mut personalized_items, &submitted_subjects)?;
                items.extend(personalized_items);
            }
            if query.include_public {
                if let Some(public_run_id) = state.last_public_feed_generation_id.as_deref() {
                    items.extend(
                        self.services
                            .library
                            .discovery
                            .list_discovery_items_for_generation(public_run_id)
                            .await?,
                    );
                }
            }
        } else if let Some(public_run_id) = state.last_public_feed_generation_id.as_deref() {
            items.extend(
                self.services
                    .library
                    .discovery
                    .list_discovery_items_for_generation(public_run_id)
                    .await?,
            );
        }

        let mut items = items
            .into_iter()
            .filter(|item| item_matches_discovery_items_query(item, &query))
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut items);
        let total_count = items.len() as i64;
        let offset = query.offset.min(items.len());
        let limit = discovery_items_limit(query.limit);
        let items = items.into_iter().skip(offset).take(limit).collect();

        Ok(DiscoveryItemsResult {
            items,
            total_count,
            can_view_personalized,
        })
    }

    async fn discovery_actor_can_view_personalized(&self, actor: &User) -> AppResult<bool> {
        Ok(!self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .is_empty())
    }

    async fn load_discovery_sync_status_for_visibility(
        &self,
        can_view_personalized: bool,
    ) -> AppResult<DiscoverySyncStatus> {
        let mut state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let mut recent_runs = self
            .services
            .library
            .discovery
            .list_recent_discovery_sync_runs(10)
            .await?;
        let mut pending_context_change_count = self
            .services
            .library
            .discovery
            .count_pending_discovery_context_changes(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?;

        if !can_view_personalized {
            state.last_success_generation_id = None;
            state.last_subject_fingerprint = None;
            state.last_context_snapshot_completed_at = None;
            state.last_incremental_reload_completed_at = None;
            state.dirty_since = None;
            state.dirty_reason_mask = 0;
            state.bootstrap_started_at = None;
            state.bootstrap_quiet_until = None;
            state.next_context_snapshot_eligible_at = None;
            state.next_incremental_reload_eligible_at = None;
            state.backoff_until = None;
            state.inflight_subject_fingerprint = None;
            state.inflight_domain_event_sequence = None;
            recent_runs.retain(|run| run.kind == "public_feed");
            pending_context_change_count = 0;
        }

        Ok(DiscoverySyncStatus {
            state,
            recent_runs,
            pending_context_change_count,
        })
    }
}

fn discovery_section_limit(limit: usize) -> usize {
    if limit == 0 { 25 } else { limit.clamp(1, 100) }
}

fn discovery_items_limit(limit: usize) -> usize {
    if limit == 0 { 50 } else { limit.clamp(1, 200) }
}

fn public_section_results(
    sections: Vec<DiscoverySectionRecord>,
    items: Vec<DiscoveryItemRecord>,
    include_unresolved: bool,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    let mut items_by_section = HashMap::<String, Vec<DiscoveryItemRecord>>::new();
    for item in items {
        if !home_item_visible(&item, include_unresolved)
            || discovery_section_is_complete_the_collection(
                item.section_id.as_deref().unwrap_or(""),
            )
        {
            continue;
        }
        if let Some(section_id) = item.section_id.clone() {
            items_by_section.entry(section_id).or_default().push(item);
        }
    }

    sections
        .into_iter()
        .filter(|section| !discovery_section_is_complete_the_collection(&section.section_type))
        .filter_map(|section| {
            let mut items = items_by_section
                .remove(&section.section_id)
                .unwrap_or_default();
            dedupe_discovery_items_preserving_order(&mut items);
            section_result(
                section.section_id,
                section.section_type,
                section.title,
                section.surface,
                items,
                limit,
            )
        })
        .collect()
}

fn personalized_section_results(
    items: &[DiscoveryItemRecord],
    include_unresolved: bool,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    let section_specs = [
        ("FOR_YOU", "For You", None, 1usize),
        ("MOVIES_FOR_YOU", "Movies For You", Some("movie"), 6usize),
        ("SERIES_FOR_YOU", "Series For You", Some("series"), 6usize),
        ("ANIME_FOR_YOU", "Anime For You", Some("anime"), 6usize),
        ("BECAUSE_YOU_HAVE", "Because You Have", None, 1usize),
    ];

    section_specs
        .into_iter()
        .filter_map(|(section_type, title, media_kind, minimum_items)| {
            let mut section_items = items
                .iter()
                .filter(|item| home_item_visible(item, include_unresolved))
                .filter(|item| {
                    media_kind.is_none_or(|kind| {
                        discovery_item_media_kind(item).eq_ignore_ascii_case(kind)
                    })
                })
                .filter(|item| section_type != "BECAUSE_YOU_HAVE" || item.matched_subject_count > 0)
                .cloned()
                .collect::<Vec<_>>();
            dedupe_and_sort_discovery_items(&mut section_items);
            if section_items.len() < minimum_items {
                return None;
            }
            section_result(
                section_type.to_ascii_lowercase(),
                section_type.to_string(),
                title.to_string(),
                "personalized".to_string(),
                section_items,
                limit,
            )
        })
        .collect()
}

fn complete_collection_section(
    items: &[DiscoveryItemRecord],
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    let mut items = items
        .iter()
        .filter(|item| {
            item.target_kind.eq_ignore_ascii_case("movie")
                && !item.owned_in_input
                && (include_unresolved || item.resolved)
                && (item.tmdb_collection_id.is_some()
                    || item
                        .tmdb_collection_name
                        .as_deref()
                        .is_some_and(|name| !name.trim().is_empty()))
                && json_text_values(&item.relation_subtypes_json)
                    .iter()
                    .any(|value| value.eq_ignore_ascii_case("tmdb.collection"))
        })
        .cloned()
        .collect::<Vec<_>>();
    dedupe_and_sort_discovery_items(&mut items);
    section_result(
        "complete_the_collection".to_string(),
        "COMPLETE_THE_COLLECTION".to_string(),
        "Complete the Collection".to_string(),
        "personalized".to_string(),
        items,
        limit,
    )
}

fn section_result(
    section_id: String,
    section_type: String,
    title: String,
    surface: String,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    if items.is_empty() {
        return None;
    }
    let total_count = items.len() as i64;
    let items = items.into_iter().take(limit).collect();
    Some(DiscoverySectionResult {
        section_id,
        section_type,
        title,
        surface,
        total_count,
        items,
    })
}

fn home_item_visible(item: &DiscoveryItemRecord, include_unresolved: bool) -> bool {
    !item.owned_in_input && (include_unresolved || item.resolved)
}

fn discovery_item_media_kind(item: &DiscoveryItemRecord) -> &str {
    item.content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| item.target_kind.trim())
}

fn resolve_discovery_matched_subjects(
    items: &mut [DiscoveryItemRecord],
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
) -> AppResult<()> {
    let titles_by_subject_key = submitted_subjects
        .iter()
        .filter_map(|subject| {
            let title = subject.display_title.as_deref()?.trim();
            if title.is_empty() {
                return None;
            }
            Some((subject.subject_key.as_str(), title.to_string()))
        })
        .collect::<HashMap<_, _>>();

    for item in items {
        let titles = json_text_values(&item.matched_subject_keys_json)
            .into_iter()
            .filter_map(|key| titles_by_subject_key.get(key.as_str()).cloned())
            .collect::<Vec<_>>();
        item.matched_subject_titles_json =
            serde_json::to_string(&titles).map_err(discovery_json_error)?;
        item.matched_subject_count = titles.len() as i32;
    }

    Ok(())
}

fn item_matches_discovery_items_query(
    item: &DiscoveryItemRecord,
    query: &DiscoveryItemsQuery,
) -> bool {
    if !query.include_owned && item.owned_in_input {
        return false;
    }
    if !query.include_unresolved && !item.resolved {
        return false;
    }
    if !matches_optional_text_query(item, query.query.as_deref()) {
        return false;
    }
    if !query.target_kinds.is_empty()
        && !contains_case_insensitive(&query.target_kinds, discovery_item_media_kind(item))
    {
        return false;
    }
    if !query.sources.is_empty()
        && !json_or_text_contains_any(
            &item.sources_json,
            item.best_source.as_deref(),
            &query.sources,
        )
    {
        return false;
    }
    if !query.relation_types.is_empty()
        && !json_contains_any(&item.relation_types_json, &query.relation_types)
    {
        return false;
    }
    if !query.relation_subtypes.is_empty()
        && !json_contains_any(&item.relation_subtypes_json, &query.relation_subtypes)
    {
        return false;
    }
    if !query.genres.is_empty() && !json_contains_any(&item.genres_json, &query.genres) {
        return false;
    }
    if !query.status_tags.is_empty()
        && !json_contains_any(&item.status_tags_json, &query.status_tags)
    {
        return false;
    }
    if !query.facet_terms.is_empty()
        && !json_contains_any(&item.facet_terms_json, &query.facet_terms)
    {
        return false;
    }
    true
}

fn matches_optional_text_query(item: &DiscoveryItemRecord, query: Option<&str>) -> bool {
    let Some(query) = query.map(str::trim).filter(|query| !query.is_empty()) else {
        return true;
    };
    let query = query.to_ascii_lowercase();
    [
        Some(item.display_title.as_str()),
        item.original_title.as_deref(),
        item.sort_title.as_deref(),
        item.overview.as_deref(),
        item.tmdb_collection_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_ascii_lowercase().contains(&query))
}

fn dedupe_and_sort_discovery_items(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.target_key.clone()));
    items.sort_by(|left, right| compare_discovery_items(left, right));
}

fn dedupe_discovery_items_preserving_order(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.target_key.clone()));
}

fn compare_discovery_items(left: &DiscoveryItemRecord, right: &DiscoveryItemRecord) -> Ordering {
    right
        .rank_score
        .partial_cmp(&left.rank_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            left.sort_title
                .as_deref()
                .unwrap_or(&left.display_title)
                .cmp(right.sort_title.as_deref().unwrap_or(&right.display_title))
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

#[derive(Default)]
struct LocalFacetCounts {
    genres: HashMap<String, i64>,
    terms: HashMap<String, i64>,
}

fn local_facet_counts(items: &[DiscoveryItemRecord], include_unresolved: bool) -> LocalFacetCounts {
    let mut counts = LocalFacetCounts::default();
    for item in items
        .iter()
        .filter(|item| home_item_visible(item, include_unresolved))
    {
        for genre in normalized_json_text_values(&item.genres_json) {
            *counts.genres.entry(genre).or_default() += 1;
        }

        let mut terms = HashSet::new();
        terms.extend(normalized_json_text_values(&item.facet_terms_json));
        terms.extend(normalized_json_text_values(&item.context_terms_json));
        for term in terms {
            *counts.terms.entry(term).or_default() += 1;
        }
    }
    counts
}

fn local_count_for_facet(counts: &LocalFacetCounts, facet: &DiscoveryFacetRecord) -> i64 {
    let value = normalize_discovery_filter_value(&facet.facet_value);
    if value.is_empty() {
        return 0;
    }
    if facet.facet_name.eq_ignore_ascii_case("genre") {
        counts.genres.get(&value).copied().unwrap_or_default()
    } else {
        counts.terms.get(&value).copied().unwrap_or_default()
    }
}

fn json_contains_any(raw: &str, filters: &[String]) -> bool {
    let values = json_text_values(raw);
    filters.iter().any(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

fn json_or_text_contains_any(raw: &str, text: Option<&str>, filters: &[String]) -> bool {
    text.is_some_and(|text| {
        filters
            .iter()
            .any(|filter| text.eq_ignore_ascii_case(filter))
    }) || json_contains_any(raw, filters)
}

fn normalized_json_text_values(raw: &str) -> Vec<String> {
    json_text_values(raw)
        .into_iter()
        .map(|value| normalize_discovery_filter_value(&value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_discovery_filter_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn contains_case_insensitive(values: &[String], candidate: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(candidate))
}

fn json_text_values(raw: &str) -> Vec<String> {
    serde_json::from_str::<JsonValue>(raw)
        .map(|value| {
            let mut values = Vec::new();
            collect_json_text_values(&value, &mut values);
            values
        })
        .unwrap_or_default()
}

fn collect_json_text_values(value: &JsonValue, values: &mut Vec<String>) {
    match value {
        JsonValue::String(value) => values.push(value.clone()),
        JsonValue::Number(value) => values.push(value.to_string()),
        JsonValue::Bool(value) => values.push(value.to_string()),
        JsonValue::Array(items) => {
            for item in items {
                collect_json_text_values(item, values);
            }
        }
        JsonValue::Object(object) => {
            for value in object.values() {
                collect_json_text_values(value, values);
            }
        }
        JsonValue::Null => {}
    }
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

    pub(crate) fn incremental_changes_input(
        &self,
        defaults: &DiscoveryContextDefaults,
        pending_changes: &[DiscoveryPendingContextChangeRecord],
        previous_context_fingerprint: &str,
    ) -> AppResult<DiscoveryContextChangesInput> {
        let resolved_key_count = pending_context_changes_resolved_key_count(pending_changes)?;
        if resolved_key_count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS {
            return Err(AppError::Validation(format!(
                "discovery incremental reload resolves to {resolved_key_count} changed subjects, above SMG limit {}",
                DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS
            )));
        }

        let context_subject_keys = self
            .subjects
            .iter()
            .map(|subject| subject.subject_key.clone())
            .collect();
        let changed_subjects = pending_changes
            .iter()
            .map(changed_subject_from_pending)
            .collect::<AppResult<Vec<_>>>()?;
        Ok(DiscoveryContextChangesInput {
            context_subject_keys,
            changed_subjects,
            region: defaults.region.clone(),
            language: defaults.language.clone(),
            max_items: defaults.max_items as i32,
            include_owned: defaults.include_owned,
            include_unresolved: defaults.include_unresolved,
            context_fingerprint: Some(self.fingerprint.clone()),
            previous_context_fingerprint: Some(previous_context_fingerprint.to_string()),
        })
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

pub(crate) fn pending_context_change_from_domain_event(
    scope_key: &str,
    event: &DomainEvent,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    match &event.payload {
        DomainEventPayload::TitleAdded(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "added", None)
        }
        DomainEventPayload::TitleUpdated(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "updated", None)
        }
        DomainEventPayload::TitleDeleted(data) => {
            title_context_change_record(scope_key, event, &data.title, None, "removed", None)
        }
        DomainEventPayload::TitleRematched(data) => {
            let mut current_ids = data.title.external_ids.clone();
            current_ids.tvdb_id = Some(data.new_tvdb_id.clone());
            let previous_ids = data.old_tvdb_id.as_ref().map(|old_tvdb_id| {
                let mut external_ids = data.title.external_ids.clone();
                external_ids.tvdb_id = Some(old_tvdb_id.clone());
                external_ids
            });
            title_context_change_record(
                scope_key,
                event,
                &data.title,
                previous_ids.as_ref(),
                "rematched",
                Some(&current_ids),
            )
        }
        _ => Ok(None),
    }
}

fn build_discovery_library_subject(title: &Title) -> Option<DiscoveryLibrarySubject> {
    let parts =
        build_discovery_subject_parts(&title.facet, normalized_supported_external_ids(title))?;
    Some(DiscoveryLibrarySubject {
        title_id: title.id.clone(),
        title_name: title.name.clone(),
        facet: parts.facet,
        subject_key: parts.subject_key,
        subject: parts.subject,
        canonical: parts.canonical,
    })
}

fn title_context_change_record(
    scope_key: &str,
    event: &DomainEvent,
    title: &TitleContextSnapshot,
    previous_external_ids: Option<&DomainExternalIds>,
    change_type: &str,
    current_external_ids: Option<&DomainExternalIds>,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let current = match build_discovery_title_context_subject(
        title,
        current_external_ids.unwrap_or(&title.external_ids),
    ) {
        Some(subject) => subject,
        None => return Ok(None),
    };
    let previous = previous_external_ids
        .and_then(|external_ids| build_discovery_title_context_subject(title, external_ids));
    let title_id = event.title_id.clone();
    let identity = title_id.as_deref().unwrap_or(current.subject_key.as_str());
    let raw_subject_json = serde_json::to_string(&current.subject).map_err(discovery_json_error)?;
    let raw_previous_subject_json = previous
        .as_ref()
        .map(|subject| serde_json::to_string(&subject.subject).map_err(discovery_json_error))
        .transpose()?;

    Ok(Some(DiscoveryPendingContextChangeRecord {
        id: format!("{scope_key}:title:{identity}"),
        scope_key: scope_key.to_string(),
        subject_key: Some(current.subject_key),
        previous_subject_key: previous.map(|subject| subject.subject_key),
        change_type: change_type.to_string(),
        title_id,
        previous_title_id: None,
        library_facet: Some(current.facet),
        raw_subject_json: Some(raw_subject_json),
        raw_previous_subject_json,
        first_seen_sequence: Some(event.sequence),
        last_seen_sequence: Some(event.sequence),
        first_seen_at: event.occurred_at,
        last_seen_at: event.occurred_at,
    }))
}

pub(crate) fn coalesce_pending_context_change(
    existing: Option<&DiscoveryPendingContextChangeRecord>,
    incoming: DiscoveryPendingContextChangeRecord,
) -> AppResult<Option<DiscoveryPendingContextChangeRecord>> {
    let Some(existing) = existing else {
        return Ok(Some(incoming));
    };

    let existing_type = discovery_change_type_from_str(&existing.change_type)?;
    let incoming_type = discovery_change_type_from_str(&incoming.change_type)?;

    if matches!(existing_type, DiscoveryContextChangeType::Added)
        && matches!(incoming_type, DiscoveryContextChangeType::Removed)
    {
        return Ok(None);
    }

    let mut merged = incoming;
    merged.id = existing.id.clone();
    merged.scope_key = existing.scope_key.clone();
    merged.first_seen_sequence = existing.first_seen_sequence.or(merged.first_seen_sequence);
    merged.first_seen_at = existing.first_seen_at;

    match (existing_type, incoming_type) {
        (DiscoveryContextChangeType::Added, _) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
        (_, DiscoveryContextChangeType::Removed) => {
            merged.change_type = "removed".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Added)
        | (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "rematched".to_string();
            merged.previous_subject_key = existing
                .previous_subject_key
                .clone()
                .or_else(|| existing.subject_key.clone());
            merged.raw_previous_subject_json = existing
                .raw_previous_subject_json
                .clone()
                .or_else(|| existing.raw_subject_json.clone());
            merged.previous_title_id = existing
                .previous_title_id
                .clone()
                .or_else(|| existing.title_id.clone());
        }
        (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Updated) => {
            merged.change_type = "updated".to_string();
            merged.previous_subject_key = existing.previous_subject_key.clone();
            merged.raw_previous_subject_json = existing.raw_previous_subject_json.clone();
            merged.previous_title_id = existing.previous_title_id.clone();
        }
        (DiscoveryContextChangeType::Removed, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Updated, DiscoveryContextChangeType::Rematched)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Updated)
        | (DiscoveryContextChangeType::Rematched, DiscoveryContextChangeType::Rematched) => {
            merged.change_type = "rematched".to_string();
            if merged.previous_subject_key.is_none() {
                merged.previous_subject_key = existing
                    .previous_subject_key
                    .clone()
                    .or_else(|| existing.subject_key.clone());
            }
            if merged.raw_previous_subject_json.is_none() {
                merged.raw_previous_subject_json = existing
                    .raw_previous_subject_json
                    .clone()
                    .or_else(|| existing.raw_subject_json.clone());
            }
            if merged.previous_title_id.is_none() {
                merged.previous_title_id = existing
                    .previous_title_id
                    .clone()
                    .or_else(|| existing.title_id.clone());
            }
        }
        (_, DiscoveryContextChangeType::Added) => {
            merged.change_type = "added".to_string();
            merged.previous_subject_key = None;
            merged.previous_title_id = None;
            merged.raw_previous_subject_json = None;
        }
    }

    Ok(Some(merged))
}

fn build_discovery_title_context_subject(
    title: &TitleContextSnapshot,
    external_ids: &DomainExternalIds,
) -> Option<DiscoverySubjectParts> {
    build_discovery_subject_parts(
        &title.facet,
        normalized_supported_domain_external_ids(external_ids),
    )
}

fn build_discovery_subject_parts(
    facet: &MediaFacet,
    external_ids: Vec<CanonicalExternalId>,
) -> Option<DiscoverySubjectParts> {
    if external_ids.is_empty() {
        return None;
    }

    let facet_name = facet.as_str().to_string();
    let kind = discovery_resolver_kind_from_facet(facet);
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
        facet: facet_name.clone(),
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

    Some(DiscoverySubjectParts {
        facet: facet_name,
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

fn normalized_supported_domain_external_ids(
    external_ids: &DomainExternalIds,
) -> Vec<CanonicalExternalId> {
    [
        ("tvdb", external_ids.tvdb_id.as_deref()),
        ("tmdb", external_ids.tmdb_id.as_deref()),
        ("anidb", external_ids.anidb_id.as_deref()),
    ]
    .into_iter()
    .filter_map(|(source, value)| normalize_supported_external_id(source, value?))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

fn discovery_resolver_kind_from_facet(facet: &MediaFacet) -> String {
    match facet {
        MediaFacet::Anime => "series".to_string(),
        _ => facet.as_str().to_string(),
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

pub(crate) fn context_changes_raw_page_record(
    run_id: &str,
    result: &DiscoveryContextChangesResult,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryRawPageRecord> {
    Ok(DiscoveryRawPageRecord {
        run_id: run_id.to_string(),
        payload_kind: "context_changes".to_string(),
        page_number: 0,
        compression: "none".to_string(),
        raw_payload: serde_json::to_string(result).map_err(discovery_json_error)?,
        created_at: now,
    })
}

pub(crate) fn public_feed_raw_page_record(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryRawPageRecord> {
    Ok(DiscoveryRawPageRecord {
        run_id: run_id.to_string(),
        payload_kind: "public_feed".to_string(),
        page_number: 0,
        compression: "none".to_string(),
        raw_payload: serde_json::to_string(result).map_err(discovery_json_error)?,
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
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_snapshot",
                None,
                index,
                item,
                now,
            )
        })
        .collect()
}

pub(crate) fn incremental_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            discovery_item_record(
                run_id,
                base_generation_id,
                "context_incremental",
                None,
                index,
                item,
                now,
            )
        })
        .collect()
}

pub(crate) fn public_feed_section_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoverySectionRecord>> {
    public_feed_sections(result)
        .enumerate()
        .map(|(index, section)| public_feed_section_record(run_id, index, section, now))
        .collect()
}

pub(crate) fn public_feed_item_records(
    run_id: &str,
    result: &DiscoveryDashboardResult,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let mut records = Vec::new();
    for (section_index, section) in public_feed_sections(result).enumerate() {
        for (item_index, item) in section.items.iter().enumerate() {
            let mut record = discovery_item_record(
                run_id,
                run_id,
                "public_feed",
                Some(section.section_id.clone()),
                section_index * 10_000 + item_index,
                item,
                now,
            )?;
            record.matched_subject_keys_json = "[]".to_string();
            record.matched_subject_titles_json = "[]".to_string();
            record.matched_subject_count = 0;
            records.push(record);
        }
    }
    Ok(records)
}

fn public_feed_sections(
    result: &DiscoveryDashboardResult,
) -> impl Iterator<Item = &DiscoveryDashboardSection> {
    result
        .sections
        .iter()
        .filter(|section| !discovery_section_is_complete_the_collection(&section.section_type))
}

fn discovery_section_is_complete_the_collection(section_type: &str) -> bool {
    section_type
        .trim()
        .eq_ignore_ascii_case("COMPLETE_THE_COLLECTION")
}

fn public_feed_section_record(
    run_id: &str,
    index: usize,
    section: &DiscoveryDashboardSection,
    now: DateTime<Utc>,
) -> AppResult<DiscoverySectionRecord> {
    Ok(DiscoverySectionRecord {
        id: format!("{run_id}:section:{index}"),
        run_id: run_id.to_string(),
        section_id: section.section_id.clone(),
        section_type: section.section_type.clone(),
        surface: "public".to_string(),
        title: section.title.clone(),
        source_signals_json: discovery_json_array(&section.source_signals)?,
        facets_json: discovery_json_array(&section.facets)?,
        sort_index: index as i32,
        raw_json: serde_json::to_string(section).map_err(discovery_json_error)?,
        created_at: now,
        updated_at: now,
    })
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

fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    source_run_kind: &str,
    section_id: Option<String>,
    index: usize,
    item: &DiscoveryTitle,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryItemRecord> {
    Ok(DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: source_run_kind.to_string(),
        section_id,
        target_key: item.target_key.clone(),
        target_kind: item.target_kind.clone(),
        resolved: item.resolved,
        // SMG does not know Scryer's local title ids; keep any SMG identifier in raw_json.
        resolved_title_id: None,
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

pub(crate) fn pending_context_changes_need_snapshot_reconciliation(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> bool {
    match pending_context_changes_resolved_key_count(pending_changes) {
        Ok(count) => count > DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS,
        Err(_) => true,
    }
}

pub(crate) fn pending_context_changes_resolved_key_count(
    pending_changes: &[DiscoveryPendingContextChangeRecord],
) -> AppResult<usize> {
    let mut keys = BTreeSet::new();
    for change in pending_changes {
        let change_type = discovery_change_type_from_str(&change.change_type)?;
        match change_type {
            DiscoveryContextChangeType::Added | DiscoveryContextChangeType::Updated => {
                keys.insert(required_pending_context_subject_key(change)?);
            }
            DiscoveryContextChangeType::Removed => {
                keys.insert(
                    change
                        .previous_subject_key
                        .clone()
                        .or_else(|| change.subject_key.clone())
                        .ok_or_else(|| {
                            AppError::Validation(format!(
                                "pending discovery removal {} is missing subject key",
                                change.id
                            ))
                        })?,
                );
            }
            DiscoveryContextChangeType::Rematched => {
                keys.insert(required_pending_context_subject_key(change)?);
                keys.insert(change.previous_subject_key.clone().ok_or_else(|| {
                    AppError::Validation(format!(
                        "pending discovery rematch {} is missing previous subject key",
                        change.id
                    ))
                })?);
            }
        }
    }
    Ok(keys.len())
}

fn required_pending_context_subject_key(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<String> {
    change.subject_key.clone().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing subject key",
            change.id
        ))
    })
}

fn changed_subject_from_pending(
    change: &DiscoveryPendingContextChangeRecord,
) -> AppResult<DiscoveryContextChangedSubjectInput> {
    let raw_subject = change.raw_subject_json.as_deref().ok_or_else(|| {
        AppError::Validation(format!(
            "pending discovery change {} is missing raw subject JSON",
            change.id
        ))
    })?;
    let subject = serde_json::from_str::<DiscoverySubjectInput>(raw_subject).map_err(|error| {
        AppError::Validation(format!(
            "pending discovery change {} has invalid raw subject JSON: {error}",
            change.id
        ))
    })?;
    let previous_subject = change
        .raw_previous_subject_json
        .as_deref()
        .map(|raw| {
            serde_json::from_str::<DiscoverySubjectInput>(raw).map_err(|error| {
                AppError::Validation(format!(
                    "pending discovery change {} has invalid previous subject JSON: {error}",
                    change.id
                ))
            })
        })
        .transpose()?;
    Ok(DiscoveryContextChangedSubjectInput {
        subject,
        change_type: discovery_change_type_from_str(&change.change_type)?,
        previous_subject,
    })
}

fn discovery_change_type_from_str(value: &str) -> AppResult<DiscoveryContextChangeType> {
    match value.trim().to_ascii_lowercase().as_str() {
        "added" | "add" => Ok(DiscoveryContextChangeType::Added),
        "updated" | "update" => Ok(DiscoveryContextChangeType::Updated),
        "removed" | "delete" | "deleted" => Ok(DiscoveryContextChangeType::Removed),
        "rematched" | "rematch" => Ok(DiscoveryContextChangeType::Rematched),
        other => Err(AppError::Validation(format!(
            "unsupported discovery context change type {other}"
        ))),
    }
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
    fn pending_context_change_coalescing_drops_add_then_delete() {
        let existing = test_pending_change("change-1", "added", 1, 10);
        let incoming = test_pending_change("change-1", "removed", 2, 10);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed");

        assert!(merged.is_none());
    }

    #[test]
    fn pending_context_change_coalescing_preserves_added_and_first_seen() {
        let existing = test_pending_change("change-1", "added", 1, 10);
        let incoming = test_pending_change("change-1", "updated", 4, 11);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "added");
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
        assert_eq!(merged.previous_subject_key, None);
    }

    #[test]
    fn pending_context_change_coalescing_update_then_delete_becomes_removed() {
        let existing = test_pending_change("change-1", "updated", 1, 10);
        let incoming = test_pending_change("change-1", "removed", 4, 10);

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "removed");
        assert_eq!(
            merged.previous_subject_key.as_deref(),
            Some("tmdb:movie:10")
        );
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
    }

    #[test]
    fn pending_context_change_coalescing_rematch_preserves_previous_subject() {
        let existing = test_pending_change("change-1", "updated", 1, 10);
        let mut incoming = test_pending_change("change-1", "rematched", 4, 11);
        incoming.previous_subject_key = Some("tmdb:movie:10".to_string());
        incoming.raw_previous_subject_json = existing.raw_subject_json.clone();

        let merged = coalesce_pending_context_change(Some(&existing), incoming)
            .expect("coalescing should succeed")
            .expect("change should remain pending");

        assert_eq!(merged.change_type, "rematched");
        assert_eq!(merged.subject_key.as_deref(), Some("tmdb:movie:11"));
        assert_eq!(
            merged.previous_subject_key.as_deref(),
            Some("tmdb:movie:10")
        );
        assert_eq!(merged.first_seen_sequence, Some(1));
        assert_eq!(merged.last_seen_sequence, Some(4));
    }

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

    #[test]
    fn discovery_item_records_do_not_persist_smg_resolved_title_id_as_local_fk() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tmdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            resolved_title_id: "smg-title-603".to_string(),
            display_title: "The Example".to_string(),
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resolved_title_id, None);
    }

    #[test]
    fn discovery_item_media_kind_prefers_content_type_over_target_kind() {
        let mut item = test_discovery_item("item-1", "series", Some("anime"));
        item.resolved = true;

        assert!(item_matches_discovery_items_query(
            &item,
            &DiscoveryItemsQuery {
                target_kinds: vec!["anime".to_string()],
                include_unresolved: false,
                ..DiscoveryItemsQuery::default()
            }
        ));
        assert!(!item_matches_discovery_items_query(
            &item,
            &DiscoveryItemsQuery {
                target_kinds: vec!["series".to_string()],
                include_unresolved: false,
                ..DiscoveryItemsQuery::default()
            }
        ));
    }

    #[test]
    fn local_facet_counts_parse_item_json_once_per_item() {
        let mut item = test_discovery_item("item-1", "series", Some("anime"));
        item.genres_json = serde_json::json!(["Action"]).to_string();
        item.facet_terms_json = serde_json::json!(["Most Popular Anime"]).to_string();
        item.context_terms_json =
            serde_json::json!(["Most Popular Anime", "Winter 2026"]).to_string();

        let counts = local_facet_counts(&[item], false);

        assert_eq!(
            local_count_for_facet(
                &counts,
                &DiscoveryFacetRecord {
                    run_id: "run-1".to_string(),
                    facet_name: "genre".to_string(),
                    facet_value: "action".to_string(),
                    smg_count: None,
                    local_count: None,
                    raw_json: "{}".to_string(),
                }
            ),
            1
        );
        assert_eq!(
            local_count_for_facet(
                &counts,
                &DiscoveryFacetRecord {
                    run_id: "run-1".to_string(),
                    facet_name: "facet_term".to_string(),
                    facet_value: "Most Popular Anime".to_string(),
                    smg_count: None,
                    local_count: None,
                    raw_json: "{}".to_string(),
                }
            ),
            1
        );
    }

    fn test_pending_change(
        id: &str,
        change_type: &str,
        sequence: i64,
        tmdb_id: i64,
    ) -> DiscoveryPendingContextChangeRecord {
        let observed_at = Utc.timestamp_opt(sequence, 0).unwrap();
        DiscoveryPendingContextChangeRecord {
            id: id.to_string(),
            scope_key: DISCOVERY_DEFAULT_SCOPE_KEY.to_string(),
            subject_key: Some(format!("tmdb:movie:{tmdb_id}")),
            previous_subject_key: None,
            change_type: change_type.to_string(),
            title_id: Some(id.to_string()),
            previous_title_id: None,
            library_facet: Some("movie".to_string()),
            raw_subject_json: Some(
                serde_json::json!({
                    "tmdbId": tmdb_id,
                    "kind": "movie",
                    "facet": "movie",
                    "externalIds": [{"source": "tmdb", "value": tmdb_id.to_string()}]
                })
                .to_string(),
            ),
            raw_previous_subject_json: None,
            first_seen_sequence: Some(sequence),
            last_seen_sequence: Some(sequence),
            first_seen_at: observed_at,
            last_seen_at: observed_at,
        }
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

    fn test_discovery_item(
        id: &str,
        target_kind: &str,
        content_type: Option<&str>,
    ) -> DiscoveryItemRecord {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        DiscoveryItemRecord {
            id: id.to_string(),
            run_id: "run-1".to_string(),
            base_generation_id: Some("run-1".to_string()),
            source_run_kind: "context_snapshot".to_string(),
            section_id: None,
            target_key: format!("{target_kind}:{id}"),
            target_kind: target_kind.to_string(),
            resolved: true,
            resolved_title_id: None,
            display_title: "Example".to_string(),
            original_title: None,
            sort_title: Some("Example".to_string()),
            year: None,
            poster_path: None,
            poster_url: None,
            background_url: None,
            overview: None,
            content_type: content_type.map(str::to_string),
            genres_json: "[]".to_string(),
            rating: None,
            rating_sources_json: "[]".to_string(),
            status_tags_json: "[]".to_string(),
            source_tags_json: "[]".to_string(),
            sources_json: "[]".to_string(),
            best_source: None,
            relation_types_json: "[]".to_string(),
            relation_subtypes_json: "[]".to_string(),
            chart_signals_json: "[]".to_string(),
            provider_signals_json: "[]".to_string(),
            rank_components_json: "[]".to_string(),
            source_count: None,
            edge_count: None,
            relation_count: None,
            source_subject_count: None,
            rank_score: None,
            matched_subject_keys_json: "[]".to_string(),
            matched_subject_titles_json: "[]".to_string(),
            matched_subject_count: 0,
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            facet_terms_json: "[]".to_string(),
            context_terms_json: "[]".to_string(),
            change_subject_keys_json: "[]".to_string(),
            removed_subject_keys_json: "[]".to_string(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            raw_json: "{}".to_string(),
            created_at: now,
            updated_at: now,
        }
    }
}
