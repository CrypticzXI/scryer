use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;

use crate::domain_events::{new_title_domain_event, title_context_snapshot};
use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::{
    AppUseCase, ReleaseDownloadAttemptOutcome, WantedSearchTransition,
    normalize_release_attempt_hint, normalize_release_attempt_title,
};
use scryer_domain::{
    DomainEventPayload, ImportRejectedEventData, ImportSkipReason, ImportStatus, MediaFacet, Title,
};
use tracing::warn;

pub(crate) enum ImportedFileGateDecision {
    Accepted(Box<ImportedFileAcceptance>),
    Rejected(ImportedFileRejection),
}

pub(crate) struct ImportedFileAcceptance {
    pub analysis: Option<crate::MediaFileAnalysis>,
    pub scan_error: Option<String>,
}

pub(crate) struct PreparedImportCandidate {
    pub parsed: crate::ParsedReleaseMetadata,
    pub accepted: Box<ImportedFileAcceptance>,
    pub rescore_changes: Vec<String>,
}

#[derive(Debug)]
pub struct ImportedFileRejection {
    pub message: String,
    pub recycle_reason: &'static str,
    pub skip_reason: Option<ImportSkipReason>,
    pub blocking_rule_codes: Vec<String>,
}

pub(crate) fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    facet.as_str()
}

pub(crate) fn build_import_profile_decision(
    profile: &crate::QualityProfile,
    required_audio_languages: &[String],
    persona: &crate::ScoringPersona,
    parsed: &crate::ParsedReleaseMetadata,
    category_hint: &str,
    runtime_minutes: Option<i32>,
    size_bytes: Option<i64>,
    has_existing_file: bool,
) -> crate::QualityProfileDecision {
    let mut resolved_profile = profile.clone();
    resolved_profile.criteria.required_audio_languages = required_audio_languages.to_vec();
    resolved_profile.criteria.scoring_persona = persona.clone();
    resolved_profile.criteria.facet_persona_overrides.clear();
    let weights = crate::scoring_weights::build_weights_for_category(
        persona,
        &resolved_profile.criteria.scoring_overrides,
        Some(category_hint),
    );
    let mut decision = crate::quality_profile::evaluate_against_profile_for_category(
        &resolved_profile,
        parsed,
        has_existing_file,
        &weights,
        Some(category_hint),
    );
    crate::quality_profile::apply_size_scoring_for_category(
        &mut decision,
        parsed,
        size_bytes,
        Some(category_hint),
        runtime_minutes,
        &weights,
    );
    decision
}

pub(crate) fn build_media_file_analysis(
    analysis: &scryer_mediainfo::MediaAnalysis,
) -> crate::MediaFileAnalysis {
    let audio_languages = crate::normalize_detected_audio_languages(
        analysis.audio_languages.iter().map(String::as_str),
    );
    let subtitle_languages = crate::normalize_detected_subtitle_languages(
        analysis.subtitle_languages.iter().map(String::as_str),
    );

    crate::MediaFileAnalysis {
        video_codec: analysis.video_codec.clone(),
        video_width: analysis.video_width,
        video_height: analysis.video_height,
        video_bitrate_kbps: analysis.video_bitrate_kbps,
        video_bit_depth: analysis.video_bit_depth,
        video_hdr_format: analysis.video_hdr_format.clone(),
        video_frame_rate: analysis.video_frame_rate.clone(),
        video_profile: analysis.video_profile.clone(),
        audio_codec: analysis.audio_codec.clone(),
        audio_profile: analysis.audio_profile.clone(),
        audio_channels: analysis.audio_channels,
        audio_bitrate_kbps: analysis.audio_bitrate_kbps,
        audio_languages,
        audio_streams: analysis
            .audio_streams
            .iter()
            .map(|stream| crate::AudioStreamDetail {
                codec: stream.codec.clone(),
                profile: stream.profile.clone(),
                channels: stream.channels,
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_audio_language_code),
                bitrate_kbps: stream.bitrate_kbps,
            })
            .collect(),
        subtitle_languages,
        subtitle_codecs: analysis.subtitle_codecs.clone(),
        subtitle_streams: analysis
            .subtitle_streams
            .iter()
            .map(|stream| crate::SubtitleStreamDetail {
                codec: stream.codec.clone(),
                language: stream
                    .language
                    .as_deref()
                    .and_then(crate::normalize_detected_subtitle_language_code),
                name: stream.name.clone(),
                forced: stream.forced,
                default: stream.default,
            })
            .collect(),
        has_multiaudio: analysis.has_multiaudio,
        duration_seconds: analysis.duration_seconds,
        num_chapters: analysis.num_chapters,
        container_format: analysis.container_format.clone(),
    }
}

/// Probe a file at the given path and validate it against the quality profile and user rules.
/// The file does NOT need to be at its final destination — this can probe a file in-place
/// at its download location before any move/copy.
pub(crate) async fn probe_and_validate(
    app: &AppUseCase,
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    quality_profile: &crate::QualityProfile,
    path: &Path,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    is_filler: bool,
) -> ImportedFileGateDecision {
    let analysis = match scryer_mediainfo::analyze_file(path) {
        Ok(analysis) => analysis,
        Err(error) => {
            warn!(error = %error, path = %path.display(), "media analysis failed");
            return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
                analysis: None,
                scan_error: Some(error.to_string()),
            }));
        }
    };

    if !scryer_mediainfo::is_valid_video(&analysis) {
        return ImportedFileGateDecision::Rejected(ImportedFileRejection {
            message: "imported file is not a valid video".to_string(),
            recycle_reason: "invalid_file",
            skip_reason: None,
            blocking_rule_codes: Vec::new(),
        });
    }

    let category_hint = facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(Some(&title.id), Some(category_hint))
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve required audio languages, using canonical default"
            );
            Vec::new()
        });
    if !required_audio_languages.is_empty() {
        let missing = crate::missing_required_audio_languages(
            &required_audio_languages,
            &analysis.audio_languages,
        );
        if !missing.is_empty() {
            return ImportedFileGateDecision::Rejected(ImportedFileRejection {
                message: format!(
                    "imported file is missing required audio language(s): {}",
                    missing.join(", ")
                ),
                recycle_reason: "language_mismatch",
                skip_reason: None,
                blocking_rule_codes: Vec::new(),
            });
        }
    }
    let persona = app
        .resolve_scoring_persona(Some(category_hint))
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve scoring persona, using canonical default"
            );
            crate::ScoringPersona::default()
        });

    let user_rules_engine = app
        .services
        .customization
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if !user_rules_engine.is_empty() {
        let decision = build_import_profile_decision(
            quality_profile,
            &required_audio_languages,
            &persona,
            parsed,
            category_hint,
            title.runtime_minutes,
            Some(size_bytes),
            has_existing_file,
        );
        let input = crate::user_rule_input::build_rule_input(
            parsed,
            quality_profile,
            &decision,
            crate::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: Some(size_bytes),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                extra: None,
                indexer_languages: None,
            },
            crate::user_rule_input::RuleContextInfo {
                title_id: Some(&title.id),
                category: Some(facet_to_category_hint(&title.facet)),
                title_tags: &title.tags,
                has_existing_file,
                existing_score,
                search_mode: "post_download",
                runtime_minutes: title.runtime_minutes,
                is_filler,
            },
            Some(crate::user_rule_input::build_file_doc(&analysis)),
        );
        let mut evaluator = user_rules_engine.evaluator();
        match evaluator.evaluate(&input, facet_to_category_hint(&title.facet)) {
            Ok(result) => {
                if !result.errors.is_empty() {
                    warn!(
                        title_id = %title.id,
                        error_count = result.errors.len(),
                        "post-download rule evaluation had runtime errors; failing open"
                    );
                }

                let blocking_rule_codes: Vec<String> = result
                    .entries
                    .iter()
                    .filter(|entry| entry.delta <= scryer_rules::BLOCK_SCORE_THRESHOLD)
                    .map(|entry| entry.code.clone())
                    .collect();

                if !blocking_rule_codes.is_empty() {
                    return ImportedFileGateDecision::Rejected(ImportedFileRejection {
                        message: format!(
                            "post-download rule(s) blocked import: {}",
                            blocking_rule_codes.join(", ")
                        ),
                        recycle_reason: "post_download_rule_blocked",
                        skip_reason: Some(ImportSkipReason::PostDownloadRuleBlocked),
                        blocking_rule_codes,
                    });
                }
            }
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    "post-download rule evaluation failed; failing open"
                );
            }
        }
    }

    ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
        analysis: Some(build_media_file_analysis(&analysis)),
        scan_error: None,
    }))
}

/// Probe a source file once, apply the existing gate, and merge detected media
/// facts back into parsed metadata so downstream rename and scoring decisions
/// use the same resolved view that will later be persisted.
pub(crate) async fn prepare_import_candidate(
    app: &AppUseCase,
    title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    quality_profile: &crate::QualityProfile,
    path: &Path,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    is_filler: bool,
) -> Result<PreparedImportCandidate, ImportedFileRejection> {
    match probe_and_validate(
        app,
        title,
        parsed,
        quality_profile,
        path,
        size_bytes,
        has_existing_file,
        existing_score,
        is_filler,
    )
    .await
    {
        ImportedFileGateDecision::Rejected(rejection) => Err(rejection),
        ImportedFileGateDecision::Accepted(accepted) => {
            let (parsed, rescore_changes) = rescore_from_mediainfo(parsed, accepted.as_ref());
            if !rescore_changes.is_empty() {
                tracing::debug!(
                    title = %title.name,
                    path = %path.display(),
                    changes = ?rescore_changes,
                    "mediainfo rescore prepared import candidate"
                );
            }

            Ok(PreparedImportCandidate {
                parsed,
                accepted,
                rescore_changes,
            })
        }
    }
}

/// Merge mediainfo-detected values into a release-name-parsed metadata struct.
/// Prefers mediainfo when it detects a concrete value that differs from the release name.
/// Returns the merged metadata and a log of what changed.
pub(crate) fn rescore_from_mediainfo(
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
) -> (crate::ParsedReleaseMetadata, Vec<String>) {
    let Some(ref analysis) = acceptance.analysis else {
        return (parsed.clone(), vec![]);
    };

    let mut merged = parsed.clone();
    let mut changes = Vec::new();
    let resolved = resolve_release_labels_from_analysis(
        analysis.video_height,
        analysis.video_codec.as_deref(),
        analysis.audio_codec.as_deref(),
        analysis.audio_profile.as_deref(),
        analysis.audio_channels,
        &analysis.audio_streams,
    );

    // Override resolution from video height
    if let Some(ref detected) = resolved.quality
        && merged.quality.as_deref() != Some(detected.as_str())
    {
        changes.push(format!(
            "resolution: {} → {}",
            merged.quality.as_deref().unwrap_or("?"),
            detected
        ));
        merged.quality = Some(detected.clone());
    }

    // Override video codec (map mediainfo names → release parser names)
    if let Some(ref normalized) = resolved.video_codec
        && merged.video_codec.as_deref() != Some(normalized.as_str())
    {
        changes.push(format!(
            "video_codec: {} → {}",
            merged.video_codec.as_deref().unwrap_or("?"),
            normalized
        ));
        merged.video_codec = Some(normalized.clone());
    }

    if analysis.video_bit_depth.unwrap_or_default() >= 10 && !merged.is_10bit {
        changes.push("video_bit_depth: detected 10-bit".to_string());
        merged.is_10bit = true;
    }

    // Override HDR format
    if let Some(ref hdr_format) = analysis.video_hdr_format {
        let hdr_upper = hdr_format.to_ascii_uppercase();
        if hdr_upper.contains("DOLBY VISION") && !merged.is_dolby_vision {
            changes.push("hdr: detected Dolby Vision".to_string());
            merged.is_dolby_vision = true;
        }
        if hdr_upper.contains("HDR10") && !merged.has_hdr_fallback {
            changes.push("hdr: detected HDR fallback".to_string());
            merged.has_hdr_fallback = true;
        }
        if (hdr_upper.contains("HDR10+") || hdr_upper.contains("HDR10PLUS")) && !merged.is_hdr10plus
        {
            changes.push("hdr: detected HDR10+".to_string());
            merged.is_hdr10plus = true;
        }
        if hdr_upper.contains("HDR10") && !merged.detected_hdr {
            changes.push("hdr: detected HDR10".to_string());
            merged.detected_hdr = true;
        }
    }

    // Override audio: iterate all streams to find best codec and max channels.
    if let Some(ref normalized) = resolved.audio_codec
        && merged.audio.as_deref() != Some(normalized.as_str())
    {
        changes.push(format!(
            "audio: {} → {}",
            merged.audio.as_deref().unwrap_or("?"),
            normalized
        ));
        merged.audio = Some(normalized.clone());
    }

    if let Some(ref ch_str) = resolved.audio_channels
        && merged.audio_channels.as_deref() != Some(ch_str.as_str())
    {
        changes.push(format!(
            "audio_channels: {} → {}",
            merged.audio_channels.as_deref().unwrap_or("?"),
            ch_str
        ));
        merged.audio_channels = Some(ch_str.clone());
    }

    if !analysis.audio_streams.is_empty() {
        // Detect multi-audio from stream count
        if analysis.audio_streams.len() > 1 && !merged.is_dual_audio {
            changes.push("dual_audio: detected multiple audio tracks".to_string());
            merged.is_dual_audio = true;
        }

        if resolved.is_atmos && !merged.is_atmos {
            changes.push("atmos: detected from audio streams".to_string());
            merged.is_atmos = true;
        }
    }

    (merged, changes)
}
/// Compute acquisition score from a gate acceptance, applying mediainfo rescoring.
/// Returns the final score and the rescored parsed metadata (for logging).
pub(crate) async fn compute_acquisition_score(
    app: &AppUseCase,
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
    profile: &crate::QualityProfile,
    title: &Title,
    size_bytes: i64,
    has_existing_file: bool,
) -> i32 {
    let (rescored, changes) = rescore_from_mediainfo(parsed, acceptance);
    let category = facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(Some(&title.id), Some(category))
        .await
        .unwrap_or_default();
    let persona = app
        .resolve_scoring_persona(Some(category))
        .await
        .unwrap_or_default();
    let decision = build_import_profile_decision(
        profile,
        &required_audio_languages,
        &persona,
        &rescored,
        category,
        title.runtime_minutes,
        Some(size_bytes),
        has_existing_file,
    );
    let score = decision.preference_score;
    if !changes.is_empty() {
        tracing::debug!(
            title = %title.name,
            score,
            changes = ?changes,
            "mediainfo rescore applied to acquisition score"
        );
    }
    score
}

pub(crate) async fn persist_media_analysis_result(
    media_files: &std::sync::Arc<dyn crate::MediaFileRepository>,
    file_id: &str,
    accepted: &ImportedFileAcceptance,
) {
    if let Some(ref analysis) = accepted.analysis {
        if let Err(error) = media_files
            .update_media_file_analysis(file_id, analysis.clone())
            .await
        {
            warn!(error = %error, file_id = %file_id, "failed to store media analysis");
            let _ = media_files
                .mark_scan_failed(file_id, &error.to_string())
                .await;
        }
        return;
    }

    if let Some(ref error) = accepted.scan_error {
        let _ = media_files.mark_scan_failed(file_id, error).await;
    }
}

pub(crate) async fn reject_source_file_before_import(
    app: &AppUseCase,
    actor_user_id: Option<&str>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    episode_ids: &[String],
    rejection: &ImportedFileRejection,
) {
    finalize_import_rejection(
        app,
        actor_user_id,
        title,
        completed_name,
        path,
        episode_ids,
        rejection,
    )
    .await;
}

async fn finalize_import_rejection(
    app: &AppUseCase,
    actor_user_id: Option<&str>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    episode_ids: &[String],
    rejection: &ImportedFileRejection,
) {
    let _ = app
        .services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            normalize_release_attempt_hint(None),
            normalize_release_attempt_title(Some(completed_name)),
            ReleaseDownloadAttemptOutcome::Failed,
            Some(rejection.message.clone()),
            None,
        )
        .await;

    reset_wanted_items_for_retry(app, &title.id, episode_ids).await;

    let reason = Some(format!(
        "{}{}",
        rejection.message,
        if rejection.blocking_rule_codes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", rejection.blocking_rule_codes.join(", "))
        }
    ));
    let _ = app
        .append_domain_event(new_title_domain_event(
            actor_user_id.map(str::to_owned),
            title,
            DomainEventPayload::ImportRejected(ImportRejectedEventData {
                title: Some(title_context_snapshot(title)),
                status: ImportStatus::Skipped,
                import_id: None,
                source_system: None,
                source_ref: None,
                source_title: Some(completed_name.to_string()),
                source_path: Some(path.display().to_string()),
                dest_path: None,
                quality: None,
                reason,
                skip_reason: Some(ImportSkipReason::PostDownloadRuleBlocked),
                episode_ids: episode_ids.to_vec(),
            }),
        ))
        .await;
}

async fn reset_wanted_items_for_retry(app: &AppUseCase, title_id: &str, episode_ids: &[String]) {
    let now_str = Utc::now().to_rfc3339();
    let targets: Vec<Option<&str>> = if episode_ids.is_empty() {
        vec![None]
    } else {
        let mut seen = HashSet::new();
        episode_ids
            .iter()
            .filter(|episode_id| seen.insert((*episode_id).clone()))
            .map(|episode_id| Some(episode_id.as_str()))
            .collect()
    };

    for episode_id in targets {
        match app
            .services
            .workflow
            .wanted_items
            .get_wanted_item_for_title(title_id, episode_id)
            .await
        {
            Ok(Some(item)) => {
                let next_search_at = now_str.clone();
                let _ = app
                    .services
                    .workflow
                    .wanted_items
                    .schedule_wanted_item_search(&WantedSearchTransition {
                        id: item.id.clone(),
                        next_search_at: Some(next_search_at),
                        last_search_at: None,
                        search_count: item.search_count,
                        current_score: item.current_score,
                        grabbed_release: None,
                    })
                    .await;
            }
            Ok(None) => {}
            Err(error) => {
                warn!(error = %error, title_id = %title_id, "failed to reset wanted item")
            }
        }
    }
}
