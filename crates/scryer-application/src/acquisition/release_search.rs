use super::acquisition::{
    collection_download_submission_scope_for_wanted_item,
    direct_download_submission_scope_for_wanted_item,
};
use super::*;
use crate::acquisition_policy::evaluate_upgrade;
use crate::acquisition_search_queries::{
    anidb_id_from_external_ids, build_search_queries, imdb_id_from_title, tvdb_id_from_external_ids,
};
use crate::delay_profile::DelayProfile;
use crate::quality::release_parser::ParseDisposition;
use chrono::{DateTime, Utc};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(crate) struct CanonicalTitleEvidence {
    pub(crate) lookup_keys: Vec<String>,
    pub(crate) year: Option<i32>,
    pub(crate) parse_context: crate::ReleaseParseContext,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedReleaseSearchSubject {
    pub(crate) title_id: String,
    pub(crate) title_tags: Vec<String>,
    pub(crate) title_evidence: CanonicalTitleEvidence,
    pub(crate) queries: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) tvdb_id: Option<String>,
    pub(crate) anidb_id: Option<String>,
    pub(crate) category: String,
    pub(crate) facet: String,
    pub(crate) runtime_minutes: Option<i32>,
    pub(crate) season: Option<u32>,
    pub(crate) episode: Option<u32>,
    pub(crate) absolute_episode: Option<u32>,
    pub(crate) subject_kind: ReleaseSearchSubjectKind,
    pub(crate) current_score: Option<i32>,
    pub(crate) last_search_at: Option<String>,
    pub(crate) grabbed_release: Option<String>,
    pub(crate) submission_scope: SubmissionScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReleaseAutoDecisionCode {
    Eligible,
    ParseAmbiguous,
    ParseUnparseable,
    TitleMismatch,
    QualityBlocked,
    NegativeScore,
    UpgradeRejected,
    CutoffReached,
    AlreadyActive,
    DbBlocklisted,
    PendingDelay,
    DownloadClientUnavailable,
    RepackGroupMismatch,
}

impl ReleaseAutoDecisionCode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "eligible" => Some(Self::Eligible),
            "parse_ambiguous" => Some(Self::ParseAmbiguous),
            "parse_unparseable" => Some(Self::ParseUnparseable),
            "title_mismatch" => Some(Self::TitleMismatch),
            "quality_blocked" => Some(Self::QualityBlocked),
            "negative_score" => Some(Self::NegativeScore),
            "upgrade_rejected" => Some(Self::UpgradeRejected),
            "cutoff_reached" => Some(Self::CutoffReached),
            "already_active" => Some(Self::AlreadyActive),
            "db_blocklisted" => Some(Self::DbBlocklisted),
            "pending_delay" => Some(Self::PendingDelay),
            "download_client_unavailable" => Some(Self::DownloadClientUnavailable),
            "repack_group_mismatch" => Some(Self::RepackGroupMismatch),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::ParseAmbiguous => "parse_ambiguous",
            Self::ParseUnparseable => "parse_unparseable",
            Self::TitleMismatch => "title_mismatch",
            Self::QualityBlocked => "quality_blocked",
            Self::NegativeScore => "negative_score",
            Self::UpgradeRejected => "upgrade_rejected",
            Self::CutoffReached => "cutoff_reached",
            Self::AlreadyActive => "already_active",
            Self::DbBlocklisted => "db_blocklisted",
            Self::PendingDelay => "pending_delay",
            Self::DownloadClientUnavailable => "download_client_unavailable",
            Self::RepackGroupMismatch => "repack_group_mismatch",
        }
    }

    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Eligible => "auto search would grab this release",
            Self::ParseAmbiguous => "release parse is ambiguous and blocks auto-grab",
            Self::ParseUnparseable => "release could not be parsed and blocks auto-grab",
            Self::TitleMismatch => "release title does not match the target title",
            Self::QualityBlocked => "quality profile blocked this release",
            Self::NegativeScore => "release score is negative after scoring penalties",
            Self::UpgradeRejected => "upgrade policy rejected this release",
            Self::CutoffReached => "existing file already meets the configured cutoff",
            Self::AlreadyActive => "release is already active or covered in the queue",
            Self::DbBlocklisted => "release is blocklisted from prior failures",
            Self::PendingDelay => "release is eligible but held by a delay profile",
            Self::DownloadClientUnavailable => "matching download clients are unavailable",
            Self::RepackGroupMismatch => "repack group does not match the existing file",
        }
    }

    pub(crate) fn is_eligible(self) -> bool {
        matches!(self, Self::Eligible)
    }
}

#[derive(Clone)]
pub(crate) struct AutoCandidateEvaluationContext<'a> {
    pub(crate) title: &'a Title,
    pub(crate) subject: &'a ResolvedReleaseSearchSubject,
    pub(crate) current_score: Option<i32>,
    pub(crate) last_search_at: Option<&'a str>,
    pub(crate) profile: &'a QualityProfile,
    pub(crate) thresholds: &'a AcquisitionThresholds,
    pub(crate) cutoff_reached: bool,
    pub(crate) now: &'a DateTime<Utc>,
    pub(crate) dl_snapshot: Option<&'a crate::acquisition_workflow::DownloadClientSnapshot>,
    pub(crate) db_blocklist: &'a HashSet<String>,
    pub(crate) existing_files: &'a [TitleMediaFile],
    pub(crate) delay_profiles: &'a [DelayProfile],
    pub(crate) failed_source_kinds: Option<&'a [DownloadSourceKind]>,
}

pub fn release_strategy_kind_for_label(label: &str, is_rss_request: bool) -> ReleaseStrategyKind {
    if is_rss_request {
        return ReleaseStrategyKind::RssFeed;
    }

    if label.starts_with("ids") {
        return ReleaseStrategyKind::IdBacked;
    }

    match label {
        "freetext" | "freetext_alias" => ReleaseStrategyKind::Freetext,
        _ => ReleaseStrategyKind::Fallback,
    }
}

pub(crate) fn canonical_title_lookup_keys(title: &Title) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();

    for candidate in std::iter::once(title.name.as_str())
        .chain(title.aliases.iter().map(String::as_str))
        .chain(title.tagged_aliases.iter().map(|alias| alias.name.as_str()))
    {
        let normalized = crate::title_matching::canonical_lookup_key(candidate);
        if !normalized.is_empty() && seen.insert(normalized.clone()) {
            keys.push(normalized);
        }
    }

    keys
}

pub(crate) fn canonical_title_evidence(title: &Title) -> CanonicalTitleEvidence {
    canonical_title_evidence_for_episode(title, None)
}

fn canonical_title_evidence_for_episode(
    title: &Title,
    episode: Option<&Episode>,
) -> CanonicalTitleEvidence {
    CanonicalTitleEvidence {
        lookup_keys: canonical_title_lookup_keys(title),
        year: title.year,
        parse_context: crate::build_release_parse_context(
            title,
            episode,
            None,
            Some(title.facet.as_str()),
        ),
    }
}

pub(crate) fn interstitial_movie_search_title(title: &Title, collection: &Collection) -> Title {
    let Some(movie) = collection.interstitial_movie.as_ref() else {
        return title.clone();
    };

    let mut search_title = title.clone();
    search_title.name = movie.name.clone();
    search_title.year = movie.year;
    search_title.imdb_id = (!movie.imdb_id.trim().is_empty()).then(|| movie.imdb_id.clone());
    search_title.runtime_minutes = Some(movie.runtime_minutes);
    search_title
        .external_ids
        .retain(|external_id| !matches!(external_id.source.as_str(), "tvdb" | "tmdb" | "anidb"));
    if !movie.tvdb_id.trim().is_empty() {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tvdb".to_string(),
            value: movie.tvdb_id.clone(),
        });
    }
    if let Some(tmdb_id) = movie.movie_tmdb_id.as_ref()
        && !tmdb_id.trim().is_empty()
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "tmdb".to_string(),
            value: tmdb_id.clone(),
        });
    }
    if let Some(anidb_id) = movie.movie_anidb_id.as_ref()
        && !anidb_id.trim().is_empty()
    {
        search_title.external_ids.push(scryer_domain::ExternalId {
            source: "anidb".to_string(),
            value: anidb_id.clone(),
        });
    }
    search_title.aliases.clear();
    search_title.tagged_aliases.clear();
    search_title
}

fn extract_titles_from_release(parsed: &ParsedReleaseMetadata) -> Vec<String> {
    let mut titles = if parsed.normalized_title_variants.is_empty() {
        vec![parsed.normalized_title.clone()]
    } else {
        parsed.normalized_title_variants.clone()
    };

    if titles.is_empty() {
        titles.push(parsed.normalized_title.clone());
    }

    titles
        .into_iter()
        .map(|title| crate::title_matching::canonical_lookup_key(&title))
        .filter(|title| !title.is_empty())
        .fold(Vec::<String>::new(), |mut acc, value| {
            if !acc.iter().any(|existing| existing == &value) {
                acc.push(value);
            }
            acc
        })
}

pub(crate) fn parsed_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    for release_title in extract_titles_from_release(parsed) {
        if evidence.lookup_keys.iter().any(|key| key == &release_title) {
            return true;
        }

        if let Some(year) = parsed.year {
            let year_suffix = format!(" {year}");
            if let Some(without_year) = release_title.strip_suffix(&year_suffix)
                && evidence.lookup_keys.iter().any(|key| key == without_year)
            {
                return true;
            }
        }

        if let Some(year) = evidence.year {
            let with_year = format!("{release_title} {year}");
            if evidence.lookup_keys.iter().any(|key| key == &with_year) {
                return true;
            }
        }
    }

    contextual_release_matches_title_evidence(parsed, evidence)
}

fn contextual_release_matches_title_evidence(
    parsed: &ParsedReleaseMetadata,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    let contextual = crate::analyze_release_for_target(&parsed.raw_title, &evidence.parse_context);
    if contextual.is_unparseable() || contextual.is_ambiguous {
        return false;
    }
    let Some(best_candidate) = contextual.best_candidate() else {
        return false;
    };

    let mut titles = best_candidate.projected.normalized_title_variants.clone();
    if !titles
        .iter()
        .any(|title| title == &best_candidate.projected.normalized_title)
    {
        titles.push(best_candidate.projected.normalized_title.clone());
    }

    titles.into_iter().any(|title| {
        let normalized = crate::title_matching::canonical_lookup_key(&title);
        !normalized.is_empty() && evidence.lookup_keys.iter().any(|key| key == &normalized)
    })
}

pub(crate) fn candidate_matches_title_subject(
    candidate: &IndexerSearchResult,
    evidence: &CanonicalTitleEvidence,
) -> bool {
    if candidate
        .provenance
        .as_ref()
        .is_some_and(|provenance| provenance.title_validated_upstream)
    {
        return true;
    }

    let parsed_owned;
    let parsed = if let Some(parsed) = candidate.parsed_release_metadata.as_ref() {
        parsed
    } else {
        parsed_owned =
            crate::parse_release_metadata_for_target(&candidate.title, &evidence.parse_context);
        &parsed_owned
    };

    parsed_release_matches_title_evidence(parsed, evidence)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateParseState {
    Parsed,
    Ambiguous,
    Unparseable,
}

fn candidate_parse_state(candidate: &IndexerSearchResult) -> CandidateParseState {
    let Some(parsed) = candidate.parsed_release_metadata.as_ref() else {
        return CandidateParseState::Unparseable;
    };

    if matches!(parsed.disposition, ParseDisposition::Unparseable)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:unparseable" || hint == "parse_status:unparseable")
    {
        return CandidateParseState::Unparseable;
    }
    if parsed.is_ambiguous
        || matches!(parsed.disposition, ParseDisposition::Ambiguous)
        || parsed
            .parse_hints
            .iter()
            .any(|hint| hint == "v2:ambiguous" || hint == "parse_status:ambiguous")
    {
        return CandidateParseState::Ambiguous;
    }
    CandidateParseState::Parsed
}

pub(crate) fn annotate_auto_decision(
    candidate: &mut IndexerSearchResult,
    code: ReleaseAutoDecisionCode,
) {
    candidate.auto_eligible = Some(code.is_eligible());
    candidate.auto_decision_code = Some(code.as_str().to_string());
    candidate.auto_decision_summary = Some(code.summary().to_string());
}

pub(crate) fn serialize_decision_explanation(candidate: &IndexerSearchResult) -> Option<String> {
    candidate.quality_profile_decision.as_ref().map(|decision| {
        serde_json::to_string(
            &decision
                .scoring_log
                .iter()
                .map(|entry| serde_json::json!({"code": entry.code, "delta": entry.delta}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default()
    })
}

pub(crate) fn evaluate_auto_candidate(
    candidate: &IndexerSearchResult,
    context: &AutoCandidateEvaluationContext<'_>,
) -> ReleaseAutoDecisionCode {
    match candidate_parse_state(candidate) {
        CandidateParseState::Ambiguous => return ReleaseAutoDecisionCode::ParseAmbiguous,
        CandidateParseState::Unparseable => return ReleaseAutoDecisionCode::ParseUnparseable,
        CandidateParseState::Parsed => {}
    }

    let is_allowed = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.allowed)
        .unwrap_or(false);
    if !is_allowed {
        return ReleaseAutoDecisionCode::QualityBlocked;
    }

    if !candidate_matches_title_subject(candidate, &context.subject.title_evidence) {
        return ReleaseAutoDecisionCode::TitleMismatch;
    }

    if context.cutoff_reached {
        return ReleaseAutoDecisionCode::CutoffReached;
    }

    let candidate_score = candidate
        .quality_profile_decision
        .as_ref()
        .map(|decision| decision.preference_score)
        .unwrap_or(0);
    if candidate_score < 0 {
        return ReleaseAutoDecisionCode::NegativeScore;
    }

    if let Some(dl_snapshot) = context.dl_snapshot
        && dl_snapshot.is_active(&candidate.title)
    {
        return ReleaseAutoDecisionCode::AlreadyActive;
    }

    if context
        .db_blocklist
        .contains(&candidate.title.to_ascii_lowercase())
    {
        return ReleaseAutoDecisionCode::DbBlocklisted;
    }

    if let Some(failed_source_kinds) = context.failed_source_kinds
        && let Some(source_kind) = candidate.source_kind
        && failed_source_kinds.contains(&source_kind)
    {
        return ReleaseAutoDecisionCode::DownloadClientUnavailable;
    }

    let decision = evaluate_upgrade(
        candidate_score,
        context.current_score,
        context.profile.criteria.allow_upgrades,
        context.last_search_at,
        context.now,
        context.thresholds,
        context.profile.criteria.min_score_to_grab,
    );
    if !decision.is_accept() {
        return ReleaseAutoDecisionCode::UpgradeRejected;
    }

    if crate::acquisition_policy::should_skip_repack_group_mismatch(
        candidate,
        context.existing_files,
        context.subject.submission_scope.episode_id(),
    ) {
        return ReleaseAutoDecisionCode::RepackGroupMismatch;
    }

    if let Some(delay_decision) = crate::delay_profile::resolve_delay_decision(
        context.delay_profiles,
        &context.title.tags,
        &context.title.facet,
        candidate.source_kind,
        candidate
            .published_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at),
        candidate_score,
        context.now,
    ) && delay_decision.should_hold()
    {
        return ReleaseAutoDecisionCode::PendingDelay;
    }

    ReleaseAutoDecisionCode::Eligible
}

fn preferred_scoped_external_id(ids: &[ScopedExternalId], source: &str) -> Option<String> {
    ids.iter()
        .find(|id| {
            id.source.eq_ignore_ascii_case(source)
                && id
                    .source_scope
                    .as_deref()
                    .is_some_and(|scope| scope.eq_ignore_ascii_case("R"))
                && !id.external_id.trim().is_empty()
        })
        .or_else(|| {
            ids.iter().find(|id| {
                id.source.eq_ignore_ascii_case(source) && !id.external_id.trim().is_empty()
            })
        })
        .map(|id| id.external_id.trim().to_string())
}

impl AppUseCase {
    fn release_search_category_for_facet(&self, facet: &MediaFacet) -> String {
        self.facet_registry
            .get(facet)
            .map(|handler| handler.search_category().to_string())
            .unwrap_or_else(|| match facet {
                MediaFacet::Movie => "movie".to_string(),
                MediaFacet::Series => "series".to_string(),
                MediaFacet::Anime => "anime".to_string(),
            })
    }

    pub(crate) async fn local_scoped_anidb_id_for_episode(
        &self,
        episode: Option<&Episode>,
    ) -> Option<String> {
        let episode = episode?;
        // AnimeTosho's `aid` is an AniDB anime/collection identity, not an
        // episode identity. Prefer season/collection-scoped mappings, then let
        // callers fall back to the title-level AniDB ID.
        let collection_id = episode.collection_id.as_deref()?;
        self.local_scoped_anidb_id_for_collection(collection_id)
            .await
    }

    async fn local_scoped_anidb_id_for_collection(&self, collection_id: &str) -> Option<String> {
        let collection_ids = self
            .services
            .catalog
            .shows
            .list_collection_external_ids(collection_id)
            .await
            .unwrap_or_default();
        preferred_scoped_external_id(&collection_ids, "anidb")
    }

    pub(crate) async fn evaluate_search_results_for_subject(
        &self,
        title: &Title,
        subject: &ResolvedReleaseSearchSubject,
        mut results: Vec<IndexerSearchResult>,
    ) -> Vec<IndexerSearchResult> {
        let db_blocklist: HashSet<String> = self
            .services
            .workflow
            .release_attempts
            .list_failed_release_signatures_for_title(&title.id, 200)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| entry.source_title)
            .map(|value| value.to_ascii_lowercase())
            .collect();

        let dl_snapshot = crate::acquisition_workflow::DownloadClientSnapshot::fetch(self).await;
        let existing_files = self
            .services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .unwrap_or_default();
        let delay_profiles = self.load_delay_profiles().await;
        let now = Utc::now();
        let upgrade_context = self
            .resolve_upgrade_context_for_title_with_category(
                title,
                subject.grabbed_release.as_deref(),
                Some(subject.category.as_str()),
            )
            .await;

        let evaluation_context = AutoCandidateEvaluationContext {
            title,
            subject,
            current_score: subject.current_score,
            last_search_at: subject.last_search_at.as_deref(),
            profile: &upgrade_context.profile,
            thresholds: &upgrade_context.thresholds,
            cutoff_reached: upgrade_context.cutoff_reached,
            now: &now,
            dl_snapshot: Some(&dl_snapshot),
            db_blocklist: &db_blocklist,
            existing_files: &existing_files,
            delay_profiles: &delay_profiles,
            failed_source_kinds: None,
        };

        for candidate in &mut results {
            let code = evaluate_auto_candidate(candidate, &evaluation_context);
            annotate_auto_decision(candidate, code);
        }

        results
    }

    pub(crate) async fn resolve_release_search_subject_for_title(
        &self,
        title: &Title,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let query = title.name.trim().to_string();
        if query.is_empty() && imdb_id.is_none() && tvdb_id.is_none() && anidb_id.is_none() {
            return Err(AppError::Validation(
                "title has no name or external IDs".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(&title.id, None)
            .await
            .ok()
            .flatten();

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title),
            queries: vec![query],
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            facet: title.facet.as_str().to_string(),
            runtime_minutes: title.runtime_minutes,
            season: None,
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Title,
            current_score: wanted.as_ref().and_then(|item| item.current_score),
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            grabbed_release: wanted.and_then(|item| item.grabbed_release),
            submission_scope: SubmissionScope::Title,
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_episode(
        &self,
        title: &Title,
        season: &str,
        episode: &str,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let season = season.trim();
        let episode = episode.trim();
        if season.is_empty() || episode.is_empty() {
            return Err(AppError::Validation(
                "season and episode are required".into(),
            ));
        }

        let season_digits: String = season
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        let episode_digits: String = episode
            .chars()
            .filter(|value| value.is_ascii_digit())
            .collect();
        if season_digits.is_empty() || episode_digits.is_empty() {
            return Err(AppError::Validation(
                "season and episode must include numeric values".into(),
            ));
        }

        let season_num = season_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid season value".into()))?;
        let episode_num = episode_digits
            .parse::<u32>()
            .map_err(|_| AppError::Validation("invalid episode value".into()))?;

        let episode_record = self
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &season_digits, &episode_digits)
            .await?;

        let wanted = self
            .services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(
                &title.id,
                episode_record.as_ref().map(|episode| episode.id.as_str()),
            )
            .await
            .ok()
            .flatten();

        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let title_anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = self
            .local_scoped_anidb_id_for_episode(episode_record.as_ref())
            .await
            .or(title_anidb_id);

        let absolute_episode = episode_record
            .as_ref()
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.trim().parse::<u32>().ok());

        let category = self.release_search_category_for_facet(&title.facet);

        let mut queries = vec![format!(
            "{} S{:0>2}E{:0>2}",
            title.name.trim(),
            season_num,
            episode_num
        )];
        queries.push(format!("{} S{:0>2}", title.name.trim(), season_num));
        if title.facet == MediaFacet::Anime {
            if let Some(absolute) = absolute_episode {
                queries.insert(0, format!("{} {:0>3}", title.name.trim(), absolute));
            }
            queries.push(title.name.trim().to_string());
        }
        let mut seen = HashSet::new();
        queries.retain(|query| !query.trim().is_empty() && seen.insert(query.to_ascii_lowercase()));

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(title, episode_record.as_ref()),
            queries,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            facet: title.facet.as_str().to_string(),
            runtime_minutes: episode_record
                .as_ref()
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(title.runtime_minutes),
            season: Some(season_num),
            episode: Some(episode_num),
            absolute_episode,
            subject_kind: ReleaseSearchSubjectKind::Episode,
            current_score: wanted.as_ref().and_then(|item| item.current_score),
            last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
            grabbed_release: wanted
                .as_ref()
                .and_then(|item| item.grabbed_release.clone()),
            submission_scope: episode_record
                .as_ref()
                .map(|episode| SubmissionScope::Episode {
                    episode_id: episode.id.clone(),
                })
                .unwrap_or(SubmissionScope::Title),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_season_pack(
        &self,
        title: &Title,
        item: &WantedItem,
        episode: Option<&Episode>,
        season_num: u32,
        runtime_minutes: Option<i32>,
    ) -> AppResult<ResolvedReleaseSearchSubject> {
        let imdb_id = imdb_id_from_title(title);
        let tvdb_id = tvdb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let collection_anidb_id = match episode.and_then(|episode| episode.collection_id.as_deref())
        {
            Some(collection_id) => {
                self.local_scoped_anidb_id_for_collection(collection_id)
                    .await
            }
            None => None,
        };
        let anidb_id = collection_anidb_id.or(anidb_id);
        let category = self.release_search_category_for_facet(&title.facet);
        let mut queries = vec![format!("{} S{:0>2}", title.name.trim(), season_num)];
        queries.retain(|query| !query.trim().is_empty());
        if queries.is_empty() && (imdb_id.is_some() || tvdb_id.is_some() || anidb_id.is_some()) {
            queries.push(String::new());
        }
        if queries.is_empty() {
            return Err(AppError::Validation(
                "season pack search subject has no searchable title or external IDs".into(),
            ));
        }

        Ok(ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence(title),
            queries,
            imdb_id,
            tvdb_id,
            anidb_id,
            category,
            facet: title.facet.as_str().to_string(),
            runtime_minutes,
            season: Some(season_num),
            episode: None,
            absolute_episode: None,
            subject_kind: ReleaseSearchSubjectKind::Season,
            current_score: item.current_score,
            last_search_at: item.last_search_at.clone(),
            grabbed_release: item.grabbed_release.clone(),
            submission_scope: collection_download_submission_scope_for_wanted_item(item, episode),
        })
    }

    pub(crate) async fn resolve_release_search_subject_for_collection(
        &self,
        title: &Title,
        collection: &Collection,
    ) -> AppResult<(Title, ResolvedReleaseSearchSubject)> {
        let search_title = interstitial_movie_search_title(title, collection);
        if search_title.name.trim().is_empty() {
            return Err(AppError::Validation(
                "collection search subject has no searchable title".into(),
            ));
        }

        let wanted = self
            .services
            .workflow
            .wanted_items
            .list_wanted_items(
                None,
                Some("interstitial_movie"),
                Some(&title.id),
                None,
                None,
                500,
                0,
            )
            .await?
            .into_iter()
            .find(|item| item.collection_id.as_deref() == Some(collection.id.as_str()));

        let imdb_id = search_title
            .imdb_id
            .as_deref()
            .and_then(crate::normalize::normalize_imdb_id);
        let tvdb_id = tvdb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let anidb_id = anidb_id_from_external_ids(&search_title.external_ids)
            .as_deref()
            .and_then(crate::normalize::normalize_numeric_id);
        let mut queries = vec![if let Some(year) = search_title.year {
            format!("{} {}", search_title.name.trim(), year)
        } else {
            search_title.name.trim().to_string()
        }];
        let mut seen = HashSet::new();
        queries.retain(|query| !query.trim().is_empty() && seen.insert(query.to_ascii_lowercase()));
        if queries.is_empty() && imdb_id.is_some() {
            queries.push(String::new());
        }

        Ok((
            search_title.clone(),
            ResolvedReleaseSearchSubject {
                title_id: title.id.clone(),
                title_tags: title.tags.clone(),
                title_evidence: canonical_title_evidence(&search_title),
                queries,
                imdb_id,
                tvdb_id,
                anidb_id,
                category: self.release_search_category_for_facet(&search_title.facet),
                facet: title.facet.as_str().to_string(),
                runtime_minutes: search_title.runtime_minutes,
                season: None,
                episode: None,
                absolute_episode: None,
                subject_kind: ReleaseSearchSubjectKind::Title,
                current_score: wanted.as_ref().and_then(|item| item.current_score),
                last_search_at: wanted.as_ref().and_then(|item| item.last_search_at.clone()),
                grabbed_release: wanted.and_then(|item| item.grabbed_release),
                submission_scope: SubmissionScope::Collection {
                    collection_id: collection.id.clone(),
                },
            },
        ))
    }

    pub(crate) async fn resolve_release_search_subject_for_wanted_item(
        &self,
        title: &Title,
        item: &WantedItem,
        episode: Option<&Episode>,
    ) -> ResolvedReleaseSearchSubject {
        let query_result = build_search_queries(title, item, episode, &self.facet_registry);
        let absolute_episode = episode
            .and_then(|episode| episode.absolute_number.as_deref())
            .and_then(|value| value.parse::<u32>().ok());

        ResolvedReleaseSearchSubject {
            title_id: title.id.clone(),
            title_tags: title.tags.clone(),
            title_evidence: canonical_title_evidence_for_episode(title, episode),
            queries: query_result.queries,
            imdb_id: query_result.imdb_id,
            tvdb_id: query_result.tvdb_id,
            anidb_id: query_result.anidb_id,
            category: query_result.category,
            facet: title.facet.as_str().to_string(),
            runtime_minutes: episode
                .and_then(|episode| episode.duration_seconds)
                .map(|seconds| (seconds / 60) as i32)
                .or(title.runtime_minutes),
            season: query_result.season,
            episode: query_result.episode,
            absolute_episode,
            subject_kind: match item.media_type.as_str() {
                "episode" => ReleaseSearchSubjectKind::Episode,
                _ => ReleaseSearchSubjectKind::Title,
            },
            current_score: item.current_score,
            last_search_at: item.last_search_at.clone(),
            grabbed_release: item.grabbed_release.clone(),
            submission_scope: direct_download_submission_scope_for_wanted_item(item, episode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scryer_domain::{MediaFacet, TaggedAlias, Title};

    fn make_title() -> Title {
        Title {
            id: "title-1".to_string(),
            name: "Bastard!!".to_string(),
            facet: MediaFacet::Anime,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            created_by: None,
            created_at: Utc::now(),
            year: Some(2022),
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![TaggedAlias {
                name: "Bastard Heavy Metal Dark Fantasy".to_string(),
                language: "eng".to_string(),
            }],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        }
    }

    fn make_candidate(
        release_title: &str,
        provenance: Option<ReleaseCandidateProvenance>,
    ) -> IndexerSearchResult {
        IndexerSearchResult {
            source: "nzbgeek".to_string(),
            title: release_title.to_string(),
            link: None,
            download_url: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            size_bytes: None,
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            indexer_languages: None,
            indexer_subtitles: None,
            indexer_grabs: None,
            password_hint: None,
            parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
            quality_profile_decision: None,
            extra: Default::default(),
            guid: None,
            info_url: None,
            provenance,
            candidate_token: None,
            queue_scope: None,
            auto_eligible: None,
            auto_decision_code: None,
            auto_decision_summary: None,
        }
    }

    #[test]
    fn canonical_title_lookup_keys_include_tagged_aliases() {
        let title = make_title();
        let keys = canonical_title_lookup_keys(&title);

        assert!(keys.iter().any(|key| key == "bastard"));
        assert!(
            keys.iter()
                .any(|key| key == "bastard heavy metal dark fantasy")
        );
    }

    #[test]
    fn candidate_matches_title_subject_trusts_upstream_validation() {
        let title = make_title();
        let candidate = make_candidate(
            "Completely.Different.Show.S01E01.1080p.WEB-DL",
            Some(ReleaseCandidateProvenance {
                search_subject_kind: ReleaseSearchSubjectKind::Episode,
                strategy_kind: ReleaseStrategyKind::IdBacked,
                title_validated_upstream: true,
            }),
        );

        assert!(candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn candidate_matches_title_subject_uses_contextual_alias_parse_when_needed() {
        let mut title = make_title();
        title.name = "Frieren Beyond Journey's End".to_string();
        title.year = Some(2023);
        title.aliases = vec!["Sousou no Frieren".to_string()];
        title.tagged_aliases = vec![TaggedAlias {
            name: "Frieren Beyond Journeys End".to_string(),
            language: "eng".to_string(),
        }];

        let candidate = make_candidate(
            "[SubsPlease] Sousou.no.Frieren.Frieren.Beyond.Journeys.End.-.01.[1080p].[HEVC]",
            None,
        );

        assert!(candidate_matches_title_subject(
            &candidate,
            &canonical_title_evidence(&title)
        ));
    }

    #[test]
    fn candidate_parse_state_marks_ambiguous_parse() {
        let mut candidate = make_candidate("Bastard.S01E01.1080p.WEB-DL", None);
        let parsed = candidate
            .parsed_release_metadata
            .as_mut()
            .expect("candidate has parsed metadata");
        parsed.is_ambiguous = true;
        parsed.disposition = ParseDisposition::Ambiguous;
        parsed.parse_hints.push("v2:ambiguous".to_string());

        assert_eq!(
            candidate_parse_state(&candidate),
            CandidateParseState::Ambiguous
        );
    }
}
