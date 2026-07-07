//! Derived wanted/upgrades views (RFC 119 §6/§7). The Missing and Upgrades tabs
//! read the SAME derived target set the convergence cursor rotates over
//! (`derive_missing_targets` / `compute_cutoff_unmet_items`), joined to the
//! activity-driven state row when one exists and enriched with the per-scope
//! convergence progress the UI shows instead of a search cadence. Convergence is
//! derived for a whole page in ONE coverage round-trip (#12): resolve each scope's
//! `(fingerprint, routed indexers)` once per title, fetch all coverage rows for the
//! page's scope keys together, then compute covered/routed counts in memory and
//! fold in the scheduler availability snapshot for the `Deferred` state.

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use scryer_domain::MediaFacet;

use super::*;
use crate::acquisition::convergence::convergence_scope_key;
use crate::acquisition::targets::AcquisitionTarget;
use crate::contracts::SubmissionScope;

/// Convergence state of a scope for the UI (RFC 119 §6). Mirrors the GraphQL
/// `ConvergenceStateValue`; the interface maps this 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WantedConvergenceState {
    /// No routed indexer searched yet under the current fingerprint.
    Queued,
    /// Some but not all routed indexers covered — sweep in progress.
    Searching,
    /// Every routed indexer covered — watching RSS.
    Converged,
    /// Not converged, and every still-uncovered indexer is currently unavailable.
    Deferred,
}

/// Per-scope convergence progress carried on a wanted view (RFC 119 §6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WantedViewConvergence {
    pub state: WantedConvergenceState,
    pub indexers_covered: i32,
    pub indexers_routed: i32,
}

/// One derived wanted/upgrades row (RFC 119 §6/§7): the target coordinates, the
/// joined activity-state row (when one exists), title/library enrichment, the
/// recency lane, and the batched convergence progress. `id`-identity is decided by
/// the interface mapper (state-row id, else scope key).
#[derive(Clone, Debug)]
pub struct WantedScopeView {
    pub scope_key: String,
    pub title_id: String,
    pub library_id: String,
    pub facet: MediaFacet,
    /// "movie" | "episode" | "series_movie".
    pub media_type: String,
    pub episode_id: Option<String>,
    pub collection_id: Option<String>,
    pub series_movie_link_id: Option<String>,
    pub season_number: Option<String>,
    pub episode_number: Option<String>,
    pub title_name: Option<String>,
    pub title_slug: Option<String>,
    pub library_name: Option<String>,
    pub library_slug: Option<String>,
    /// Recency lane (`true` = hot). Upgrades are always cold.
    pub is_hot: bool,
    /// The activity-driven acquisition-state row, when one exists for this scope.
    pub state: Option<WantedItem>,
    pub convergence: WantedViewConvergence,
}

/// Resolved `(fingerprint, facet, routed indexers)` for one title — identical
/// across all of a title's scopes (same profile, routing and match identity), so
/// it is resolved once per title and reused for every scope of that title (#12).
#[derive(Clone)]
struct TitleConvergenceContext {
    fingerprint: String,
    routed_indexer_ids: Vec<String>,
}

impl AppUseCase {
    /// One page of the derived Missing/Upgrades view (RFC 119 §6/§7). Mirrors the
    /// cutoff-unmet authorization: results are limited to the actor's authorized
    /// libraries. `MISSING` derives from the same fileless-scope query the cursor
    /// uses; `CUTOFF_UPGRADE` reuses the cutoff-unmet compute. Both join the state
    /// row (excluding paused/grabbed-active scopes), enrich title/library names,
    /// sort deterministically, then slice — the convergence progress for the sliced
    /// page is derived in one batched coverage round-trip.
    pub async fn list_wanted_scope_views(
        &self,
        actor: &User,
        kind: WantedKind,
        facet: Option<MediaFacet>,
        library_ids: Vec<String>,
        title_search: Option<String>,
        limit: i64,
        offset: i64,
    ) -> AppResult<(Vec<WantedScopeView>, i64)> {
        let authorized = self
            .list_libraries_for_permission(actor, facet.clone(), scryer_domain::LibraryPermission::View)
            .await?;
        let mut authorized_ids: HashSet<String> =
            authorized.iter().map(|library| library.id.clone()).collect();
        let requested: HashSet<String> = library_ids
            .iter()
            .map(|id| id.trim())
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .collect();
        if !requested.is_empty() {
            authorized_ids.retain(|id| requested.contains(id));
        }
        if authorized_ids.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let library_name_by_id: HashMap<String, String> = authorized
            .iter()
            .map(|library| (library.id.clone(), library.name.clone()))
            .collect();
        let library_slug_by_id: HashMap<String, String> = authorized
            .iter()
            .map(|library| (library.id.clone(), library.slug.clone()))
            .collect();

        // Every non-`wanted` state row (paused or an in-flight grab) whose scope is
        // still derivable — excluded from the active view (§D-B: the view lists
        // actionable targets, not scopes already paused or grabbed).
        let excluded_scope_keys = self.non_wanted_state_scope_keys().await?;

        let facet_str = facet.as_ref().map(|facet| facet.as_str().to_string());
        let title_needle = title_search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);

        // Raw target coordinates for the requested kind, already availability-gated.
        let mut rows: Vec<WantedScopeView> = match kind {
            WantedKind::Missing => {
                let now = Utc::now();
                self.derive_missing_targets(&now)
                    .await?
                    .into_iter()
                    .filter(|target| authorized_ids.contains(&target.library_id))
                    .filter(|target| {
                        facet_str
                            .as_deref()
                            .is_none_or(|facet| target.facet.as_str() == facet)
                    })
                    .filter(|target| !excluded_scope_keys.contains(&target.scope_key))
                    .map(missing_target_to_view)
                    .collect()
            }
            WantedKind::CutoffUpgrade => {
                let library_filter = authorized_ids.iter().cloned().collect::<Vec<_>>();
                self.compute_cutoff_unmet_items(facet.clone(), Some(library_filter))
                    .await?
                    .into_iter()
                    .filter_map(cutoff_item_to_view)
                    .filter(|view| !excluded_scope_keys.contains(&view.scope_key))
                    .collect()
            }
        };

        // Enrich title name/slug (page-independent identity is needed for the title
        // search filter and the deterministic sort, so it is loaded before slicing;
        // one lookup per unique title).
        self.enrich_view_titles(&mut rows).await;
        for view in &mut rows {
            view.library_name = library_name_by_id.get(&view.library_id).cloned();
            view.library_slug = library_slug_by_id.get(&view.library_id).cloned();
        }

        if let Some(needle) = title_needle.as_deref() {
            rows.retain(|view| {
                view.title_name
                    .as_deref()
                    .is_some_and(|name| name.to_ascii_lowercase().contains(needle))
            });
        }

        sort_wanted_views(&mut rows);

        let total = rows.len() as i64;
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let mut page: Vec<WantedScopeView> =
            rows.into_iter().skip(offset).take(limit).collect();

        // Join the state row + derive convergence for the sliced page only.
        self.attach_state_rows(&mut page).await?;
        self.attach_page_convergence(&mut page).await?;

        Ok((page, total))
    }

    /// Scope keys whose state row is paused or grabbed — excluded from the active
    /// derived view. One list query, keyed by the same scope identity the cursor
    /// uses.
    async fn non_wanted_state_scope_keys(&self) -> AppResult<HashSet<String>> {
        let mut excluded = HashSet::new();
        for status in [WantedStatus::Paused, WantedStatus::Grabbed] {
            let items = self
                .services
                .workflow
                .wanted_items
                .list_wanted_items(WantedItemsQuery {
                    statuses: vec![status.as_str().to_string()],
                    limit: i64::MAX,
                    ..WantedItemsQuery::default()
                })
                .await?;
            for item in items {
                let scope = SubmissionScope::from_persisted(
                    &item.title_id,
                    item.episode_id.clone(),
                    item.collection_id.clone(),
                    item.series_movie_link_id.clone(),
                    None,
                );
                if let Some(scope_key) = convergence_scope_key(&scope, &item.title_id) {
                    excluded.insert(scope_key);
                }
            }
        }
        Ok(excluded)
    }

    /// Fill in `title_name`/`title_slug` for the derived rows — one `get_by_id` per
    /// unique title, cached across the row's scopes.
    async fn enrich_view_titles(&self, rows: &mut [WantedScopeView]) {
        let unique_title_ids: HashSet<String> =
            rows.iter().map(|view| view.title_id.clone()).collect();
        let mut names: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for title_id in unique_title_ids {
            if let Ok(Some(title)) = self.services.catalog.titles.get_by_id(&title_id).await {
                names.insert(title_id, (Some(title.name), title.slug));
            }
        }
        for view in rows.iter_mut() {
            if view.title_name.is_some() {
                continue;
            }
            if let Some((name, slug)) = names.get(&view.title_id) {
                view.title_name = name.clone();
                view.title_slug = slug.clone();
            }
        }
    }

    /// Attach the activity-driven state row for each page scope (page-sized, so a
    /// per-scope lookup is bounded).
    async fn attach_state_rows(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        for view in page.iter_mut() {
            view.state = self
                .find_wanted_state_for_scope(
                    &view.title_id,
                    view.episode_id.as_deref(),
                    view.collection_id.as_deref(),
                    view.series_movie_link_id.as_deref(),
                )
                .await?;
        }
        Ok(())
    }

    /// Derive per-scope convergence progress for a page in ONE coverage round-trip
    /// (RFC 119 §6 #12). Resolves `(fingerprint, routed indexers)` once per title,
    /// fetches all coverage rows for the page's scope keys together, computes
    /// covered/routed counts in memory, and folds in the scheduler availability
    /// snapshot to distinguish `Deferred` from `Queued`.
    async fn attach_page_convergence(&self, page: &mut [WantedScopeView]) -> AppResult<()> {
        if page.is_empty() {
            return Ok(());
        }

        // One (fingerprint, routed) resolution per unique title — identical across a
        // title's scopes.
        let mut title_context: HashMap<String, Option<TitleConvergenceContext>> = HashMap::new();
        for view in page.iter() {
            if title_context.contains_key(&view.title_id) {
                continue;
            }
            let context = self.resolve_title_convergence_context(&view.title_id).await;
            title_context.insert(view.title_id.clone(), context);
        }

        // One coverage fetch for the whole page.
        let scope_keys: Vec<String> = page.iter().map(|view| view.scope_key.clone()).collect();
        let coverage_rows = self
            .services
            .integrations
            .scope_indexer_coverage
            .list_coverage_for_scope_keys(&scope_keys)
            .await
            .unwrap_or_default();
        let mut coverage_by_scope: HashMap<String, Vec<crate::ScopeCoverageRow>> = HashMap::new();
        for row in coverage_rows {
            coverage_by_scope
                .entry(row.scope_key.clone())
                .or_default()
                .push(row);
        }

        let availability = self.scheduler_availability().await;
        let host_keys = self.indexer_scheduler_host_keys().await;

        for view in page.iter_mut() {
            let Some(Some(context)) = title_context.get(&view.title_id) else {
                // No routing/profile resolvable — nothing to converge; present as
                // converged (0/0) so the UI does not show a perpetual sweep.
                view.convergence = WantedViewConvergence {
                    state: WantedConvergenceState::Converged,
                    indexers_covered: 0,
                    indexers_routed: 0,
                };
                continue;
            };
            let routed = &context.routed_indexer_ids;
            let covered: HashSet<&str> = coverage_by_scope
                .get(&view.scope_key)
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.fingerprint == context.fingerprint)
                        .map(|row| row.indexer_id.as_str())
                        .collect()
                })
                .unwrap_or_default();
            let covered_count = routed
                .iter()
                .filter(|id| covered.contains(id.as_str()))
                .count();
            let routed_count = routed.len();
            let uncovered: Vec<&String> = routed
                .iter()
                .filter(|id| !covered.contains(id.as_str()))
                .collect();

            let state = if uncovered.is_empty() {
                WantedConvergenceState::Converged
            } else if uncovered.iter().all(|id| {
                !availability.indexer_available(host_keys.get(id.as_str()).map(String::as_str), id)
            }) {
                WantedConvergenceState::Deferred
            } else if covered_count == 0 {
                WantedConvergenceState::Queued
            } else {
                WantedConvergenceState::Searching
            };

            view.convergence = WantedViewConvergence {
                state,
                indexers_covered: covered_count as i32,
                indexers_routed: routed_count as i32,
            };
        }

        Ok(())
    }

    /// Resolve `(fingerprint, routed indexers)` for a title via its title-level
    /// search subject — the values every scope of the title shares. `None` when the
    /// title is gone or nothing is routed.
    async fn resolve_title_convergence_context(
        &self,
        title_id: &str,
    ) -> Option<TitleConvergenceContext> {
        let title = self.services.catalog.titles.get_by_id(title_id).await.ok()??;
        let subject = self.resolve_release_search_subject_for_title(&title).await.ok()?;
        let convergence = self.resolve_scope_convergence(&title, &subject).await?;
        Some(TitleConvergenceContext {
            fingerprint: convergence.fingerprint,
            routed_indexer_ids: convergence.routed_indexer_ids,
        })
    }
}

/// A missing target's coordinates as a view row (state/enrichment/convergence
/// filled in later).
fn missing_target_to_view(target: AcquisitionTarget) -> WantedScopeView {
    WantedScopeView {
        scope_key: target.scope_key,
        title_id: target.title_id,
        library_id: target.library_id,
        facet: target.facet,
        media_type: target.media_type,
        episode_id: target.episode_id,
        collection_id: target.collection_id,
        series_movie_link_id: target.series_movie_link_id,
        season_number: target.season_number,
        episode_number: target.episode_number,
        title_name: None,
        title_slug: None,
        library_name: None,
        library_slug: None,
        is_hot: target.is_hot,
        state: None,
        convergence: pending_convergence(),
    }
}

/// A cutoff-unmet item as a view row. Upgrades are always cold. `None` when the
/// item's scope has no derivable convergence key.
fn cutoff_item_to_view(item: CutoffUnmetItem) -> Option<WantedScopeView> {
    let scope = SubmissionScope::from_persisted(
        &item.title_id,
        item.episode_id.clone(),
        None,
        None,
        None,
    );
    let scope_key = convergence_scope_key(&scope, &item.title_id)?;
    let media_type = if item.episode_id.is_some() {
        "episode"
    } else {
        "movie"
    };
    Some(WantedScopeView {
        scope_key,
        title_id: item.title_id,
        library_id: item.library_id,
        facet: item.title_facet,
        media_type: media_type.to_string(),
        episode_id: item.episode_id,
        collection_id: None,
        series_movie_link_id: None,
        season_number: item.season_number,
        episode_number: item.episode_number,
        title_name: Some(item.title_name),
        title_slug: item.title_slug,
        library_name: item.library_name,
        library_slug: item.library_slug,
        is_hot: false,
        state: None,
        convergence: pending_convergence(),
    })
}

/// Placeholder convergence used before the batched per-page derivation fills it in.
fn pending_convergence() -> WantedViewConvergence {
    WantedViewConvergence {
        state: WantedConvergenceState::Queued,
        indexers_covered: 0,
        indexers_routed: 0,
    }
}

/// Digits of `value`, or `i64::MAX` when absent — matches the cutoff-unmet sort so
/// Missing and Upgrades order identically.
fn parse_sort_number(value: Option<&str>) -> i64 {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| {
            let digits: String = value.chars().filter(char::is_ascii_digit).collect();
            (!digits.is_empty()).then(|| digits.parse::<i64>().ok()).flatten()
        })
        .unwrap_or(i64::MAX)
}

/// Deterministic order: title name, then numeric season, then numeric episode —
/// the same ordering the cutoff-unmet view uses.
fn sort_wanted_views(rows: &mut [WantedScopeView]) {
    rows.sort_by(|left, right| {
        left.title_name
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(&right.title_name.as_deref().unwrap_or_default().to_ascii_lowercase())
            .then_with(|| {
                parse_sort_number(left.season_number.as_deref())
                    .cmp(&parse_sort_number(right.season_number.as_deref()))
            })
            .then_with(|| {
                parse_sort_number(left.episode_number.as_deref())
                    .cmp(&parse_sort_number(right.episode_number.as_deref()))
            })
            .then_with(|| left.scope_key.cmp(&right.scope_key))
    });
}
