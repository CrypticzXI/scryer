use scryer_domain::{CompletedDownload, Id};

use crate::{DownloadSourceKind, DownloadSubmission, DownloadSubmissionIdentity, SubmissionScope};

pub const DOWNLOAD_ID_PARAMETER: &str = "*scryer_download_id";

pub struct AcceptedDownloadIdentityInput<'a> {
    pub initial_download_id: Option<&'a str>,
    pub source_kind: Option<DownloadSourceKind>,
    pub source_hint: Option<&'a str>,
    pub info_hash_hint: Option<&'a str>,
    pub client_type: Option<&'a str>,
    pub client_item_id: Option<&'a str>,
    pub accepted_info_hash: Option<&'a str>,
}

pub(crate) fn new_download_id() -> String {
    format!("scryer-download:{}", Id::new().0)
}

pub struct ObservedDownloadIdentityInput<'a> {
    pub download_id: Option<&'a str>,
    pub parameters: &'a [(String, String)],
    pub info_hash_hint: Option<&'a str>,
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
    let download_id = normalize_token(input.download_id)
        .or_else(|| observed_identity_parameter(input.parameters, DOWNLOAD_ID_PARAMETER))
        .or_else(|| download_id_from_info_hash(input.info_hash_hint));

    DownloadSubmissionIdentity { download_id }
}

pub fn download_submission_identity_is_empty(identity: &DownloadSubmissionIdentity) -> bool {
    identity
        .download_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
}

pub fn download_id_from_info_hash(info_hash_hint: Option<&str>) -> Option<String> {
    normalize_torrent_info_hash(info_hash_hint)
}

pub fn normalize_torrent_info_hash(raw: Option<&str>) -> Option<String> {
    let mut value = normalize_lower(raw)?;
    if let Some(stripped) = value.strip_prefix("urn:btih:") {
        value = stripped.to_string();
    }
    let normalized = value
        .chars()
        .filter(|ch| ch.is_ascii_hexdigit())
        .collect::<String>();
    matches!(normalized.len(), 40 | 64).then_some(normalized)
}

pub fn accepted_download_submission_identity(
    input: AcceptedDownloadIdentityInput<'_>,
) -> DownloadSubmissionIdentity {
    let expected_info_hash = normalize_torrent_info_hash(input.info_hash_hint)
        .or_else(|| normalize_magnet_info_hash(input.source_hint));
    let actual_info_hash = normalize_torrent_info_hash(input.accepted_info_hash).or_else(|| {
        if source_kind_is_torrent(input.source_kind) || expected_info_hash.is_some() {
            normalize_torrent_info_hash(input.client_item_id)
        } else {
            None
        }
    });

    if let (Some(expected), Some(actual)) = (&expected_info_hash, &actual_info_hash)
        && expected != actual
    {
        tracing::debug!(
            initial_download_id = input.initial_download_id.unwrap_or(""),
            client_type = input.client_type.unwrap_or(""),
            expected_info_hash = expected.as_str(),
            actual_info_hash = actual.as_str(),
            "download_torrent_info_hash_mismatch"
        );
    }

    let download_id = actual_info_hash
        .or(expected_info_hash)
        .or_else(|| {
            client_round_trips_download_id(input.client_type)
                .then(|| normalize_token(input.initial_download_id))
                .flatten()
        })
        .or_else(|| normalize_token(input.client_item_id))
        .or_else(|| normalize_token(input.initial_download_id));

    DownloadSubmissionIdentity { download_id }
}

fn client_round_trips_download_id(client_type: Option<&str>) -> bool {
    matches!(
        normalize_lower(client_type).as_deref(),
        Some("nzbget" | "weaver")
    )
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
        SubmissionScope::SeriesMovie {
            series_movie_link_id,
        } => format!("series_movie:{series_movie_link_id}"),
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

fn normalize_magnet_info_hash(raw: Option<&str>) -> Option<String> {
    let value = normalize_lower(raw)?;
    if !value.starts_with("magnet:?") {
        return None;
    }
    let hash = value
        .split('&')
        .find_map(|part| part.strip_prefix("xt=urn:btih:").map(str::to_string))?;
    normalize_torrent_info_hash(Some(&hash))
}

fn source_kind_is_torrent(source_kind: Option<DownloadSourceKind>) -> bool {
    matches!(
        source_kind,
        Some(DownloadSourceKind::TorrentFile | DownloadSourceKind::MagnetUri)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_torrent_hash_is_download_id() {
        let accepted_hash = "ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD";
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::TorrentFile),
            source_hint: Some("https://indexer.example/release.torrent"),
            info_hash_hint: None,
            client_type: Some("qbittorrent"),
            client_item_id: Some("job-1"),
            accepted_info_hash: Some(accepted_hash),
        });

        assert_eq!(
            identity.download_id.as_deref(),
            Some("abcdefabcdefabcdefabcdefabcdefabcdefabcd")
        );
    }

    #[test]
    fn nzbget_keeps_round_trip_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("nzbget"),
            client_item_id: Some("10010"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("scryer-download:1"));
    }

    #[test]
    fn sab_uses_client_download_id() {
        let identity = accepted_download_submission_identity(AcceptedDownloadIdentityInput {
            initial_download_id: Some("scryer-download:1"),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_hint: Some("https://indexer.example/release.nzb"),
            info_hash_hint: None,
            client_type: Some("sabnzbd"),
            client_item_id: Some("SABnzbd_nzo_abc"),
            accepted_info_hash: None,
        });

        assert_eq!(identity.download_id.as_deref(), Some("SABnzbd_nzo_abc"));
    }
}
