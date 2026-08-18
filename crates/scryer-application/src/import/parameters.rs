use crate::DownloadSubmission;

/// Whether a submission row is Scryer's own identity for the download: a grab
/// Scryer made, or a title an operator explicitly assigned (recorded like a
/// grab — the store reads any titled row back with a real scope). Only the
/// title-less orphan stub rows the tracker records for admitted observations
/// carry no Scryer origin.
pub(crate) fn submission_has_scryer_origin(submission: &DownloadSubmission) -> bool {
    !matches!(&submission.scope, crate::SubmissionScope::Orphan)
        && !submission.title_id.trim().is_empty()
}

pub(crate) fn extract_parameter(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(candidate_key, _)| candidate_key == key)
        .map(|(_, value)| value.clone())
}
