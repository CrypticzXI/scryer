fn base_completed_import_result(
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    started_at: DateTime<Utc>,
) -> ImportResult {
    ImportResult {
        import_id: import_id.to_string(),
        decision: ImportDecision::Skipped,
        skip_reason: None,
        title_id: None,
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: release_evidence.release_title(None),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: Vec::new(),
        file_size_bytes: None,
        link_type: None,
        error_message: None,
        release_burned: false,
        started_at,
        completed_at: Utc::now(),
    }
}
fn facet_for_completed_download(completed: &CompletedDownload) -> Option<MediaFacet> {
    match extract_parameter(&completed.parameters, "*scryer_facet")
        .as_deref()
        .map(str::trim)
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
pub(crate) fn facet_from_tracked_label(value: Option<&str>) -> Option<MediaFacet> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("movie") => Some(MediaFacet::Movie),
        Some("series") => Some(MediaFacet::Series),
        Some("anime") => Some(MediaFacet::Anime),
        _ => None,
    }
}
// ---------------------------------------------------------------------------
// Series import: process ALL video files, link each to its episode
// ---------------------------------------------------------------------------

#[expect(
    clippy::too_many_arguments,
    reason = "series import keeps operational completion and release evidence as separate inputs"
)]
async fn import_series_download(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    video_files: &[PathBuf],
    started_at: chrono::DateTime<Utc>,
) -> AppResult<ImportResult> {
    let ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    } = resolve_import_paths(app, title).await?;
    let full_folder_path = effective_title_folder_path(&media_root, title, &folder_template, None);
    ensure_import_title_folder_available(app, title, &full_folder_path).await?;

    let quality_profile = resolve_import_quality_profile(app, title).await?;

    let nfo_enabled = app
        .resolve_nfo_write_on_import(Some(&title.library_id), &title.facet)
        .await?;
    let import_mode = crate::seeding_gate::resolve_seeding_safe_import_mode(
        app,
        Some(&title.library_id),
        &title.facet,
        Some(completed),
    )
    .await?;

    let mut imported_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut rejected_count: usize = 0;
    let mut release_burned = false;
    let mut failed_count: usize = 0;
    let mut last_error: Option<String> = None;
    let mut last_rejection_skip_reason: Option<ImportSkipReason> = None;
    let mut last_skipped_message: Option<String> = None;
    let mut last_skipped_skip_reason: Option<ImportSkipReason> = None;
    let mut imported_updates: Vec<NotificationMediaUpdate> = Vec::new();
    // Total bytes across every file this import brought in. Stays `None` until
    // at least one file reports a size, so a legacy-shaped import that knows no
    // sizes reports null rather than a misleading zero.
    let mut imported_size_bytes: Option<i64> = None;
    let mut imported_episode_ids: Vec<String> = Vec::new();
    let mut attributed_episode_ids: Vec<String> = Vec::new();
    let mut imported_link_type: Option<scryer_domain::ImportStrategy> = None;
    let expected_episode_ids =
        expected_episode_ids_for_completed_download(app, title, release_evidence).await;
    // `video_files` came from `find_video_files(dir, true)`: samples are already
    // excluded, so this is the count Sonarr's `OtherVideoFiles` rule wants.
    let video_file_count = video_files.len();
    // One release, one blocklist row — accumulated across the members and
    // written once after the loop. See [`DownloadBlocklistLedger`].
    let mut blocklist_ledger = DownloadBlocklistLedger::for_download(release_evidence);

    for source_video in video_files {
        match import_single_episode_file(
            app,
            actor,
            title,
            import_id,
            rename_enabled,
            &rename_template,
            &season_folder_template,
            &specials_folder_template,
            &full_folder_path,
            completed,
            release_evidence,
            source_video,
            &quality_profile,
            nfo_enabled,
            expected_episode_ids.as_ref(),
            video_file_count,
            &mut blocklist_ledger,
        )
        .await
        {
            Ok(EpisodeImportOutcome::Imported {
                dest_path,
                episode_ids,
                link_type,
                size_bytes,
                ..
            }) => {
                imported_count += 1;
                if let Some(size_bytes) = size_bytes {
                    imported_size_bytes =
                        Some(imported_size_bytes.unwrap_or(0).saturating_add(size_bytes));
                }
                imported_updates.push(NotificationMediaUpdate::created(dest_path));
                append_unique_episode_ids(&mut imported_episode_ids, &episode_ids);
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                if link_type == Some(scryer_domain::ImportStrategy::Move) {
                    imported_link_type = link_type;
                }
            }
            Ok(EpisodeImportOutcome::Skipped {
                message,
                skip_reason,
                episode_ids,
                ..
            }) => {
                skipped_count += 1;
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_skipped_message = Some(message);
                last_skipped_skip_reason = skip_reason;
            }
            Ok(EpisodeImportOutcome::Rejected {
                rejection,
                disposition,
                episode_ids,
                ..
            }) => {
                rejected_count += 1;
                release_burned |= matches!(
                    disposition,
                    crate::import_decide::RejectionDisposition::Blocklist
                );
                append_unique_episode_ids(&mut attributed_episode_ids, &episode_ids);
                last_error = Some(rejection.message.clone());
                last_rejection_skip_reason = rejection.skip_reason.clone();
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    file = %source_video.display(),
                    title = %title.name,
                    "failed to import episode file"
                );
                last_error = Some(err.to_string());
                failed_count += 1;
            }
        }
    }

    blocklist_ledger.finalize(app, actor, title).await;

    if imported_count > 0 {
        persist_title_folder_path_if_missing(app, title, &full_folder_path).await?;
        write_series_sidecars(app, title, &full_folder_path, nfo_enabled).await;
    }

    let move_import_has_failure =
        import_mode == scryer_domain::ImportMode::Move && failed_count > 0;
    let (decision, status, skip_reason) = if move_import_has_failure {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if imported_count > 0 {
        (ImportDecision::Imported, ImportStatus::Completed, None)
    } else if failed_count > 0 {
        (ImportDecision::Failed, ImportStatus::Failed, None)
    } else if rejected_count > 0 {
        (
            ImportDecision::Rejected,
            ImportStatus::Failed,
            last_rejection_skip_reason,
        )
    } else {
        // All files skipped (no parseable episode info, already imported, etc.)
        // — this is a permanent condition, not worth retrying.
        (
            ImportDecision::Skipped,
            ImportStatus::Skipped,
            last_skipped_skip_reason,
        )
    };
    let release_burned = matches!(&decision, ImportDecision::Rejected) && release_burned;

    let error_message = if imported_count == 0
        && failed_count == 0
        && rejected_count == 0
        && skipped_count > 0
    {
        last_skipped_message
    } else if failed_count > 0 || skipped_count > 0 || rejected_count > 0 {
        Some(format!(
            "{imported_count} imported, {skipped_count} skipped, {rejected_count} rejected, {failed_count} failed{}",
            last_error
                .as_ref()
                .map(|e| format!(". Last error: {e}"))
                .unwrap_or_default()
        ))
    } else {
        None
    };

    let result = ImportResult {
        import_id: import_id.to_string(),
        decision,
        skip_reason,
        title_id: Some(title.id.clone()),
        source_system: Some(completed.client_type.clone()),
        source_ref: Some(completed.download_client_item_id.clone()),
        source_title: release_evidence.release_title(None),
        source_path: completed.dest_dir.clone(),
        dest_path: None,
        quality: None,
        episode_ids: attributed_episode_ids,
        file_size_bytes: None,
        link_type: imported_link_type,
        error_message,
        release_burned,
        started_at,
        completed_at: Utc::now(),
    };
    let result_json = serde_json::to_string(&result).ok();
    let status = completed_import_status_for_result(&result, status);
    app.update_import_status_and_notify(import_id, status, result_json)
        .await?;

    if imported_count > 0 && !move_import_has_failure {
        app.append_domain_event(new_title_domain_event(
            actor,
            title,
            DomainEventPayload::ImportCompleted(ImportCompletedEventData {
                title: title_context_snapshot(title),
                media_updates: imported_updates
                    .into_iter()
                    .map(|update| created_media_update(update.path))
                    .collect(),
                imported_count: imported_count as i32,
                import_id: Some(import_id.to_string()),
                source_system: Some(completed.client_type.clone()),
                source_ref: Some(completed.download_client_item_id.clone()),
                source_title: release_evidence.release_title(None),
                source_path: Some(completed.dest_dir.clone()),
                dest_path: None,
                quality: None,
                episode_ids: imported_episode_ids,
                size_bytes: imported_size_bytes,
            }),
        ))
        .await?;
    }

    Ok(result)
}
enum EpisodeImportOutcome {
    Imported {
        dest_path: String,
        episode_ids: Vec<String>,
        imported_media_file_id: Option<String>,
        reason_code: Option<String>,
        link_type: Option<scryer_domain::ImportStrategy>,
        source_cleanup: Option<Box<scryer_domain::ImportSourceCleanupGuard>>,
        /// Bytes written for this file, so multi-file imports can report a
        /// total without re-stating the destination paths.
        size_bytes: Option<i64>,
        /// The file was imported *and* its release must be burned (D2: an
        /// honest 720p fills an empty scope, but must never come back as an
        /// "upgrade" to the 1080p it advertised).
        ///
        /// Reported rather than carried out, because the write is deduplicated
        /// per download: twelve members of a pack that all trip this must not
        /// write twelve identical blocklist rows. See [`DownloadBlocklistLedger`].
        blocklist_after_import: Option<crate::import_decide::BlocklistDirective>,
    },
    Skipped {
        message: String,
        reason_code: Option<String>,
        skip_reason: Option<ImportSkipReason>,
        episode_ids: Vec<String>,
    },
    Rejected {
        rejection: crate::post_download_gate::ImportedFileRejection,
        /// What the refusal costs the release (D17). Replaces a
        /// `finalize_before_import: bool` that could only say "blocklist and
        /// reopen" or "do nothing", and so had no way to express a hold — the
        /// third case the import gate genuinely produces.
        disposition: crate::import_decide::RejectionDisposition,
        reason_code: Option<String>,
        episode_ids: Vec<String>,
    },
}

/// One blocklist row per completed download, not one per member file.
///
/// A twelve-file season pack that trips a truth verdict used to write twelve
/// identical blocklist entries, each attributed to the one episode its member
/// happened to cover — so the operator saw the same release burned a dozen
/// times and no single row said which season it was. The release is the unit
/// being blocklisted, and a download carries exactly one, so the write is
/// deferred to the end of the file loop and attributed to the *union* of the
/// members' episode ids plus the download's collection scope (review m9).
///
/// The release attempt and the `ImportRejected` domain event ride along with the
/// blocklist for the same reason: one download, one recorded failure.
#[derive(Default)]
pub(super) struct DownloadBlocklistLedger {
    release_title: Option<String>,
    source_path: Option<PathBuf>,
    /// Set by the first member that was *refused*. A refusal outranks an
    /// imported-but-mis-advertised member: it carries the recycle reason and it
    /// is what reopens the scopes.
    rejection: Option<crate::post_download_gate::ImportedFileRejection>,
    /// Reason text from an imported-and-blocklisted member, used only when no
    /// member was refused outright.
    import_reason: Option<String>,
    episode_ids: Vec<String>,
    /// The members whose import was *refused* — the only scopes a reopen may
    /// touch. `episode_ids` above is the union the blocklist row is attributed
    /// to; a member that imported mis-advertised is in that union but has already
    /// been marked completed and must not be flipped back to `wanted`.
    rejected_episode_ids: Vec<String>,
    collection_id: Option<String>,
}

impl DownloadBlocklistLedger {
    fn for_download(release_evidence: &ReleaseEvidence) -> Self {
        let collection_id = match release_evidence.scope() {
            Some(SubmissionScope::Collection { collection_id }) => Some(collection_id.clone()),
            _ => None,
        };
        Self {
            collection_id,
            ..Self::default()
        }
    }

    fn note_release(&mut self, release_title: &str, source_path: &Path, episode_ids: &[String]) {
        if self.release_title.is_none() {
            self.release_title = Some(release_title.to_string());
            self.source_path = Some(source_path.to_path_buf());
        }
        append_unique_episode_ids(&mut self.episode_ids, episode_ids);
    }

    fn record_rejection(
        &mut self,
        release_title: &str,
        source_path: &Path,
        episode_ids: &[String],
        rejection: &crate::post_download_gate::ImportedFileRejection,
    ) {
        self.note_release(release_title, source_path, episode_ids);
        append_unique_episode_ids(&mut self.rejected_episode_ids, episode_ids);
        if self.rejection.is_none() {
            self.rejection = Some(crate::post_download_gate::ImportedFileRejection {
                message: rejection.message.clone(),
                recycle_reason: rejection.recycle_reason,
                skip_reason: rejection.skip_reason.clone(),
                blocking_rule_codes: rejection.blocking_rule_codes.clone(),
            });
        }
    }

    fn record_import_blocklist(
        &mut self,
        release_title: &str,
        source_path: &Path,
        episode_ids: &[String],
        reason: String,
    ) {
        self.note_release(release_title, source_path, episode_ids);
        if self.import_reason.is_none() {
            self.import_reason = Some(reason);
        }
    }

    /// The one write this download earned, or `None` if it earned none.
    ///
    /// Separated from carrying it out so the accumulation — one release, one
    /// row, the union of the members' episodes — is testable without an app.
    fn planned_write(&self) -> Option<PlannedBlocklistWrite<'_>> {
        let release_title = self.release_title.as_deref()?;
        let source_path = self.source_path.as_deref()?;
        Some(PlannedBlocklistWrite {
            release_title,
            source_path,
            rejection: self.rejection.as_ref(),
            import_reason: self.import_reason.as_deref(),
            attribution: crate::post_download_gate::BlocklistAttribution {
                episode_ids: &self.episode_ids,
                collection_id: self.collection_id.as_deref(),
                series_movie_link_id: None,
            },
            reopen_episode_ids: &self.rejected_episode_ids,
        })
    }

    /// Write the single entry this download earned, if any.
    async fn finalize(self, app: &AppUseCase, actor: &User, title: &scryer_domain::Title) {
        let Some(write) = self.planned_write() else {
            return;
        };
        if let Some(rejection) = write.rejection {
            crate::post_download_gate::reject_source_file_before_import(
                app,
                crate::domain_events::DomainEventActor::from(actor),
                title,
                write.release_title,
                write.source_path,
                write.attribution,
                // Reopen only the refused members; the row is attributed to
                // the whole union. Empty cannot happen for a recorded
                // rejection, but `None` keeps the default path if it did.
                (!write.reopen_episode_ids.is_empty()).then_some(write.reopen_episode_ids),
                rejection,
            )
            .await;
            return;
        }
        if let Some(reason) = write.import_reason {
            crate::post_download_gate::blocklist_release_for_title(
                app,
                title,
                write.release_title,
                Some(reason.to_string()),
                write.attribution,
            )
            .await;
        }
    }
}

/// The single blocklist write a download earned, resolved but not yet performed.
pub(super) struct PlannedBlocklistWrite<'a> {
    pub release_title: &'a str,
    pub source_path: &'a Path,
    /// `Some` when a member was refused: the write recycles, reopens and
    /// blocklists. `None` with an `import_reason` means every member imported
    /// and one of them was mis-advertised — blocklist only.
    pub rejection: Option<&'a crate::post_download_gate::ImportedFileRejection>,
    pub import_reason: Option<&'a str>,
    pub attribution: crate::post_download_gate::BlocklistAttribution<'a>,
    /// The refused members — the only scopes the write may reopen. The
    /// attribution above is the union of every member the download covered, so
    /// the blocklist row names the whole release, but a member that imported
    /// mis-advertised has already been marked completed and must not be
    /// flipped back to `wanted` with a file on disk.
    pub reopen_episode_ids: &'a [String],
}

/// A skipped episode file whose destination already holds the identical file
/// (`check_not_already_imported`) is not a rejection: the unit is in place, so
/// the automatic and manual paths both record it as `already_present` and let
/// the download finalize as imported instead of retrying forever.
pub(super) fn episode_skip_is_already_present(
    reason_code: Option<&str>,
    skip_reason: Option<&ImportSkipReason>,
) -> bool {
    reason_code == Some("duplicate_file")
        || matches!(
            skip_reason,
            Some(ImportSkipReason::AlreadyImported | ImportSkipReason::DuplicateFile)
        )
}

fn append_unique_episode_ids(target: &mut Vec<String>, source: &[String]) {
    for episode_id in source {
        if !target.contains(episode_id) {
            target.push(episode_id.clone());
        }
    }
}
async fn expected_episode_ids_for_completed_download(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_evidence: &ReleaseEvidence,
) -> Option<HashSet<String>> {
    if let Some(scope) = release_evidence.scope()
        && let Some(ids) = expected_episode_ids_from_submission_scope(app, title, scope).await
        && !ids.is_empty()
    {
        return Some(ids);
    }
    let release_title = release_evidence.release_title(None)?;
    expected_episode_ids_from_release_title(app, title, &release_title).await
}
/// The episodes a grab's submission scope names outright — the one derivation
/// both the import's grabbed-release gate and the post-import verification use.
/// A collection (season) scope expects its monitored episodes, or every episode
/// when none is monitored; title/series-movie/orphan scopes name none.
pub(crate) async fn expected_episode_ids_from_submission_scope(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    scope: &SubmissionScope,
) -> Option<HashSet<String>> {
    match scope {
        SubmissionScope::Episode { episode_id } => Some(HashSet::from([episode_id.clone()])),
        SubmissionScope::EpisodeSet { episode_ids } => Some(episode_ids.iter().cloned().collect()),
        SubmissionScope::Collection { collection_id } => {
            match episode_ids_for_collection(app, title, collection_id, true).await {
                Some(monitored) => Some(monitored),
                None => episode_ids_for_collection(app, title, collection_id, false).await,
            }
        }
        SubmissionScope::Title | SubmissionScope::SeriesMovie { .. } | SubmissionScope::Orphan => {
            None
        }
    }
}
async fn expected_episode_ids_from_release_title(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    release_title: &str,
) -> Option<HashSet<String>> {
    let parsed = normalize_release_title_signal(parse_release_metadata(release_title));
    let ep_meta = parsed.episode.as_ref()?;
    let season = ep_meta.season.unwrap_or(1).to_string();
    let mut episodes = resolve_target_episodes(app, title, ep_meta, &season).await;

    if ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack {
        let monitored: Vec<_> = episodes
            .iter()
            .filter(|episode| episode.monitored)
            .map(|episode| episode.id.clone())
            .collect();
        if !monitored.is_empty() {
            return Some(monitored.into_iter().collect());
        }
    }

    if episodes.is_empty() {
        None
    } else {
        Some(episodes.drain(..).map(|episode| episode.id).collect())
    }
}
fn resolved_episode_ids_are_within_expected(
    target_episode_ids: &[String],
    expected_episode_ids: &HashSet<String>,
) -> bool {
    // An unresolved file binds to nothing, so it is never "within" the grabbed
    // release; the caller rejects that case first with a more precise reason.
    !target_episode_ids.is_empty()
        && target_episode_ids
            .iter()
            .all(|episode_id| expected_episode_ids.contains(episode_id))
}
async fn episode_ids_for_collection(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    collection_id: &str,
    monitored_only: bool,
) -> Option<HashSet<String>> {
    match app
        .services
        .catalog
        .shows
        .list_episodes_for_collection(collection_id)
        .await
    {
        Ok(episodes) => {
            let ids: HashSet<String> = episodes
                .into_iter()
                .filter(|episode| episode.title_id == title.id)
                .filter(|episode| !monitored_only || episode.monitored)
                .map(|episode| episode.id)
                .collect();
            (!ids.is_empty()).then_some(ids)
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                collection_id,
                title_id = %title.id,
                "failed to resolve expected grabbed-release episode set"
            );
            None
        }
    }
}
async fn cleanup_superseded_episode_incumbents(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    superseded: &[crate::EpisodeScopedMediaFile],
    replacement_file_id: &str,
    replacement_path: &Path,
) {
    for incumbent in superseded {
        let mut recycle_result = None;
        let old_path =
            crate::stored_paths::stored_path_to_path_buf(&incumbent.media_file.file_path);
        if old_path.exists() {
            let old_file_recycle_context = match crate::upgrade::resolve_old_file_recycle_context(
                app,
                title,
                &incumbent.media_file,
            )
            .await
            {
                Ok(context) => context,
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        path = %old_path.display(),
                        file_id = %incumbent.media_file.id,
                        "failed to resolve recycle context for superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                    );
                    continue;
                }
            };
            let metadata = crate::recycle_bin::ReplacedMediaRecycleMetadata {
                original_path: &incumbent.media_file.file_path,
                original_file_id: &incumbent.media_file.id,
                size_bytes: incumbent.media_file.size_bytes as u64,
                title_id: &title.id,
                media_root: Some(old_file_recycle_context.media_root.as_str()),
            };

            match crate::recycle_bin::recycle_replaced_media_file(
                &old_file_recycle_context.recycle_config,
                &old_path,
                metadata,
                true,
            )
            .await
            {
                Ok(result) => recycle_result = result,
                Err(error) => {
                    // Physical cleanup failed or was refused for safety. The file is
                    // still on disk, so keep its database record rather than orphaning
                    // the file; a later upgrade can retry cleanup.
                    tracing::warn!(
                        error = %error,
                        path = %old_path.display(),
                        file_id = %incumbent.media_file.id,
                        "failed to recycle superseded episode incumbent; keeping its database record to avoid orphaning the on-disk file"
                    );
                    continue;
                }
            }
        }

        if let Err(error) = app
            .append_domain_event(new_title_domain_event(
                None,
                title,
                DomainEventPayload::MediaFileDeleted(scryer_domain::MediaFileDeletedEventData {
                    title: title_context_snapshot(title),
                    media_updates: vec![deleted_media_update(
                        incumbent.media_file.file_path.clone(),
                    )],
                    file_id: Some(incumbent.media_file.id.clone()),
                    reason: scryer_domain::MediaFileDeletedReason::UpgradeCleanup,
                    episode_ids: incumbent.episode_ids.clone(),
                }),
            ))
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "failed to emit superseded episode cleanup event"
            );
        }

        let deleted_record = match app
            .delete_media_file_record_with_dependents(&incumbent.media_file.id)
            .await
        {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    file_id = %incumbent.media_file.id,
                    "failed to delete superseded episode media file record"
                );
                false
            }
        };

        if deleted_record
            && let Err(error) = crate::recycle_bin::commit_recycle_entry(
                &recycle_result,
                replacement_file_id,
                replacement_path,
            )
            .await
        {
            tracing::warn!(
                error = %error,
                file_id = %incumbent.media_file.id,
                "superseded recycle entry could not be committed; it will not auto-purge"
            );
        }
    }
}
/// Why an obfuscated video file's episode identity could not be trusted, as an
/// actionable operator message; `None` when the file name carries usable
/// release signal of its own (the generic message then applies).
fn ambiguous_obfuscated_episode_message(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    video_file_count: usize,
) -> Option<String> {
    let file_info = parsed_release_from_file_stem(source_video);
    if has_usable_release_title_signal(&file_info) {
        return None;
    }

    if video_file_count > 1 {
        // With other video files in the download each member must identify
        // itself (`build_augmented_episode_import_metadata_for_title`); the
        // release name's numbering was never applied to this file.
        return Some(format!(
            "Automatic import could not identify the episode for this file: this download contains {video_file_count} video files and this file's name is obfuscated. Open Manual Import and assign the correct season and episode."
        ));
    }

    let release_title = release_evidence.release_title(Some(source_video))?;
    let release_info = normalize_release_title_signal(parse_release_metadata(&release_title));
    let episode = release_info.episode.as_ref()?;
    if episode.season.is_some() {
        return None;
    }
    let episode_number = episode
        .episode_numbers
        .first()
        .copied()
        .or(episode.absolute_episode)
        .or_else(|| episode.absolute_episode_numbers.first().copied())?;

    Some(format!(
        "Automatic import could not choose a season for episode {episode_number}: the release name does not include a season and the downloaded filename is obfuscated. Open Manual Import and assign the correct season and episode."
    ))
}
/// Import a single episode video file: parse, gate, import, and link.
#[expect(
    clippy::too_many_arguments,
    reason = "single-episode imports need the full source, rename, and persistence context together"
)]
async fn import_single_episode_file(
    app: &AppUseCase,
    actor: &User,
    title: &scryer_domain::Title,
    import_id: &str,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    title_folder_path: &Path,
    completed: &CompletedDownload,
    release_evidence: &ReleaseEvidence,
    source_video: &Path,
    quality_profile: &crate::QualityProfile,
    nfo_enabled: bool,
    expected_episode_ids: Option<&HashSet<String>>,
    video_file_count: usize,
    blocklist_ledger: &mut DownloadBlocklistLedger,
) -> AppResult<EpisodeImportOutcome> {
    // Sonarr's `OtherVideoFiles`: with more than one (non-sample) video in the
    // download, each file must identify itself.
    let other_video_files = video_file_count > 1;
    let parsed = build_augmented_episode_import_metadata_for_title(
        source_video,
        release_evidence,
        title,
        other_video_files,
    );

    // Must have episode info to proceed
    let ep_meta = match parsed.episode.as_ref() {
        Some(ep) if !ep.episode_numbers.is_empty() => ep,
        Some(ep)
            if ep.absolute_episode.is_some() && title.facet == scryer_domain::MediaFacet::Anime =>
        {
            ep
        }
        Some(ep) if ep.air_date.is_some() => ep,
        Some(ep) if ep.release_type == crate::ParsedEpisodeReleaseType::SeasonPack => ep,
        _ => {
            tracing::debug!(
                file = %source_video.display(),
                other_video_files,
                "skipping file with no parseable episode info"
            );
            return Ok(EpisodeImportOutcome::Skipped {
                message: ambiguous_obfuscated_episode_message(
                    source_video,
                    release_evidence,
                    video_file_count,
                )
                .unwrap_or_else(|| {
                    "Automatic import could not determine a season and episode from the downloaded file. Open Manual Import and assign the correct season and episode."
                        .to_string()
                }),
                reason_code: None,
                skip_reason: Some(ImportSkipReason::UnparseableEpisode),
                episode_ids: Vec::new(),
            });
        }
    };

    let season = ep_meta.season.unwrap_or(1);
    let season_str = season.to_string();

    // Resolve target episodes early so we can enrich rename tokens with DB
    // metadata (e.g. absolute_number from TVDB).
    let target_episodes = resolve_target_episodes(app, title, ep_meta, &season_str).await;
    let target_episode_ids: Vec<String> = target_episodes
        .iter()
        .map(|episode| episode.id.clone())
        .collect();
    // Fail closed: a parseable episodic file that binds to no episode of this
    // title is not part of what was grabbed and must never reach the library.
    // Ordered ahead of the grabbed-release scope check so the reported reason
    // names the missing episode instead of the broader scope violation, and
    // returned before any destination rendering, scoring, media-file insertion,
    // or source cleanup can run.
    if target_episodes.is_empty() {
        // The early return skips the shared outcome handling below, so record
        // the rejected artifact here: the file must still be visible in the
        // import results even though nothing was transferred.
        persist_file_import_artifact(
            app,
            import_id,
            completed,
            title.id.as_str(),
            source_video,
            "episode",
            "rejected",
            Some("episode_not_found_for_title"),
            None,
            &target_episodes,
        )
        .await?;
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: "file resolves to no episode of this title".to_string(),
                recycle_reason: "episode_not_found_for_title",
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: vec!["episode_not_found_for_title".to_string()],
            },
            // The file stays in the completed-download directory: leaving the
            // rest of the pack importable is Sonarr-compatible, and burning the
            // release for one stray file would be wrong. An operator decides
            // through Manual Import, so this is a hold rather than a skip.
            disposition: crate::import_decide::RejectionDisposition::Hold,
            reason_code: Some("episode_not_found_for_title".to_string()),
            episode_ids: Vec::new(),
        });
    }
    if let Some(expected_episode_ids) = expected_episode_ids
        && !resolved_episode_ids_are_within_expected(&target_episode_ids, expected_episode_ids)
    {
        // The obfuscation explainer describes a season guessed from the
        // release name; a file in a multi-video download identified itself,
        // so it simply resolved outside the grabbed release.
        let obfuscated_message = if other_video_files {
            None
        } else {
            ambiguous_obfuscated_episode_message(source_video, release_evidence, video_file_count)
        };
        return Ok(EpisodeImportOutcome::Rejected {
            rejection: crate::post_download_gate::ImportedFileRejection {
                message: obfuscated_message.unwrap_or_else(|| {
                    "Automatic import resolved the downloaded file to episode(s) outside the grabbed release. Open Manual Import and assign the correct season and episode."
                        .to_string()
                }),
                recycle_reason: "episode_outside_grabbed_release",
                skip_reason: Some(ImportSkipReason::PolicyMismatch),
                blocking_rule_codes: vec!["episode_outside_grabbed_release".to_string()],
            },
            disposition: crate::import_decide::RejectionDisposition::Hold,
            reason_code: Some("episode_outside_grabbed_release".to_string()),
            episode_ids: target_episode_ids.clone(),
        });
    }
    let ep_num_str = episode_number_token_for_import(
        &ep_meta.episode_numbers,
        target_episodes
            .first()
            .and_then(|episode| episode.episode_number.as_deref()),
    );
    let abs_str = ep_meta.absolute_episode.map(|n| n.to_string()).or_else(|| {
        target_episodes
            .first()
            .and_then(|ep| ep.absolute_number.clone())
    });
    let episode_title = target_episodes.first().and_then(|ep| ep.title.as_deref());
    let import_purpose = release_evidence.purpose();
    let origin = release_evidence.import_origin();
    let additional_import = import_purpose.is_additional_file();
    let runtime_sample_mode = if import_purpose.is_manual_replacement() {
        crate::post_download_gate::RuntimeSampleValidationMode::BypassRuntimeSampleCheck
    } else {
        crate::post_download_gate::RuntimeSampleValidationMode::EnforceAutomatic
    };
    let outcome = execute_resolved_episode_import(
        app,
        actor,
        title,
        import_id,
        Some(completed),
        rename_enabled,
        rename_template,
        season_folder_template,
        specials_folder_template,
        title_folder_path,
        source_video,
        &parsed,
        &target_episodes,
        &target_episodes,
        season as u32,
        &ep_num_str,
        abs_str.as_deref(),
        episode_title,
        quality_profile,
        None,
        runtime_sample_mode,
        origin,
        release_evidence.announced_size_bytes(),
        additional_import,
    )
    .await?;

    match &outcome {
        EpisodeImportOutcome::Imported {
            dest_path,
            imported_media_file_id,
            reason_code,
            blocklist_after_import,
            source_cleanup,
            ..
        } => {
            // Imported, but the release lied about its quality: burn it so the
            // next upgrade search cannot re-grab the same lie. Recorded, not
            // written — one row per download, not one per member.
            if let Some(directive) = blocklist_after_import {
                tracing::info!(
                    title_id = %title.id,
                    code = directive.code,
                    "{}",
                    directive.reason
                );
                blocklist_ledger.record_import_blocklist(
                    &release_evidence
                        .release_title(Some(source_video))
                        .unwrap_or_default(),
                    source_video,
                    &target_episode_ids,
                    directive.reason.clone(),
                );
            }
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "imported",
                reason_code.as_deref(),
                imported_media_file_id.as_deref(),
                &target_episodes,
            )
            .await?;

            finalize_deferred_import_source_cleanup(
                app,
                source_cleanup.as_deref().cloned(),
                &crate::stored_paths::stored_path_to_path_buf(dest_path),
                Some(completed),
            )
            .await?;

            if imported_media_file_id.is_some() && reason_code.as_deref() != Some("additional_file")
            {
                if nfo_enabled {
                    let nfo_path = std::path::Path::new(dest_path).with_extension("nfo");
                    if let Some(episode) = target_episodes.first() {
                        let nfo_content = render_episode_nfo(title, episode);
                        if let Err(err) = tokio::fs::write(&nfo_path, nfo_content.as_bytes()).await
                        {
                            tracing::warn!(
                                error = %err,
                                path = %nfo_path.display(),
                                "failed to write episode NFO sidecar"
                            );
                        }
                    }
                }

                spawn_post_processing(PostProcessingContext {
                    app: app.clone(),
                    actor: crate::domain_events::DomainEventActor::from(actor),
                    title_id: title.id.clone(),
                    title_name: title.name.clone(),
                    facet: title.facet.clone(),
                    dest_path: PathBuf::from(dest_path),
                    year: title.year,
                    imdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "imdb")
                        .map(|e| e.value.clone()),
                    tvdb_id: title
                        .external_ids
                        .iter()
                        .find(|e| e.source == "tvdb")
                        .map(|e| e.value.clone()),
                    season: Some(season),
                    episode: ep_meta.episode_numbers.first().copied(),
                    quality: parsed.quality.clone(),
                });
            }
        }
        EpisodeImportOutcome::Skipped {
            reason_code,
            skip_reason,
            ..
        } => {
            let artifact_result =
                if episode_skip_is_already_present(reason_code.as_deref(), skip_reason.as_ref()) {
                    "already_present"
                } else {
                    "rejected"
                };
            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                artifact_result,
                reason_code.as_deref(),
                None,
                &target_episodes,
            )
            .await?;
        }
        EpisodeImportOutcome::Rejected {
            rejection,
            disposition,
            reason_code,
            ..
        } => {
            // Only a release that provably lied is burned, and only once per
            // download. `Skip` and `Hold` record the decision and stop: the
            // download sits in `ImportBlocked` for the operator either way, and
            // reopening a scope whose refusal will repeat is pure churn (D17).
            if matches!(
                disposition,
                crate::import_decide::RejectionDisposition::Blocklist
            ) {
                let source_title = release_evidence
                    .release_title(Some(source_video))
                    .unwrap_or_default();
                blocklist_ledger.record_rejection(
                    &source_title,
                    source_video,
                    &target_episode_ids,
                    rejection,
                );
            }

            persist_file_import_artifact(
                app,
                import_id,
                completed,
                title.id.as_str(),
                source_video,
                "episode",
                "rejected",
                reason_code
                    .as_deref()
                    .or_else(|| rejection.skip_reason.as_ref().map(ImportSkipReason::as_str)),
                None,
                &target_episodes,
            )
            .await?;
        }
    }

    Ok(outcome)
}

fn episode_number_token_for_import(
    parsed_episode_numbers: &[u32],
    resolved_episode_number: Option<&str>,
) -> String {
    parsed_episode_numbers
        .first()
        .map(ToString::to_string)
        .or_else(|| {
            resolved_episode_number
                .map(str::trim)
                .filter(|number| !number.is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod episode_number_token_for_import_tests {
    use super::*;

    #[test]
    fn parsed_regular_episode_number_takes_precedence() {
        assert_eq!(episode_number_token_for_import(&[7], Some("1")), "7");
    }

    #[test]
    fn resolved_episode_number_fills_an_absolute_only_parse() {
        assert_eq!(episode_number_token_for_import(&[], Some("1")), "1");
    }

    #[test]
    fn episode_number_token_stays_empty_without_a_regular_number() {
        assert_eq!(episode_number_token_for_import(&[], Some("  ")), "");
        assert_eq!(episode_number_token_for_import(&[], None), "");
    }

    #[test]
    fn resolved_episode_number_renders_a_padded_destination_token() {
        let episode = episode_number_token_for_import(&[], Some("1"));
        let tokens = BTreeMap::from([
            ("season".to_string(), "1".to_string()),
            ("episode".to_string(), episode),
        ]);

        assert_eq!(render_rename_template("S{season:2}E{episode:2}", &tokens), "S01E01");
    }
}
/// Resolve media root path and rename template for a title's facet.
pub(crate) async fn resolve_import_paths(
    app: &AppUseCase,
    title: &scryer_domain::Title,
) -> AppResult<ImportPathSettings> {
    let media_root = app.title_root_folder_path_override(title).await?;

    let rename_enabled = app.resolve_rename_enabled(&title.facet).await?;
    let rename_template = app.resolve_rename_template(&title.facet).await?;
    let folder_template = app
        .read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?;
    let default_folder_template = match title.facet {
        MediaFacet::Movie => super::DEFAULT_FOLDER_TEMPLATE_MOVIE,
        MediaFacet::Series => super::DEFAULT_FOLDER_TEMPLATE_SERIES,
        MediaFacet::Anime => super::DEFAULT_FOLDER_TEMPLATE_ANIME,
    };
    let folder_template = crate::normalize_title_folder_template_or_default(
        folder_template,
        default_folder_template,
        title.facet.as_str(),
    );
    let season_folder_template = crate::normalize_season_folder_template_or_default(
        app.read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::SEASON_FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?,
    );
    let specials_folder_template = crate::normalize_specials_folder_template_or_default(
        app.read_setting_string_value_for_scope(
            super::SETTINGS_SCOPE_SYSTEM,
            super::SPECIALS_FOLDER_TEMPLATE_KEY,
            Some(title.facet.as_str()),
        )
        .await?,
    );

    Ok(ImportPathSettings {
        media_root,
        rename_enabled,
        rename_template,
        folder_template,
        season_folder_template,
        specials_folder_template,
    })
}

/// Compute the parent directory for an episode import: the season or specials
/// folder beneath the title folder, or the title folder itself when the library
/// is not configured to use season folders.
pub(crate) fn episodic_import_parent_path(
    title: &scryer_domain::Title,
    use_season_folders: bool,
    title_folder_path: &Path,
    season_folder_template: &str,
    specials_folder_template: &str,
    season_num: u32,
) -> PathBuf {
    if use_season_folders {
        let season_folder = crate::render_episode_folder_name(
            title,
            season_num,
            season_folder_template,
            specials_folder_template,
        );
        title_folder_path.join(season_folder)
    } else {
        title_folder_path.to_path_buf()
    }
}

/// Return the explicit season-folder title override encoded in legacy tags.
/// The application resolver combines this value with library and facet settings.
pub(crate) fn season_folder_tag_override(title: &scryer_domain::Title) -> Option<bool> {
    title
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix("scryer:season-folder:"))
        .map(|value| !value.trim().eq_ignore_ascii_case("disabled"))
}

/// Legacy title-tag interpretation retained for focused tag parsing tests.
/// Runtime import, scan, and rename paths use `AppUseCase::resolve_use_season_folders`.
#[cfg(test)]
pub(crate) fn use_season_folders(title: &scryer_domain::Title) -> bool {
    season_folder_tag_override(title).unwrap_or(true)
}

/// Compute the destination path for an episode import using the canonical
/// token set: base tokens from parsed release metadata, overridden by the
/// explicit episode values supplied by the caller.
///
/// `ep_num_str` may be empty to leave `{episode}` blank (anime absolute-only
/// files where no per-season episode number is known).
/// `quality_override` replaces the filename-parsed quality token when the
/// caller supplies an explicit label (e.g. manual import).
#[expect(
    clippy::too_many_arguments,
    reason = "episode rename rendering uses the full canonical token set explicitly"
)]
pub(crate) fn episode_import_dest_path(
    title: &scryer_domain::Title,
    use_season_folders: bool,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
    source_path: &Path,
    title_folder_path: &Path,
    rename_enabled: bool,
    rename_template: &str,
    season_folder_template: &str,
    specials_folder_template: &str,
    season_num: u32,
    ep_num_str: &str,
    absolute_number: Option<&str>,
    episode_title: Option<&str>,
    quality_override: Option<&str>,
) -> PathBuf {
    let mut tokens = build_rename_tokens(title, parsed, ext);
    tokens.insert("season".to_string(), season_num.to_string());
    tokens.insert("season_order".to_string(), season_num.to_string());
    tokens.insert("episode".to_string(), ep_num_str.to_string());
    tokens.insert(
        "absolute_episode".to_string(),
        absolute_number.unwrap_or("").to_string(),
    );
    tokens.insert(
        "episode_title".to_string(),
        episode_title.unwrap_or("").to_string(),
    );
    if let Some(q) = quality_override {
        tokens.insert("quality".to_string(), q.to_string());
    }
    let rendered = if rename_enabled {
        render_rename_template(rename_template, &tokens)
    } else {
        preserved_import_filename(source_path)
    };
    episodic_import_parent_path(
        title,
        use_season_folders,
        title_folder_path,
        season_folder_template,
        specials_folder_template,
        season_num,
    )
    .join(rendered)
}
/// Build the common rename token map from parsed release metadata.
pub(crate) fn build_rename_tokens(
    title: &scryer_domain::Title,
    parsed: &crate::ParsedReleaseMetadata,
    ext: &str,
) -> BTreeMap<String, String> {
    let mut tokens = BTreeMap::new();
    let fallback_title_year = title.year;
    let resolved_year = parsed.year.or(fallback_title_year);
    tokens.insert("title".to_string(), title.name.clone());
    tokens.insert(
        "year".to_string(),
        resolved_year.map(|y| y.to_string()).unwrap_or_default(),
    );
    tokens.insert(
        "quality".to_string(),
        parsed
            .quality
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );
    tokens.insert(
        "source".to_string(),
        parsed
            .source
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "video_codec".to_string(),
        parsed
            .video_codec
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "audio".to_string(),
        parsed
            .audio
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    tokens.insert(
        "release_group".to_string(),
        parsed.release_group.clone().unwrap_or_default(),
    );
    tokens.insert(
        "season".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.season)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.episode_numbers.first().copied())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert(
        "absolute_episode".to_string(),
        parsed
            .episode
            .as_ref()
            .and_then(|e| e.absolute_episode)
            .map(|v| v.to_string())
            .unwrap_or_default(),
    );
    tokens.insert("episode_title".to_string(), String::new());
    tokens.insert("ext".to_string(), ext.to_string());
    tokens
}
pub(crate) async fn resolve_target_episodes(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    ep_meta: &crate::ParsedEpisodeMetadata,
    season_str: &str,
) -> Vec<scryer_domain::Episode> {
    let mut episodes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let target_season = crate::parsed_episode_lookup_season(ep_meta, season_str);

    if let Some(air_date) = ep_meta.air_date {
        let air_date_str = air_date.format("%Y-%m-%d").to_string();
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                let mut matches = Vec::new();
                for collection in collections {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            matches.extend(collection_episodes.into_iter().filter(|episode| {
                                episode.title_id == title.id
                                    && episode.air_date.as_deref() == Some(air_date_str.as_str())
                            }));
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "daily episode lookup failed during import")
                        }
                    }
                }

                matches.sort_by_key(|episode| {
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(u32::MAX)
                });

                if let Some(part) = ep_meta.daily_part {
                    let part_index = part.saturating_sub(1) as usize;
                    if let Some(episode) = matches.into_iter().nth(part_index)
                        && seen.insert(episode.id.clone())
                    {
                        episodes.push(episode);
                    }
                } else {
                    for episode in matches {
                        if seen.insert(episode.id.clone()) {
                            episodes.push(episode);
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "daily collection lookup failed during import")
            }
        }
    }

    for episode_number in &ep_meta.episode_numbers {
        let episode_str = episode_number.to_string();
        match app
            .services
            .catalog
            .shows
            .find_episode_by_title_and_numbers(&title.id, &target_season, &episode_str)
            .await
        {
            Ok(Some(episode)) => {
                if seen.insert(episode.id.clone()) {
                    episodes.push(episode);
                }
            }
            Ok(None) => {
                tracing::debug!(
                    title_id = %title.id,
                    season = %season_str,
                    episode = %episode_str,
                    "no matching episode found for imported file"
                );
            }
            Err(err) => tracing::warn!(error = %err, "episode lookup failed during import"),
        }
    }

    if episodes.is_empty()
        && ep_meta.season.is_some()
        && ep_meta.episode_numbers.is_empty()
        && ep_meta.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    {
        match app
            .services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
        {
            Ok(collections) => {
                for collection in collections
                    .into_iter()
                    .filter(|collection| collection.collection_index == target_season)
                {
                    match app
                        .services
                        .catalog
                        .shows
                        .list_episodes_for_collection(&collection.id)
                        .await
                    {
                        Ok(collection_episodes) => {
                            let mut collection_episodes: Vec<_> = collection_episodes
                                .into_iter()
                                .filter(|episode| {
                                    episode.title_id == title.id
                                        && episode.season_number.as_deref()
                                            == Some(target_season.as_str())
                                })
                                .collect();
                            collection_episodes.sort_by_key(|episode| {
                                episode
                                    .episode_number
                                    .as_deref()
                                    .and_then(|value| value.parse::<u32>().ok())
                                    .unwrap_or(u32::MAX)
                            });
                            for episode in collection_episodes {
                                if seen.insert(episode.id.clone()) {
                                    episodes.push(episode);
                                }
                            }
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "season episode lookup failed during import")
                        }
                    }
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "season collection lookup failed during import")
            }
        }
    }

    if episodes.is_empty() && !ep_meta.special_absolute_episode_numbers.is_empty() {
        for special_number in &ep_meta.special_absolute_episode_numbers {
            let episode_str = special_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_numbers(&title.id, "0", &episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        special = %episode_str,
                        "no matching special episode found during import"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "special episode lookup failed during import")
                }
            }
        }
    }

    if episodes.is_empty()
        && (ep_meta.absolute_episode.is_some() || !ep_meta.absolute_episode_numbers.is_empty())
    {
        let absolute_numbers: Vec<u32> = if !ep_meta.absolute_episode_numbers.is_empty() {
            ep_meta.absolute_episode_numbers.clone()
        } else if ep_meta.episode_numbers.is_empty() {
            vec![ep_meta.absolute_episode.unwrap_or_default()]
        } else {
            ep_meta.episode_numbers.clone()
        };

        for absolute_number in absolute_numbers {
            let absolute_episode_str = absolute_number.to_string();
            match app
                .services
                .catalog
                .shows
                .find_episode_by_title_and_absolute_number(&title.id, &absolute_episode_str)
                .await
            {
                Ok(Some(episode)) => {
                    if seen.insert(episode.id.clone()) {
                        episodes.push(episode);
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        title_id = %title.id,
                        absolute = absolute_number,
                        "no matching episode found by absolute number"
                    );
                }
                Err(err) => {
                    tracing::warn!(error = %err, "episode absolute lookup failed during import")
                }
            }
        }
    }

    episodes
}
async fn write_series_sidecars(
    app: &AppUseCase,
    title: &scryer_domain::Title,
    title_folder_path: &Path,
    nfo_enabled: bool,
) {
    if nfo_enabled {
        let tvshow_nfo_path = title_folder_path.join("tvshow.nfo");
        if !tvshow_nfo_path.exists() {
            if let Some(parent) = tvshow_nfo_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let nfo_content = render_tvshow_nfo(title);
            if let Err(err) = tokio::fs::write(&tvshow_nfo_path, nfo_content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %tvshow_nfo_path.display(),
                    "failed to write tvshow NFO sidecar"
                );
            }
        }
    }

    let plexmatch_enabled = match app
        .resolve_plexmatch_write_on_import(Some(&title.library_id), &title.facet)
        .await
    {
        Ok(value) => value.unwrap_or(false),
        Err(error) => {
            tracing::warn!(
                error = %error,
                title_id = %title.id,
                "failed to resolve plexmatch sidecar setting"
            );
            false
        }
    };
    if plexmatch_enabled {
        let plexmatch_path = title_folder_path.join(".plexmatch");
        if !plexmatch_path.exists() {
            if let Some(parent) = plexmatch_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let content = render_plexmatch(title);
            if let Err(err) = tokio::fs::write(&plexmatch_path, content.as_bytes()).await {
                tracing::warn!(
                    error = %err,
                    path = %plexmatch_path.display(),
                    "failed to write .plexmatch hint file"
                );
            }
        }
    }
}
#[expect(
    clippy::too_many_arguments,
    reason = "import artifact persistence records the full import outcome for later inspection"
)]
async fn persist_file_import_artifact(
    app: &AppUseCase,
    import_id: &str,
    completed: &CompletedDownload,
    title_id: &str,
    source_path: &Path,
    media_kind: &str,
    result: &str,
    reason_code: Option<&str>,
    imported_media_file_id: Option<&str>,
    episodes: &[scryer_domain::Episode],
) -> AppResult<()> {
    let relative_path = source_path
        .strip_prefix(&completed.dest_dir)
        .ok()
        .map(path_to_stored_string)
        .filter(|path| !path.is_empty());
    let normalized_file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
        .unwrap_or_else(|| source_path.to_string_lossy().to_ascii_lowercase());

    let episode_rows: Vec<(Option<String>, Option<i32>, Option<i32>)> = if episodes.is_empty() {
        vec![(None, None, None)]
    } else {
        episodes
            .iter()
            .map(|episode| {
                (
                    Some(episode.id.clone()),
                    episode
                        .season_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                    episode
                        .episode_number
                        .as_deref()
                        .and_then(|value| value.parse().ok()),
                )
            })
            .collect()
    };

    let source_identity = DownloadSourceIdentity::for_import_artifact(
        Some(completed.client_id.as_str()),
        &completed.client_type,
        &completed.download_client_item_id,
    );
    let artifacts = episode_rows
        .into_iter()
        .map(|(episode_id, season_number, episode_number)| ImportArtifact {
            id: Id::new().0,
            source_client_id: source_identity.client_id.clone(),
            source_system: source_identity.client_type.clone(),
            source_ref: source_identity.item_id.clone(),
            import_id: Some(import_id.to_string()),
            relative_path: relative_path.clone(),
            normalized_file_name: normalized_file_name.clone(),
            media_kind: media_kind.to_string(),
            title_id: Some(title_id.to_string()),
            episode_id,
            season_number,
            episode_number,
            result: result.to_string(),
            reason_code: reason_code.map(str::to_string),
            imported_media_file_id: imported_media_file_id.map(str::to_string),
            created_at: Utc::now(),
        })
        .collect();
    if let Err(error) = app
        .services
        .workflow
        .import_artifacts
        .insert_artifacts(artifacts)
        .await
    {
        tracing::warn!(
            error = %error,
            import_id,
            source_ref = %completed.download_client_item_id,
            file = %source_path.display(),
            "failed to persist import artifacts"
        );
        return Err(AppError::ImportEvidenceUnavailable(error.to_string()));
    }
    Ok(())
}
// 50 MB

/// Name-only sample detection: the file stem contains "sample"
/// (case-insensitive). Unlike `is_sample_file` this carries no size heuristic,
/// so a legitimately small movie (short film, old cartoon, low-bitrate SD) is
/// never mistaken for a sample; the automatic movie path never size-filters,
/// and manual import must not be stricter than it.
pub(crate) fn is_sample_named_file(path: &Path) -> bool {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|stem| stem.contains("sample"))
}

pub(crate) fn is_sample_file(path: &Path) -> bool {
    if is_sample_named_file(path) {
        return true;
    }

    if scryer_domain::canonical_video_extension(path) == Some("strm") {
        return false;
    }

    // Small files in multi-episode directories are almost certainly samples/promos
    std::fs::metadata(path)
        .map(|m| m.len() < SAMPLE_SIZE_THRESHOLD)
        .unwrap_or(false)
}
fn resolve_title_from_release_candidate(
    titles: &[Title],
    candidate: &ParsedReleaseMetadata,
    facet_hint: Option<&str>,
) -> Option<Title> {
    if candidate.episode.is_some() {
        crate::import_title_resolution::resolve_monitored_episode_title_from_release(
            titles, candidate, facet_hint,
        )
        .map(|resolved| resolved.title.clone())
    } else {
        crate::import_title_resolution::resolve_monitored_movie_title_from_release(
            titles, candidate,
        )
        .map(|resolved| resolved.title.clone())
    }
}
/// Canonical import-time release metadata for an episode file: the release
/// evidence parsed with the title's canonical grab-time context (see
/// `parse_import_release_for_title`) supplies every score-bearing fact; the
/// episode identity follows Sonarr's `OtherVideoFiles` rule
/// (`AggregateEpisodes.GetBestEpisodeInfo`).
///
/// The release name's numbering is applied to a file only when that file is
/// the download's sole video and the release names concrete episodes. When
/// the download holds other video files, or the release is a season pack
/// (whole or partial — it has no episode numbers to hand out), every file must
/// identify itself from its own name; a file that cannot gets no episode at
/// all, so the caller parks it for manual import instead of guessing.
fn build_augmented_episode_import_metadata_for_title(
    source_video: &Path,
    release_evidence: &ReleaseEvidence,
    title: &scryer_domain::Title,
    other_video_files: bool,
) -> ParsedReleaseMetadata {
    let Some(release_title) = release_evidence.release_title(Some(source_video)) else {
        return ParsedReleaseMetadata::default();
    };

    let mut parsed =
        normalize_release_title_signal(parse_import_release_for_title(&release_title, title));
    // The title-anchored parse keeps score-bearing facts but drops the release
    // name's own numbering when that name does not match the title's canonical
    // identity (a user-assigned or parameter-matched download); the release
    // name's context-free numbering is still what the release claims.
    let release_episode = parsed
        .episode
        .take()
        .or_else(|| parse_release_metadata(&release_title).episode);
    let release_is_season_pack = release_episode.as_ref().is_some_and(|episode| {
        episode.full_season || episode.release_type == crate::ParsedEpisodeReleaseType::SeasonPack
    });
    parsed.episode = if other_video_files || release_is_season_pack {
        file_episode_identity_for_title(source_video, title)
    } else if let Some(scene_episode) = scene_titled_file_episode(source_video) {
        // Sonarr's `!SceneChecker.IsSceneTitle(fileName)` guard: a sole video
        // that is itself a properly named scene release (dotted, grouped,
        // quality-tagged, episode-numbered) identifies itself; the release
        // name's numbering is not applied over it. A disagreement with the
        // grabbed release then surfaces through the grabbed-release gate
        // rather than being papered over.
        Some(scene_episode)
    } else {
        // Sole video of a non-pack release: the release name is the best
        // episode evidence, and only after it the file name — which may locate
        // an episode but cannot supplement score-bearing release metadata.
        release_episode.or_else(|| file_episode_identity_for_title(source_video, title))
    };
    parsed
}

/// The episode a sole video names when its stem is a scene-style release name
/// (Sonarr `SceneChecker.IsSceneTitle`): dotted, no spaces, and a context-free
/// parse yields a release group, a quality, a title, and episode numbering.
/// Anything less (obfuscated, renamed, "episode 2.mkv") is not scene-titled
/// and does not override the release name.
fn scene_titled_file_episode(source_video: &Path) -> Option<crate::ParsedEpisodeMetadata> {
    let stem = source_video_stem(Some(source_video))?;
    if !stem.contains('.') || stem.contains(' ') {
        return None;
    }
    let parsed = normalize_release_title_signal(parse_release_metadata(&stem));
    if parsed
        .release_group
        .as_deref()
        .is_none_or(|group| group.trim().is_empty())
        || parsed.quality.is_none()
        || parsed.normalized_title.trim().is_empty()
    {
        return None;
    }
    parsed.episode
}

/// The episode a video file names on its own: its stem parsed with the
/// title's canonical context (so absolute/anime numbering resolves the way
/// the grab path resolves it), then the context-free stem parse the manual
/// preview and obfuscation checks use.
fn file_episode_identity_for_title(
    source_video: &Path,
    title: &scryer_domain::Title,
) -> Option<crate::ParsedEpisodeMetadata> {
    source_video_stem(Some(source_video))
        .and_then(|stem| parse_import_release_for_title(&stem, title).episode)
        .or_else(|| parsed_release_from_file_stem(source_video).episode)
}
