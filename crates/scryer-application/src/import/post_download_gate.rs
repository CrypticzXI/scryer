use std::collections::HashSet;
use std::path::Path;

use chrono::Utc;

use crate::domain_events::{DomainEventActor, new_title_domain_event, title_context_snapshot};
use crate::media::release_labels::resolve_release_labels_from_analysis;
use crate::release_parser::AudioCodec;
use crate::{
    AppUseCase, NewBlocklistEntry, ReleaseDownloadAttemptOutcome, WantedSearchTransition,
    normalize_release_attempt_hint, normalize_release_attempt_title,
};
use scryer_domain::{
    DomainEventPayload, ImportRejectedEventData, ImportSkipReason, ImportStatus, MediaFacet, Title,
};
use tracing::warn;

const SOURCE_CHANGED_AFTER_PROBE_CODE: &str = "source_changed_after_probe";

pub(crate) enum ImportedFileGateDecision {
    Accepted(Box<ImportedFileAcceptance>),
    #[cfg_attr(not(feature = "runtime-media-analysis"), allow(dead_code))]
    Rejected(ImportedFileRejection),
}

pub(crate) struct ImportedFileAcceptance {
    pub analysis: Option<crate::MediaFileAnalysis>,
    pub scan_error: Option<String>,
    pub rule_file_doc: Option<scryer_rules::FileDoc>,
    /// Set when the file was accepted but a required audio language could not be
    /// verified (untagged tracks, or the requirement could not be resolved).
    /// Currently emitted as an operator `warn!` log line at import time; the
    /// durable/UI review surface is deferred.
    pub audio_language_warning: Option<String>,
}

pub(crate) struct PreparedImportCandidate {
    pub parsed: crate::ParsedReleaseMetadata,
    pub accepted: Box<ImportedFileAcceptance>,
    pub rescore_changes: Vec<String>,
    pub source_snapshot: scryer_domain::ImportSourceSnapshot,
}

pub(crate) struct PostDownloadAcquisitionDecision {
    pub parsed: crate::ParsedReleaseMetadata,
    pub score: i32,
    pub scoring_log: Option<String>,
}

#[derive(Debug)]
pub struct ImportedFileRejection {
    pub message: String,
    pub recycle_reason: &'static str,
    pub skip_reason: Option<ImportSkipReason>,
    pub blocking_rule_codes: Vec<String>,
}

fn import_source_changed_rejection(
    path: &Path,
    detail: impl std::fmt::Display,
) -> ImportedFileRejection {
    ImportedFileRejection {
        message: format!(
            "import source changed after validation probe: {} ({detail})",
            path.display()
        ),
        recycle_reason: "import_source_changed_after_probe",
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: vec![SOURCE_CHANGED_AFTER_PROBE_CODE.to_string()],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RuntimeSampleValidationMode {
    EnforceAutomatic,
    BypassRuntimeSampleCheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeSampleValidation {
    pub mode: RuntimeSampleValidationMode,
    pub expected_runtime_seconds: Option<i32>,
}

impl RuntimeSampleValidation {
    pub(crate) fn automatic(expected_runtime_seconds: Option<i32>) -> Self {
        Self {
            mode: RuntimeSampleValidationMode::EnforceAutomatic,
            expected_runtime_seconds,
        }
    }

    pub(crate) fn manual_override(expected_runtime_seconds: Option<i32>) -> Self {
        Self {
            mode: RuntimeSampleValidationMode::BypassRuntimeSampleCheck,
            expected_runtime_seconds,
        }
    }
}

#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_ZERO_CODE: &str = "sample_runtime_zero";
#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_TOO_SHORT_CODE: &str = "sample_runtime_too_short";
#[cfg(any(feature = "runtime-media-analysis", test))]
pub(crate) const SAMPLE_RUNTIME_INDETERMINATE_CODE: &str = "sample_runtime_indeterminate";

#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_EXPECTED_RUNTIME_FOR_SAMPLE_RATIO_SECONDS: i32 = 5 * 60;
#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_UNKNOWN_RUNTIME_SAMPLE_SECONDS: i32 = 60;
#[cfg(any(feature = "runtime-media-analysis", test))]
const MIN_RATIO_SAMPLE_SECONDS: i32 = 90;
#[cfg(any(feature = "runtime-media-analysis", test))]
const SAMPLE_RUNTIME_PERCENT: i32 = 10;

pub(crate) fn facet_to_category_hint(facet: &MediaFacet) -> &'static str {
    facet.as_str()
}

#[cfg(any(feature = "runtime-media-analysis", test))]
fn runtime_sample_rejection(
    validation: RuntimeSampleValidation,
    actual_runtime_seconds: Option<i32>,
) -> Option<ImportedFileRejection> {
    if validation.mode == RuntimeSampleValidationMode::BypassRuntimeSampleCheck {
        return None;
    }

    let Some(actual_seconds) = actual_runtime_seconds else {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_INDETERMINATE_CODE,
            "imported file runtime could not be determined for automatic import".to_string(),
        ));
    };

    if actual_seconds <= 0 {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_ZERO_CODE,
            "imported file runtime is zero for automatic import".to_string(),
        ));
    }

    if let Some(expected_seconds) = validation.expected_runtime_seconds
        && expected_seconds >= MIN_EXPECTED_RUNTIME_FOR_SAMPLE_RATIO_SECONDS
    {
        let threshold_seconds = MIN_RATIO_SAMPLE_SECONDS
            .max(expected_seconds.saturating_mul(SAMPLE_RUNTIME_PERCENT) / 100);
        if actual_seconds < threshold_seconds {
            return Some(imported_runtime_sample_rejection(
                SAMPLE_RUNTIME_TOO_SHORT_CODE,
                format!(
                    "imported file runtime is too short for automatic import: expected about {} minutes, probed file is {} seconds",
                    (expected_seconds + 59) / 60,
                    actual_seconds
                ),
            ));
        }

        return None;
    }

    if validation.expected_runtime_seconds.is_none()
        && actual_seconds < MIN_UNKNOWN_RUNTIME_SAMPLE_SECONDS
    {
        return Some(imported_runtime_sample_rejection(
            SAMPLE_RUNTIME_TOO_SHORT_CODE,
            format!(
                "imported file runtime is too short for automatic import: probed file is {} seconds",
                actual_seconds
            ),
        ));
    }

    None
}

#[cfg(any(feature = "runtime-media-analysis", test))]
fn imported_runtime_sample_rejection(code: &'static str, message: String) -> ImportedFileRejection {
    ImportedFileRejection {
        message,
        recycle_reason: code,
        skip_reason: Some(ImportSkipReason::PolicyMismatch),
        blocking_rule_codes: vec![code.to_string()],
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "post-download scoring needs the complete import context to match search-time policy decisions"
)]
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

#[cfg(feature = "runtime-media-analysis")]
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
        video_codec: analysis
            .video_codec
            .as_deref()
            .and_then(crate::release_parser::VideoCodec::parse),
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
                name: stream.name.clone(),
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

pub(crate) fn build_stream_pointer_media_file_analysis() -> crate::MediaFileAnalysis {
    crate::MediaFileAnalysis {
        video_codec: None,
        video_width: None,
        video_height: None,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format: Some("strm".to_string()),
    }
}

fn build_synthetic_media_file_analysis(
    parsed: &crate::ParsedReleaseMetadata,
    container_format: Option<String>,
) -> crate::MediaFileAnalysis {
    let (video_width, video_height) = infer_video_dimensions(parsed.quality.as_deref());

    crate::MediaFileAnalysis {
        video_codec: None,
        video_width,
        video_height,
        video_bitrate_kbps: None,
        video_bit_depth: None,
        video_hdr_format: None,
        video_frame_rate: None,
        video_profile: None,
        audio_codec: None,
        audio_profile: None,
        audio_channels: None,
        audio_bitrate_kbps: None,
        audio_languages: Vec::new(),
        audio_streams: Vec::new(),
        subtitle_languages: Vec::new(),
        subtitle_codecs: Vec::new(),
        subtitle_streams: Vec::new(),
        has_multiaudio: false,
        duration_seconds: None,
        num_chapters: None,
        container_format,
    }
}

#[cfg(feature = "runtime-media-analysis")]
fn build_stream_pointer_media_file_analysis_from_parsed(
    parsed: &crate::ParsedReleaseMetadata,
) -> crate::MediaFileAnalysis {
    build_synthetic_media_file_analysis(parsed, Some("strm".to_string()))
}

fn infer_video_dimensions(quality: Option<&str>) -> (Option<i32>, Option<i32>) {
    match quality
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("2160p") => (Some(3840), Some(2160)),
        Some("1080p") => (Some(1920), Some(1080)),
        Some("720p") => (Some(1280), Some(720)),
        Some("480p") => (Some(854), Some(480)),
        _ => (None, None),
    }
}

fn inferred_container_format_for_path(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
}

#[cfg(feature = "runtime-media-analysis")]
fn path_is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

/// Probe a file at the given path and validate it against the quality profile and user rules.
/// The file does NOT need to be at its final destination — this can probe a file in-place
/// at its download location before any move/copy.
#[expect(
    clippy::too_many_arguments,
    reason = "probe-and-validate carries the full import gate context through one decision point"
)]
#[cfg(feature = "runtime-media-analysis")]
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
    runtime_sample_validation: RuntimeSampleValidation,
) -> ImportedFileGateDecision {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("strm"))
    {
        return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
            analysis: Some(build_stream_pointer_media_file_analysis_from_parsed(parsed)),
            scan_error: None,
            rule_file_doc: None,
            audio_language_warning: None,
        }));
    }

    let analysis = match scryer_mediainfo::analyze_file(path) {
        Ok(analysis) => analysis,
        Err(error) => {
            warn!(error = %error, path = %path.display(), "media analysis failed");
            if let Some(rejection) = runtime_sample_rejection(runtime_sample_validation, None) {
                return ImportedFileGateDecision::Rejected(rejection);
            }
            let synthetic_analysis = path_is_symlink(path).then(|| {
                build_synthetic_media_file_analysis(
                    parsed,
                    inferred_container_format_for_path(path),
                )
            });
            return ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
                analysis: synthetic_analysis,
                scan_error: Some(error.to_string()),
                rule_file_doc: None,
                audio_language_warning: None,
            }));
        }
    };

    if analysis.video_codec.is_none() {
        return ImportedFileGateDecision::Rejected(ImportedFileRejection {
            message: "imported file is not a valid video".to_string(),
            recycle_reason: "invalid_file",
            skip_reason: None,
            blocking_rule_codes: Vec::new(),
        });
    }

    if let Some(rejection) =
        runtime_sample_rejection(runtime_sample_validation, analysis.duration_seconds)
    {
        return ImportedFileGateDecision::Rejected(rejection);
    }

    let category_hint = facet_to_category_hint(&title.facet);
    let required_audio_resolution = app
        .resolve_required_audio_languages(
            Some(&title.id),
            Some(title.library_id.as_str()),
            Some(category_hint),
        )
        .await;
    let required_audio_resolution_failed = required_audio_resolution.is_err();
    let required_audio_languages = required_audio_resolution.unwrap_or_else(|error| {
        warn!(
            error = %error,
            title_id = %title.id,
            "failed to resolve required audio languages; importing without language verification"
        );
        Vec::new()
    });

    let accepted_analysis = build_media_file_analysis(&analysis);

    // Required audio language gate (post-download, file truth). Distinguishes a
    // provable absence (reject) from an untagged/indeterminate result (accept +
    // flag), so a correctly-dubbed file with "und"/untagged tracks is not falsely
    // rejected. Uses the same title context + release hints as the search gate.
    //
    // Manual imports (operator-chosen files) always land: they bypass this gate
    // entirely, exactly as they bypass the runtime-sample check.
    let mut audio_language_warning: Option<String> = None;
    let enforce_required_audio =
        runtime_sample_validation.mode == RuntimeSampleValidationMode::EnforceAutomatic;
    if enforce_required_audio && required_audio_resolution_failed {
        audio_language_warning = Some(
            "required audio languages could not be resolved; imported without language verification"
                .to_string(),
        );
    }
    if enforce_required_audio && !required_audio_languages.is_empty() {
        let title_audio_context = crate::title_audio_language_context(
            title.language.as_deref(),
            title.country.as_deref(),
            Some(category_hint),
            &title.tags,
        );
        let release_audio_hints = crate::release_audio_language_hints_for_title(
            parsed,
            None,
            Some(&title_audio_context),
            true,
        );
        match crate::classify_required_audio(
            &required_audio_languages,
            &accepted_analysis.audio_streams,
            &release_audio_hints,
        ) {
            crate::RequiredAudioVerdict::Satisfied => {}
            crate::RequiredAudioVerdict::Missing(missing) => {
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
            crate::RequiredAudioVerdict::Indeterminate(unverified) => {
                // Neither provably present nor provably absent (untagged tracks):
                // accept rather than bury a possibly-good release, but flag it.
                audio_language_warning = Some(format!(
                    "audio language(s) {} could not be verified from file metadata (untagged track(s)); imported for review",
                    unverified.join(", ")
                ));
            }
        }
    }

    let persona = app
        .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category_hint))
        .await
        .unwrap_or_else(|error| {
            warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve scoring persona, using canonical default"
            );
            crate::ScoringPersona::default()
        });

    let rule_file_doc = crate::user_rule_input::build_file_doc(&analysis);
    let accepted_for_rules = ImportedFileAcceptance {
        analysis: Some(accepted_analysis.clone()),
        scan_error: None,
        rule_file_doc: Some(rule_file_doc.clone()),
        audio_language_warning: None,
    };
    let (rescored_for_rules, _) = rescore_from_mediainfo(parsed, &accepted_for_rules);

    let user_rules_engine = app
        .services
        .customization
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if !user_rules_engine.is_empty() {
        let library_name = match app
            .services
            .catalog
            .libraries
            .get_by_id(&title.library_id)
            .await
        {
            Ok(Some(library)) => Some(library.name),
            Ok(None) => None,
            Err(error) => {
                warn!(
                    error = %error,
                    library_id = %title.library_id,
                    "failed to resolve library name for post-download rule context"
                );
                None
            }
        };
        let decision = build_import_profile_decision(
            quality_profile,
            &required_audio_languages,
            &persona,
            &rescored_for_rules,
            category_hint,
            title.runtime_minutes,
            Some(size_bytes),
            has_existing_file,
        );
        let input = crate::user_rule_input::build_rule_input(
            &rescored_for_rules,
            quality_profile,
            &decision,
            crate::user_rule_input::ReleaseRuntimeInfo {
                size_bytes: Some(size_bytes),
                published_at: None,
                thumbs_up: None,
                thumbs_down: None,
                is_password_protected: None,
                extra: None,
                indexer_languages: None,
            },
            crate::user_rule_input::RuleContextInfo {
                title_id: Some(&title.id),
                library_name: library_name.as_deref(),
                category: Some(facet_to_category_hint(&title.facet)),
                original_language: title.language.as_deref(),
                original_country: title.country.as_deref(),
                title_tags: &title.tags,
                has_existing_file,
                existing_score,
                search_mode: "post_download",
                runtime_minutes: title.runtime_minutes,
                is_filler,
            },
            Some(rule_file_doc.clone()),
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
        analysis: Some(accepted_analysis),
        scan_error: None,
        rule_file_doc: Some(rule_file_doc),
        audio_language_warning,
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "probe-and-validate carries the full import gate context through one decision point"
)]
#[cfg(not(feature = "runtime-media-analysis"))]
pub(crate) async fn probe_and_validate(
    _app: &AppUseCase,
    _title: &Title,
    parsed: &crate::ParsedReleaseMetadata,
    _quality_profile: &crate::QualityProfile,
    path: &Path,
    _size_bytes: i64,
    _has_existing_file: bool,
    _existing_score: Option<i32>,
    _is_filler: bool,
    _runtime_sample_validation: RuntimeSampleValidation,
) -> ImportedFileGateDecision {
    ImportedFileGateDecision::Accepted(Box::new(ImportedFileAcceptance {
        analysis: Some(build_synthetic_media_file_analysis(
            parsed,
            inferred_container_format_for_path(path),
        )),
        scan_error: Some("native media analysis is not compiled into this target".to_string()),
        rule_file_doc: None,
        audio_language_warning: None,
    }))
}

/// Probe a source file once, apply the existing gate, and merge detected media
/// facts back into parsed metadata so downstream rename and scoring decisions
/// use the same resolved view that will later be persisted.
#[expect(
    clippy::too_many_arguments,
    reason = "prepared import candidates need the full gate context plus caller scoring state"
)]
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
    runtime_sample_validation: RuntimeSampleValidation,
) -> Result<PreparedImportCandidate, ImportedFileRejection> {
    let source_snapshot_before = app
        .services
        .workflow
        .file_importer
        .snapshot_import_source(path)
        .await
        .map_err(|err| import_source_changed_rejection(path, err))?;

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
        runtime_sample_validation,
    )
    .await
    {
        ImportedFileGateDecision::Rejected(rejection) => Err(rejection),
        ImportedFileGateDecision::Accepted(accepted) => {
            let source_snapshot_after = app
                .services
                .workflow
                .file_importer
                .snapshot_import_source(path)
                .await
                .map_err(|err| import_source_changed_rejection(path, err))?;
            if source_snapshot_after != source_snapshot_before {
                return Err(import_source_changed_rejection(
                    path,
                    "source identity or content proof changed",
                ));
            }

            let (parsed, rescore_changes) = rescore_from_mediainfo(parsed, accepted.as_ref());
            if !rescore_changes.is_empty() {
                tracing::debug!(
                    title = %title.name,
                    path = %path.display(),
                    changes = ?rescore_changes,
                    "mediainfo rescore prepared import candidate"
                );
            }
            // Surface a required-audio "could not verify" flag (untagged tracks):
            // the file was accepted for review rather than falsely rejected.
            if let Some(warning) = accepted.audio_language_warning.as_deref() {
                warn!(
                    title_id = %title.id,
                    title = %title.name,
                    path = %path.display(),
                    warning,
                    "imported file accepted with unverified required audio language(s) for review"
                );
            }

            Ok(PreparedImportCandidate {
                parsed,
                accepted,
                rescore_changes,
                source_snapshot: source_snapshot_after,
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
        analysis.video_width,
        analysis.video_height,
        analysis.video_codec.as_ref(),
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
        && let Some(parsed_codec) = crate::release_parser::VideoCodec::parse(normalized.as_str())
        && merged.video_codec.as_ref() != Some(&parsed_codec)
    {
        changes.push(format!(
            "video_codec: {} → {}",
            merged
                .video_codec
                .as_ref()
                .map(ToString::to_string)
                .as_deref()
                .unwrap_or("?"),
            normalized
        ));
        merged.video_codec = Some(parsed_codec);
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
        && let Some(codec) = AudioCodec::parse(normalized)
        && merged.audio.as_ref() != Some(&codec)
    {
        changes.push(format!(
            "audio: {} → {}",
            merged.audio.as_ref().map(AudioCodec::as_str).unwrap_or("?"),
            normalized
        ));
        merged.audio = Some(codec);
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
#[expect(
    clippy::too_many_arguments,
    reason = "post-download scoring needs the full import context to match search-time policy decisions"
)]
pub(crate) async fn compute_post_download_acquisition_decision(
    app: &AppUseCase,
    parsed: &crate::ParsedReleaseMetadata,
    acceptance: &ImportedFileAcceptance,
    profile: &crate::QualityProfile,
    title: &Title,
    runtime_minutes: Option<i32>,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    prior_rescore_changes: &[String],
    is_filler: bool,
) -> PostDownloadAcquisitionDecision {
    let (rescored, changes) = rescore_from_mediainfo(parsed, acceptance);
    let mut rescore_changes = prior_rescore_changes.to_vec();
    for change in changes {
        if !rescore_changes
            .iter()
            .any(|existing_change| existing_change == &change)
        {
            rescore_changes.push(change);
        }
    }
    let category = facet_to_category_hint(&title.facet);
    let required_audio_languages = app
        .resolve_required_audio_languages(
            Some(&title.id),
            Some(title.library_id.as_str()),
            Some(category),
        )
        .await
        .unwrap_or_default();
    let persona = app
        .resolve_scoring_persona(Some(title.library_id.as_str()), Some(category))
        .await
        .unwrap_or_default();
    let mut decision = build_import_profile_decision(
        profile,
        &required_audio_languages,
        &persona,
        &rescored,
        category,
        runtime_minutes,
        Some(size_bytes),
        has_existing_file,
    );
    append_post_download_user_rule_scores(
        app,
        title,
        profile,
        &rescored,
        &mut decision,
        acceptance,
        size_bytes,
        has_existing_file,
        existing_score,
        is_filler,
        runtime_minutes,
    )
    .await;
    let score = decision.preference_score;
    if !rescore_changes.is_empty() {
        tracing::debug!(
            title = %title.name,
            score,
            changes = ?rescore_changes,
            "mediainfo rescore applied to acquisition score"
        );
    }
    let scoring_log = serialize_post_download_scoring_log(&decision, &rescore_changes);
    PostDownloadAcquisitionDecision {
        parsed: rescored,
        score,
        scoring_log,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "post-download user-rule scoring needs the same context as import policy scoring"
)]
async fn append_post_download_user_rule_scores(
    app: &AppUseCase,
    title: &Title,
    profile: &crate::QualityProfile,
    parsed: &crate::ParsedReleaseMetadata,
    decision: &mut crate::QualityProfileDecision,
    acceptance: &ImportedFileAcceptance,
    size_bytes: i64,
    has_existing_file: bool,
    existing_score: Option<i32>,
    is_filler: bool,
    runtime_minutes: Option<i32>,
) {
    let user_rules_engine = app
        .services
        .customization
        .user_rules
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_else(|_| scryer_rules::UserRulesEngine::empty());
    if user_rules_engine.is_empty() {
        return;
    }

    let library_name = match app
        .services
        .catalog
        .libraries
        .get_by_id(&title.library_id)
        .await
    {
        Ok(Some(library)) => Some(library.name),
        Ok(None) => None,
        Err(error) => {
            warn!(
                error = %error,
                library_id = %title.library_id,
                "failed to resolve library name for post-download score rule context"
            );
            None
        }
    };
    let category = facet_to_category_hint(&title.facet);
    let file_doc = acceptance.rule_file_doc.clone();
    let input = crate::user_rule_input::build_rule_input(
        parsed,
        profile,
        decision,
        crate::user_rule_input::ReleaseRuntimeInfo {
            size_bytes: Some(size_bytes),
            published_at: None,
            thumbs_up: None,
            thumbs_down: None,
            is_password_protected: None,
            extra: None,
            indexer_languages: None,
        },
        crate::user_rule_input::RuleContextInfo {
            title_id: Some(&title.id),
            library_name: library_name.as_deref(),
            category: Some(category),
            original_language: title.language.as_deref(),
            original_country: title.country.as_deref(),
            title_tags: &title.tags,
            has_existing_file,
            existing_score,
            search_mode: "post_download",
            runtime_minutes,
            is_filler,
        },
        file_doc,
    );
    let mut evaluator = user_rules_engine.evaluator();
    match evaluator.evaluate(&input, category) {
        Ok(result) => {
            for entry in result.entries {
                decision.log_with_source(
                    &entry.code,
                    entry.delta,
                    crate::ScoringSource::UserRule {
                        id: entry.rule_set_id,
                        name: entry.rule_set_name,
                    },
                );
            }
            for err in result.errors {
                decision.log_with_source(
                    "user_rule_error",
                    0,
                    crate::ScoringSource::UserRule {
                        id: err.rule_set_id,
                        name: err.rule_set_name,
                    },
                );
            }
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                "post-download score rule evaluation failed; scoring built-in decision only"
            );
        }
    }
}

fn serialize_post_download_scoring_log(
    decision: &crate::QualityProfileDecision,
    rescore_changes: &[String],
) -> Option<String> {
    let scoring_log = decision
        .scoring_log
        .iter()
        .map(|entry| {
            serde_json::json!({
                "code": entry.code,
                "delta": entry.delta,
                "source": scoring_source_json(&entry.source),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&serde_json::json!({
        "kind": "post_download_acquisition_score",
        "release_score": decision.release_score,
        "preference_score": decision.preference_score,
        "allowed": decision.allowed,
        "block_codes": decision.block_codes,
        "rescore_changes": rescore_changes,
        "scoring_log": scoring_log,
    }))
    .ok()
}

fn scoring_source_json(source: &crate::ScoringSource) -> serde_json::Value {
    match source {
        crate::ScoringSource::Builtin => {
            serde_json::json!({"kind": "builtin"})
        }
        crate::ScoringSource::UserRule { id, name } => {
            serde_json::json!({"kind": "user_rule", "id": id, "name": name})
        }
        crate::ScoringSource::SystemRule { id, name } => {
            serde_json::json!({"kind": "system_rule", "id": id, "name": name})
        }
    }
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
    actor: impl Into<DomainEventActor>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    episode_ids: &[String],
    rejection: &ImportedFileRejection,
) {
    finalize_import_rejection(
        app,
        actor,
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
    actor: impl Into<DomainEventActor>,
    title: &Title,
    completed_name: &str,
    path: &Path,
    episode_ids: &[String],
    rejection: &ImportedFileRejection,
) {
    let normalized_source_title = normalize_release_attempt_title(Some(completed_name));
    let failure_reason = Some(rejection.message.clone());
    let _ = app
        .services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            normalize_release_attempt_hint(None),
            normalized_source_title.clone(),
            ReleaseDownloadAttemptOutcome::Failed,
            failure_reason,
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
    let mut blocklist_data = std::collections::HashMap::new();
    if !episode_ids.is_empty() {
        blocklist_data.insert("episode_ids".to_string(), serde_json::json!(episode_ids));
    }
    if let Err(error) = app
        .services
        .workflow
        .blocklist_repo
        .add(&NewBlocklistEntry {
            title_id: title.id.clone(),
            source_title: normalized_source_title.clone(),
            source_hint: None,
            quality: crate::parse_release_metadata(completed_name).quality,
            download_id: None,
            reason: reason.clone(),
            data: blocklist_data,
        })
        .await
    {
        warn!(
            error = %error,
            title_id = %title.id,
            source_title = normalized_source_title.as_deref().unwrap_or(""),
            "failed to persist blocklist entry for rejected import"
        );
    }
    let _ = app
        .append_domain_event(new_title_domain_event(
            actor,
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

// Reschedules a wanted item for an immediate fresh search after a rejected
// import. For a `language_mismatch` rejection this is intentional: the rejected
// release is blocklisted by title (a provable absence — it genuinely lacks the
// required audio), so the immediate retry seeks a *different*, correct candidate
// rather than re-grabbing the same one. Trustworthy verdicts (see
// `classify_required_audio`) keep this from churning on falsely-rejected files.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn automatic(expected_runtime_seconds: Option<i32>) -> RuntimeSampleValidation {
        RuntimeSampleValidation::automatic(expected_runtime_seconds)
    }

    fn manual(expected_runtime_seconds: Option<i32>) -> RuntimeSampleValidation {
        RuntimeSampleValidation::manual_override(expected_runtime_seconds)
    }

    #[test]
    fn automatic_movie_import_rejects_twenty_second_runtime_for_normal_movie() {
        let rejection = runtime_sample_rejection(automatic(Some(90 * 60)), Some(20))
            .expect("short normal-runtime movie should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
        assert_eq!(
            rejection.skip_reason,
            Some(ImportSkipReason::PolicyMismatch)
        );
        assert_eq!(
            rejection.blocking_rule_codes,
            vec![SAMPLE_RUNTIME_TOO_SHORT_CODE.to_string()]
        );
    }

    #[test]
    fn automatic_episode_import_rejects_twenty_second_runtime_for_normal_episode() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), Some(20))
            .expect("short normal-runtime episode should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
    }

    #[test]
    fn automatic_import_accepts_short_form_movie_above_fixture_runtime_floor() {
        let rejection = runtime_sample_rejection(automatic(Some(3 * 60)), Some(180));

        assert!(rejection.is_none());
    }

    #[test]
    fn automatic_import_rejects_unknown_positive_runtime_under_one_minute() {
        let rejection = runtime_sample_rejection(automatic(None), Some(59))
            .expect("unknown-runtime short clip should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_TOO_SHORT_CODE);
    }

    #[test]
    fn automatic_import_rejects_zero_runtime() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), Some(0))
            .expect("zero runtime should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_ZERO_CODE);
    }

    #[test]
    fn automatic_import_rejects_indeterminate_runtime() {
        let rejection = runtime_sample_rejection(automatic(Some(42 * 60)), None)
            .expect("indeterminate runtime should reject");

        assert_eq!(rejection.recycle_reason, SAMPLE_RUNTIME_INDETERMINATE_CODE);
    }

    #[test]
    fn manual_queued_import_bypasses_runtime_sample_rejection() {
        let rejection = runtime_sample_rejection(manual(Some(42 * 60)), Some(20));

        assert!(rejection.is_none());
    }
}
