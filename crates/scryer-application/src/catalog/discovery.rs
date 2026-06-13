use super::*;
use crate::acquisition_release_search::ResolvedReleaseSearchSubject;
use crate::quality_profile::ScoringSource;
use crate::quality_profile::evaluate_against_profile_for_category;
use crate::settings::keys::default_indexer_routing_categories_for_scope;
use scryer_domain::{MediaFacet, TaggedAlias};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

fn release_search_tagged_aliases(title: &Title) -> Vec<TaggedAlias> {
    let mut aliases = title.tagged_aliases.clone();
    let mut seen: HashSet<String> = aliases
        .iter()
        .map(|alias| alias.name.trim().to_ascii_lowercase())
        .filter(|alias| !alias.is_empty())
        .collect();
    for alias in &title.aliases {
        let key = alias.trim().to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        aliases.push(TaggedAlias {
            name: alias.clone(),
            language: "und".to_string(),
        });
    }
    aliases
}

fn merge_newznab_category_codes(
    base: impl IntoIterator<Item = String>,
    extras: &[String],
) -> Vec<String> {
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for category in base.into_iter().chain(extras.iter().cloned()) {
        let category = category.trim().to_string();
        if !category.is_empty() && seen.insert(category.clone()) {
            merged.push(category);
        }
    }
    merged
}

fn merge_series_movie_categories_into_routing(
    plan: &mut IndexerRoutingPlan,
    owner_facet: &MediaFacet,
    extra_categories: &[String],
) {
    if extra_categories.is_empty() {
        return;
    }
    for entry in plan.entries.values_mut().filter(|entry| entry.enabled) {
        let base_categories = if entry.categories.is_empty() {
            default_indexer_routing_categories_for_scope(owner_facet.as_str())
        } else {
            std::mem::take(&mut entry.categories)
        };
        entry.categories = merge_newznab_category_codes(base_categories, extra_categories);
    }
}

fn source_kind_matches_preference(result: &IndexerSearchResult, preferred: &str) -> bool {
    match result.source_kind {
        Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => preferred == "nzb",
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
            preferred == "torrent"
        }
        None => false,
    }
}

#[cfg(test)]
pub(crate) fn extract_http_status_from_message(message: &str) -> Option<u16> {
    let marker = "status ";
    let lowered = message.to_ascii_lowercase();
    let marker_position = lowered.find(marker)?;
    let mut digits = String::new();

    for character in lowered[marker_position + marker.len()..].chars() {
        if character.is_ascii_digit() {
            digits.push(character);
        } else if !digits.is_empty() {
            break;
        }
    }

    digits.parse::<u16>().ok()
}

#[cfg(test)]
pub(crate) fn is_4xx_or_5xx_status(status: u16) -> bool {
    (400..=599).contains(&status)
}

fn resolve_requested_episode(
    episodes: &[Episode],
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Option<&Episode> {
    if let (Some(season), Some(episode_number)) = (season, episode)
        && let Some(found) = episodes.iter().find(|candidate| {
            candidate
                .season_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(season)
                && candidate
                    .episode_number
                    .as_deref()
                    .and_then(|value| value.parse::<u32>().ok())
                    == Some(episode_number)
        })
    {
        return Some(found);
    }

    absolute_episode.and_then(|wanted_absolute| {
        episodes.iter().find(|candidate| {
            candidate
                .absolute_number
                .as_deref()
                .and_then(|value| value.parse::<u32>().ok())
                == Some(wanted_absolute)
        })
    })
}

#[cfg(test)]
fn extract_indexer_http_status(error: &AppError) -> Option<u16> {
    match error {
        AppError::Repository(message) => extract_http_status_from_message(message),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn is_indexer_http_error(error: &AppError) -> bool {
    extract_indexer_http_status(error).is_some_and(is_4xx_or_5xx_status)
}

pub(crate) fn release_search_key(result: &IndexerSearchResult) -> String {
    if let Some(download_url) = result
        .download_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return download_url.to_string();
    }

    if let Some(link) = result
        .link
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return link.to_string();
    }

    result.title.clone()
}

fn looks_like_structured_query_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
    if trimmed.is_empty() {
        return false;
    }

    let upper = trimmed.to_ascii_uppercase();
    if upper == "OVA" || upper == "SPECIAL" {
        return true;
    }

    if let Some(rest) = upper.strip_prefix('S') {
        if rest.chars().all(|ch| ch.is_ascii_digit()) {
            return true;
        }
        if let Some((season_part, episode_part)) = rest.split_once('E') {
            return !season_part.is_empty()
                && !episode_part.is_empty()
                && season_part.chars().all(|ch| ch.is_ascii_digit())
                && episode_part.chars().all(|ch| ch.is_ascii_digit());
        }
    }

    false
}

fn normalize_structured_dispatch_query(query: &str, absolute_episode: Option<u32>) -> String {
    let mut tokens: Vec<&str> = query.split_whitespace().collect();
    while let Some(last) = tokens.last().copied() {
        let trimmed = last.trim_matches(|ch: char| !ch.is_ascii_alphanumeric());
        if trimmed.is_empty() {
            tokens.pop();
            continue;
        }

        let removable_numeric = absolute_episode.is_some_and(|value| {
            trimmed.chars().all(|ch| ch.is_ascii_digit())
                && trimmed.parse::<u32>().ok() == Some(value)
        });
        let removable_structured = removable_numeric || looks_like_structured_query_token(trimmed);
        if removable_structured {
            tokens.pop();
            continue;
        }
        break;
    }

    tokens.join(" ").trim().to_string()
}

fn dedupe_structured_dispatch_queries(
    queries: Vec<String>,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
) -> Vec<String> {
    if season.is_none() && episode.is_none() && absolute_episode.is_none() {
        return queries;
    }

    let mut deduped = Vec::with_capacity(queries.len());
    let mut seen = std::collections::HashSet::new();

    for query in queries {
        let normalized = normalize_structured_dispatch_query(&query, absolute_episode);
        let key_source = if normalized.is_empty() {
            query.trim()
        } else {
            normalized.as_str()
        };
        if seen.insert(key_source.to_ascii_lowercase()) {
            deduped.push(query);
        }
    }

    deduped
}

fn should_collapse_structured_nab_queries(
    configs: &[IndexerConfig],
    routing: Option<&IndexerRoutingPlan>,
    mode: SearchMode,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if configs.is_empty() {
        return false;
    }

    let mut saw_nab_transport = false;

    for config in configs {
        if !config.is_enabled {
            continue;
        }
        if config.disabled_until.is_some_and(|until| until > now) {
            continue;
        }

        let mode_ok = match mode {
            SearchMode::Interactive => config.enable_interactive_search,
            SearchMode::Auto => auto_mode_enabled_for_structured_collapse(config),
        };
        if !mode_ok {
            continue;
        }

        let routing_entry = routing.and_then(|plan| plan.entries.get(&config.id));
        if routing_entry.is_some_and(|entry| !entry.enabled) {
            continue;
        }

        match config.nab_transport_kind() {
            Some(_) => saw_nab_transport = true,
            None => return false,
        }
    }

    saw_nab_transport
}

#[derive(Debug, Default, Deserialize)]
struct ManagedIndexerAutoModeMetadata {
    enable_automatic_search: Option<bool>,
}

fn auto_mode_enabled_for_structured_collapse(config: &IndexerConfig) -> bool {
    if !config.enable_auto_search {
        return false;
    }

    let Some(raw) = config.managed_metadata_json.as_deref() else {
        return true;
    };
    let Ok(metadata) = serde_json::from_str::<ManagedIndexerAutoModeMetadata>(raw) else {
        return true;
    };

    metadata.enable_automatic_search.unwrap_or(true)
}

pub(crate) fn dedupe_cross_indexer_release_results(
    results: Vec<IndexerSearchResult>,
    indexer_priority_by_name: &HashMap<String, i64>,
    preferred_source_kind: &str,
) -> Vec<IndexerSearchResult> {
    let mut best_by_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut remove_indices: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for (idx, result) in results.iter().enumerate() {
        let key = result
            .parsed_release_metadata
            .as_ref()
            .map(crate::release_dedup::build_release_dedup_key)
            .unwrap_or_default();
        if key.is_empty() {
            continue;
        }

        if let Some(&existing_idx) = best_by_key.get(&key) {
            let existing = &results[existing_idx];

            let existing_prio = indexer_priority_by_name
                .get(&existing.source)
                .copied()
                .unwrap_or(i64::MAX);
            let new_prio = indexer_priority_by_name
                .get(&result.source)
                .copied()
                .unwrap_or(i64::MAX);

            let new_wins = if new_prio != existing_prio {
                new_prio < existing_prio
            } else {
                let existing_preferred =
                    source_kind_matches_preference(existing, preferred_source_kind);
                let new_preferred = source_kind_matches_preference(result, preferred_source_kind);
                new_preferred && !existing_preferred
            };

            if new_wins {
                remove_indices.insert(existing_idx);
                best_by_key.insert(key, idx);
            } else {
                remove_indices.insert(idx);
            }
        } else {
            best_by_key.insert(key, idx);
        }
    }

    if remove_indices.is_empty() {
        return results;
    }

    let before = results.len();
    let mut idx = 0usize;
    let mut deduped = results;
    deduped.retain(|_| {
        let keep = !remove_indices.contains(&idx);
        idx += 1;
        keep
    });
    info!(before, after = deduped.len(), "cross-indexer release dedup");
    deduped
}

impl AppUseCase {
    pub(crate) async fn download_source_capabilities(&self) -> (bool, bool, String) {
        let clients = self
            .services
            .integrations
            .download_client_configs
            .list(None)
            .await
            .unwrap_or_default();
        let enabled: Vec<_> = clients.iter().filter(|c| c.is_enabled).collect();
        let plugin_provider = self
            .services
            .integrations
            .download_client_plugin_provider
            .available();
        let client_accepts = |c: &&scryer_domain::DownloadClientConfig,
                              kind: DownloadSourceKind| {
            let inputs = crate::accepted_inputs_for_client(&c.client_type, plugin_provider);
            inputs.contains(&kind)
        };
        let has_usenet = enabled
            .iter()
            .any(|c| client_accepts(c, DownloadSourceKind::NzbFile));
        let has_torrent = enabled.iter().any(|c| {
            client_accepts(c, DownloadSourceKind::TorrentFile)
                || client_accepts(c, DownloadSourceKind::MagnetUri)
        });
        let preferred = enabled
            .iter()
            .min_by_key(|c| c.client_priority)
            .map(|c| {
                if client_accepts(c, DownloadSourceKind::NzbFile) {
                    "nzb"
                } else {
                    "torrent"
                }
            })
            .unwrap_or("nzb")
            .to_string();

        (has_usenet, has_torrent, preferred)
    }

    pub(crate) async fn build_indexer_priority_by_name(
        &self,
        indexer_routing: Option<&IndexerRoutingPlan>,
    ) -> HashMap<String, i64> {
        let Some(plan) = indexer_routing else {
            return HashMap::new();
        };

        let configs = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_default();
        let id_to_name: std::collections::HashMap<&str, &str> = configs
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect();
        plan.entries
            .iter()
            .filter_map(|(id, entry)| {
                id_to_name
                    .get(id.as_str())
                    .map(|name| (name.to_string(), entry.priority))
            })
            .collect()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "release scoring needs the full search context to produce deterministic ranking decisions"
    )]
    pub(crate) async fn score_release_results(
        &self,
        mut raw_results: Vec<IndexerSearchResult>,
        quality_profile: &QualityProfile,
        title_id: &str,
        library_id: Option<&str>,
        scope_id: Option<&str>,
        indexer_routing: Option<&IndexerRoutingPlan>,
        category: Option<&str>,
        title_tags: &[String],
        runtime_minutes: Option<i32>,
        parse_context: &ReleaseParseContext,
        season: Option<u32>,
        episode: Option<u32>,
        absolute_episode: Option<u32>,
    ) -> Vec<IndexerSearchResult> {
        let failed_signatures = match self
            .services
            .workflow
            .release_attempts
            .list_failed_release_signatures(5000)
            .await
        {
            Ok(items) => items,
            Err(error) => {
                warn!(error = %error, "failed to load failed release blocklist signatures");
                Vec::new()
            }
        };

        let failed_source_hints: std::collections::HashSet<String> = failed_signatures
            .iter()
            .filter_map(|signature| {
                normalize_release_attempt_hint(signature.source_hint.as_deref())
            })
            .collect();
        let failed_source_titles: std::collections::HashSet<String> = failed_signatures
            .iter()
            .filter_map(|signature| {
                normalize_release_attempt_title(signature.source_title.as_deref())
            })
            .collect();

        let (has_usenet_client, has_torrent_client, preferred_source_kind) =
            self.download_source_capabilities().await;

        raw_results.retain(|result| match result.source_kind {
            Some(DownloadSourceKind::NzbFile | DownloadSourceKind::NzbUrl) => has_usenet_client,
            Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri) => {
                has_torrent_client
            }
            None => true,
        });

        let user_rules_engine = self
            .services
            .customization
            .user_rules
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
        let mut user_evaluator = user_rules_engine.evaluator();
        let resolved_persona = self
            .resolve_scoring_persona(library_id, scope_id)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to resolve scoring persona, using canonical default");
                crate::ScoringPersona::default()
            });
        let required_audio_languages = self
            .resolve_required_audio_languages(Some(title_id), library_id, scope_id)
            .await
            .unwrap_or_else(|error| {
                warn!(
                    error = %error,
                    "failed to resolve required audio languages, using canonical default"
                );
                Vec::new()
            });
        let mut resolved_profile = quality_profile.clone();
        resolved_profile.criteria.required_audio_languages = required_audio_languages;
        resolved_profile.criteria.scoring_persona = resolved_persona.clone();
        resolved_profile.criteria.facet_persona_overrides.clear();
        let library_name = match library_id {
            Some(library_id) => match self.services.catalog.libraries.get_by_id(library_id).await {
                Ok(Some(library)) => Some(library.name),
                Ok(None) => None,
                Err(error) => {
                    warn!(
                        error = %error,
                        library_id = library_id,
                        "failed to resolve library name for custom rule context"
                    );
                    None
                }
            },
            None => None,
        };

        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(title_id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(title_id)
            .await
            .unwrap_or_default();
        let requested_episode =
            resolve_requested_episode(&catalog_episodes, season, episode, absolute_episode);
        let mut scored = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for result in raw_results {
            let key = release_search_key(&result);
            if !seen.insert(key) {
                continue;
            }

            if is_release_blocklisted(&result, &failed_source_hints, &failed_source_titles) {
                continue;
            }

            let parsed_release_metadata =
                parse_release_metadata_for_target(&result.title, parse_context);
            let mut scored_release_metadata = parsed_release_metadata.clone();
            scored_release_metadata.languages_audio = crate::release_audio_language_hints(
                &parsed_release_metadata,
                result.indexer_languages.as_deref(),
            );

            let release_coverage = crate::acquisition_coverage::resolve_release_coverage(
                &scored_release_metadata,
                &catalog_episodes,
                &catalog_collections,
                requested_episode,
            );
            if let Some(wanted_episode) = requested_episode
                && !release_coverage.covers_episode(wanted_episode)
            {
                continue;
            }
            let candidate_runtime_minutes = crate::acquisition_coverage::coverage_runtime_minutes(
                &release_coverage,
                &scored_release_metadata,
                &catalog_episodes,
                runtime_minutes,
            );

            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_season) = season
                && let Some(parsed_season) = ep_meta.season
                && parsed_season != wanted_season
            {
                continue;
            }
            if requested_episode.is_none()
                && let Some(ref ep_meta) = scored_release_metadata.episode
                && let Some(wanted_episode) = episode
            {
                if !ep_meta.episode_numbers.is_empty()
                    && !ep_meta.episode_numbers.contains(&wanted_episode)
                {
                    continue;
                }
                if ep_meta.episode_numbers.is_empty()
                    && ep_meta.absolute_episode_numbers.is_empty()
                    && let (Some(parsed_abs), Some(wanted_abs)) =
                        (ep_meta.absolute_episode, absolute_episode)
                    && parsed_abs != wanted_abs
                {
                    continue;
                }
            }

            let weights = crate::scoring_weights::build_weights_for_category(
                &resolved_persona,
                &resolved_profile.criteria.scoring_overrides,
                category,
            );
            let mut decision = evaluate_against_profile_for_category(
                &resolved_profile,
                &scored_release_metadata,
                false,
                &weights,
                category,
            );
            apply_age_scoring(&mut decision, result.published_at.as_deref());
            crate::quality_profile::apply_size_scoring_for_category(
                &mut decision,
                &scored_release_metadata,
                result.size_bytes,
                category,
                candidate_runtime_minutes,
                &weights,
            );
            let pack_penalty =
                release_coverage.single_episode_preference_penalty(requested_episode);
            if pack_penalty != 0 {
                decision.log_with_source(
                    "coverage:single_episode_pack_fallback",
                    pack_penalty,
                    ScoringSource::Builtin,
                );
            }

            if !user_rules_engine.is_empty() {
                let user_input = crate::app_usecase_discovery::build_user_rule_input(
                    &scored_release_metadata,
                    &resolved_profile,
                    &result,
                    &decision,
                    crate::user_rule_input::SearchRuleInputContext {
                        category,
                        library_name: library_name.as_deref(),
                        title_tags,
                        runtime_minutes: candidate_runtime_minutes,
                    },
                );
                let facet = category.unwrap_or("movie");
                match user_evaluator.evaluate(&user_input, facet) {
                    Ok(eval_result) => {
                        for entry in eval_result.entries {
                            decision.log_with_source(
                                &entry.code,
                                entry.delta,
                                ScoringSource::UserRule {
                                    id: entry.rule_set_id,
                                    name: entry.rule_set_name,
                                },
                            );
                        }
                        for err in eval_result.errors {
                            decision.log_with_source(
                                "user_rule_error",
                                0,
                                ScoringSource::UserRule {
                                    id: err.rule_set_id,
                                    name: err.rule_set_name,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        warn!(error = %error, "user rule evaluation failed for release");
                    }
                }
            }

            scored.push(IndexerSearchResult {
                parsed_release_metadata: Some(scored_release_metadata),
                quality_profile_decision: Some(decision),
                ..result
            });
        }

        let indexer_priority_by_name = self.build_indexer_priority_by_name(indexer_routing).await;
        let mut scored = dedupe_cross_indexer_release_results(
            scored,
            &indexer_priority_by_name,
            preferred_source_kind.as_str(),
        );

        scored.sort_by(|left, right| {
            let left_allowed = left
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.allowed)
                .unwrap_or(true);
            let right_allowed = right
                .quality_profile_decision
                .as_ref()
                .map(|decision| decision.allowed)
                .unwrap_or(true);

            match (left_allowed, right_allowed) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => {
                    let left_score = left
                        .quality_profile_decision
                        .as_ref()
                        .map(|decision| decision.preference_score)
                        .unwrap_or(0);
                    let right_score = right
                        .quality_profile_decision
                        .as_ref()
                        .map(|decision| decision.preference_score)
                        .unwrap_or(0);

                    right_score.cmp(&left_score)
                }
            }
        });

        scored
    }

    /// Internal search+score pipeline shared by both user-facing search and background acquisition.
    pub(crate) async fn search_and_score_releases(
        &self,
        request: ReleaseSearchRequest<'_>,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let ReleaseSearchRequest {
            queries,
            imdb_id,
            tmdb_id,
            tvdb_id,
            anidb_id,
            mal_id,
            category,
            owner_facet,
            search_facet,
            id_search_facet,
            newznab_categories,
            title_id,
            title_tags,
            library_id,
            caller_label,
            mode,
            runtime_minutes,
            parse_context,
            season,
            episode,
            absolute_episode,
            tagged_aliases,
            search_subject_kind,
        } = request;
        let quality_profile_lookup = QualityProfileLookup {
            title_tags,
            library_id,
            imdb_id: imdb_id.as_deref(),
            tvdb_id: tvdb_id.as_deref(),
            category_hint: Some(owner_facet.as_str()),
        };
        let quality_profile = self.resolve_quality_profile(quality_profile_lookup).await?;

        let scope_id = self.quality_profile_scope_id(quality_profile_lookup);
        let mut indexer_routing = self
            .resolve_indexer_routing(library_id, scope_id.as_deref())
            .await;
        let newznab_categories = if newznab_categories.is_empty() {
            None
        } else {
            if let Some(plan) = indexer_routing.as_mut() {
                merge_series_movie_categories_into_routing(plan, &owner_facet, &newznab_categories);
            }
            Some(merge_newznab_category_codes(
                default_indexer_routing_categories_for_scope(owner_facet.as_str()),
                &newznab_categories,
            ))
        };

        // If routing exists and every indexer is disabled, skip the search entirely.
        if let Some(ref plan) = indexer_routing {
            let any_enabled = plan.entries.values().any(|e| e.enabled);
            if !any_enabled {
                info!(
                    caller = caller_label,
                    scope_id = scope_id.as_deref().unwrap_or("none"),
                    "all indexers disabled for scope, skipping search"
                );
                return Ok(Vec::new());
            }
        }

        let configured_indexers = self
            .services
            .integrations
            .indexer_configs
            .list(None)
            .await
            .unwrap_or_else(|error| {
                warn!(error = %error, "failed to load indexer configs for transport-aware query collapse");
                vec![]
            });
        let collapse_structured_queries = search_subject_kind == ReleaseSearchSubjectKind::Episode
            && should_collapse_structured_nab_queries(
                &configured_indexers,
                indexer_routing.as_ref(),
                mode,
                chrono::Utc::now(),
            );

        // Auto mode normally conserves API calls by using the first query, but
        // episode acquisition needs season/title fallbacks so packs and ranges
        // can be considered for a single requested episode. Equivalent
        // structured variants are only collapsed when the eligible search set
        // is *nab-only, because non-*nab indexers may still need the full
        // variant fanout.
        let effective_queries = match mode {
            SearchMode::Auto if search_subject_kind == ReleaseSearchSubjectKind::Episode => queries,
            SearchMode::Auto => queries.into_iter().take(1).collect(),
            SearchMode::Interactive => queries,
        };
        let effective_queries = if collapse_structured_queries {
            dedupe_structured_dispatch_queries(effective_queries, season, episode, absolute_episode)
        } else {
            effective_queries
        };

        let mut set = JoinSet::new();
        let mut ids = HashMap::new();
        if let Some(imdb_id) = imdb_id.clone() {
            ids.insert("imdb_id".to_string(), imdb_id);
        }
        if let Some(tmdb_id) = tmdb_id.clone() {
            ids.insert("tmdb_id".to_string(), tmdb_id);
        }
        if let Some(tvdb_id) = tvdb_id.clone() {
            ids.insert("tvdb_id".to_string(), tvdb_id);
        }
        if let Some(anidb_id) = anidb_id.clone() {
            ids.insert("anidb_id".to_string(), anidb_id);
        }
        if let Some(mal_id) = mal_id.clone() {
            ids.insert("mal_id".to_string(), mal_id);
        }

        for query in effective_queries {
            let indexer_client = self.services.integrations.indexer_client.clone();
            let ids = ids.clone();
            let category = category.clone();
            let facet = Some(search_facet.as_str().to_string());
            let id_search_facet = id_search_facet
                .as_ref()
                .map(|facet| facet.as_str().to_string());
            let indexer_routing = indexer_routing.clone();
            let newznab_categories = newznab_categories.clone();
            let tagged_aliases = tagged_aliases.to_vec();
            let query = query.clone();

            set.spawn(async move {
                indexer_client
                    .search(
                        query,
                        ids,
                        category.clone(),
                        facet,
                        id_search_facet,
                        newznab_categories,
                        indexer_routing,
                        mode,
                        season,
                        episode,
                        absolute_episode,
                        tagged_aliases,
                    )
                    .await
            });
        }

        let mut query_failures = 0usize;
        let mut successful_searches = 0usize;
        let mut first_failure: Option<String> = None;
        let mut raw_results: Vec<IndexerSearchResult> = Vec::new();

        while let Some(result) = set.join_next().await {
            match result {
                Ok(Ok(mut response)) => {
                    successful_searches += 1;
                    for result in &mut response.results {
                        let provenance =
                            result.provenance.get_or_insert(ReleaseCandidateProvenance {
                                search_subject_kind,
                                strategy_kind: ReleaseStrategyKind::Fallback,
                                title_validated_upstream: false,
                            });
                        provenance.search_subject_kind = search_subject_kind;
                    }
                    raw_results.append(&mut response.results);
                }
                Ok(Err(error)) => {
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search query failed"
                    );
                }
                Err(error) => {
                    query_failures += 1;
                    first_failure = first_failure.or_else(|| Some(error.to_string()));
                    warn!(
                        caller = caller_label,
                        error = %error,
                        "indexer search task panicked"
                    );
                }
            }
        }

        if raw_results.is_empty() && successful_searches == 0 && query_failures > 0 {
            let details =
                first_failure.unwrap_or_else(|| "all indexer search queries failed".to_string());
            return Err(AppError::Repository(details));
        }

        Ok(self
            .score_release_results(
                raw_results,
                &quality_profile,
                title_id,
                library_id,
                scope_id.as_deref(),
                indexer_routing.as_ref(),
                category.as_deref(),
                title_tags,
                runtime_minutes,
                parse_context,
                season,
                episode,
                absolute_episode,
            )
            .await)
    }

    pub(crate) async fn search_and_evaluate_subject(
        &self,
        title: &Title,
        subject: &crate::acquisition_release_search::ResolvedReleaseSearchSubject,
        caller_label: &str,
        mode: SearchMode,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let tagged_aliases = release_search_tagged_aliases(title);
        let results = self
            .search_and_score_releases(ReleaseSearchRequest {
                queries: subject.queries.clone(),
                imdb_id: subject.imdb_id.clone(),
                tmdb_id: subject.tmdb_id.clone(),
                tvdb_id: subject.tvdb_id.clone(),
                anidb_id: subject.anidb_id.clone(),
                mal_id: subject.mal_id.clone(),
                category: Some(subject.category.clone()),
                owner_facet: subject.owner_facet.clone(),
                search_facet: subject.search_facet.clone(),
                id_search_facet: subject.id_search_facet.clone(),
                newznab_categories: subject.newznab_categories.clone(),
                title_id: subject.title_id.as_str(),
                title_tags: &subject.title_tags,
                library_id: Some(title.library_id.as_str()),
                caller_label,
                mode,
                runtime_minutes: subject.runtime_minutes,
                season: subject.season,
                episode: subject.episode,
                absolute_episode: subject.absolute_episode,
                tagged_aliases: &tagged_aliases,
                search_subject_kind: subject.subject_kind,
                parse_context: &subject.title_evidence.parse_context,
            })
            .await?;

        Ok(self
            .evaluate_search_results_for_subject(title, subject, results)
            .await)
    }

    /// Interactive search for a title (movie or standalone). Resolves all
    /// external IDs and search category from the title record so the frontend
    /// only needs to pass the title ID.
    async fn attach_candidate_tokens(
        &self,
        actor: &User,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        results: &mut [IndexerSearchResult],
        preserve_subject_scope: bool,
    ) {
        let signing_key = match self.release_candidate_signing_key_for_actor(actor).await {
            Ok(signing_key) => signing_key,
            Err(err) => {
                warn!(
                    actor = actor.id.as_str(),
                    title_id = title.id.as_str(),
                    scope = ?subject.submission_scope,
                    error = %err,
                    "failed to resolve candidate-token signing key for title-aware search"
                );
                for result in results.iter_mut() {
                    result.candidate_token = None;
                }
                return;
            }
        };

        let catalog_episodes = self
            .services
            .catalog
            .shows
            .list_episodes_for_title(&title.id)
            .await
            .unwrap_or_default();
        let catalog_collections = self
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .unwrap_or_default();
        let requested_episode = resolve_requested_episode(
            &catalog_episodes,
            subject.season,
            subject.episode,
            subject.absolute_episode,
        );

        for result in results.iter_mut() {
            let scope = if preserve_subject_scope {
                subject.submission_scope.clone()
            } else {
                result
                    .parsed_release_metadata
                    .as_ref()
                    .map(|parsed| {
                        crate::acquisition_coverage::resolve_release_coverage(
                            parsed,
                            &catalog_episodes,
                            &catalog_collections,
                            requested_episode,
                        )
                        .submission_scope_or(&subject.submission_scope)
                    })
                    .unwrap_or_else(|| subject.submission_scope.clone())
            };
            result.queue_scope = Some(scope.clone());
            let selection = QueuedReleaseSelection {
                source_hint: result.download_url.clone().or(result.link.clone()),
                source_kind: result.source_kind,
                source_title: Some(result.title.clone()),
            };
            result.candidate_token = if selection.source_hint.is_some() {
                match self.issue_release_candidate_token_with_signing_key(
                    actor,
                    &title.id,
                    &scope,
                    &selection,
                    &signing_key,
                ) {
                    Ok(token) => Some(token),
                    Err(err) => {
                        warn!(
                            actor = actor.id.as_str(),
                            title_id = title.id.as_str(),
                            scope = ?scope,
                            release_title = result.title.as_str(),
                            error = %err,
                            "failed to attach candidate token to title-aware search result"
                        );
                        None
                    }
                }
            } else {
                None
            };
        }
    }

    pub async fn search_indexers_for_title(
        &self,
        actor: &User,
        title_id: String,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_title(&title)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for title"
        );

        let mut results = self
            .search_and_evaluate_subject(&title, &subject, &actor.id, SearchMode::Interactive)
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }

    pub async fn search_indexers_for_series_movie(
        &self,
        actor: &User,
        title_id: String,
        series_movie_link_id: String,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let link = self
            .services
            .catalog
            .shows
            .get_series_movie_link_by_id(&series_movie_link_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("series movie {series_movie_link_id}")))?;
        if link.series_title_id != title.id {
            return Err(AppError::Validation(
                "series movie does not belong to title".into(),
            ));
        }

        let (search_title, subject) = self
            .resolve_release_search_subject_for_series_movie(&title, &link)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            series_movie_link_id = series_movie_link_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for series movie"
        );

        let mut results = self
            .search_and_evaluate_subject(
                &search_title,
                &subject,
                &actor.id,
                SearchMode::Interactive,
            )
            .await?;
        self.attach_candidate_tokens(actor, &search_title, &subject, &mut results, true)
            .await;

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }

    /// Interactive search for a specific episode. Resolves all external IDs,
    /// search category, and absolute episode number from the title/episode
    /// records so the frontend only needs to pass title ID + season + episode.
    pub async fn search_indexers_for_episode(
        &self,
        actor: &User,
        title_id: String,
        season: String,
        episode: String,
    ) -> AppResult<Vec<IndexerSearchResult>> {
        let title = self
            .services
            .catalog
            .titles
            .get_by_id(&title_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("title {title_id}")))?;
        self.require_library_permission(
            actor,
            &title.library_id,
            scryer_domain::LibraryPermission::ManageTitles,
        )
        .await?;
        let subject = self
            .resolve_release_search_subject_for_episode(&title, &season, &episode)
            .await?;

        info!(
            actor = actor.id.as_str(),
            title_id = title_id.as_str(),
            query = subject.queries.first().map(String::as_str).unwrap_or(""),
            category = subject.category.as_str(),
            "searching indexers for episode"
        );

        let mut results = self
            .search_and_evaluate_subject(&title, &subject, &actor.id, SearchMode::Interactive)
            .await?;
        self.attach_candidate_tokens(actor, &title, &subject, &mut results, false)
            .await;

        self.emit_discovery_search_completed_event(
            Some(actor.id.clone()),
            subject.category.clone(),
            subject.queries.first().cloned(),
            results.len() as i64,
        )
        .await;

        Ok(results)
    }
}

pub(crate) fn is_release_blocklisted(
    result: &IndexerSearchResult,
    failed_source_hints: &std::collections::HashSet<String>,
    failed_source_titles: &std::collections::HashSet<String>,
) -> bool {
    if let Some(download_url) = normalize_release_attempt_hint(result.download_url.as_deref())
        && failed_source_hints.contains(&download_url)
    {
        return true;
    }

    if let Some(link) = normalize_release_attempt_hint(result.link.as_deref())
        && failed_source_hints.contains(&link)
    {
        return true;
    }

    if let Some(title) = normalize_release_attempt_title(Some(result.title.as_str()))
        && failed_source_titles.contains(&title)
    {
        return true;
    }

    false
}

#[derive(Clone, Copy)]
pub(crate) struct QualityProfileLookup<'a> {
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) imdb_id: Option<&'a str>,
    pub(crate) tvdb_id: Option<&'a str>,
    pub(crate) category_hint: Option<&'a str>,
}

pub(crate) struct ReleaseSearchRequest<'a> {
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tmdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) mal_id: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) owner_facet: MediaFacet,
    pub(crate) search_facet: MediaFacet,
    pub(crate) id_search_facet: Option<MediaFacet>,
    pub(crate) newznab_categories: Vec<String>,
    pub(crate) title_id: &'a str,
    pub(crate) title_tags: &'a [String],
    pub(crate) library_id: Option<&'a str>,
    pub(crate) caller_label: &'a str,
    pub(crate) mode: SearchMode,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) absolute_episode: Option<u32>,
    pub(crate) tagged_aliases: &'a [TaggedAlias],
    pub(crate) search_subject_kind: ReleaseSearchSubjectKind,
    pub(crate) parse_context: &'a ReleaseParseContext,
}

impl AppUseCase {
    pub(crate) async fn resolve_quality_profile(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> AppResult<QualityProfile> {
        let catalog = self.load_quality_profiles().await?;
        let category_scope_id = self.quality_profile_scope_id(lookup);

        let title_profile_id = lookup
            .title_tags
            .iter()
            .find(|t| t.starts_with("scryer:quality-profile:"))
            .map(|t| t.trim_start_matches("scryer:quality-profile:").to_string());

        let category_profile_id = self
            .read_setting_string_value_explicit(
                QUALITY_PROFILE_ID_KEY,
                category_scope_id.as_deref(),
            )
            .await?;
        let library_profile_id = match lookup.library_id {
            Some(library_id) => {
                self.read_setting_string_value_explicit(QUALITY_PROFILE_ID_KEY, Some(library_id))
                    .await?
            }
            None => None,
        };
        let global_profile_id = self
            .read_setting_string_value(QUALITY_PROFILE_ID_KEY, None)
            .await?;

        let active_profile_id = resolve_profile_id_for_title(
            title_profile_id.as_deref(),
            library_profile_id.as_deref(),
            category_profile_id.as_deref(),
            global_profile_id.as_deref(),
        );
        if let Some(profile_id) = active_profile_id.as_deref()
            && let Some(profile) = catalog.iter().find(|profile| profile.id == profile_id)
        {
            return Ok(profile.clone());
        }

        warn!(
            active_profile_id = active_profile_id.as_deref().unwrap_or("none"),
            "quality profile id not found in catalog, using default"
        );

        Ok(default_quality_profile_for_search())
    }

    async fn load_quality_profiles(&self) -> AppResult<Vec<QualityProfile>> {
        match self
            .services
            .config
            .quality_profiles
            .list_quality_profiles(SETTINGS_SCOPE_SYSTEM, None)
            .await
        {
            Ok(catalog) if !catalog.is_empty() => return Ok(catalog),
            Ok(_) => warn!("quality profile DB catalog is empty; using default"),
            Err(err) => {
                warn!(error = %err, "failed to load quality profiles from DB; using default")
            }
        }

        Ok(vec![default_quality_profile_for_search()])
    }

    pub(crate) async fn read_setting_string_value(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_explicit(
        &self,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        self.read_setting_string_value_for_scope_explicit(SETTINGS_SCOPE_SYSTEM, key_name, scope_id)
            .await
    }

    pub(crate) async fn read_setting_string_value_for_scope(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
    }

    pub(crate) async fn read_setting_string_value_for_scope_explicit(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<&str>,
    ) -> AppResult<Option<String>> {
        let scope_id = scope_id.map(std::string::ToString::to_string);
        let Some(raw_value) = self
            .services
            .config
            .settings
            .get_setting_json_explicit(scope, key_name, scope_id)
            .await?
        else {
            return Ok(None);
        };

        let trimmed = raw_value.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        if trimmed == INHERIT_QUALITY_PROFILE_VALUE {
            return Ok(None);
        }

        let Ok(parsed) = serde_json::from_str::<Value>(trimmed) else {
            return Ok(Some(trimmed.to_string()));
        };
        match parsed {
            Value::Null => Ok(None),
            Value::String(value) => {
                let normalized = value.trim();
                if normalized.is_empty() || normalized == INHERIT_QUALITY_PROFILE_VALUE {
                    Ok(None)
                } else {
                    Ok(Some(normalized.to_string()))
                }
            }
            _ => Ok(Some(trimmed.to_string())),
        }
    }

    pub(crate) fn quality_profile_scope_id(
        &self,
        lookup: QualityProfileLookup<'_>,
    ) -> Option<String> {
        if let Some(value) = lookup.category_hint {
            let normalized = value.to_ascii_lowercase();
            match normalized.as_str() {
                "movie" => return Some("movie".to_string()),
                "series" => return Some("series".to_string()),
                "anime" => return Some("anime".to_string()),
                "5070" => return Some("series".to_string()),
                _ => {}
            }
        }

        if lookup.imdb_id.is_some() {
            return Some("movie".to_string());
        }
        if lookup.tvdb_id.is_some() {
            return Some("series".to_string());
        }

        None
    }

    /// Resolve Newznab category codes from the user's indexer routing settings
    /// for the given scope_id (movie/series/anime).
    ///
    /// Returns `None` if no routing is configured (caller falls back to
    /// hardcoded defaults). Returns `Some(vec![])` if all indexers are
    /// disabled for this scope (caller should skip search).
    pub(crate) async fn resolve_indexer_routing(
        &self,
        library_id: Option<&str>,
        scope_id: Option<&str>,
    ) -> Option<IndexerRoutingPlan> {
        if let Some(library_id) = library_id {
            match self
                .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(library_id))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(plan) = self.parse_indexer_routing_plan(library_id, &value) {
                        return Some(plan);
                    }
                }
                Ok(None) => {}
                Err(err) => {
                    warn!(
                        error = %err,
                        library_id = library_id,
                        "failed to read library indexer routing setting, falling back to facet defaults"
                    );
                }
            }
        }

        let scope_id = scope_id?;

        let raw_json = match self
            .read_setting_string_value(INDEXER_ROUTING_SETTINGS_KEY, Some(scope_id))
            .await
        {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(err) => {
                warn!(
                    error = %err,
                    scope_id = scope_id,
                    "failed to read indexer routing setting, falling back to defaults"
                );
                return None;
            }
        };

        self.parse_indexer_routing_plan(scope_id, &raw_json)
    }

    pub(crate) fn parse_indexer_routing_plan(
        &self,
        scope_id: &str,
        raw_json: &str,
    ) -> Option<IndexerRoutingPlan> {
        let parsed: Value = match serde_json::from_str(raw_json) {
            Ok(value) => value,
            Err(_) => return None,
        };

        let obj = parsed.as_object()?;
        if obj.is_empty() {
            return None;
        }

        let mut entries = std::collections::HashMap::new();

        // The canonical write paths in settings.rs and the startup
        // `normalize_routing_settings` migration always emit `enabled` and
        // `priority`. The `unwrap_or` fallbacks here are transitional
        // legacy-compat for installs that haven't yet been normalized.
        for (indexer_id, config) in obj {
            let enabled = match config.get("enabled").and_then(|v| v.as_bool()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `enabled`; using legacy default `true`"
                    );
                    true
                }
            };

            let mut categories: Vec<String> = Vec::new();
            if let Some(cats) = config.get("categories").and_then(|v| v.as_array()) {
                for cat in cats {
                    if let Some(cat_str) = cat.as_str() {
                        let trimmed = cat_str.trim();
                        if !trimmed.is_empty() {
                            categories.push(trimmed.to_string());
                        }
                    }
                }
            }

            let priority = match config.get("priority").and_then(|v| v.as_i64()) {
                Some(value) => value,
                None => {
                    debug!(
                        scope_id,
                        indexer_id,
                        "indexer routing entry missing `priority`; using legacy default `i64::MAX`"
                    );
                    i64::MAX
                }
            };

            entries.insert(
                indexer_id.clone(),
                IndexerRoutingEntry {
                    enabled,
                    categories,
                    priority,
                },
            );
        }

        info!(
            scope_id = scope_id,
            indexer_count = entries.len(),
            "resolved per-indexer routing plan"
        );
        Some(IndexerRoutingPlan { entries })
    }
}

pub(crate) fn build_user_rule_input(
    parsed: &ParsedReleaseMetadata,
    profile: &QualityProfile,
    result: &IndexerSearchResult,
    decision: &QualityProfileDecision,
    context: crate::user_rule_input::SearchRuleInputContext<'_>,
) -> scryer_rules::UserRuleInput {
    crate::user_rule_input::build_search_rule_input(parsed, profile, result, decision, context)
}

#[cfg(test)]
#[path = "app_usecase_discovery_tests.rs"]
mod app_usecase_discovery_tests;
