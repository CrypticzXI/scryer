use chrono::{DateTime, Utc};
use scryer_domain::{CompletedDownload, Id};

use crate::{
    DownloadSourceKind, DownloadSubmission, DownloadSubmissionIdentity, SubmissionScope,
    helpers::{normalize_release_attempt_title, sha256_hex},
};

pub const DOWNLOAD_REQUEST_ID_PARAMETER: &str = "*scryer_request_id";
pub const DOWNLOAD_FINGERPRINT_PARAMETER: &str = "*scryer_download_fingerprint";

pub struct DownloadFingerprintInput<'a> {
    pub request_id: Option<&'a str>,
    pub title_id: Option<&'a str>,
    pub facet: Option<&'a str>,
    pub scope: Option<&'a SubmissionScope>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_hint: Option<&'a str>,
    pub source_title: Option<&'a str>,
    pub info_hash_hint: Option<&'a str>,
    pub indexer_name: Option<&'a str>,
    pub size_bytes: Option<i64>,
    pub client_type: Option<&'a str>,
    pub output_path: Option<&'a str>,
    pub category: Option<&'a str>,
    pub completed_at: Option<DateTime<Utc>>,
}

pub(crate) fn new_download_request_id() -> String {
    format!("scryer-request:{}", Id::new().0)
}

pub struct ObservedDownloadIdentityInput<'a> {
    pub download_request_id: Option<&'a str>,
    pub download_fingerprint: Option<&'a str>,
    pub parameters: &'a [(String, String)],
    pub info_hash_hint: Option<&'a str>,
}

pub(crate) struct DownloadSubmissionCompatibilityEvidence<'a> {
    pub title_id: Option<&'a str>,
    pub episode_id: Option<&'a str>,
    pub source_title: Option<&'a str>,
}

pub(crate) fn download_submission_is_compatible_with_evidence(
    submission: &DownloadSubmission,
    evidence: DownloadSubmissionCompatibilityEvidence<'_>,
) -> bool {
    let mut has_target_evidence = false;

    if let Some(title_id) = normalize_token(evidence.title_id) {
        has_target_evidence = true;
        if !submission.title_id.trim().is_empty() && submission.title_id.trim() != title_id {
            return false;
        }
    }

    if let Some(episode_id) = normalize_token(evidence.episode_id) {
        has_target_evidence = true;
        if !submission_scope_contains_episode(&submission.scope, episode_id.as_str()) {
            return false;
        }
    }

    let release_title_matches = match (
        normalize_release_title(submission.source_title.as_deref()),
        normalize_release_title(evidence.source_title),
    ) {
        (Some(submission_title), Some(observed_title)) => submission_title == observed_title,
        _ => false,
    };

    release_title_matches && has_target_evidence
}

pub(crate) fn coalesce_download_submissions_by_release_attempt(
    submissions: &[DownloadSubmission],
) -> Option<DownloadSubmission> {
    let first = submissions.first()?;
    let first_key = download_submission_release_attempt_key(first);
    submissions
        .iter()
        .all(|submission| download_submission_release_attempt_key(submission) == first_key)
        .then(|| first.clone())
}

pub(crate) fn coalesce_completed_downloads_by_release_observation(
    downloads: &[CompletedDownload],
) -> Option<CompletedDownload> {
    let first = downloads.first()?;
    let first_key = completed_download_release_observation_key(first);
    downloads
        .iter()
        .all(|download| completed_download_release_observation_key(download) == first_key)
        .then(|| first.clone())
}

pub fn observed_download_identity(
    input: ObservedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let download_request_id = normalize_token(input.download_request_id)
        .or_else(|| observed_identity_parameter(input.parameters, DOWNLOAD_REQUEST_ID_PARAMETER));
    let download_fingerprint = normalize_token(input.download_fingerprint)
        .or_else(|| observed_identity_parameter(input.parameters, DOWNLOAD_FINGERPRINT_PARAMETER))
        .or_else(|| download_fingerprint_from_info_hash(input.info_hash_hint));

    DownloadSubmissionIdentity {
        download_request_id,
        download_fingerprint,
    }
}

pub fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_request_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
        && identity
            .download_fingerprint
            .as_deref()
            .map(str::trim)
            .is_none_or(str::is_empty)
}

pub fn download_fingerprint_from_info_hash(info_hash_hint: Option<&str>) -> Option<String> {
    let info_hash = normalize_explicit_info_hash(info_hash_hint)?;
    Some(hash_fields(
        "tier1:torrent_info_hash",
        vec![("info_hash", info_hash)],
    ))
}

pub fn build_download_fingerprint(input: DownloadFingerprintInput<'_>) -> Option<String> {
    if let Some(info_hash) = normalize_explicit_info_hash(input.info_hash_hint)
        .or_else(|| normalize_magnet_info_hash(input.source_hint))
    {
        return Some(hash_fields(
            "tier1:torrent_info_hash",
            vec![("info_hash", info_hash)],
        ));
    }

    if let Some(source_hint) = normalize_source_hint(input.source_hint)
        && input.source_kind.is_some()
    {
        return Some(hash_fields(
            "tier1:source_hint",
            vec![
                (
                    "source_kind",
                    input.source_kind.map(|kind| kind.as_str().to_string())?,
                ),
                ("source_hint", source_hint),
            ],
        ));
    }

    if let Some(request_id) = normalize_token(input.request_id) {
        return Some(hash_fields(
            "tier1:scryer_request_id",
            vec![("request_id", request_id)],
        ));
    }

    let title_id = normalize_token(input.title_id)?;
    let mut fields = vec![("title_id", title_id)];
    if let Some(facet) = normalize_lower(input.facet) {
        fields.push(("facet", facet));
    }
    if let Some(scope) = normalize_scope(input.scope) {
        fields.push(("scope", scope));
    }
    if let Some(source_kind) = input.source_kind {
        fields.push(("source_kind", source_kind.as_str().to_string()));
    }
    if let Some(source_hint) = normalize_source_hint(input.source_hint) {
        fields.push(("source_hint", source_hint));
    }
    if let Some(source_title) = normalize_release_title(input.source_title) {
        fields.push(("source_title", source_title));
    }
    if let Some(indexer_name) = normalize_lower(input.indexer_name) {
        fields.push(("indexer_name", indexer_name));
    }
    if let Some(size_bytes) = input.size_bytes.filter(|value| *value > 0) {
        fields.push(("size_bytes", size_bytes.to_string()));
    }

    if fields.len() > 1
        && fields
            .iter()
            .any(|(key, _)| matches!(*key, "scope" | "source_hint" | "source_title"))
    {
        return Some(hash_fields("tier2:scryer_release_attempt", fields));
    }

    let mut foreign = Vec::new();
    if let Some(client_type) = normalize_lower(input.client_type) {
        foreign.push(("client_type", client_type));
    }
    if let Some(path) = normalize_output_path_basename(input.output_path) {
        foreign.push(("output", path));
    }
    if let Some(source_title) = normalize_release_title(input.source_title) {
        foreign.push(("source_title", source_title));
    }
    if let Some(size_bytes) = input.size_bytes.filter(|value| *value > 0) {
        foreign.push(("size_bytes", size_bytes.to_string()));
    }
    if let Some(category) = normalize_lower(input.category) {
        foreign.push(("category", category));
    }
    if let Some(bucket) = input.completed_at.map(timestamp_bucket) {
        foreign.push(("completed_bucket", bucket));
    }

    (foreign.len() >= 4).then(|| hash_fields("tier3:foreign_completed_item", foreign))
}

fn hash_fields(tier: &str, fields: Vec<(&'static str, String)>) -> String {
    let mut material = String::from("scryer-download-fingerprint:v1\n");
    material.push_str(tier);
    material.push('\n');
    for (key, value) in fields {
        material.push_str(key);
        material.push('=');
        material.push_str(&value.len().to_string());
        material.push(':');
        material.push_str(&value);
        material.push('\n');
    }
    format!("sha256:{}", sha256_hex(material))
}

fn normalize_token(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn observed_identity_parameter(parameters: &[(String, String)], key: &str) -> Option<String> {
    parameters
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| normalize_token(Some(value)))
}

fn normalize_lower(raw: Option<&str>) -> Option<String> {
    normalize_token(raw).map(|value| value.to_ascii_lowercase())
}

fn normalize_release_title(raw: Option<&str>) -> Option<String> {
    normalize_release_attempt_title(raw)
}

fn normalize_source_hint(raw: Option<&str>) -> Option<String> {
    normalize_token(raw).map(|value| value.to_ascii_lowercase())
}

fn normalize_scope(scope: Option<&SubmissionScope>) -> Option<String> {
    match scope? {
        SubmissionScope::Episode { episode_id } => {
            normalize_token(Some(episode_id)).map(|id| format!("episode:{id}"))
        }
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids = episode_ids
                .iter()
                .filter_map(|id| normalize_token(Some(id)))
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            (!ids.is_empty()).then(|| format!("episodes:{}", ids.join(",")))
        }
        SubmissionScope::Collection { collection_id } => {
            normalize_token(Some(collection_id)).map(|id| format!("collection:{id}"))
        }
        SubmissionScope::Title => Some("title".to_string()),
        SubmissionScope::Orphan => None,
    }
}

fn submission_scope_contains_episode(scope: &SubmissionScope, episode_id: &str) -> bool {
    match scope {
        SubmissionScope::Episode {
            episode_id: submission_episode_id,
        } => submission_episode_id.trim() == episode_id,
        SubmissionScope::EpisodeSet { episode_ids } => episode_ids
            .iter()
            .any(|submission_episode_id| submission_episode_id.trim() == episode_id),
        SubmissionScope::Title | SubmissionScope::Collection { .. } | SubmissionScope::Orphan => {
            false
        }
    }
}

fn download_submission_release_attempt_key(submission: &DownloadSubmission) -> String {
    let scope = match &submission.scope {
        SubmissionScope::Episode { episode_id } => format!("episode:{}", episode_id.trim()),
        SubmissionScope::EpisodeSet { episode_ids } => {
            let mut ids = episode_ids
                .iter()
                .map(|episode_id| episode_id.trim().to_string())
                .filter(|episode_id| !episode_id.is_empty())
                .collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            format!("episodes:{}", ids.join(","))
        }
        SubmissionScope::Collection { collection_id } => {
            format!("collection:{}", collection_id.trim())
        }
        SubmissionScope::Title => "title".to_string(),
        SubmissionScope::Orphan => "orphan".to_string(),
    };
    format!(
        "{}\u{1f}{}\u{1f}{}",
        submission.title_id.trim(),
        submission.facet.trim().to_ascii_lowercase(),
        scope
    )
}

fn completed_download_release_observation_key(completed: &CompletedDownload) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:?}",
        normalize_observed_identity_field(&completed.client_type),
        normalize_observed_identity_field(&completed.name),
        normalize_observed_identity_field(&completed.dest_dir),
        completed
            .category
            .as_deref()
            .map(normalize_observed_identity_field)
            .unwrap_or_default(),
        completed.size_bytes
    )
}

fn normalize_observed_identity_field(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_explicit_info_hash(raw: Option<&str>) -> Option<String> {
    let mut value = normalize_lower(raw)?;
    if let Some(stripped) = value.strip_prefix("urn:btih:") {
        value = stripped.to_string();
    }
    let normalized = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();
    (!normalized.is_empty()).then_some(normalized)
}

fn normalize_magnet_info_hash(raw: Option<&str>) -> Option<String> {
    let value = normalize_lower(raw)?;
    if !value.starts_with("magnet:?") {
        return None;
    }
    let hash = value
        .split('&')
        .find_map(|part| part.strip_prefix("xt=urn:btih:").map(str::to_string))?;
    normalize_explicit_info_hash(Some(&hash))
}

fn normalize_output_path_basename(raw: Option<&str>) -> Option<String> {
    let value = normalize_token(raw)?;
    let basename = value
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|part| !part.is_empty())?;
    let stem = basename
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(basename);
    normalize_release_title(Some(stem))
}

fn timestamp_bucket(value: DateTime<Utc>) -> String {
    let bucket = value.timestamp().div_euclid(6 * 60 * 60);
    bucket.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input<'a>(
        title_id: &'a str,
        scope: &'a SubmissionScope,
        source_title: &'a str,
    ) -> DownloadFingerprintInput<'a> {
        DownloadFingerprintInput {
            request_id: None,
            title_id: Some(title_id),
            facet: Some("series"),
            scope: Some(scope),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: None,
            source_title: Some(source_title),
            info_hash_hint: None,
            indexer_name: Some("Indexer"),
            size_bytes: None,
            client_type: None,
            output_path: None,
            category: None,
            completed_at: None,
        }
    }

    fn base_submission(scope: SubmissionScope, source_title: Option<&str>) -> DownloadSubmission {
        DownloadSubmission {
            title_id: "title-1".to_string(),
            facet: "series".to_string(),
            download_client_id: Some("client-1".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "item-1".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: source_title.map(str::to_string),
            request_signature: None,
            scope,
        }
    }

    #[test]
    fn title_id_alone_does_not_create_fingerprint() {
        let fingerprint = build_download_fingerprint(DownloadFingerprintInput {
            request_id: None,
            title_id: Some("title-1"),
            facet: None,
            scope: None,
            source_kind: None,
            source_hint: None,
            source_title: None,
            info_hash_hint: None,
            indexer_name: None,
            size_bytes: None,
            client_type: None,
            output_path: None,
            category: None,
            completed_at: None,
        });

        assert_eq!(fingerprint, None);
    }

    #[test]
    fn legacy_compatibility_requires_release_title() {
        let submission = base_submission(SubmissionScope::Title, None);

        assert!(!download_submission_is_compatible_with_evidence(
            &submission,
            DownloadSubmissionCompatibilityEvidence {
                title_id: Some("title-1"),
                episode_id: None,
                source_title: Some("Show.2026.1080p.WEB-DL"),
            },
        ));
    }

    #[test]
    fn title_scope_does_not_satisfy_episode_evidence() {
        let submission = base_submission(SubmissionScope::Title, Some("Show.S01E05.1080p.WEB-DL"));

        assert!(!download_submission_is_compatible_with_evidence(
            &submission,
            DownloadSubmissionCompatibilityEvidence {
                title_id: Some("title-1"),
                episode_id: Some("episode-5"),
                source_title: Some("Show.S01E05.1080p.WEB-DL"),
            },
        ));
    }

    #[test]
    fn episode_scope_distinguishes_same_title_release_attempts() {
        let episode_five = SubmissionScope::Episode {
            episode_id: "episode-5".to_string(),
        };
        let episode_seven = SubmissionScope::Episode {
            episode_id: "episode-7".to_string(),
        };

        let first = build_download_fingerprint(base_input(
            "title-witch-hat",
            &episode_five,
            "Witch.Hat.Atelier.S01E05.1080p.WEB-DL",
        ))
        .expect("episode fingerprint");
        let second = build_download_fingerprint(base_input(
            "title-witch-hat",
            &episode_seven,
            "Witch.Hat.Atelier.S01E07.1080p.WEB-DL",
        ))
        .expect("episode fingerprint");

        assert_ne!(first, second);
    }

    #[test]
    fn retry_with_different_release_evidence_changes_fingerprint() {
        let episode = SubmissionScope::Episode {
            episode_id: "episode-5".to_string(),
        };

        let first = build_download_fingerprint(base_input(
            "title-1",
            &episode,
            "Show.S01E05.1080p.WEB-DL-GROUP",
        ))
        .expect("first fingerprint");
        let second = build_download_fingerprint(base_input(
            "title-1",
            &episode,
            "Show.S01E05.1080p.BluRay-OTHER",
        ))
        .expect("second fingerprint");

        assert_ne!(first, second);
    }

    #[test]
    fn torrent_info_hash_is_stable_across_label_changes() {
        let episode = SubmissionScope::Episode {
            episode_id: "episode-5".to_string(),
        };
        let first = DownloadFingerprintInput {
            info_hash_hint: Some("ABCDEF0123456789ABCDEF0123456789ABCDEF01"),
            ..base_input("title-1", &episode, "First.Label")
        };
        let second = DownloadFingerprintInput {
            info_hash_hint: Some("abcdef0123456789abcdef0123456789abcdef01"),
            ..base_input("title-1", &episode, "Second.Label")
        };

        assert_eq!(
            build_download_fingerprint(first),
            build_download_fingerprint(second)
        );
    }
}
