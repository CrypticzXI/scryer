fn build_augmented_movie_import_metadata(
    source_video: &Path,
    completed: &CompletedDownload,
) -> ParsedReleaseMetadata {
    let mut parsed = parsed_release_from_file_stem(source_video);
    clear_unusable_release_title_signal(&mut parsed);
    if !has_usable_release_title_signal(&parsed)
        && let Some(source_parent_info) = parsed_usable_release_from_parent_folder(source_video)
    {
        fill_missing_release_metadata(&mut parsed, &source_parent_info, false);
    }
    let download_client_info =
        normalize_release_title_signal(parse_release_metadata(&completed.name));
    fill_missing_release_metadata(&mut parsed, &download_client_info, false);
    if let Some(folder_info) = parsed_release_from_folder_name(Path::new(&completed.dest_dir)) {
        fill_missing_release_metadata(&mut parsed, &folder_info, false);
    }
    parsed
}
