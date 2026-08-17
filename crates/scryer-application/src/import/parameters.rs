use crate::DownloadSubmission;

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
