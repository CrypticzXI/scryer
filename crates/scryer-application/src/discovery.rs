use crate::library_scan::{
    DiscoveryContextChangeType, DiscoveryContextChangedSubjectInput, DiscoveryContextChangesInput,
    DiscoveryContextSnapshotPageResult, DiscoveryContextSnapshotSubmitInput,
    DiscoveryDashboardResult, DiscoveryDashboardSection, DiscoveryExternalIdInput,
    DiscoveryPublicFeedInput, DiscoverySubjectInput, DiscoveryTitle,
};
use crate::ports::{
    CatalogDiscoveryGroup, CatalogDiscoveryGroupKind, CatalogDiscoveryQuery,
    CatalogDiscoveryResult, CatalogDiscoverySectionCandidatesRecord, CatalogDiscoverySurface,
    DISCOVERY_DEFAULT_SCOPE_KEY, DiscoveryExternalIdRecord, DiscoveryFacetRecord,
    DiscoveryHomeQuery, DiscoveryHomeResult, DiscoveryItemDetailQuery,
    DiscoveryItemLibraryProvenanceRecord, DiscoveryItemRecord, DiscoveryItemsQuery,
    DiscoveryItemsResult, DiscoveryItemsStorageQuery, DiscoveryPendingContextChangeRecord,
    DiscoveryRankComponentRecord, DiscoverySectionItemsRecord, DiscoverySectionRecord,
    DiscoverySectionResult, DiscoverySourceTagRecord, DiscoverySubmittedSubjectRecord,
    DiscoverySyncStatus, TitleExternalIdLookup,
};
use crate::{AppError, AppResult, AppUseCase};
use chrono::{DateTime, Utc};
use scryer_domain::{
    CanonicalMediaTag, DomainEvent, DomainEventPayload, DomainExternalIds, ExternalId,
    LibraryPermission, MediaFacet, Title, TitleContextSnapshot, User, title_catalog_sort_input,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use tracing::warn;

pub(crate) const DISCOVERY_CONTEXT_CHANGES_MAX_CHANGED_SUBJECTS: usize = 250;
const DISCOVERY_DERIVED_SECTION_MINIMUM_ITEMS: usize = 2;
const DISCOVERY_HOME_MIN_CANDIDATES: usize = 500;
const DISCOVERY_HOME_MAX_CANDIDATES: usize = 2_000;
const DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES: usize = 100;
const DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryContextDefaults {
    pub(crate) region: String,
    pub(crate) language: String,
    pub(crate) max_items: usize,
    pub(crate) include_owned: bool,
    pub(crate) include_unresolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryLibraryProvenance {
    pub(crate) subject_key: String,
    pub(crate) title_id: Option<String>,
    pub(crate) library_id: String,
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
    pub(crate) subject_provenance: Vec<DiscoveryLibrarySubject>,
    pub(crate) fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveryLibrarySubject {
    pub(crate) title_id: String,
    pub(crate) library_id: String,
    pub(crate) title_name: String,
    pub(crate) facet: String,
    pub(crate) subject_key: String,
    pub(crate) subject: DiscoverySubjectInput,
    canonical: CanonicalSubject,
}

#[derive(Default)]
struct DiscoveryVisibility {
    readable_library_ids: HashSet<String>,
    allowed_media_kinds: HashSet<&'static str>,
}

impl DiscoveryVisibility {
    fn allows_facet(&self, facet: &MediaFacet) -> bool {
        self.allowed_media_kinds
            .contains(discovery_media_kind_for_facet(facet.clone()))
    }

    fn allows_item(&self, item: &DiscoveryItemRecord) -> bool {
        discovery_item_media_kind(item)
            .is_some_and(|media_kind| self.allowed_media_kinds.contains(media_kind))
    }

    fn sorted_allowed_media_kinds(&self) -> Vec<String> {
        let mut media_kinds = self
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        media_kinds.sort();
        media_kinds
    }
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
    pub async fn title_more_like_this(
        &self,
        actor: &User,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<DiscoveryItemRecord>> {
        let requested_limit = limit.clamp(0, 100) as usize;
        if requested_limit == 0 {
            return Ok(Vec::new());
        }
        let source_title = self
            .get_title(actor, title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        if let Err(error) = self
            .queue_title_more_like_this_refresh_if_due(
                &source_title,
                crate::catalog_workflow::HydrationSource::Interactive,
            )
            .await
        {
            warn!(
                title_id = %title_id,
                error = %error,
                "failed to refresh title recommendations while loading more-like-this cache"
            );
        }
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let candidate_limit = requested_limit.saturating_mul(4).clamp(24, 100) as i64;
        let mut items = self
            .services
            .library
            .discovery
            .list_title_more_like_this_items(title_id, candidate_limit)
            .await?;
        let mut item_lookup_indexes = vec![Vec::<usize>::new(); items.len()];
        let mut lookups = Vec::new();
        for item in &mut items {
            item.resolved_title_id = None;
            item.owned_in_input = false;
        }
        for (item_index, item) in items.iter().enumerate() {
            let Some((source, kind, value)) = discovery_target_key_parts(&item.target_key) else {
                continue;
            };
            let values = discovery_local_external_id_values(&kind, &value);
            for source in discovery_local_external_id_sources(&source, &kind) {
                for external_id in &values {
                    let lookup_index = lookups.len();
                    lookups.push(TitleExternalIdLookup {
                        lookup_index,
                        source: source.clone(),
                        external_id: external_id.clone(),
                    });
                    item_lookup_indexes[item_index].push(lookup_index);
                }
            }
        }
        let mut matches_by_lookup_index = HashMap::<usize, Vec<Title>>::new();
        for lookup_match in self
            .services
            .catalog
            .titles
            .list_by_external_id_lookups(&lookups)
            .await?
        {
            matches_by_lookup_index
                .entry(lookup_match.lookup_index)
                .or_default()
                .push(lookup_match.title);
        }
        let mut filtered_items = Vec::with_capacity(requested_limit.min(items.len()));
        for (mut item, lookup_indexes) in items.into_iter().zip(item_lookup_indexes) {
            let readable_local_title = lookup_indexes.iter().find_map(|lookup_index| {
                matches_by_lookup_index
                    .get(lookup_index)
                    .and_then(|titles| {
                        titles.iter().find(|candidate| {
                            readable_library_ids.contains(candidate.library_id.as_str())
                        })
                    })
            });
            if readable_local_title.is_some() {
                continue;
            }

            item.resolved = false;
            item.resolved_title_id = None;
            item.owned_in_input = false;
            filtered_items.push(item);
            if filtered_items.len() >= requested_limit {
                break;
            }
        }
        Ok(filtered_items)
    }

    pub async fn discovery_home(
        &self,
        actor: &User,
        query: DiscoveryHomeQuery,
    ) -> AppResult<DiscoveryHomeResult> {
        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        let can_view_personalized = !readable_library_ids.is_empty();
        let status = self
            .load_discovery_sync_status_for_visibility(can_view_personalized)
            .await?;
        let limit = discovery_section_limit(query.limit_per_section);
        let include_unresolved = query.include_unresolved;
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let mut allowed_media_kinds = visibility
            .allowed_media_kinds
            .iter()
            .map(|media_kind| (*media_kind).to_string())
            .collect::<Vec<_>>();
        allowed_media_kinds.sort();
        let owned_visibility = self
            .discovery_home_owned_visibility(&readable_library_id_list)
            .await?;

        let mut public_sections = Vec::<DiscoverySectionResult>::new();
        let mut top_rated_live_public_sections = Vec::<DiscoverySectionResult>::new();
        if query.include_public {
            let public_candidate_limit = public_home_candidate_limit(limit);
            if let Some(public_run_id) = status.state.last_public_feed_generation_id.as_deref() {
                let public_section_items = self
                    .services
                    .library
                    .discovery
                    .list_public_discovery_section_items(
                        public_run_id,
                        &allowed_media_kinds,
                        include_unresolved,
                        public_candidate_limit as i64,
                    )
                    .await?;
                let public_section_results = public_section_items
                    .into_iter()
                    .filter_map(section_items_record_to_result)
                    .collect::<Vec<_>>();
                public_sections = filter_discovery_sections_for_owned_items(
                    public_section_results,
                    &owned_visibility,
                    &visibility,
                    limit,
                );
            }
            if public_sections.is_empty() && status.state.last_success_generation_id.is_none() {
                let live_public_sections = self
                    .live_public_section_results(
                        &visibility,
                        include_unresolved,
                        public_candidate_limit,
                    )
                    .await?;
                top_rated_live_public_sections = filter_discovery_sections_for_owned_items(
                    live_public_sections.clone(),
                    &owned_visibility,
                    &visibility,
                    public_candidate_limit,
                );
                public_sections = filter_discovery_sections_for_owned_items(
                    live_public_sections,
                    &owned_visibility,
                    &visibility,
                    limit,
                );
            }
        }

        let mut personalized_sections = Vec::new();
        let mut complete_collection = None;
        let mut facets = Vec::new();
        if can_view_personalized
            && query.include_personalized
            && let Some(context_run_id) = status.state.last_success_generation_id.as_deref()
        {
            let mut personalized_items = self
                .services
                .library
                .discovery
                .list_personalized_discovery_home_items(
                    context_run_id,
                    &readable_library_id_list,
                    &allowed_media_kinds,
                    include_unresolved,
                    personalized_home_candidate_limit(limit) as i64,
                )
                .await?;
            personalized_items.retain(|item| visibility.allows_item(item));
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
            resolve_discovery_matched_subjects(&mut personalized_items, &submitted_subjects)?;
            let library_profile = self
                .discovery_library_affinity_profile(readable_library_ids, &submitted_subjects)
                .await?;
            let mut complete_collection_items = self
                .services
                .library
                .discovery
                .list_personalized_complete_collection_items(
                    context_run_id,
                    &readable_library_id_list,
                    &allowed_media_kinds,
                    include_unresolved,
                    complete_collection_candidate_limit(limit) as i64,
                )
                .await?;
            complete_collection_items.retain(|item| visibility.allows_item(item));
            resolve_discovery_matched_subjects(
                &mut complete_collection_items,
                &submitted_subjects,
            )?;
            complete_collection =
                complete_collection_section(&complete_collection_items, include_unresolved, limit);
            personalized_sections = personalized_section_results(
                &personalized_items,
                &library_profile,
                include_unresolved,
                limit,
            );
            facets = self
                .services
                .library
                .discovery
                .list_personalized_discovery_facets(
                    context_run_id,
                    &readable_library_id_list,
                    &allowed_media_kinds,
                    include_unresolved,
                )
                .await?;
        }

        let public_top_rated_run_id = query
            .include_public
            .then_some(status.state.last_public_feed_generation_id.as_deref())
            .flatten();
        let context_top_rated_run_id = if can_view_personalized && query.include_personalized {
            status.state.last_success_generation_id.as_deref()
        } else {
            None
        };
        let mut top_rated_items = self
            .services
            .library
            .discovery
            .list_discovery_home_top_rated_items(
                public_top_rated_run_id,
                context_top_rated_run_id,
                &readable_library_id_list,
                &allowed_media_kinds,
                &readable_library_id_list,
                &owned_visibility.excluded_discovery_identity_keys(),
                include_unresolved,
                top_rated_home_candidate_limit(limit) as i64,
            )
            .await?;
        top_rated_items
            .retain(|item| visibility.allows_item(item) && !owned_visibility.item_is_owned(item));

        if let Some(top_rated_section) = top_rated_discovery_home_section(
            &top_rated_items,
            &top_rated_live_public_sections,
            include_unresolved,
            limit,
        ) {
            if top_rated_section
                .items
                .iter()
                .any(discovery_home_item_is_personalized)
            {
                personalized_sections.push(top_rated_section);
            } else {
                public_sections.push(top_rated_section);
            }
        }

        let hero_item = select_discovery_home_hero(&public_sections, &personalized_sections);

        Ok(DiscoveryHomeResult {
            status,
            hero_item,
            public_sections,
            personalized_sections,
            complete_collection,
            facets,
            can_view_personalized,
        })
    }

    async fn live_public_section_results(
        &self,
        visibility: &DiscoveryVisibility,
        include_unresolved: bool,
        limit: usize,
    ) -> AppResult<Vec<DiscoverySectionResult>> {
        let defaults = DiscoveryContextDefaults {
            region: self.discovery_region().await,
            language: self.metadata_language().await,
            include_unresolved,
            ..DiscoveryContextDefaults::default()
        };
        let input = defaults.public_feed_input();
        let result = match self
            .services
            .library
            .metadata_gateway
            .discover_public_feed(&input)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                warn!(error = %error, "discovery live public feed fallback failed");
                return Ok(Vec::new());
            }
        };
        let run_id = format!("public-feed-live-{}", uuid::Uuid::new_v4());
        let now = self.runtime.environment.now();
        let sections = public_feed_section_records(&run_id, &result, now)?;
        let items = public_feed_item_records(&run_id, &result, now)?
            .into_iter()
            .filter(|item| visibility.allows_item(item))
            .collect();
        Ok(public_section_results(
            sections,
            items,
            include_unresolved,
            limit,
        ))
    }

    pub async fn discovery_items(
        &self,
        actor: &User,
        query: DiscoveryItemsQuery,
    ) -> AppResult<DiscoveryItemsResult> {
        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        let can_view_personalized = !readable_library_ids.is_empty();
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let limit = discovery_items_limit(query.limit);
        let offset = query.offset;
        let storage_query = DiscoveryItemsStorageQuery {
            context_run_id: can_view_personalized
                .then(|| state.last_success_generation_id.clone())
                .flatten(),
            public_run_id: (query.include_public || !can_view_personalized)
                .then(|| state.last_public_feed_generation_id.clone())
                .flatten(),
            readable_library_ids: readable_library_id_list,
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            filters: query,
            limit,
            offset,
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if let Some(context_run_id) = state.last_success_generation_id.as_deref() {
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
            resolve_discovery_matched_subjects(&mut page.items, &submitted_subjects)?;
        }

        Ok(DiscoveryItemsResult {
            items: page.items,
            total_count: page.total_count,
            can_view_personalized,
        })
    }

    pub async fn discovery_item_detail(
        &self,
        actor: &User,
        query: DiscoveryItemDetailQuery,
    ) -> AppResult<Option<DiscoveryItemRecord>> {
        let target_key = query.target_key.trim();
        if target_key.is_empty() {
            return Ok(None);
        }

        let visibility = self.discovery_visibility(actor).await?;
        let readable_library_ids = &visibility.readable_library_ids;
        let can_view_personalized = !readable_library_ids.is_empty();
        let readable_library_id_list = sorted_discovery_library_ids(readable_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let context_run_id = can_view_personalized
            .then(|| state.last_success_generation_id.clone())
            .flatten();
        let storage_query = DiscoveryItemsStorageQuery {
            context_run_id: context_run_id.clone(),
            public_run_id: state.last_public_feed_generation_id.clone(),
            readable_library_ids: readable_library_id_list,
            allowed_media_kinds: visibility.sorted_allowed_media_kinds(),
            filters: DiscoveryItemsQuery {
                target_keys: vec![target_key.to_string()],
                include_owned: true,
                include_unresolved: query.include_unresolved,
                include_public: true,
                limit: 1,
                offset: 0,
                ..DiscoveryItemsQuery::default()
            },
            limit: 1,
            offset: 0,
        };
        let mut page = self
            .services
            .library
            .discovery
            .query_discovery_items(&storage_query)
            .await?;
        if page.items.is_empty() {
            return Ok(None);
        }
        if let Some(context_run_id) = context_run_id.as_deref() {
            let submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            let submitted_subjects =
                filter_submitted_subjects_for_libraries(&submitted_subjects, readable_library_ids);
            resolve_discovery_matched_subjects(&mut page.items, &submitted_subjects)?;
        }

        Ok(page.items.into_iter().next())
    }

    pub async fn catalog_discovery(
        &self,
        actor: &User,
        query: CatalogDiscoveryQuery,
    ) -> AppResult<CatalogDiscoveryResult> {
        let visibility = self.discovery_visibility(actor).await?;
        if !visibility.allows_facet(&query.facet) {
            return Ok(CatalogDiscoveryResult {
                groups: Vec::new(),
                can_view_personalized: false,
            });
        }

        let readable_library_ids = self
            .authorized_library_ids(actor, Some(query.facet.clone()), LibraryPermission::View)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
        let requested_library_ids = query
            .library_ids
            .iter()
            .map(|library_id| library_id.trim())
            .filter(|library_id| !library_id.is_empty())
            .collect::<HashSet<_>>();
        let effective_library_ids = if requested_library_ids.is_empty() {
            readable_library_ids.clone()
        } else {
            readable_library_ids
                .iter()
                .filter(|library_id| requested_library_ids.contains(library_id.as_str()))
                .cloned()
                .collect()
        };
        let can_view_personalized = !effective_library_ids.is_empty();
        let effective_library_id_list = sorted_discovery_library_ids(&effective_library_ids);
        let state = self
            .services
            .library
            .discovery
            .get_discovery_sync_state(DISCOVERY_DEFAULT_SCOPE_KEY)
            .await?
            .unwrap_or_default();
        let media_kind = discovery_media_kind_for_facet(query.facet.clone());
        let limit = catalog_discovery_group_limit(query.limit_per_group);
        let max_groups = catalog_discovery_max_groups(query.max_groups);
        let candidate_limit = catalog_discovery_candidate_limit(limit, max_groups);
        let owned_visibility = self
            .catalog_owned_visibility(query.facet, &effective_library_id_list)
            .await?;
        let excluded_public_identity_keys = owned_visibility.excluded_discovery_identity_keys();

        let public_sections =
            if let Some(public_run_id) = state.last_public_feed_generation_id.as_deref() {
                self.services
                    .library
                    .discovery
                    .list_catalog_public_discovery_sections(
                        public_run_id,
                        &effective_library_id_list,
                        &excluded_public_identity_keys,
                        media_kind,
                        query.include_unresolved,
                        candidate_limit as i64,
                    )
                    .await?
            } else {
                Default::default()
            };

        let mut personalized_candidates = Vec::new();
        let mut submitted_subjects = Vec::new();
        if can_view_personalized
            && let Some(context_run_id) = state.last_success_generation_id.as_deref()
        {
            let mut candidates = self
                .services
                .library
                .discovery
                .list_catalog_personalized_discovery_items(
                    context_run_id,
                    &effective_library_id_list,
                    media_kind,
                    query.include_unresolved,
                    candidate_limit as i64,
                )
                .await?
                .items;
            candidates.retain(|item| !owned_visibility.item_is_owned(item));
            submitted_subjects = self
                .services
                .library
                .discovery
                .list_discovery_submitted_subjects(context_run_id)
                .await?;
            submitted_subjects = filter_submitted_subjects_for_libraries(
                &submitted_subjects,
                &effective_library_ids,
            );
            resolve_discovery_matched_subjects(&mut candidates, &submitted_subjects)?;
            personalized_candidates = candidates;
        }

        let mut groups = Vec::new();
        let mut emitted_item_keys = HashSet::new();
        let mut public_sections = public_sections.into_iter();
        if let Some(public_top_section) = public_sections.next()
            && let Some(group) = catalog_public_top_group(
                public_top_section,
                media_kind,
                limit,
                &mut emitted_item_keys,
            )
        {
            groups.push(group);
        }
        let remaining_public_sections = public_sections.collect::<Vec<_>>();
        let personalized_group_start = groups.len();

        if !personalized_candidates.is_empty() && groups.len() < max_groups {
            let library_profile = self
                .discovery_library_affinity_profile(&effective_library_ids, &submitted_subjects)
                .await?;
            catalog_personalized_groups(
                &mut groups,
                &personalized_candidates,
                &library_profile,
                limit,
                max_groups,
                &mut emitted_item_keys,
            );
        }

        if groups.len() == personalized_group_start {
            for public_section in remaining_public_sections {
                if groups.len() >= max_groups {
                    break;
                }
                if let Some(group) =
                    catalog_public_section_group(public_section, limit, &mut emitted_item_keys)
                {
                    groups.push(group);
                }
            }
        }

        Ok(CatalogDiscoveryResult {
            groups,
            can_view_personalized,
        })
    }

    async fn discovery_visibility(&self, actor: &User) -> AppResult<DiscoveryVisibility> {
        let requestable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::Request)
            .await?;
        let manageable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::ManageTitles)
            .await?;
        let readable_library_ids = self
            .authorized_library_ids(actor, None, LibraryPermission::View)
            .await?;

        let mut facets_by_library_id = self
            .services
            .catalog
            .libraries
            .list(None)
            .await?
            .into_iter()
            .map(|library| (library.id, library.facet))
            .collect::<HashMap<_, _>>();
        for facet in [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime] {
            facets_by_library_id
                .entry(scryer_domain::default_library_id_for_facet(&facet))
                .or_insert(facet);
        }

        let discoverable_library_ids = requestable_library_ids
            .into_iter()
            .chain(manageable_library_ids)
            .collect::<HashSet<_>>();
        let mut visibility = DiscoveryVisibility::default();
        for library_id in &discoverable_library_ids {
            if let Some(facet) = facets_by_library_id.get(library_id) {
                visibility
                    .allowed_media_kinds
                    .insert(discovery_media_kind_for_facet(facet.clone()));
            }
        }
        visibility
            .readable_library_ids
            .extend(readable_library_ids.into_iter().filter(|library_id| {
                facets_by_library_id.get(library_id).is_some_and(|facet| {
                    visibility
                        .allowed_media_kinds
                        .contains(discovery_media_kind_for_facet(facet.clone()))
                })
            }));
        Ok(visibility)
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

    async fn catalog_owned_visibility(
        &self,
        facet: MediaFacet,
        readable_library_ids: &[String],
    ) -> AppResult<CatalogOwnedVisibility> {
        if readable_library_ids.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(Some(facet), readable_library_ids, None)
            .await?;
        Ok(CatalogOwnedVisibility::from_titles(&titles))
    }

    async fn discovery_home_owned_visibility(
        &self,
        readable_library_ids: &[String],
    ) -> AppResult<CatalogOwnedVisibility> {
        if readable_library_ids.is_empty() {
            return Ok(CatalogOwnedVisibility::default());
        }
        let titles = self
            .services
            .catalog
            .titles
            .list_for_libraries(None, readable_library_ids, None)
            .await?;
        Ok(CatalogOwnedVisibility::from_titles(&titles))
    }
}

impl AppUseCase {
    async fn discovery_library_affinity_profile(
        &self,
        allowed_library_ids: &HashSet<String>,
        submitted_subjects: &[DiscoverySubmittedSubjectRecord],
    ) -> AppResult<DiscoveryLibraryAffinityProfile> {
        let mut library_ids = allowed_library_ids.iter().cloned().collect::<Vec<_>>();
        library_ids.sort();
        let mut titles = Vec::new();
        let mut seen_title_ids = HashSet::new();
        for title_id in submitted_subjects
            .iter()
            .filter_map(|subject| subject.title_id.as_deref())
            .filter(|title_id| seen_title_ids.insert((*title_id).to_string()))
        {
            if let Some(title) = self.services.catalog.titles.get_by_id(title_id).await?
                && allowed_library_ids.contains(&title.library_id)
            {
                titles.push(title);
            }
        }
        if titles.is_empty() {
            titles = self
                .services
                .catalog
                .titles
                .list_for_libraries(None, &library_ids, None)
                .await?;
        }
        Ok(DiscoveryLibraryAffinityProfile {
            genre_labels: top_owned_title_labels(
                &titles,
                |title| canonical_tag_labels(&title.canonical_tags, "genre"),
                2,
            ),
            tag_labels: top_owned_title_labels(&titles, |title| title.tags.iter(), 2),
        })
    }
}

fn discovery_section_limit(limit: usize) -> usize {
    if limit == 0 { 25 } else { limit.clamp(1, 100) }
}

fn public_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(1) * 4).clamp(section_limit, 100)
}

fn top_rated_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 80).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

fn personalized_home_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 40).clamp(DISCOVERY_HOME_MIN_CANDIDATES, DISCOVERY_HOME_MAX_CANDIDATES)
}

fn complete_collection_candidate_limit(section_limit: usize) -> usize {
    (section_limit.max(25) * 8).clamp(
        DISCOVERY_COMPLETE_COLLECTION_MIN_CANDIDATES,
        DISCOVERY_COMPLETE_COLLECTION_MAX_CANDIDATES,
    )
}

fn discovery_items_limit(limit: usize) -> usize {
    if limit == 0 { 50 } else { limit.clamp(1, 200) }
}

fn sorted_discovery_library_ids(library_ids: &HashSet<String>) -> Vec<String> {
    let mut library_ids = library_ids.iter().cloned().collect::<Vec<_>>();
    library_ids.sort();
    library_ids
}

fn section_items_record_to_result(
    record: DiscoverySectionItemsRecord,
) -> Option<DiscoverySectionResult> {
    if record.items.is_empty() {
        return None;
    }
    Some(DiscoverySectionResult {
        section_id: record.section.section_id,
        section_type: record.section.section_type,
        title: record.section.title,
        surface: record.section.surface,
        total_count: record.total_count,
        items: record.items,
    })
}

fn filter_discovery_sections_for_owned_items(
    sections: Vec<DiscoverySectionResult>,
    owned_visibility: &CatalogOwnedVisibility,
    visibility: &DiscoveryVisibility,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    sections
        .into_iter()
        .filter_map(|mut section| {
            let original_len = section.items.len();
            section.items.retain(|item| {
                visibility.allows_item(item) && !owned_visibility.item_is_owned(item)
            });
            if section.items.len() > limit {
                section.items.truncate(limit);
            }
            if section.items.is_empty() {
                return None;
            }
            let removed_count = original_len.saturating_sub(section.items.len()) as i64;
            section.total_count = section
                .total_count
                .saturating_sub(removed_count)
                .max(section.items.len() as i64);
            Some(section)
        })
        .collect()
}

fn select_discovery_home_hero(
    public_sections: &[DiscoverySectionResult],
    personalized_sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    select_personalized_discovery_home_hero(personalized_sections)
        .or_else(|| select_public_discovery_home_hero(public_sections))
}

fn top_rated_discovery_home_section(
    top_rated_items: &[DiscoveryItemRecord],
    live_public_sections: &[DiscoverySectionResult],
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    let mut candidates = top_rated_items
        .iter()
        .chain(
            live_public_sections
                .iter()
                .flat_map(|section| section.items.iter()),
        )
        .filter(|item| !item.owned_in_input && home_item_visible(item, include_unresolved))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(compare_top_rated_discovery_home_items);
    let mut seen = HashSet::new();
    candidates.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
    section_result(
        "top_rated".to_string(),
        "TOP_RATED".to_string(),
        "Top Rated".to_string(),
        "mixed".to_string(),
        candidates,
        limit,
    )
}

fn discovery_home_item_is_personalized(item: &DiscoveryItemRecord) -> bool {
    !item
        .source_run_kind
        .trim()
        .eq_ignore_ascii_case("public_feed")
}

fn select_personalized_discovery_home_hero(
    sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    let mut candidates = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .filter(|item| discovery_home_item_is_personalized(item))
        .filter(|item| !item.owned_in_input)
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(compare_personalized_discovery_home_hero_items);
    candidates.into_iter().next()
}

fn select_public_discovery_home_hero(
    sections: &[DiscoverySectionResult],
) -> Option<DiscoveryItemRecord> {
    let mut candidates = sections
        .iter()
        .flat_map(|section| section.items.iter())
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(compare_public_discovery_home_hero_items);
    candidates.into_iter().next()
}

fn compare_personalized_discovery_home_hero_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    discovery_item_has_hero_backdrop(right)
        .cmp(&discovery_item_has_hero_backdrop(left))
        .then_with(|| right.matched_subject_count.cmp(&left.matched_subject_count))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

fn compare_public_discovery_home_hero_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    discovery_item_has_hero_backdrop(right)
        .cmp(&discovery_item_has_hero_backdrop(left))
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| left.target_key.cmp(&right.target_key))
}

fn discovery_item_has_hero_backdrop(item: &DiscoveryItemRecord) -> bool {
    item.background_url
        .as_deref()
        .is_some_and(|url| !url.trim().is_empty())
}

fn compare_discovery_item_rating_desc(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    discovery_item_comparable_rating(right)
        .partial_cmp(&discovery_item_comparable_rating(left))
        .unwrap_or(Ordering::Equal)
}

fn compare_top_rated_discovery_home_items(
    left: &DiscoveryItemRecord,
    right: &DiscoveryItemRecord,
) -> Ordering {
    let left_external_rating = discovery_item_best_external_rating_score(left);
    let right_external_rating = discovery_item_best_external_rating_score(right);
    right_external_rating
        .is_some()
        .cmp(&left_external_rating.is_some())
        .then_with(|| compare_optional_f64_desc(left_external_rating, right_external_rating))
        .then_with(|| {
            discovery_item_external_rating_vote_count(right)
                .cmp(&discovery_item_external_rating_vote_count(left))
        })
        .then_with(|| compare_discovery_item_rating_desc(left, right))
        .then_with(|| compare_optional_f64_desc(left.rank_score, right.rank_score))
        .then_with(|| {
            right
                .source_count
                .unwrap_or_default()
                .cmp(&left.source_count.unwrap_or_default())
        })
        .then_with(|| discovery_item_identity_key(left).cmp(discovery_item_identity_key(right)))
}

fn discovery_item_best_external_rating_score(item: &DiscoveryItemRecord) -> Option<f64> {
    item.external_ratings
        .iter()
        .filter_map(|rating| {
            let normalized = rating.normalized;
            normalized.is_finite().then_some(if normalized <= 1.0 {
                normalized * 10.0
            } else {
                normalized
            })
        })
        .filter(|rating| *rating > 0.0)
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal))
}

fn discovery_item_external_rating_vote_count(item: &DiscoveryItemRecord) -> i32 {
    item.external_ratings
        .iter()
        .filter_map(|rating| rating.votes)
        .max()
        .unwrap_or_default()
}

fn compare_optional_f64_desc(left: Option<f64>, right: Option<f64>) -> Ordering {
    right
        .unwrap_or_default()
        .partial_cmp(&left.unwrap_or_default())
        .unwrap_or(Ordering::Equal)
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
    library_profile: &DiscoveryLibraryAffinityProfile,
    include_unresolved: bool,
    limit: usize,
) -> Vec<DiscoverySectionResult> {
    let visible_items = items
        .iter()
        .filter(|item| home_item_visible(item, include_unresolved))
        .cloned()
        .collect::<Vec<_>>();
    let mut sections = Vec::new();
    let mut emitted_item_keys = HashSet::new();

    sections.extend(label_affinity_sections(
        &visible_items,
        &library_profile.genre_labels,
        "genre",
        "BECAUSE_YOU_LIKE_GENRE",
        "because_you_like_genre",
        limit,
        &mut emitted_item_keys,
    ));
    sections.extend(label_affinity_sections(
        &visible_items,
        &library_profile.tag_labels,
        "theme",
        "BECAUSE_YOU_LIKE_TAG",
        "because_you_like_tag",
        limit,
        &mut emitted_item_keys,
    ));
    if let Some(section) =
        acclaimed_not_in_library_section(&visible_items, limit, &mut emitted_item_keys)
    {
        sections.push(section);
    }

    let section_specs = [
        ("FOR_YOU", "For You", None, 1usize),
        ("MOVIES_FOR_YOU", "Movies For You", Some("movie"), 6usize),
        ("SERIES_FOR_YOU", "Series For You", Some("series"), 6usize),
        ("ANIME_FOR_YOU", "Anime For You", Some("anime"), 6usize),
        ("BECAUSE_YOU_HAVE", "Because You Have", None, 1usize),
    ];

    sections.extend(section_specs.into_iter().filter_map(
        |(section_type, title, media_kind, minimum_items)| {
            let mut section_items = visible_items
                .iter()
                .filter(|item| {
                    media_kind.is_none_or(|kind| {
                        discovery_item_media_kind(item).is_some_and(|item_kind| item_kind == kind)
                    })
                })
                .filter(|item| section_type != "BECAUSE_YOU_HAVE" || item.matched_subject_count > 0)
                .cloned()
                .collect::<Vec<_>>();
            dedupe_and_sort_discovery_items(&mut section_items);
            if section_items.len() < minimum_items {
                return None;
            }
            section_result_excluding_emitted(
                section_type.to_ascii_lowercase(),
                section_type.to_string(),
                title.to_string(),
                "personalized".to_string(),
                section_items,
                limit,
                &mut emitted_item_keys,
            )
        },
    ));

    sections
}

fn canonical_affinity_labels_for_profile(
    items: &[DiscoveryItemRecord],
    profile_labels: &[String],
    canonical_kind: &str,
) -> Vec<String> {
    let mut canonical_labels_by_key = HashMap::new();
    for item in items {
        for label in discovery_item_canonical_facet_labels(item, canonical_kind) {
            let key = normalize_discovery_affinity_key(&label);
            if !key.is_empty() {
                canonical_labels_by_key.entry(key).or_insert(label);
            }
        }
    }

    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for profile_label in profile_labels {
        let key = normalize_discovery_affinity_key(profile_label);
        if let Some(label) = canonical_labels_by_key.get(&key) {
            push_unique_discovery_label(&mut labels, &mut seen, label.clone());
        }
    }
    labels
}

fn label_affinity_sections(
    items: &[DiscoveryItemRecord],
    labels: &[String],
    canonical_kind: &str,
    section_type: &str,
    section_id_prefix: &str,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Vec<DiscoverySectionResult> {
    let mut sections = Vec::new();
    for label in canonical_affinity_labels_for_profile(items, labels, canonical_kind) {
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, canonical_kind)
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(section) = section_result_excluding_emitted(
            format!(
                "{}_{}",
                section_id_prefix,
                slugify_discovery_section_part(&label)
            ),
            section_type.to_string(),
            format!("Because You Like {}", label),
            "personalized".to_string(),
            section_items,
            limit,
            emitted_item_keys,
        ) {
            sections.push(section);
        }
    }
    sections
}

fn acclaimed_not_in_library_section(
    items: &[DiscoveryItemRecord],
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<DiscoverySectionResult> {
    let mut section_items = items
        .iter()
        .filter(|item| discovery_item_is_acclaimed(item))
        .cloned()
        .collect::<Vec<_>>();
    dedupe_and_sort_discovery_items(&mut section_items);
    section_items.sort_by(|left, right| {
        discovery_item_comparable_rating(right)
            .partial_cmp(&discovery_item_comparable_rating(left))
            .unwrap_or(Ordering::Equal)
            .then_with(|| compare_discovery_items(left, right))
    });
    if section_items.len() < DISCOVERY_DERIVED_SECTION_MINIMUM_ITEMS {
        return None;
    }
    section_result_excluding_emitted(
        "acclaimed_not_in_library".to_string(),
        "TOP_RATED_ACCLAIMED_NOT_IN_LIBRARY".to_string(),
        "Acclaimed - Not in Your Library".to_string(),
        "personalized".to_string(),
        section_items,
        limit,
        emitted_item_keys,
    )
}

#[derive(Clone, Debug, Default)]
struct DiscoveryLibraryAffinityProfile {
    genre_labels: Vec<String>,
    tag_labels: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct CatalogOwnedVisibility {
    title_ids: HashSet<String>,
    keys: HashSet<String>,
    identity_keys: HashSet<String>,
}

impl CatalogOwnedVisibility {
    fn from_titles(titles: &[Title]) -> Self {
        let mut visibility = Self::default();
        for title in titles {
            visibility.title_ids.insert(title.id.clone());
            add_catalog_owned_external_keys(
                &mut visibility.keys,
                &mut visibility.identity_keys,
                "imdb",
                title.imdb_id.as_deref(),
                title.facet.clone(),
            );
            for external_id in &title.external_ids {
                add_catalog_owned_external_keys(
                    &mut visibility.keys,
                    &mut visibility.identity_keys,
                    &external_id.source,
                    Some(external_id.value.as_str()),
                    title.facet.clone(),
                );
            }
        }
        visibility
    }

    fn excluded_discovery_identity_keys(&self) -> Vec<String> {
        let mut keys = self.identity_keys.iter().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn item_is_owned(&self, item: &DiscoveryItemRecord) -> bool {
        if item.owned_in_input {
            return true;
        }
        if item
            .resolved_title_id
            .as_deref()
            .is_some_and(|title_id| self.title_ids.contains(title_id))
        {
            return true;
        }
        discovery_item_ownership_keys(item)
            .into_iter()
            .any(|key| self.keys.contains(&key))
    }
}

fn add_catalog_owned_external_keys(
    keys: &mut HashSet<String>,
    identity_keys: &mut HashSet<String>,
    source: &str,
    value: Option<&str>,
    facet: MediaFacet,
) {
    let raw_source = normalize_catalog_owned_key(source);
    let raw_value = normalize_catalog_owned_key(value.unwrap_or_default());
    if raw_source.is_empty() || raw_value.is_empty() {
        return;
    }

    let mut source_aliases = HashSet::from([raw_source]);
    if let Some(canonical_source) = normalize_supported_external_id_source(source) {
        source_aliases.insert(canonical_source);
    }
    let mut value_aliases = HashSet::from([raw_value]);
    if let Some(canonical_value) = parse_positive_external_numeric_id(value.unwrap_or_default()) {
        value_aliases.insert(canonical_value.to_string());
    }

    for source in source_aliases {
        for value in &value_aliases {
            insert_catalog_owned_key(keys, identity_keys, &source, value);
            insert_catalog_owned_key(
                keys,
                identity_keys,
                &source,
                &format!("{}:{value}", facet.as_str()),
            );
            if facet == MediaFacet::Anime {
                insert_catalog_owned_key(keys, identity_keys, &source, &format!("series:{value}"));
                insert_catalog_owned_key(keys, identity_keys, &source, &format!("anime:{value}"));
            }
        }
    }
}

fn insert_catalog_owned_key(
    keys: &mut HashSet<String>,
    identity_keys: &mut HashSet<String>,
    source: &str,
    value: &str,
) {
    let key = format!("{source}:{value}");
    keys.insert(key.clone());
    identity_keys.insert(key);
}

fn discovery_item_ownership_keys(item: &DiscoveryItemRecord) -> HashSet<String> {
    let mut keys = HashSet::new();
    let target_key = normalize_catalog_owned_key(&item.target_key);
    if !target_key.is_empty() {
        keys.insert(target_key);
    }
    let target_parts = item
        .target_key
        .split(':')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if target_parts.len() >= 3 {
        let source = normalize_catalog_owned_key(target_parts[0]);
        let value = normalize_catalog_owned_key(&target_parts[2..].join(":"));
        if !source.is_empty() && !value.is_empty() {
            keys.insert(format!("{source}:{value}"));
        }
    }
    keys
}

fn normalize_catalog_owned_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn discovery_media_kind_for_facet(facet: MediaFacet) -> &'static str {
    match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series => "series",
        MediaFacet::Anime => "anime",
    }
}

fn catalog_discovery_group_limit(limit: usize) -> usize {
    if limit == 0 { 12 } else { limit.clamp(1, 12) }
}

fn catalog_discovery_max_groups(max_groups: usize) -> usize {
    if max_groups == 0 {
        6
    } else {
        max_groups.clamp(1, 10)
    }
}

fn catalog_discovery_candidate_limit(limit: usize, max_groups: usize) -> usize {
    (limit.max(6) * max_groups.max(4) * 8).clamp(48, 400)
}

fn catalog_public_top_group(
    section: CatalogDiscoverySectionCandidatesRecord,
    media_kind: &str,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    catalog_group_excluding_emitted(
        CatalogDiscoveryGroupDraft {
            id: format!("public_top_{media_kind}"),
            kind: CatalogDiscoveryGroupKind::PublicTop,
            surface: CatalogDiscoverySurface::Public,
            label_value: None,
            total_count: Some(section.total_count),
        },
        section.items,
        limit,
        emitted_item_keys,
    )
}

fn catalog_public_section_group(
    section: CatalogDiscoverySectionCandidatesRecord,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    let id = format!(
        "public_section_{}",
        normalized_catalog_group_id(&section.section_id)
    );
    let label_value = if section.section_id == "evergreen_popular" {
        Some("Netflix Most Watched".to_string())
    } else {
        section.title.or(Some(section.section_type))
    };
    catalog_group_excluding_emitted(
        CatalogDiscoveryGroupDraft {
            id,
            kind: CatalogDiscoveryGroupKind::PublicSection,
            surface: CatalogDiscoverySurface::Public,
            label_value,
            total_count: Some(section.total_count),
        },
        section.items,
        limit,
        emitted_item_keys,
    )
}

fn normalized_catalog_group_id(value: &str) -> String {
    let normalized = normalize_discovery_affinity_key(value).replace(' ', "_");
    if normalized.is_empty() {
        "section".to_string()
    } else {
        normalized
    }
}

fn catalog_personalized_groups(
    groups: &mut Vec<CatalogDiscoveryGroup>,
    items: &[DiscoveryItemRecord],
    library_profile: &DiscoveryLibraryAffinityProfile,
    limit: usize,
    max_groups: usize,
    emitted_item_keys: &mut HashSet<String>,
) {
    let personalized_group_start = groups.len();

    for label in
        canonical_affinity_labels_for_profile(items, &library_profile.genre_labels, "genre")
    {
        if groups.len() >= max_groups {
            return;
        }
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, "genre")
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: format!("genre_{}", slugify_discovery_section_part(&label)),
                kind: CatalogDiscoveryGroupKind::GenreAffinity,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: Some(label),
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    for label in canonical_affinity_labels_for_profile(items, &library_profile.tag_labels, "theme")
    {
        if groups.len() >= max_groups {
            return;
        }
        let mut section_items = items
            .iter()
            .filter(|item| {
                item.matched_subject_count > 0
                    && discovery_item_matches_affinity_label(item, &label, "theme")
            })
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: format!("theme_{}", slugify_discovery_section_part(&label)),
                kind: CatalogDiscoveryGroupKind::ThemeAffinity,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: Some(label),
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    if groups.len() < max_groups {
        let mut section_items = items
            .iter()
            .filter(|item| discovery_item_is_acclaimed(item))
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        section_items.sort_by(|left, right| {
            discovery_item_comparable_rating(right)
                .partial_cmp(&discovery_item_comparable_rating(left))
                .unwrap_or(Ordering::Equal)
                .then_with(|| compare_discovery_items(left, right))
        });
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: "acclaimed_not_in_library".to_string(),
                kind: CatalogDiscoveryGroupKind::Acclaimed,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: None,
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    if groups.len() < max_groups {
        let mut section_items = items
            .iter()
            .filter(|item| discovery_item_has_collection_signal(item))
            .cloned()
            .collect::<Vec<_>>();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: "complete_the_collection".to_string(),
                kind: CatalogDiscoveryGroupKind::CompleteCollection,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: None,
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }

    if groups.len() == personalized_group_start && groups.len() < max_groups {
        let mut section_items = items.to_vec();
        dedupe_and_sort_discovery_items(&mut section_items);
        if let Some(group) = catalog_group_excluding_emitted(
            CatalogDiscoveryGroupDraft {
                id: "fallback".to_string(),
                kind: CatalogDiscoveryGroupKind::Fallback,
                surface: CatalogDiscoverySurface::Personalized,
                label_value: None,
                total_count: None,
            },
            section_items,
            limit,
            emitted_item_keys,
        ) {
            groups.push(group);
        }
    }
}

struct CatalogDiscoveryGroupDraft {
    id: String,
    kind: CatalogDiscoveryGroupKind,
    surface: CatalogDiscoverySurface,
    label_value: Option<String>,
    total_count: Option<i64>,
}

fn catalog_group_excluding_emitted(
    draft: CatalogDiscoveryGroupDraft,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<CatalogDiscoveryGroup> {
    let mut available = Vec::new();
    for item in items {
        let key = discovery_item_identity_key(&item).to_string();
        if emitted_item_keys.contains(&key) {
            continue;
        }
        available.push((key, item));
    }
    if available.is_empty() {
        return None;
    }

    let available_count = available.len() as i64;
    let mut items = Vec::new();
    for (key, item) in available.into_iter().take(limit) {
        emitted_item_keys.insert(key);
        items.push(item);
    }
    Some(CatalogDiscoveryGroup {
        id: draft.id,
        kind: draft.kind,
        surface: draft.surface,
        label_value: draft.label_value,
        total_count: draft.total_count.unwrap_or(available_count),
        items,
    })
}

const ACCLAIMED_SIGNALS: &[&str] = &[
    "acclaim",
    "award",
    "best picture",
    "top rated",
    "favorite",
    "critically",
];

fn discovery_item_canonical_facet_labels(item: &DiscoveryItemRecord, kind: &str) -> Vec<String> {
    let mut labels = Vec::new();
    let mut seen = HashSet::new();
    for label in item
        .facet_terms
        .iter()
        .filter_map(|term| canonical_discovery_facet_label(term, kind))
    {
        push_unique_discovery_label(&mut labels, &mut seen, label);
    }
    labels
}

fn discovery_item_matches_affinity_label(
    item: &DiscoveryItemRecord,
    label: &str,
    canonical_kind: &str,
) -> bool {
    let label_key = normalize_discovery_affinity_key(label);
    if label_key.is_empty() {
        return false;
    }
    discovery_item_canonical_facet_labels(item, canonical_kind)
        .into_iter()
        .any(|candidate| affinity_value_matches_label(&candidate, &label_key))
}

fn affinity_value_matches_label(value: &str, label_key: &str) -> bool {
    let value_key = normalize_discovery_affinity_key(value);
    value_key == label_key
}

#[cfg(test)]
fn discovery_item_matches_canonical_facet_filters(
    item: &DiscoveryItemRecord,
    kind: &str,
    filters: &[String],
) -> bool {
    let mut filter_keys = filters
        .iter()
        .map(|filter| normalize_discovery_filter_value(filter))
        .filter(|filter| !filter.is_empty())
        .collect::<HashSet<_>>();
    if filter_keys.is_empty() {
        return true;
    }
    item.facet_terms.iter().any(|term| {
        let Some(label) = canonical_discovery_facet_label(term, kind) else {
            return false;
        };
        filter_keys.contains(&normalize_discovery_filter_value(term))
            || filter_keys.remove(&normalize_discovery_filter_value(&label))
    })
}

fn push_unique_discovery_label(
    labels: &mut Vec<String>,
    seen: &mut HashSet<String>,
    label: String,
) {
    let label = label.trim();
    if label.is_empty() {
        return;
    }
    let key = normalize_discovery_filter_value(label);
    if seen.insert(key) {
        labels.push(label.to_string());
    }
}

fn canonical_tag_labels(tags: &[CanonicalMediaTag], category: &str) -> Vec<String> {
    tags.iter()
        .filter(|tag| tag.category.eq_ignore_ascii_case(category))
        .map(|tag| tag.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

fn top_owned_title_labels<'a, F, I>(
    titles: &'a [Title],
    labels_for_title: F,
    limit: usize,
) -> Vec<String>
where
    F: Fn(&'a Title) -> I,
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let mut counts = HashMap::<String, (String, usize)>::new();
    for title in titles {
        let mut seen_for_title = HashSet::new();
        for raw_label in labels_for_title(title) {
            let raw_label = raw_label.as_ref();
            let label_key = normalize_discovery_affinity_key(raw_label);
            if label_key.is_empty()
                || discovery_affinity_label_is_generic(&label_key)
                || raw_label.trim_start().starts_with("scryer:")
                || !seen_for_title.insert(label_key.clone())
            {
                continue;
            }
            let label = display_discovery_affinity_label(raw_label);
            let entry = counts.entry(label_key).or_insert((label, 0));
            entry.1 += 1;
        }
    }

    let mut counts = counts.into_values().collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    counts
        .into_iter()
        .take(limit)
        .map(|(label, _)| label)
        .collect()
}

fn display_discovery_affinity_label(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed
        .chars()
        .any(|character| character.is_ascii_uppercase())
    {
        return trimmed.to_string();
    }

    trimmed
        .split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn discovery_item_has_any_signal(item: &DiscoveryItemRecord, signals: &[&str]) -> bool {
    discovery_item_signal_values(item).into_iter().any(|value| {
        let value = value.to_ascii_lowercase();
        signals.iter().any(|signal| value.contains(signal))
    })
}

fn discovery_item_signal_values(item: &DiscoveryItemRecord) -> Vec<String> {
    let mut values = Vec::new();
    values.extend(item.status_tags.iter().cloned());
    values.extend(source_tag_text_values(&item.source_tags));
    values.extend(item.sources.iter().cloned());
    values.extend(item.relation_types.iter().cloned());
    values.extend(item.relation_subtypes.iter().cloned());
    values.extend(item.facet_terms.iter().cloned());
    values.extend(item.context_terms.iter().cloned());
    values.extend(item.chart_signals.iter().cloned());
    values.extend(item.provider_signals.iter().cloned());
    if let Some(best_source) = item.best_source.as_deref() {
        values.push(best_source.to_string());
    }
    if let Some(collection_name) = item.tmdb_collection_name.as_deref() {
        values.push(collection_name.to_string());
    }
    values
}

fn source_tag_text_values(tags: &[DiscoverySourceTagRecord]) -> Vec<String> {
    let mut values = Vec::new();
    for tag in tags {
        if let Some(category) = tag.category.as_deref().map(str::trim)
            && !category.is_empty()
        {
            values.push(category.to_string());
        }
        if let Some(name) = tag.name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            values.push(name.to_string());
        }
        values.extend(tag.values.iter().cloned());
    }
    values
}

fn discovery_item_is_acclaimed(item: &DiscoveryItemRecord) -> bool {
    discovery_item_comparable_rating(item) >= 8.0
        || discovery_item_has_any_signal(item, ACCLAIMED_SIGNALS)
}

fn discovery_item_comparable_rating(item: &DiscoveryItemRecord) -> f64 {
    item.rating
        .map(|rating| if rating <= 1.0 { rating * 10.0 } else { rating })
        .unwrap_or_default()
}

fn discovery_affinity_label_is_generic(normalized: &str) -> bool {
    matches!(
        normalized,
        "movie"
            | "movies"
            | "series"
            | "show"
            | "shows"
            | "anime"
            | "recommendation"
            | "recommendations"
            | "similar"
            | "relation"
            | "list"
            | "community"
            | "tmdb"
            | "tvdb"
            | "mal"
            | "anilist"
            | "myanimelist"
    )
}

fn normalize_discovery_affinity_key(value: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;
    for character in value.trim().chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            normalized.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !normalized.is_empty() {
            normalized.push(' ');
            last_was_separator = true;
        }
    }
    while normalized.ends_with(' ') {
        normalized.pop();
    }
    normalized
}

fn slugify_discovery_section_part(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('_');
            last_was_separator = true;
        }
    }
    while slug.ends_with('_') {
        slug.pop();
    }
    if slug.is_empty() {
        "section".to_string()
    } else {
        slug
    }
}

fn complete_collection_section(
    items: &[DiscoveryItemRecord],
    include_unresolved: bool,
    limit: usize,
) -> Option<DiscoverySectionResult> {
    let mut items = items
        .iter()
        .filter(|item| {
            discovery_item_media_kind(item) == Some("movie")
                && !item.owned_in_input
                && (include_unresolved || item.resolved)
                && discovery_item_has_collection_signal(item)
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

fn discovery_item_has_collection_signal(item: &DiscoveryItemRecord) -> bool {
    if item.tmdb_collection_id.is_some()
        || item
            .tmdb_collection_name
            .as_deref()
            .is_some_and(|name| !name.trim().is_empty())
    {
        return true;
    }

    item.relation_types
        .iter()
        .chain(item.relation_subtypes.iter())
        .any(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "tmdb.collection"
                || value.contains("collection")
                || value.contains("franchise")
        })
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

fn section_result_excluding_emitted(
    section_id: String,
    section_type: String,
    title: String,
    surface: String,
    items: Vec<DiscoveryItemRecord>,
    limit: usize,
    emitted_item_keys: &mut HashSet<String>,
) -> Option<DiscoverySectionResult> {
    let mut available = Vec::new();
    for item in items {
        let key = discovery_item_identity_key(&item).to_string();
        if emitted_item_keys.contains(&key) {
            continue;
        }
        available.push((key, item));
    }
    if available.is_empty() {
        return None;
    }

    let total_count = available.len() as i64;
    let mut items = Vec::new();
    for (key, item) in available.into_iter().take(limit) {
        emitted_item_keys.insert(key);
        items.push(item);
    }

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

fn discovery_item_media_kind(item: &DiscoveryItemRecord) -> Option<&'static str> {
    if let Some(content_type) = item
        .content_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return normalized_discovery_media_kind(content_type);
    }

    normalized_discovery_media_kind(&item.target_kind)
}

fn normalized_discovery_media_kind(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "anime" => Some("anime"),
        "movie" => Some("movie"),
        "series" => Some("series"),
        _ => None,
    }
}

fn resolve_discovery_matched_subjects(
    items: &mut [DiscoveryItemRecord],
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
) -> AppResult<()> {
    let mut titles_by_subject_key = HashMap::<&str, Vec<String>>::new();
    for subject in submitted_subjects {
        let Some(title) = subject.display_title.as_deref().map(str::trim) else {
            continue;
        };
        if !title.is_empty() {
            titles_by_subject_key
                .entry(subject.subject_key.as_str())
                .or_default()
                .push(title.to_string());
        }
    }

    for item in items {
        let titles = item
            .matched_subject_keys
            .iter()
            .flat_map(|key| {
                titles_by_subject_key
                    .get(key.as_str())
                    .cloned()
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        item.matched_subject_titles = titles;
        item.matched_subject_count = item.matched_subject_titles.len() as i32;
    }

    Ok(())
}

fn filter_submitted_subjects_for_libraries(
    submitted_subjects: &[DiscoverySubmittedSubjectRecord],
    readable_library_ids: &HashSet<String>,
) -> Vec<DiscoverySubmittedSubjectRecord> {
    submitted_subjects
        .iter()
        .filter(|subject| {
            subject
                .library_id
                .as_deref()
                .is_some_and(|library_id| readable_library_ids.contains(library_id))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
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
    if !query.target_keys.is_empty()
        && !contains_case_insensitive(&query.target_keys, item.target_key.as_str())
    {
        return false;
    }
    if !query.target_kinds.is_empty()
        && !discovery_item_media_kind(item)
            .is_some_and(|kind| contains_case_insensitive(&query.target_kinds, kind))
    {
        return false;
    }
    if !query.sources.is_empty()
        && !text_values_or_optional_contains_any(
            &item.sources,
            item.best_source.as_deref(),
            &query.sources,
        )
    {
        return false;
    }
    if !query.relation_types.is_empty()
        && !text_values_contain_any(&item.relation_types, &query.relation_types)
    {
        return false;
    }
    if !query.relation_subtypes.is_empty()
        && !text_values_contain_any(&item.relation_subtypes, &query.relation_subtypes)
    {
        return false;
    }
    if !query.genres.is_empty()
        && !discovery_item_matches_canonical_facet_filters(item, "genre", &query.genres)
    {
        return false;
    }
    if !query.status_tags.is_empty()
        && !text_values_contain_any(&item.status_tags, &query.status_tags)
    {
        return false;
    }
    if !query.facet_terms.is_empty()
        && !text_values_contain_any(&item.facet_terms, &query.facet_terms)
    {
        return false;
    }
    true
}

#[cfg(test)]
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
    items.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
    items.sort_by(compare_discovery_items);
}

fn dedupe_discovery_items_preserving_order(items: &mut Vec<DiscoveryItemRecord>) {
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(discovery_item_identity_key(item).to_string()));
}

fn discovery_item_identity_key(item: &DiscoveryItemRecord) -> &str {
    if item.target_key.trim().is_empty() {
        item.id.as_str()
    } else {
        item.target_key.as_str()
    }
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

#[cfg(test)]
fn text_values_contain_any(values: &[String], filters: &[String]) -> bool {
    filters.iter().any(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

#[cfg(test)]
fn text_values_or_optional_contains_any(
    values: &[String],
    text: Option<&str>,
    filters: &[String],
) -> bool {
    text.is_some_and(|text| {
        filters
            .iter()
            .any(|filter| text.eq_ignore_ascii_case(filter))
    }) || text_values_contain_any(values, filters)
}

fn normalize_discovery_filter_value(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
fn contains_case_insensitive(values: &[String], candidate: &str) -> bool {
    values
        .iter()
        .any(|value| value.eq_ignore_ascii_case(candidate))
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
    let mut subject_provenance = titles
        .iter()
        .filter_map(build_discovery_library_subject)
        .collect::<Vec<_>>();

    subject_provenance.sort_by(|left, right| {
        left.subject_key
            .cmp(&right.subject_key)
            .then_with(|| left.canonical.cmp(&right.canonical))
            .then_with(|| left.library_id.cmp(&right.library_id))
            .then_with(|| left.title_id.cmp(&right.title_id))
    });
    subject_provenance.dedup_by(|left, right| {
        left.subject_key == right.subject_key
            && left.title_id == right.title_id
            && left.library_id == right.library_id
    });

    let mut subjects = subject_provenance.clone();
    subjects.dedup_by(|left, right| left.subject_key == right.subject_key);

    let canonical_subjects = subjects
        .iter()
        .map(|subject| subject.canonical.clone())
        .collect::<Vec<_>>();

    DiscoveryLibraryContext {
        subjects,
        subject_provenance,
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
        self.subject_provenance
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
                    library_id: Some(subject.library_id.clone()),
                    library_facet: Some(subject.facet.clone()),
                    title_kind: subject.subject.kind.clone(),
                    display_title: Some(subject.title_name.clone()),
                    external_ids_json,
                    raw_subject_json,
                })
            })
            .collect()
    }

    pub(crate) fn subject_provenance_by_key(
        &self,
    ) -> HashMap<String, Vec<DiscoveryLibraryProvenance>> {
        let mut provenance_by_key = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
        for subject in &self.subject_provenance {
            provenance_by_key
                .entry(subject.subject_key.clone())
                .or_default()
                .push(DiscoveryLibraryProvenance {
                    subject_key: subject.subject_key.clone(),
                    title_id: Some(subject.title_id.clone()),
                    library_id: subject.library_id.clone(),
                });
        }
        provenance_by_key
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
        library_id: title.library_id.clone(),
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

pub(crate) fn snapshot_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
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
                provenance_by_subject_key,
                now,
            )
        })
        .collect()
}

pub(crate) fn incremental_item_records(
    run_id: &str,
    base_generation_id: &str,
    items: &[DiscoveryTitle],
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
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
                provenance_by_subject_key,
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
    let empty_provenance = HashMap::<String, Vec<DiscoveryLibraryProvenance>>::new();
    for (section_index, section) in public_feed_sections(result).enumerate() {
        for (item_index, item) in section.items.iter().enumerate() {
            let mut record = discovery_item_record(
                run_id,
                run_id,
                "public_feed",
                Some(section.section_id.clone()),
                section_index * 10_000 + item_index,
                item,
                &empty_provenance,
                now,
            )?;
            record.matched_subject_keys.clear();
            record.matched_subject_titles.clear();
            record.matched_subject_count = 0;
            record.library_provenance.clear();
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
        sort_index: index as i32,
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
                facets.push(DiscoveryFacetRecord {
                    run_id: run_id.to_string(),
                    facet_name: group.name.clone(),
                    facet_value: value.value.clone(),
                    smg_count: Some(i64::from(value.count)),
                    local_count: None,
                });
            }
        }
    }
    Ok(facets)
}

#[expect(
    clippy::too_many_arguments,
    reason = "discovery item persistence maps explicit run and projection fields"
)]
fn discovery_item_record(
    run_id: &str,
    base_generation_id: &str,
    source_run_kind: &str,
    section_id: Option<String>,
    index: usize,
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
    now: DateTime<Utc>,
) -> AppResult<DiscoveryItemRecord> {
    let library_provenance =
        discovery_item_library_provenance_records(item, provenance_by_subject_key);
    Ok(DiscoveryItemRecord {
        id: format!("{run_id}:item:{index}"),
        run_id: run_id.to_string(),
        base_generation_id: Some(base_generation_id.to_string()),
        source_run_kind: source_run_kind.to_string(),
        section_id,
        sort_index: index as i32,
        target_key: item.target_key.clone(),
        target_kind: item.target_kind.clone(),
        resolved: item.resolved,
        resolved_title_id: None,
        display_title: discovery_display_title(item).unwrap_or_default(),
        original_title: non_identifier_discovery_title(&item.original_title).map(str::to_string),
        sort_title: discovery_sort_title(item),
        year: item.year,
        poster_path: non_empty_string(&item.poster_path),
        poster_url: non_empty_string(&item.poster_url),
        background_url: non_empty_string(&item.background_url),
        overview: non_empty_string(&item.overview),
        content_type: non_empty_string(&item.content_type),
        canonical_tags: discovery_canonical_tags(item),
        rating: item.rating,
        rating_sources: item.rating_sources.clone(),
        external_ratings: item.external_ratings.clone(),
        external_ids: discovery_external_id_records(item),
        status_tags: item.status_tags.clone(),
        source_tags: discovery_source_tag_records(&item.source_tags),
        sources: item.sources.clone(),
        best_source: non_empty_string(&item.best_source),
        relation_types: item.relation_types.clone(),
        relation_subtypes: item.relation_subtypes.clone(),
        chart_signals: discovery_json_signal_values(&item.chart_signals),
        provider_signals: discovery_json_signal_values(&item.provider_signals),
        rank_components: discovery_rank_component_records(&item.rank_components),
        source_count: Some(item.source_count),
        edge_count: Some(item.edge_count),
        relation_count: Some(item.relation_count),
        source_subject_count: Some(item.source_subject_count),
        rank_score: Some(item.rank_score),
        matched_subject_keys: item.matched_subject_keys.clone(),
        matched_subject_titles: item.matched_subject_titles.clone(),
        matched_subject_count: item.matched_subject_count,
        library_provenance,
        tmdb_collection_id: item.tmdb_collection_id.map(|id| id.to_string()),
        tmdb_collection_name: non_empty_string(&item.tmdb_collection_name),
        owned_in_input: item.owned_in_input,
        studio_slug: item.studio_slug.clone(),
        person_ids: item.person_ids.clone(),
        facet_terms: discovery_canonical_facet_terms(item),
        context_terms: item.context_terms.clone(),
        change_subject_keys: item.change_subject_keys.clone(),
        removed_subject_keys: item.removed_subject_keys.clone(),
        tombstoned_by_run_id: None,
        tombstoned_at: None,
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn title_more_like_this_item_records(
    title_id: &str,
    source_target_keys: &[String],
    items: &[DiscoveryTitle],
    limit: usize,
    now: DateTime<Utc>,
) -> AppResult<Vec<DiscoveryItemRecord>> {
    let run_id = format!("title:{title_id}:more_like_this");
    let provenance = HashMap::new();
    let source_keys = source_target_keys
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    for item in items {
        let target_key = item.target_key.trim();
        if target_key.is_empty()
            || discovery_target_key_parts(target_key).is_none()
            || discovery_display_title(item).is_none()
            || source_keys.contains(target_key)
            || !seen.insert(target_key.to_string())
        {
            continue;
        }
        let mut record = discovery_item_record(
            &run_id,
            &run_id,
            "title_more_like_this",
            None,
            records.len(),
            item,
            &provenance,
            now,
        )?;
        record.base_generation_id = None;
        record.matched_subject_keys.clear();
        record.matched_subject_titles.clear();
        record.matched_subject_count = 0;
        record.library_provenance.clear();
        record.owned_in_input = false;
        record.resolved_title_id = None;
        records.push(record);
        if records.len() >= limit {
            break;
        }
    }
    Ok(records)
}

pub(crate) fn title_recommendations_subject(
    title: &Title,
    external_ids: &[ExternalId],
) -> Option<(DiscoverySubjectInput, Vec<String>)> {
    let mut ids = BTreeSet::<(String, String)>::new();
    for external_id in title.external_ids.iter().chain(external_ids.iter()) {
        let raw_source = external_id.source.trim().to_ascii_lowercase();
        let source = if matches!(raw_source.as_str(), "imdb" | "imdb_id") {
            "imdb".to_string()
        } else {
            let Some(source) = normalize_supported_external_id_source(&external_id.source) else {
                continue;
            };
            source
        };
        let value = external_id.value.trim();
        if value.is_empty() {
            continue;
        }
        let value = if source == "imdb" {
            let Some(normalized) = crate::normalize::normalize_imdb_id(value) else {
                continue;
            };
            normalized
        } else {
            let Some(id) = parse_positive_external_numeric_id(value) else {
                continue;
            };
            id.to_string()
        };
        ids.insert((source, value));
    }
    if let Some(imdb_id) = title
        .imdb_id
        .as_deref()
        .and_then(crate::normalize::normalize_imdb_id)
    {
        ids.insert(("imdb".to_string(), imdb_id));
    }

    if ids.is_empty() {
        return None;
    }

    let mut subject = DiscoverySubjectInput {
        kind: Some(title.facet.as_str().to_string()),
        facet: Some(title.facet.as_str().to_string()),
        ..Default::default()
    };
    let mut target_keys = Vec::new();
    for (source, value) in &ids {
        subject.external_ids.push(DiscoveryExternalIdInput {
            source: source.clone(),
            value: value.clone(),
        });
        match source.as_str() {
            "tvdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tvdb_id = subject.tvdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tvdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "tmdb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.tmdb_id = subject.tmdb_id.or(Some(id));
                    let key = keyed_discovery_target_key("tmdb", &title.facet, value);
                    target_keys.push(key);
                }
            }
            "mal" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.mal_id = subject.mal_id.or(Some(id));
                    let key = format!("mal:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anidb" => {
                if let Some(id) = parse_positive_i32(value) {
                    subject.anidb_id = subject.anidb_id.or(Some(id));
                    let key = format!("anidb:anime:{value}");
                    target_keys.push(key);
                }
            }
            "anilist" => {
                let key = format!("anilist:anime:{value}");
                target_keys.push(key);
            }
            "imdb" => {
                let key = format!("imdb:title:{value}");
                target_keys.push(key);
            }
            _ => {}
        }
    }
    subject.key = title_recommendations_preferred_subject_key(&ids, &title.facet);
    Some((subject, unique_discovery_text_terms(target_keys)))
}

fn title_recommendations_preferred_subject_key(
    ids: &BTreeSet<(String, String)>,
    facet: &MediaFacet,
) -> Option<String> {
    for source in ["tvdb", "tmdb", "mal", "anidb", "anilist", "imdb"] {
        let Some((_, value)) = ids.iter().find(|(candidate, _)| candidate == source) else {
            continue;
        };
        return Some(match source {
            "tvdb" | "tmdb" => keyed_discovery_target_key(source, facet, value),
            "mal" | "anidb" | "anilist" => format!("{source}:anime:{value}"),
            "imdb" => format!("imdb:title:{value}"),
            _ => unreachable!("source priority list contains only known sources"),
        });
    }
    None
}

fn keyed_discovery_target_key(source: &str, facet: &MediaFacet, value: &str) -> String {
    let kind = match facet {
        MediaFacet::Movie => "movie",
        MediaFacet::Series | MediaFacet::Anime => "series",
    };
    format!("{source}:{kind}:{value}")
}

fn parse_positive_i32(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok().filter(|value| *value > 0)
}

fn discovery_target_key_parts(target_key: &str) -> Option<(String, String, String)> {
    let mut parts = target_key.split(':');
    let source = parts.next()?.trim();
    let kind = parts.next()?.trim();
    let value = parts.next()?.trim();
    if source.is_empty() || kind.is_empty() || value.is_empty() {
        return None;
    }
    Some((
        source.to_ascii_lowercase(),
        kind.to_ascii_lowercase(),
        value.to_string(),
    ))
}

fn discovery_local_external_id_sources(source: &str, kind: &str) -> Vec<String> {
    match source {
        "tvdb" => match kind {
            "movie" => vec!["tvdb".to_string(), "tvdb_movie".to_string()],
            _ => vec![
                "tvdb".to_string(),
                "tvdb_series".to_string(),
                "tvdb_show".to_string(),
            ],
        },
        "tmdb" => match kind {
            "movie" => vec!["tmdb".to_string(), "tmdb_movie".to_string()],
            _ => vec![
                "tmdb".to_string(),
                "tmdb_series".to_string(),
                "tmdb_tv".to_string(),
                "tmdb_show".to_string(),
            ],
        },
        "mal" => vec!["mal".to_string(), "myanimelist".to_string()],
        "anilist" => vec![
            "anilist".to_string(),
            "anilist_anime".to_string(),
            "anilist:anime".to_string(),
        ],
        "anidb" => vec!["anidb".to_string()],
        "imdb" => vec!["imdb".to_string()],
        _ => vec![source.to_string()],
    }
}

fn discovery_local_external_id_values(kind: &str, value: &str) -> Vec<String> {
    unique_discovery_text_terms(vec![value.to_string(), format!("{kind}:{value}")])
}

fn discovery_item_library_provenance_records(
    item: &DiscoveryTitle,
    provenance_by_subject_key: &HashMap<String, Vec<DiscoveryLibraryProvenance>>,
) -> Vec<DiscoveryItemLibraryProvenanceRecord> {
    let mut provenance = Vec::new();
    let mut seen = BTreeSet::new();
    let mut push_subject_key = |subject_key: &str| {
        let subject_key = subject_key.trim();
        if subject_key.is_empty() {
            return;
        }
        let Some(entries) = provenance_by_subject_key.get(subject_key) else {
            return;
        };
        for entry in entries {
            if seen.insert((
                entry.subject_key.clone(),
                entry.title_id.clone(),
                entry.library_id.clone(),
            )) {
                provenance.push(DiscoveryItemLibraryProvenanceRecord {
                    subject_key: entry.subject_key.clone(),
                    title_id: entry.title_id.clone(),
                    library_id: Some(entry.library_id.clone()),
                });
            }
        }
    };

    for subject_key in &item.matched_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.change_subject_keys {
        push_subject_key(subject_key);
    }
    for subject_key in &item.removed_subject_keys {
        push_subject_key(subject_key);
    }
    if item.owned_in_input {
        push_subject_key(&item.target_key);
    }

    provenance
}

fn discovery_source_tag_records(values: &[JsonValue]) -> Vec<DiscoverySourceTagRecord> {
    values
        .iter()
        .map(|value| {
            let category = json_object_string(value, &["category", "type"]);
            let name = json_object_string(value, &["name", "label", "value"]);
            DiscoverySourceTagRecord {
                category,
                name,
                values: unique_json_text_values(value),
            }
        })
        .collect()
}

fn discovery_external_id_records(item: &DiscoveryTitle) -> Vec<DiscoveryExternalIdRecord> {
    item.external_ids
        .iter()
        .filter_map(|external_id| {
            let source = external_id.source.trim();
            let id = external_id.id.trim();
            let key = external_id.key.trim();
            if source.is_empty() || (id.is_empty() && key.is_empty()) {
                return None;
            }
            Some(DiscoveryExternalIdRecord {
                source: source.to_ascii_lowercase(),
                kind: external_id.kind.trim().to_ascii_lowercase(),
                id: id.to_string(),
                key: key.to_string(),
            })
        })
        .collect()
}

fn discovery_canonical_facet_terms(item: &DiscoveryTitle) -> Vec<String> {
    let mut values = item.facet_terms.clone();
    for canonical_tag in &item.canonical_tags {
        values.extend(canonical_discovery_terms_from_canonical_tag(canonical_tag));
    }
    unique_discovery_text_terms(values)
}

fn discovery_canonical_tags(item: &DiscoveryTitle) -> Vec<scryer_domain::CanonicalMediaTag> {
    item.canonical_tags
        .iter()
        .filter_map(|value| serde_json::from_value(value.clone()).ok())
        .collect()
}

fn canonical_discovery_terms_from_canonical_tag(value: &JsonValue) -> Vec<String> {
    let mut terms = unique_json_text_values(value)
        .into_iter()
        .filter_map(|value| canonical_discovery_term(&value).map(str::to_string))
        .collect::<Vec<_>>();
    if !terms.is_empty() {
        return unique_discovery_text_terms(terms);
    }

    if !value.is_object() {
        return Vec::new();
    }
    let Some(category) = json_object_string(value, &["category", "type"]) else {
        return Vec::new();
    };
    let category = category.trim().to_ascii_lowercase();
    if category != "genre" && category != "theme" {
        return Vec::new();
    }
    let Some(label) = json_object_string(value, &["key", "name", "label", "value"]) else {
        return Vec::new();
    };
    let label = label.trim();
    if label.is_empty() {
        return Vec::new();
    }
    let tail = label
        .rsplit(':')
        .next()
        .unwrap_or(label)
        .trim()
        .to_ascii_lowercase()
        .replace(|character: char| !character.is_ascii_alphanumeric(), "-")
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if tail.is_empty() {
        return Vec::new();
    }
    terms.push(format!("canonical:{category}:{tail}"));
    unique_discovery_text_terms(terms)
}

fn unique_discovery_text_terms(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            seen.insert(normalize_discovery_filter_value(&value))
                .then_some(value)
        })
        .collect()
}

fn canonical_discovery_term(value: &str) -> Option<&str> {
    let value = value.trim();
    if canonical_discovery_term_tail(value, "genre").is_some()
        || canonical_discovery_term_tail(value, "theme").is_some()
    {
        Some(value)
    } else {
        None
    }
}

fn canonical_discovery_term_tail<'a>(value: &'a str, kind: &str) -> Option<&'a str> {
    let value = value.trim();
    let mut parts = value.splitn(3, ':');
    if !parts.next()?.eq_ignore_ascii_case("canonical") {
        return None;
    }
    if !parts.next()?.eq_ignore_ascii_case(kind) {
        return None;
    }
    let tail = parts.next()?.trim();
    if tail.is_empty() {
        return None;
    }
    Some(tail)
}

fn canonical_discovery_facet_label(value: &str, kind: &str) -> Option<String> {
    canonical_discovery_term_tail(value, kind).map(format_canonical_discovery_label)
}

fn format_canonical_discovery_label(value: &str) -> String {
    value
        .split(|character: char| {
            character == '-' || character == '_' || character == ':' || character.is_whitespace()
        })
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            match characters.next() {
                Some(first) => {
                    let mut word = first.to_uppercase().collect::<String>();
                    word.extend(characters.flat_map(char::to_lowercase));
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn discovery_json_signal_values(values: &[JsonValue]) -> Vec<String> {
    let mut signals = Vec::new();
    let mut seen = HashSet::new();
    for value in values {
        for signal in unique_json_text_values(value) {
            let key = normalize_discovery_filter_value(&signal);
            if !key.is_empty() && seen.insert(key) {
                signals.push(signal);
            }
        }
    }
    signals
}

fn discovery_rank_component_records(values: &[JsonValue]) -> Vec<DiscoveryRankComponentRecord> {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| DiscoveryRankComponentRecord {
            component_index: index as i32,
            component_name: json_object_string(value, &["name", "key", "component", "type"]),
            component_value: json_object_string(
                value,
                &["value", "score", "weight", "contribution"],
            )
            .or_else(|| unique_json_text_values(value).first().cloned()),
        })
        .collect()
}

fn json_object_string(value: &JsonValue, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn json_scalar_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.clone()),
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn unique_json_text_values(value: &JsonValue) -> Vec<String> {
    let mut values = Vec::new();
    collect_json_text_values(value, &mut values);
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| {
            let value = value.trim().to_string();
            if value.is_empty() {
                return None;
            }
            let key = normalize_discovery_filter_value(&value);
            seen.insert(key).then_some(value)
        })
        .collect()
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

fn non_identifier_discovery_title(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() || value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let mut parts = value.splitn(3, ':');
    let Some(provider) = parts.next() else {
        return Some(value);
    };
    let Some(kind) = parts.next() else {
        return Some(value);
    };
    let Some(_) = parts.next() else {
        return Some(value);
    };
    let source_like = !provider.is_empty()
        && provider
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'))
        && provider
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        && !kind.is_empty()
        && kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-'));
    (!source_like).then_some(value)
}

fn discovery_display_title(item: &DiscoveryTitle) -> Option<String> {
    non_identifier_discovery_title(&item.display_title)
        .or_else(|| non_identifier_discovery_title(&item.original_title))
        .map(str::to_string)
}

fn discovery_sort_title(item: &DiscoveryTitle) -> Option<String> {
    let title = discovery_display_title(item)?;
    let sort_title = title_catalog_sort_input(&title);
    non_identifier_discovery_title(&sort_title)
        .or_else(|| non_identifier_discovery_title(&title))
        .map(str::to_string)
}

fn discovery_json_error(error: serde_json::Error) -> AppError {
    AppError::Repository(format!("failed to encode discovery payload JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TitleExternalRating;
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
    fn discovery_home_public_sections_filter_owned_catalog_titles() {
        let owned_visibility = CatalogOwnedVisibility::from_titles(&[test_title(
            "house-of-the-dragon",
            "House of the Dragon",
            MediaFacet::Series,
            vec![("tvdb", "371572")],
        )]);
        let mut owned_item = test_discovery_item("owned", "series", Some("series"));
        owned_item.target_key = "tvdb:series:371572".to_string();
        owned_item.display_title = "House of the Dragon".to_string();
        let mut visible_item = test_discovery_item("visible", "series", Some("series"));
        visible_item.target_key = "tmdb:series:100".to_string();
        visible_item.display_title = "Visible".to_string();
        let mut refill_item = test_discovery_item("refill", "series", Some("series"));
        refill_item.target_key = "tmdb:series:101".to_string();
        refill_item.display_title = "Refill".to_string();
        let visibility = DiscoveryVisibility {
            allowed_media_kinds: HashSet::from(["series"]),
            ..DiscoveryVisibility::default()
        };

        let sections = filter_discovery_sections_for_owned_items(
            vec![DiscoverySectionResult {
                section_id: "trending_now".to_string(),
                section_type: "TRENDING_NOW".to_string(),
                title: "Top Series This Week".to_string(),
                surface: "public".to_string(),
                total_count: 3,
                items: vec![owned_item, visible_item, refill_item],
            }],
            &owned_visibility,
            &visibility,
            2,
        );

        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].total_count, 2);
        assert_eq!(
            sections[0]
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Visible", "Refill"]
        );
    }

    #[test]
    fn discovery_home_top_rated_prefers_external_rating_provenance_and_dedupes() {
        let mut scalar_only = test_discovery_item("scalar", "movie", Some("movie"));
        scalar_only.source_run_kind = "public_feed".to_string();
        scalar_only.target_key = "tmdb:movie:scalar".to_string();
        scalar_only.rating = Some(10.0);
        scalar_only.rank_score = Some(100.0);

        let mut weaker_public_duplicate =
            test_discovery_item("shared-public", "movie", Some("movie"));
        weaker_public_duplicate.source_run_kind = "public_feed".to_string();
        weaker_public_duplicate.target_key = "tmdb:movie:shared".to_string();
        weaker_public_duplicate.rating = Some(6.0);
        weaker_public_duplicate.rank_score = Some(1.0);

        let mut external_rated = test_discovery_item("shared-context", "movie", Some("movie"));
        external_rated.target_key = "tmdb:movie:shared".to_string();
        external_rated.rating = Some(5.0);
        external_rated.external_ratings = vec![TitleExternalRating {
            source: "imdb".to_string(),
            value: Some(8.8),
            score: Some(8.8),
            normalized: 0.88,
            votes: Some(100_000),
            url: "https://imdb.com/title/tt0000001".to_string(),
        }];

        let section = top_rated_discovery_home_section(
            &[scalar_only, weaker_public_duplicate, external_rated],
            &[],
            true,
            10,
        )
        .expect("top rated section");

        assert_eq!(section.section_type, "TOP_RATED");
        assert_eq!(section.total_count, 2);
        assert_eq!(
            section
                .items
                .iter()
                .map(|item| item.target_key.as_str())
                .collect::<Vec<_>>(),
            vec!["tmdb:movie:shared", "tmdb:movie:scalar"]
        );
    }

    #[test]
    fn discovery_home_top_rated_keeps_short_sections() {
        let mut only_item = test_discovery_item("only", "series", Some("series"));
        only_item.source_run_kind = "public_feed".to_string();
        only_item.target_key = "tmdb:series:only".to_string();
        only_item.rating = Some(7.0);

        let section = top_rated_discovery_home_section(&[only_item], &[], true, 6)
            .expect("top rated section");

        assert_eq!(section.total_count, 1);
        assert_eq!(section.items.len(), 1);
        assert_eq!(section.items[0].target_key, "tmdb:series:only");
    }

    #[test]
    fn discovery_home_hero_prefers_visible_personalized_item() {
        let mut public_item = test_discovery_item("public", "movie", Some("movie"));
        public_item.target_key = "tmdb:movie:public".to_string();
        public_item.rating = Some(10.0);
        public_item.rank_score = Some(99.0);
        public_item.source_count = Some(9);
        public_item.background_url = Some("https://images.example/public.jpg".to_string());

        let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
        personalized_item.target_key = "tmdb:movie:personalized".to_string();
        personalized_item.rating = Some(1.0);
        personalized_item.rank_score = Some(1.0);
        personalized_item.matched_subject_count = 1;
        personalized_item.background_url =
            Some("https://images.example/personalized.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section("public", vec![public_item])],
            &[test_discovery_section(
                "personalized",
                vec![personalized_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:personalized");
    }

    #[test]
    fn discovery_home_hero_ignores_public_items_inside_mixed_personalized_sections() {
        let mut public_item = test_discovery_item("public", "movie", Some("movie"));
        public_item.source_run_kind = "public_feed".to_string();
        public_item.target_key = "tmdb:movie:public".to_string();
        public_item.rating = Some(10.0);
        public_item.rank_score = Some(100.0);
        public_item.background_url = Some("https://images.example/public.jpg".to_string());

        let mut personalized_item = test_discovery_item("personalized", "movie", Some("movie"));
        personalized_item.source_run_kind = "context_snapshot".to_string();
        personalized_item.target_key = "tmdb:movie:personalized".to_string();
        personalized_item.matched_subject_count = 1;
        personalized_item.rating = Some(1.0);
        personalized_item.rank_score = Some(1.0);
        personalized_item.background_url =
            Some("https://images.example/personalized.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[],
            &[test_discovery_section(
                "top_rated",
                vec![public_item, personalized_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:personalized");
    }

    #[test]
    fn discovery_home_hero_skips_owned_personalized_items() {
        let mut owned_item = test_discovery_item("owned", "series", Some("series"));
        owned_item.target_key = "tmdb:series:owned".to_string();
        owned_item.owned_in_input = true;
        owned_item.matched_subject_count = 100;
        owned_item.background_url = Some("https://images.example/owned.jpg".to_string());

        let mut visible_item = test_discovery_item("visible", "series", Some("series"));
        visible_item.target_key = "tmdb:series:visible".to_string();
        visible_item.matched_subject_count = 1;
        visible_item.background_url = Some("https://images.example/visible.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[],
            &[test_discovery_section(
                "personalized",
                vec![owned_item, visible_item],
            )],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:series:visible");
    }

    #[test]
    fn discovery_home_hero_falls_back_to_highest_rated_public_item() {
        let mut lower_rated = test_discovery_item("lower", "movie", Some("movie"));
        lower_rated.target_key = "tmdb:movie:lower".to_string();
        lower_rated.rating = Some(6.0);
        lower_rated.rank_score = Some(100.0);
        lower_rated.background_url = Some("https://images.example/lower.jpg".to_string());

        let mut higher_rated = test_discovery_item("higher", "movie", Some("movie"));
        higher_rated.target_key = "tmdb:movie:higher".to_string();
        higher_rated.rating = Some(8.5);
        higher_rated.rank_score = Some(1.0);
        higher_rated.background_url = Some("https://images.example/higher.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![lower_rated, higher_rated],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:higher");
    }

    #[test]
    fn discovery_home_hero_treats_blank_backdrop_as_missing() {
        let mut blank_backdrop = test_discovery_item("blank", "movie", Some("movie"));
        blank_backdrop.target_key = "tmdb:movie:blank".to_string();
        blank_backdrop.rating = Some(10.0);
        blank_backdrop.rank_score = Some(100.0);
        blank_backdrop.background_url = Some("   ".to_string());

        let mut real_backdrop = test_discovery_item("real", "movie", Some("movie"));
        real_backdrop.target_key = "tmdb:movie:real".to_string();
        real_backdrop.rating = Some(1.0);
        real_backdrop.rank_score = Some(1.0);
        real_backdrop.background_url = Some("https://images.example/real.jpg".to_string());

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![blank_backdrop, real_backdrop],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:movie:real");
    }

    #[test]
    fn discovery_home_hero_tie_breaks_by_target_key_without_raw_labels() {
        let mut later_key = test_discovery_item("later", "anime", Some("anime"));
        later_key.target_key = "tmdb:anime:z".to_string();
        later_key.background_url = Some("https://images.example/z.jpg".to_string());
        later_key.source_tags = vec![DiscoverySourceTagRecord {
            category: Some("theme".to_string()),
            name: Some("Isekai".to_string()),
            values: vec!["Isekai".to_string()],
        }];

        let mut earlier_key = test_discovery_item("earlier", "anime", Some("anime"));
        earlier_key.target_key = "tmdb:anime:a".to_string();
        earlier_key.background_url = Some("https://images.example/a.jpg".to_string());
        earlier_key.facet_terms = vec!["canonical:theme:isekai".to_string()];

        let hero = select_discovery_home_hero(
            &[test_discovery_section(
                "public",
                vec![later_key, earlier_key],
            )],
            &[],
        )
        .expect("hero item");

        assert_eq!(hero.target_key, "tmdb:anime:a");
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
    fn title_recommendations_subject_prefers_tvdb_then_tmdb_then_imdb() {
        let title = test_title(
            "movie",
            "Movie",
            MediaFacet::Movie,
            vec![("imdb", "tt0133093"), ("tmdb", "603"), ("tvdb", "78874")],
        );

        let (subject, source_target_keys) =
            title_recommendations_subject(&title, &[]).expect("subject should build");

        assert_eq!(subject.key.as_deref(), Some("tvdb:movie:78874"));
        assert_eq!(subject.tvdb_id, Some(78874));
        assert_eq!(subject.tmdb_id, Some(603));
        assert!(
            source_target_keys
                .iter()
                .any(|key| key == "imdb:title:tt0133093")
        );
        assert!(
            subject
                .external_ids
                .iter()
                .any(|external_id| external_id.source == "imdb")
        );
    }

    #[test]
    fn title_recommendations_subject_uses_anime_ids_after_tvdb_tmdb() {
        let tvdb_title = test_title(
            "anime-tvdb",
            "Anime",
            MediaFacet::Anime,
            vec![("mal", "200"), ("anidb", "10"), ("tvdb", "100")],
        );
        let (subject, _) =
            title_recommendations_subject(&tvdb_title, &[]).expect("subject should build");
        assert_eq!(subject.key.as_deref(), Some("tvdb:series:100"));

        let anime_id_title = test_title(
            "anime-mal",
            "Anime",
            MediaFacet::Anime,
            vec![("anidb", "10"), ("mal", "200"), ("anilist", "300")],
        );
        let (subject, _) =
            title_recommendations_subject(&anime_id_title, &[]).expect("subject should build");
        assert_eq!(subject.key.as_deref(), Some("mal:anime:200"));
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

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].resolved_title_id, None);
    }

    #[test]
    fn discovery_item_records_derive_local_sort_title_from_human_title() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tvdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            display_title: "tvdb:movie:603".to_string(),
            original_title: "\u{ff34}\u{ff48}\u{ff45} Matrix".to_string(),
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].display_title, "\u{ff34}\u{ff48}\u{ff45} Matrix");
        assert_eq!(records[0].sort_title.as_deref(), Some("Matrix"));
    }

    #[test]
    fn discovery_item_records_wire_canonical_genre_and_theme_terms() {
        let now = Utc.timestamp_opt(0, 0).unwrap();
        let item = DiscoveryTitle {
            target_key: "tmdb:movie:603".to_string(),
            target_kind: "movie".to_string(),
            resolved: true,
            display_title: "The Example".to_string(),
            source_tags: vec![
                serde_json::json!({
                    "source": "mal",
                    "category": "theme",
                    "name": "mal:theme:psychological",
                    "canonical": "canonical:theme:psychological"
                }),
                serde_json::json!("canonical:theme:survival"),
            ],
            canonical_tags: vec![
                serde_json::json!({
                    "key": "canonical:genre:action",
                    "category": "genre",
                    "name": "action",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "canonical:genre:drama",
                    "category": "genre",
                    "name": "Drama",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "canonical:theme:isekai",
                    "category": "theme",
                    "name": "Isekai",
                    "confidence": 1.0,
                }),
                serde_json::json!({
                    "key": "adult-cast",
                    "category": "theme",
                    "name": "Adult Cast",
                    "confidence": 1.0,
                }),
            ],
            facet_terms: vec![
                "raw:compat".to_string(),
                "canonical:genre:drama".to_string(),
            ],
            ..DiscoveryTitle::default()
        };

        let records = snapshot_item_records("run-1", "run-1", &[item], &HashMap::new(), now)
            .expect("discovery item records should build");

        assert_eq!(records.len(), 1);
        assert!(records[0].facet_terms.contains(&"raw:compat".to_string()));
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:genre:action".to_string())
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:genre:drama".to_string())
        );
        assert_eq!(
            records[0]
                .facet_terms
                .iter()
                .filter(|term| term.as_str() == "canonical:genre:action")
                .count(),
            1
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:theme:isekai".to_string())
        );
        assert!(
            records[0]
                .facet_terms
                .contains(&"canonical:theme:adult-cast".to_string())
        );
        assert!(
            !records[0]
                .facet_terms
                .contains(&"canonical:theme:psychological".to_string())
        );
    }

    #[test]
    fn discovery_item_genre_query_uses_canonical_facet_terms() {
        fn matches_genre(item: &DiscoveryItemRecord, genre: &str) -> bool {
            item_matches_discovery_items_query(
                item,
                &DiscoveryItemsQuery {
                    genres: vec![genre.to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
            )
        }

        let mut item = test_discovery_item("canonical", "movie", Some("movie"));
        item.facet_terms = vec!["canonical:genre:action".to_string()];

        assert!(matches_genre(&item, "Action"));
        assert!(matches_genre(&item, "canonical:genre:action"));
        assert!(!matches_genre(&item, "Drama"));
    }

    #[test]
    fn discovery_item_media_kind_uses_v1_content_type_contract() {
        fn matches_target_kind(item: &DiscoveryItemRecord, target_kind: &str) -> bool {
            item_matches_discovery_items_query(
                item,
                &DiscoveryItemsQuery {
                    target_kinds: vec![target_kind.to_string()],
                    include_unresolved: false,
                    ..DiscoveryItemsQuery::default()
                },
            )
        }

        let anime = test_discovery_item("anime", "series", Some("anime"));
        assert!(matches_target_kind(&anime, "anime"));
        assert!(!matches_target_kind(&anime, "series"));

        let series = test_discovery_item("series", "series", Some("series"));
        assert!(matches_target_kind(&series, "series"));
        assert!(!matches_target_kind(&series, "anime"));

        let movie = test_discovery_item("movie", "movie", Some("movie"));
        assert!(matches_target_kind(&movie, "movie"));
        assert!(!matches_target_kind(&movie, "series"));

        let fallback = test_discovery_item("fallback", "anime", Some(""));
        assert!(matches_target_kind(&fallback, "anime"));
        assert!(!matches_target_kind(&fallback, "series"));

        let unknown = test_discovery_item("unknown", "series", Some("tv"));
        assert!(!matches_target_kind(&unknown, "series"));
        assert!(!matches_target_kind(&unknown, "anime"));
    }

    #[test]
    fn personalized_sections_dedupe_derived_items_and_require_subject_match() {
        fn discovery_item(
            id: &str,
            title: &str,
            genre_labels: &[&str],
            rank_score: f64,
            matched_subject_count: i32,
        ) -> DiscoveryItemRecord {
            let mut item = test_discovery_item(id, "movie", Some("movie"));
            item.target_key = format!("tmdb:movie:{id}");
            item.display_title = title.to_string();
            item.sort_title = Some(title.to_string());
            item.facet_terms = genre_labels
                .iter()
                .map(|genre| format!("canonical:genre:{}", genre.to_ascii_lowercase()))
                .collect();
            item.rank_score = Some(rank_score);
            item.matched_subject_count = matched_subject_count;
            item
        }

        let profile = DiscoveryLibraryAffinityProfile {
            genre_labels: vec!["Adventure".to_string(), "Animation".to_string()],
            tag_labels: Vec::new(),
        };
        let items = vec![
            discovery_item("1", "Shared Match", &["Adventure", "Animation"], 100.0, 1),
            discovery_item("2", "Unlinked Animation", &["Animation"], 95.0, 0),
            discovery_item("3", "Adventure Match", &["Adventure"], 90.0, 1),
            discovery_item("4", "Animation Match", &["Animation"], 80.0, 1),
        ];

        let sections = personalized_section_results(&items, &profile, true, 10);
        let adventure = sections
            .iter()
            .find(|section| section.title == "Because You Like Adventure")
            .expect("adventure section");
        let animation = sections
            .iter()
            .find(|section| section.title == "Because You Like Animation")
            .expect("animation section");

        assert_eq!(
            adventure
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Shared Match", "Adventure Match"]
        );
        assert_eq!(
            animation
                .items
                .iter()
                .map(|item| item.display_title.as_str())
                .collect::<Vec<_>>(),
            vec!["Animation Match"]
        );

        let mut seen = HashSet::new();
        for item in sections.iter().flat_map(|section| section.items.iter()) {
            assert!(
                seen.insert(discovery_item_identity_key(item).to_string()),
                "duplicate discovery item {} in derived sections",
                item.display_title
            );
        }
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
            canonical_tags: vec![],
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
            catalog_sort_key: String::new(),
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            popularity: None,
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
            sort_index: 0,
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
            canonical_tags: vec![],
            rating: None,
            rating_sources: Vec::new(),
            external_ratings: Vec::new(),
            external_ids: Vec::new(),
            status_tags: Vec::new(),
            source_tags: Vec::new(),
            sources: Vec::new(),
            best_source: None,
            relation_types: Vec::new(),
            relation_subtypes: Vec::new(),
            chart_signals: Vec::new(),
            provider_signals: Vec::new(),
            rank_components: Vec::new(),
            source_count: None,
            edge_count: None,
            relation_count: None,
            source_subject_count: None,
            rank_score: None,
            matched_subject_keys: Vec::new(),
            matched_subject_titles: Vec::new(),
            matched_subject_count: 0,
            library_provenance: Vec::new(),
            tmdb_collection_id: None,
            tmdb_collection_name: None,
            owned_in_input: false,
            studio_slug: None,
            person_ids: Vec::new(),
            facet_terms: Vec::new(),
            context_terms: Vec::new(),
            change_subject_keys: Vec::new(),
            removed_subject_keys: Vec::new(),
            tombstoned_by_run_id: None,
            tombstoned_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_discovery_section(
        surface: &str,
        items: Vec<DiscoveryItemRecord>,
    ) -> DiscoverySectionResult {
        DiscoverySectionResult {
            section_id: format!("{surface}_section"),
            section_type: "TEST".to_string(),
            title: "Test".to_string(),
            surface: surface.to_string(),
            total_count: items.len() as i64,
            items,
        }
    }
}
