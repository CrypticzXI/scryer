use super::*;

struct ExistingScannedMediaFile<'a> {
    file_id: &'a str,
    should_skip_analysis: bool,
    should_refresh_source_signature: bool,
}

struct PersistedScannedMediaFile {
    file_id: String,
    should_analyze: bool,
    title_updated: bool,
    db_elapsed: Duration,
}

async fn persist_or_reuse_scanned_media_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    parsed: &crate::ParsedReleaseMetadata,
    snapshot: &FileSourceSnapshot,
    existing: Option<ExistingScannedMediaFile<'_>>,
    summary: &mut LibraryScanSummary,
    update_error_message: &'static str,
    insert_error_message: &'static str,
) -> Option<PersistedScannedMediaFile> {
    let source_signature_scheme = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.scheme.clone());
    let source_signature_value = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.value.clone());

    if let Some(existing) = existing {
        let mut db_elapsed = Duration::default();
        summary.skipped += 1;

        if existing.should_refresh_source_signature {
            let db_started = Instant::now();
            let update_result = app
                .services
                .library
                .media_files
                .update_media_file_source_signature(
                    existing.file_id,
                    snapshot.size_bytes,
                    source_signature_scheme.clone(),
                    source_signature_value.clone(),
                )
                .await;
            db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            if let Err(error) = update_result {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_id = %existing.file_id,
                    "{update_error_message}"
                );
            }
        }

        return Some(PersistedScannedMediaFile {
            file_id: existing.file_id.to_string(),
            should_analyze: !existing.should_skip_analysis,
            title_updated: false,
            db_elapsed,
        });
    }

    let media_file_input = crate::InsertMediaFileInput {
        title_id: title.id.clone(),
        file_path: file.path.clone(),
        size_bytes: snapshot.size_bytes,
        source_signature_scheme,
        source_signature_value,
        quality_label: parsed.quality.clone(),
        scene_name: Some(parsed.raw_title.clone()),
        release_group: parsed.release_group.clone(),
        source_type: parsed.source.clone(),
        resolution: parsed.quality.clone(),
        video_codec_parsed: parsed.video_codec.clone(),
        audio_codec_parsed: parsed.audio.clone(),
        audio_channels_parsed: parsed.audio_channels.clone(),
        ..Default::default()
    };

    let db_started = Instant::now();
    let insert_result = app
        .services
        .library
        .media_files
        .insert_media_file(&media_file_input)
        .await;
    let db_elapsed = db_started.elapsed();

    match insert_result {
        Ok(file_id) => {
            summary.imported += 1;
            Some(PersistedScannedMediaFile {
                file_id,
                should_analyze: true,
                title_updated: true,
                db_elapsed,
            })
        }
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_path = %file.path,
                "{insert_error_message}"
            );
            summary.skipped += 1;
            None
        }
    }
}

async fn persist_scanned_media_analysis_outcome(
    app: &AppUseCase,
    title: &Title,
    file_id: &str,
    outcome: MediaAnalysisOutcome,
) -> (Duration, bool) {
    let db_started = Instant::now();

    let persisted = match outcome {
        MediaAnalysisOutcome::Valid(analysis) => {
            let update_result = app
                .services
                .library
                .media_files
                .update_media_file_analysis(file_id, *analysis)
                .await;
            match update_result {
                Ok(()) => true,
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_id = %file_id,
                        "failed to persist scanned media analysis"
                    );
                    false
                }
            }
        }
        MediaAnalysisOutcome::Invalid(error_message) => {
            let mark_result = app
                .services
                .library
                .media_files
                .mark_scan_failed(file_id, &error_message)
                .await;
            match mark_result {
                Ok(()) => true,
                Err(error) => {
                    warn!(
                        error = %error,
                        title_id = %title.id,
                        file_id = %file_id,
                        "failed to mark scanned media analysis failure"
                    );
                    false
                }
            }
        }
    };

    (db_started.elapsed(), persisted)
}

fn scanned_media_analysis_status(outcome: &MediaAnalysisOutcome) -> &'static str {
    match outcome {
        MediaAnalysisOutcome::Valid(_) => "scanned",
        MediaAnalysisOutcome::Invalid(_) => "failed",
    }
}

async fn emit_scanned_media_file_analyzed_event(
    app: &AppUseCase,
    title: &Title,
    file_id: &str,
    file_path: &str,
    analysis_status: &str,
    episode_ids: Vec<String>,
) {
    let event = crate::domain_events::new_title_domain_event(
        None,
        title,
        scryer_domain::DomainEventPayload::MediaFileAnalyzed(
            scryer_domain::MediaFileAnalyzedEventData {
                title: crate::domain_events::title_context_snapshot(title),
                media_updates: vec![crate::domain_events::modified_media_update(file_path)],
                file_id: file_id.to_string(),
                analysis_status: analysis_status.to_string(),
                episode_ids,
            },
        ),
    );

    if let Err(error) = app.append_domain_event(event).await {
        warn!(
            error = %error,
            title_id = %title.id,
            file_id = %file_id,
            "failed to append scanned media file analyzed domain event"
        );
    }
}

async fn ensure_movie_collection_for_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    parsed: &crate::ParsedReleaseMetadata,
    collections: &[Collection],
) -> bool {
    let already_tracked = collections.iter().any(|collection| {
        collection
            .ordered_path
            .as_deref()
            .is_some_and(|path| path == file.path)
    });

    if already_tracked {
        return false;
    }

    let next_collection_index = collections
        .iter()
        .filter_map(|collection| collection.collection_index.parse::<u32>().ok())
        .max()
        .map_or(1, |max| max + 1);
    let quality_label = parsed.quality.as_ref().filter(|q| !q.is_empty()).cloned();

    let collection = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: CollectionType::Movie,
        collection_index: next_collection_index.to_string(),
        label: quality_label,
        ordered_path: Some(file.path.clone()),
        narrative_order: None,
        first_episode_number: None,
        last_episode_number: None,
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: title.monitored,
        created_at: Utc::now(),
    };

    if let Err(err) = app
        .services
        .catalog
        .shows
        .create_collection(collection)
        .await
    {
        info!(
            title_id = %title.id,
            path = %file.path,
            error = %err,
            "failed to create collection for library file"
        );
        false
    } else {
        true
    }
}

pub(crate) async fn finalize_title_scan_file(
    app: &AppUseCase,
    title: &Title,
    plan: PlannedTitleScanFile,
    analysis_outcome: Option<MediaAnalysisOutcome>,
    scan_mode: LibraryScanMode,
    episode_links: &mut HashSet<(String, String)>,
    summary: &mut LibraryScanSummary,
    db_elapsed: &mut Duration,
) -> TitleScanFinalizeOutcome {
    let PlannedTitleScanFile {
        file,
        parsed,
        target_episodes,
        snapshot,
        record,
    } = plan;

    let existing = match &record {
        PlannedTitleScanRecord::Existing {
            file_id,
            should_skip_analysis,
            should_refresh_source_signature,
        } => Some(ExistingScannedMediaFile {
            file_id,
            should_skip_analysis: *should_skip_analysis,
            should_refresh_source_signature: *should_refresh_source_signature,
        }),
        PlannedTitleScanRecord::New => None,
    };

    let Some(persisted_file) = persist_or_reuse_scanned_media_file(
        app,
        title,
        &file,
        &parsed,
        &snapshot,
        existing,
        summary,
        "failed to refresh media file source signature during title scan",
        "failed to insert media file during title scan",
    )
    .await
    else {
        return TitleScanFinalizeOutcome {
            progress: TitleScanProgressDelta::failed(1),
            title_updated: false,
        };
    };
    *db_elapsed = db_elapsed.saturating_add(persisted_file.db_elapsed);

    let should_link_target_episodes = !matches!(
        (&scan_mode, &record),
        (
            LibraryScanMode::Additive,
            PlannedTitleScanRecord::Existing { .. }
        )
    );
    let mut title_updated = persisted_file.title_updated;

    for episode in &target_episodes {
        if !should_link_target_episodes {
            continue;
        }
        if episode_links.insert((persisted_file.file_id.clone(), episode.id.clone())) {
            title_updated = true;
            let db_started = Instant::now();
            let link_result = app
                .services
                .library
                .media_files
                .link_file_to_episode(&persisted_file.file_id, &episode.id)
                .await;
            *db_elapsed = db_elapsed.saturating_add(db_started.elapsed());
            if let Err(error) = link_result {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    episode_id = %episode.id,
                    file_id = %persisted_file.file_id,
                    "failed to link scanned file to episode"
                );
            }
        }
        crate::import_workflow::mark_wanted_completed(app, &title.id, Some(&episode.id), None)
            .await;
    }

    if let Some(outcome) = analysis_outcome {
        let analysis_status = scanned_media_analysis_status(&outcome);
        let (analysis_db_elapsed, analysis_persisted) =
            persist_scanned_media_analysis_outcome(app, title, &persisted_file.file_id, outcome)
                .await;
        *db_elapsed = db_elapsed.saturating_add(analysis_db_elapsed);
        if analysis_persisted {
            emit_scanned_media_file_analyzed_event(
                app,
                title,
                &persisted_file.file_id,
                &file.path,
                analysis_status,
                target_episodes
                    .iter()
                    .map(|episode| episode.id.clone())
                    .collect(),
            )
            .await;
        }
    }

    TitleScanFinalizeOutcome {
        progress: TitleScanProgressDelta::completed(1),
        title_updated,
    }
}

/// Register a discovered movie file the same way episodic title scans do:
/// persist or reuse a media-file row, run media analysis when needed, and
/// ensure a movie collection points at the file path for overview UI.
pub(super) async fn finalize_movie_scan_file(
    app: &AppUseCase,
    title: &Title,
    file: &LibraryFile,
    summary: &mut LibraryScanSummary,
    cancel_token: Option<&CancellationToken>,
) {
    let file_stem = Path::new(&file.path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();
    let parsed = parse_release_metadata(file_stem);

    let snapshot = if let Some(snapshot) = file_source_snapshot_from_library_file(file) {
        snapshot
    } else {
        let metadata = match tokio::fs::metadata(&file.path).await {
            Ok(metadata) => metadata,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "failed to read movie file metadata during library scan"
                );
                summary.skipped += 1;
                return;
            }
        };

        FileSourceSnapshot {
            size_bytes: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
            signature: file_source_signature_from_metadata(&metadata),
        }
    };

    let existing_files = match app
        .services
        .library
        .media_files
        .list_media_files_for_title(&title.id)
        .await
    {
        Ok(files) => files,
        Err(error) => {
            warn!(
                error = %error,
                title_id = %title.id,
                file_path = %file.path,
                "failed to list media files during movie library scan"
            );
            summary.skipped += 1;
            return;
        }
    };

    let desired_source_signature_scheme = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.scheme.clone());
    let desired_source_signature_value = snapshot
        .signature
        .as_ref()
        .map(|signature| signature.value.clone());
    let existing = existing_files
        .iter()
        .find(|item| item.file_path == file.path)
        .map(|existing| ExistingScannedMediaFile {
            file_id: existing.id.as_str(),
            should_skip_analysis: title_media_file_matches_snapshot(existing, &snapshot),
            should_refresh_source_signature: existing.size_bytes != snapshot.size_bytes
                || existing.source_signature_scheme != desired_source_signature_scheme.clone()
                || existing.source_signature_value != desired_source_signature_value.clone()
                || existing.scan_status != "scanned",
        });

    let Some(mut persisted_file) = persist_or_reuse_scanned_media_file(
        app,
        title,
        file,
        &parsed,
        &snapshot,
        existing,
        summary,
        "failed to refresh movie media file source signature during library scan",
        "failed to insert movie media file during library scan",
    )
    .await
    else {
        return;
    };

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    if persisted_file.should_analyze {
        let analysis_outcome = match app
            .services
            .library
            .media_analyzer
            .analyze_file(PathBuf::from(&file.path))
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                warn!(
                    error = %error,
                    title_id = %title.id,
                    file_path = %file.path,
                    "movie media analysis task failed during library scan"
                );
                MediaAnalysisOutcome::Invalid(error.to_string())
            }
        };
        if library_scan_cancel_requested(cancel_token) {
            return;
        }
        let analysis_status = scanned_media_analysis_status(&analysis_outcome);
        let (_, analysis_persisted) = persist_scanned_media_analysis_outcome(
            app,
            title,
            &persisted_file.file_id,
            analysis_outcome,
        )
        .await;
        if analysis_persisted {
            emit_scanned_media_file_analyzed_event(
                app,
                title,
                &persisted_file.file_id,
                &file.path,
                analysis_status,
                Vec::new(),
            )
            .await;
        }
    }

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    let collections = match app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
    {
        Ok(c) => c,
        Err(err) => {
            warn!(
                title_id = %title.id,
                error = %err,
                "failed to list collections during movie scan"
            );
            crate::import_workflow::mark_wanted_completed(app, &title.id, None, None).await;
            if persisted_file.title_updated {
                app.emit_title_updated_activity(None, title).await;
            }
            return;
        }
    };

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    if ensure_movie_collection_for_file(app, title, file, &parsed, &collections).await {
        persisted_file.title_updated = true;
    }

    if library_scan_cancel_requested(cancel_token) {
        return;
    }

    crate::import_workflow::mark_wanted_completed(app, &title.id, None, None).await;
    if persisted_file.title_updated {
        app.emit_title_updated_activity(None, title).await;
    }
}
