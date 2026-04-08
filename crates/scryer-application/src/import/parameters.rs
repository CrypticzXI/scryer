use crate::DownloadSubmission;

/// Returns true if the download was submitted by scryer.
pub(crate) fn has_scryer_origin(params: &[(String, String)]) -> bool {
    params.iter().any(|(key, _)| key == "*scryer_title_id")
}

pub(crate) fn submission_has_scryer_origin(submission: &DownloadSubmission) -> bool {
    !submission.title_id.trim().is_empty()
}

pub(crate) fn extract_parameter(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(candidate_key, _)| candidate_key == key)
        .map(|(_, value)| value.clone())
}
