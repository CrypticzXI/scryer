fn parsed_with_quality_override(
    parsed: &crate::ParsedReleaseMetadata,
    quality_label: Option<&str>,
) -> crate::ParsedReleaseMetadata {
    let mut effective = parsed.clone();
    if let Some(quality_label) = quality_label {
        effective.quality = Some(quality_label.to_string());
    }
    effective
}
