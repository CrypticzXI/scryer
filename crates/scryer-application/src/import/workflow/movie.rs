fn build_augmented_movie_import_metadata(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
) -> ParsedReleaseMetadata {
    release_evidence
        .release_title(Some(source_video))
        .map(|release_title| {
            normalize_release_title_signal(parse_release_metadata(&release_title))
        })
        .unwrap_or_default()
}
