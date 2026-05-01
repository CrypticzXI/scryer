use super::*;
use async_trait::async_trait;
use scryer_domain::{
    DomainEventFilter, DomainEventPayload, DomainEventType, EventType, ImportType,
    JobRunCompletedEventData, JobRunStartedEventData, RootFolderEntry, TrackedDownloadState,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, Instant, sleep, timeout};

#[derive(Default)]
struct MockTitleRepo {
    store: Arc<Mutex<Vec<Title>>>,
    create_or_get_existing_error: Arc<Mutex<Option<String>>>,
}

impl MockTitleRepo {
    async fn fail_create_or_get_existing(&self, message: &str) {
        *self.create_or_get_existing_error.lock().await = Some(message.to_string());
    }
}

#[async_trait]
impl TitleRepository for MockTitleRepo {
    async fn list(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        let list = self.store.lock().await.clone();
        let normalized_query = query.map(|value| value.to_lowercase());
        Ok(list
            .into_iter()
            .filter(|title| {
                let facet_match = facet
                    .as_ref()
                    .is_none_or(|expected| &title.facet == expected);
                let query_match = normalized_query
                    .as_ref()
                    .is_none_or(|term| title.name.to_lowercase().contains(term));
                facet_match && query_match
            })
            .collect())
    }

    async fn list_by_external_ids(&self, source: &str, values: &[String]) -> AppResult<Vec<Title>> {
        let requested: Vec<&str> = values
            .iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .collect();
        let list = self.store.lock().await;
        let mut matches = Vec::new();
        let mut seen = HashSet::new();
        for value in requested {
            if let Some(title) = list.iter().find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            }) && seen.insert(title.id.clone())
            {
                matches.push(title.clone());
            }
        }
        Ok(matches)
    }

    async fn list_for_matching(
        &self,
        facet: Option<MediaFacet>,
        query: Option<String>,
    ) -> AppResult<Vec<Title>> {
        self.list(facet, query).await
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list.iter().find(|title| title.id == id).cloned())
    }

    async fn get_by_facet_and_slug(
        &self,
        facet: MediaFacet,
        slug: &str,
    ) -> AppResult<Option<Title>> {
        let normalized_slug = slug.trim();
        if normalized_slug.is_empty() {
            return Ok(None);
        }

        let list = self.store.lock().await;
        let matches = list
            .iter()
            .filter(|title| {
                title.facet == facet
                    && title.slug.as_deref().is_some_and(|candidate| {
                        candidate.trim().eq_ignore_ascii_case(normalized_slug)
                    })
            })
            .cloned()
            .collect::<Vec<_>>();

        match matches.as_slice() {
            [] => Ok(None),
            [title] => Ok(Some(title.clone())),
            _ => Err(AppError::Validation(
                "multiple titles found for slug lookup".into(),
            )),
        }
    }

    async fn find_by_external_id(&self, source: &str, value: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.external_ids.iter().any(|external_id| {
                    external_id.source.eq_ignore_ascii_case(source) && external_id.value == value
                })
            })
            .cloned())
    }

    async fn find_by_external_id_in_facet(
        &self,
        facet: MediaFacet,
        source: &str,
        value: &str,
    ) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list
            .iter()
            .find(|title| {
                title.facet == facet
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case(source)
                            && external_id.value == value
                    })
            })
            .cloned())
    }

    async fn create_or_get_existing(&self, title: Title) -> AppResult<CreateTitleOutcome> {
        if let Some(message) = self.create_or_get_existing_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }

        let mut list = self.store.lock().await;
        let mut matching_ids = list
            .iter()
            .filter(|existing| {
                existing.facet == title.facet
                    && existing.external_ids.iter().any(|existing_external_id| {
                        title.external_ids.iter().any(|incoming_external_id| {
                            existing_external_id
                                .source
                                .eq_ignore_ascii_case(&incoming_external_id.source)
                                && existing_external_id.value == incoming_external_id.value
                        })
                    })
            })
            .map(|existing| existing.id.clone())
            .collect::<Vec<_>>();
        matching_ids.sort();
        matching_ids.dedup();

        if matching_ids.len() > 1 {
            return Err(AppError::Validation(
                "external ids already map to multiple titles".into(),
            ));
        }

        if let Some(existing_id) = matching_ids.first()
            && let Some(existing) = list.iter().find(|entry| entry.id == *existing_id)
        {
            return Ok(CreateTitleOutcome {
                title: existing.clone(),
                reused_existing: true,
            });
        }

        list.push(title.clone());
        Ok(CreateTitleOutcome {
            title,
            reused_existing: false,
        })
    }

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.store.lock().await.push(title.clone());
        Ok(title)
    }

    async fn list_titles_due_for_hydration(
        &self,
        _limit: usize,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<PendingTitleHydration>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|title| {
                title.metadata_fetched_at.is_none()
                    && !excluded_facets.iter().any(|facet| facet == &title.facet)
                    && title.external_ids.iter().any(|external_id| {
                        external_id.source.eq_ignore_ascii_case("tvdb")
                            && !external_id.value.trim().is_empty()
                    })
            })
            .cloned()
            .map(|title| PendingTitleHydration {
                title,
                attempt_count: 0,
            })
            .collect())
    }

    async fn list_anime_title_ids_missing_anibridge_scoped_external_ids(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<String>> {
        Ok(vec![])
    }

    async fn mark_title_metadata_hydration_due_now(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn schedule_title_metadata_hydration_retry(
        &self,
        _: &str,
        _: &str,
        _: i64,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn clear_title_metadata_hydration_retry_state(&self, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn update_metadata(
        &self,
        id: &str,
        name: Option<String>,
        facet: Option<MediaFacet>,
        tags: Option<Vec<String>>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;

        if let Some(name) = name {
            let normalized = name.trim();
            if normalized.is_empty() {
                return Err(AppError::Validation("title name cannot be empty".into()));
            }
            title.name = normalized.to_string();
        }

        if let Some(facet) = facet {
            title.facet = facet;
        }

        if let Some(tags) = tags {
            title.tags = normalize_tags(&tags);
        }

        Ok(title.clone())
    }

    async fn update_monitored(&self, id: &str, monitored: bool) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.monitored = monitored;
        Ok(title.clone())
    }

    async fn update_title_hydrated_metadata(
        &self,
        id: &str,
        metadata: TitleMetadataUpdate,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.name = metadata.name.unwrap_or(title.name.clone());
        title.year = metadata.year;
        title.overview = metadata.overview;
        title.poster_url = metadata.poster_url;
        title.banner_url = metadata.banner_url;
        title.background_url = metadata.background_url;
        title.sort_title = metadata.sort_title;
        title.slug = metadata.slug;
        title.imdb_id = metadata.imdb_id;
        title.runtime_minutes = metadata.runtime_minutes;
        title.genres = metadata.genres;
        title.content_status = metadata.content_status;
        title.language = metadata.language;
        title.first_aired = metadata.first_aired;
        title.network = metadata.network;
        title.studio = metadata.studio;
        title.country = metadata.country;
        title.aliases = metadata.aliases;
        title.tagged_aliases = metadata.tagged_aliases;
        title.metadata_language = metadata.metadata_language;
        title.metadata_fetched_at = Some(chrono::Utc::now());
        Ok(title.clone())
    }

    async fn replace_match_state(
        &self,
        id: &str,
        external_ids: Vec<ExternalId>,
        tags: Vec<String>,
    ) -> AppResult<Title> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.external_ids = external_ids;
        title.tags = tags;
        Ok(title.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let position = list
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        list.remove(position);
        Ok(())
    }

    async fn set_folder_path(&self, id: &str, folder_path: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = Some(folder_path.to_string());
        Ok(())
    }

    async fn clear_folder_path(&self, id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let title = list
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("title {}", id)))?;
        title.folder_path = None;
        Ok(())
    }

    async fn clear_metadata_language_for_all(&self) -> AppResult<u64> {
        let mut list = self.store.lock().await;
        let mut count = 0u64;
        for title in list.iter_mut() {
            if title.metadata_language.is_some() {
                title.metadata_language = None;
                count += 1;
            }
        }
        Ok(count)
    }
}

#[derive(Default)]
struct MockUserRepo {
    store: Arc<Mutex<Vec<User>>>,
    get_by_id_calls: Arc<AtomicUsize>,
    list_all_calls: Arc<AtomicUsize>,
}

impl MockUserRepo {
    fn get_by_id_call_count(&self) -> usize {
        self.get_by_id_calls.load(Ordering::SeqCst)
    }

    fn list_all_call_count(&self) -> usize {
        self.list_all_calls.load(Ordering::SeqCst)
    }
}

#[derive(Default, Clone)]
struct MockMediaFileRepo {
    store: Arc<Mutex<Vec<TitleMediaFile>>>,
}

#[async_trait]
impl MediaFileRepository for MockMediaFileRepo {
    async fn insert_media_file(&self, input: &InsertMediaFileInput) -> AppResult<String> {
        let id = Id::new().0;
        self.store.lock().await.push(TitleMediaFile {
            id: id.clone(),
            title_id: input.title_id.clone(),
            episode_id: None,
            file_path: input.file_path.clone(),
            size_bytes: input.size_bytes,
            source_signature_scheme: input.source_signature_scheme.clone(),
            source_signature_value: input.source_signature_value.clone(),
            quality_label: input.quality_label.clone(),
            scan_status: "pending".to_string(),
            created_at: Utc::now().to_rfc3339(),
            video_codec: None,
            video_width: None,
            video_height: None,
            video_bitrate_kbps: None,
            video_bit_depth: None,
            video_hdr_format: None,
            video_frame_rate: None,
            video_profile: None,
            audio_codec: None,
            audio_profile: None,
            audio_channels: None,
            audio_bitrate_kbps: None,
            audio_languages: Vec::new(),
            audio_streams: Vec::new(),
            subtitle_languages: Vec::new(),
            subtitle_codecs: Vec::new(),
            subtitle_streams: Vec::new(),
            has_multiaudio: false,
            duration_seconds: None,
            num_chapters: None,
            container_format: None,
            scene_name: input.scene_name.clone(),
            release_group: input.release_group.clone(),
            source_type: input.source_type.clone(),
            resolution: input.resolution.clone(),
            video_codec_parsed: input.video_codec_parsed.clone(),
            audio_codec_parsed: input.audio_codec_parsed.clone(),
            audio_channels_parsed: input.audio_channels_parsed.clone(),
            acquisition_score: input.acquisition_score,
            scoring_log: input.scoring_log.clone(),
            indexer_source: input.indexer_source.clone(),
            grabbed_release_title: input.grabbed_release_title.clone(),
            grabbed_at: input.grabbed_at.clone(),
            edition: input.edition.clone(),
            original_file_path: input.original_file_path.clone(),
            release_hash: input.release_hash.clone(),
        });
        Ok(id)
    }

    async fn link_file_to_episode(&self, file_id: &str, episode_id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.episode_id = Some(episode_id.to_string());
        Ok(())
    }

    async fn list_media_files_for_title(&self, title_id: &str) -> AppResult<Vec<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_live_media_files_for_episode_ids(
        &self,
        title_id: &str,
        episode_ids: &[String],
    ) -> AppResult<Vec<EpisodeScopedMediaFile>> {
        let episode_ids = episode_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|entry| {
                entry.title_id == title_id
                    && entry
                        .episode_id
                        .as_deref()
                        .is_some_and(|episode_id| episode_ids.contains(episode_id))
            })
            .cloned()
            .map(|media_file| {
                let episode_ids = media_file
                    .episode_id
                    .clone()
                    .into_iter()
                    .collect::<Vec<_>>();
                EpisodeScopedMediaFile {
                    media_file,
                    episode_ids,
                }
            })
            .collect())
    }

    async fn list_title_media_size_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        Ok(Vec::new())
    }

    async fn list_title_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<TitleQualitySummary>> {
        let rank = |value: &str| match value.trim().to_ascii_uppercase().as_str() {
            "4320P" => 0,
            "2160P" => 1,
            "1440P" => 2,
            "1080P" => 3,
            "1080I" => 4,
            "720P" => 5,
            "480P" => 6,
            "360P" => 7,
            _ => 999,
        };

        let store = self.store.lock().await;
        let mut out = Vec::new();
        for title_id in title_ids {
            let mut selected: Option<(i32, String)> = None;
            for entry in store.iter().filter(|entry| &entry.title_id == title_id) {
                let Some(label) = entry.quality_label.as_ref() else {
                    continue;
                };
                let normalized = label.trim().to_ascii_uppercase();
                if normalized.is_empty() {
                    continue;
                }
                let candidate = (rank(&normalized), normalized);
                if selected
                    .as_ref()
                    .is_none_or(|current| candidate.0 > current.0)
                {
                    selected = Some(candidate);
                }
            }
            if let Some((_, quality_tier)) = selected {
                out.push(TitleQualitySummary {
                    title_id: title_id.clone(),
                    quality_tier,
                });
            }
        }

        Ok(out)
    }

    async fn list_cutoff_unmet_quality_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<CutoffUnmetQualitySummary>> {
        let store = self.store.lock().await;
        let mut out = Vec::new();
        for title_id in title_ids {
            for entry in store.iter().filter(|entry| &entry.title_id == title_id) {
                let Some(label) = entry.quality_label.as_ref() else {
                    continue;
                };
                let normalized = label.trim().to_ascii_uppercase();
                if normalized.is_empty() {
                    continue;
                }
                out.push(CutoffUnmetQualitySummary {
                    title_id: title_id.clone(),
                    episode_id: entry.episode_id.clone(),
                    season_number: None,
                    episode_number: None,
                    quality_tier: normalized,
                });
            }
        }

        Ok(out)
    }

    async fn list_title_episode_progress_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleEpisodeProgressSummary>> {
        Ok(Vec::new())
    }

    async fn update_media_file_analysis(
        &self,
        file_id: &str,
        analysis: MediaFileAnalysis,
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.scan_status = "scanned".to_string();
        entry.video_codec = analysis.video_codec;
        entry.video_width = analysis.video_width;
        entry.video_height = analysis.video_height;
        entry.video_bitrate_kbps = analysis.video_bitrate_kbps;
        entry.video_bit_depth = analysis.video_bit_depth;
        entry.video_hdr_format = analysis.video_hdr_format;
        entry.video_frame_rate = analysis.video_frame_rate;
        entry.video_profile = analysis.video_profile;
        entry.audio_codec = analysis.audio_codec;
        entry.audio_channels = analysis.audio_channels;
        entry.audio_bitrate_kbps = analysis.audio_bitrate_kbps;
        entry.audio_languages = analysis.audio_languages;
        entry.audio_streams = analysis.audio_streams;
        entry.subtitle_languages = analysis.subtitle_languages;
        entry.subtitle_codecs = analysis.subtitle_codecs;
        entry.subtitle_streams = analysis.subtitle_streams;
        entry.has_multiaudio = analysis.has_multiaudio;
        entry.duration_seconds = analysis.duration_seconds;
        entry.num_chapters = analysis.num_chapters;
        entry.container_format = analysis.container_format;
        Ok(())
    }

    async fn update_media_file_source_signature(
        &self,
        file_id: &str,
        size_bytes: i64,
        source_signature_scheme: Option<String>,
        source_signature_value: Option<String>,
    ) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.size_bytes = size_bytes;
        entry.source_signature_scheme = source_signature_scheme;
        entry.source_signature_value = source_signature_value;
        Ok(())
    }

    async fn update_media_file_path(&self, file_id: &str, file_path: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.file_path = file_path.to_string();
        Ok(())
    }

    async fn mark_scan_failed(&self, file_id: &str, _error: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let entry = list
            .iter_mut()
            .find(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        entry.scan_status = "failed".to_string();
        Ok(())
    }

    async fn get_media_file_by_id(&self, file_id: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|entry| entry.id == file_id)
            .cloned())
    }

    async fn get_media_file_by_path(&self, file_path: &str) -> AppResult<Option<TitleMediaFile>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|entry| entry.file_path == file_path)
            .cloned())
    }
    async fn delete_media_file(&self, file_id: &str) -> AppResult<()> {
        let mut list = self.store.lock().await;
        let position = list
            .iter()
            .position(|entry| entry.id == file_id)
            .ok_or_else(|| AppError::NotFound(format!("media file {}", file_id)))?;
        list.remove(position);
        Ok(())
    }
}

#[derive(Default, Clone)]
struct TrackingImportRepo {
    records: Arc<Mutex<Vec<ImportRecord>>>,
}

#[async_trait]
impl ImportRepository for TrackingImportRepo {
    async fn queue_import_request(
        &self,
        source_system: String,
        source_ref: String,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        let id = Id::new().0;
        let now = Utc::now().to_rfc3339();
        self.records.lock().await.push(ImportRecord {
            id: id.clone(),
            source_system,
            source_ref,
            import_type: ImportType::parse(&import_type).unwrap_or(ImportType::ManualImport),
            status: ImportStatus::Pending,
            payload_json,
            result_json: None,
            started_at: None,
            finished_at: None,
            created_at: now.clone(),
            updated_at: now,
        });
        Ok(id)
    }

    async fn get_import_by_id(&self, id: &str) -> AppResult<Option<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .find(|record| record.id == id)
            .cloned())
    }

    async fn get_import_by_source_ref(
        &self,
        source_system: &str,
        source_ref: &str,
    ) -> AppResult<Option<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .rev()
            .find(|record| record.source_system == source_system && record.source_ref == source_ref)
            .cloned())
    }

    async fn get_import_by_source_ref_and_type(
        &self,
        source_system: &str,
        source_ref: &str,
        import_type: ImportType,
    ) -> AppResult<Option<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .rev()
            .find(|record| {
                record.source_system == source_system
                    && record.source_ref == source_ref
                    && record.import_type == import_type
            })
            .cloned())
    }

    async fn update_import_status(
        &self,
        id: &str,
        status: ImportStatus,
        result_json: Option<String>,
    ) -> AppResult<()> {
        let now = Utc::now().to_rfc3339();
        let mut records = self.records.lock().await;
        let record = records
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("import record {id}")))?;
        record.status = status;
        record.result_json = result_json;
        if record.started_at.is_none() {
            record.started_at = Some(now.clone());
        }
        if status.is_terminal() {
            record.finished_at = Some(now.clone());
        }
        record.updated_at = now;
        Ok(())
    }

    async fn recover_stale_processing_imports(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(0)
    }

    async fn recover_stale_processing_imports_for_type(
        &self,
        _import_type: ImportType,
        _stale_seconds: i64,
    ) -> AppResult<u64> {
        Ok(0)
    }

    async fn list_pending_imports(&self) -> AppResult<Vec<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .filter(|record| record.status.is_active())
            .cloned()
            .collect())
    }

    async fn list_pending_imports_for_type(
        &self,
        import_type: ImportType,
    ) -> AppResult<Vec<ImportRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .filter(|record| record.import_type == import_type && record.status.is_active())
            .cloned()
            .collect())
    }

    async fn list_imports_for_sources(
        &self,
        sources: &[(String, String)],
    ) -> AppResult<Vec<ImportRecord>> {
        let records = self.records.lock().await;
        Ok(sources
            .iter()
            .filter_map(|(source_system, source_ref)| {
                records
                    .iter()
                    .rev()
                    .find(|record| {
                        record.source_system == *source_system && record.source_ref == *source_ref
                    })
                    .cloned()
            })
            .collect())
    }

    async fn is_already_imported(&self, source_system: &str, source_ref: &str) -> AppResult<bool> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .rev()
            .find(|record| record.source_system == source_system && record.source_ref == source_ref)
            .is_some_and(|record| {
                matches!(
                    record.status,
                    ImportStatus::Completed | ImportStatus::Skipped
                )
            }))
    }

    async fn list_imports(&self, limit: usize) -> AppResult<Vec<ImportRecord>> {
        let mut records = self.records.lock().await.clone();
        records.reverse();
        records.truncate(limit);
        Ok(records)
    }
}

#[derive(Default, Clone)]
struct BlockingFileImporter {
    release: Arc<Notify>,
    call_count: Arc<AtomicUsize>,
}

#[async_trait]
impl FileImporter for BlockingFileImporter {
    async fn import_file(
        &self,
        source: &Path,
        dest: &Path,
    ) -> AppResult<scryer_domain::ImportFileResult> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.release.notified().await;
        Ok(scryer_domain::ImportFileResult {
            strategy: scryer_domain::ImportStrategy::Copy,
            source_path: source.to_path_buf(),
            dest_path: dest.to_path_buf(),
            size_bytes: std::fs::metadata(source)
                .map(|metadata| metadata.len())
                .unwrap_or(0),
        })
    }
}

#[async_trait]
impl UserRepository for MockUserRepo {
    async fn get_by_username(&self, username: &str) -> AppResult<Option<User>> {
        let users = self.store.lock().await;
        Ok(users.iter().find(|user| user.username == username).cloned())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<User>> {
        self.get_by_id_calls.fetch_add(1, Ordering::SeqCst);
        let users = self.store.lock().await;
        Ok(users.iter().find(|user| user.id == id).cloned())
    }

    async fn create(&self, user: User) -> AppResult<User> {
        self.store.lock().await.push(user.clone());
        Ok(user)
    }

    async fn list_all(&self) -> AppResult<Vec<User>> {
        self.list_all_calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.store.lock().await.clone())
    }

    async fn update_entitlements(
        &self,
        id: &str,
        entitlements: Vec<Entitlement>,
    ) -> AppResult<User> {
        let mut users = self.store.lock().await;
        let user = users
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
        user.entitlements = entitlements;
        Ok(user.clone())
    }

    async fn update_password_hash(&self, id: &str, password_hash: String) -> AppResult<User> {
        let mut users = self.store.lock().await;
        let user = users
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
        user.password_hash = Some(password_hash);
        Ok(user.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut users = self.store.lock().await;
        let index = users
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("user {}", id)))?;
        users.remove(index);
        Ok(())
    }
}

#[derive(Default)]
struct MockDomainEventRepo {
    events: Arc<Mutex<Vec<DomainEvent>>>,
    subscriber_offsets: Arc<Mutex<HashMap<String, i64>>>,
}

#[derive(Default)]
struct MockExternalImportMonitorSnapshotRepo {
    snapshots: Arc<Mutex<Vec<ExternalImportMonitorSnapshot>>>,
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for MockExternalImportMonitorSnapshotRepo {
    async fn upsert_external_import_monitor_snapshot(
        &self,
        snapshot: &ExternalImportMonitorSnapshot,
    ) -> AppResult<()> {
        let mut snapshots = self.snapshots.lock().await;
        snapshots.retain(|existing| existing.facet != snapshot.facet);
        snapshots.push(snapshot.clone());
        Ok(())
    }

    async fn get_external_import_monitor_snapshot(
        &self,
        facet: &MediaFacet,
    ) -> AppResult<Option<ExternalImportMonitorSnapshot>> {
        let snapshots = self.snapshots.lock().await;
        Ok(snapshots
            .iter()
            .find(|snapshot| &snapshot.facet == facet)
            .cloned())
    }

    async fn delete_external_import_monitor_snapshot(&self, facet: &MediaFacet) -> AppResult<()> {
        let mut snapshots = self.snapshots.lock().await;
        snapshots.retain(|snapshot| &snapshot.facet != facet);
        Ok(())
    }
}

#[async_trait]
impl DomainEventRepository for MockDomainEventRepo {
    async fn append(&self, event: NewDomainEvent) -> AppResult<DomainEvent> {
        let mut events = self.events.lock().await;
        let sequence = events
            .last()
            .map(|existing| existing.sequence + 1)
            .unwrap_or(1);
        let stored = DomainEvent {
            sequence,
            event_id: event.event_id,
            occurred_at: event.occurred_at,
            actor_user_id: event.actor_user_id,
            title_id: event.title_id,
            facet: event.facet,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            schema_version: event.schema_version,
            stream: event.stream,
            payload: event.payload,
        };
        events.push(stored.clone());
        Ok(stored)
    }

    async fn append_many(&self, events: Vec<NewDomainEvent>) -> AppResult<Vec<DomainEvent>> {
        let mut stored = Vec::with_capacity(events.len());
        for event in events {
            stored.push(self.append(event).await?);
        }
        Ok(stored)
    }

    async fn list(&self, filter: &DomainEventFilter) -> AppResult<Vec<DomainEvent>> {
        let events = self.events.lock().await;
        let limit = if filter.limit == 0 {
            usize::MAX
        } else {
            filter.limit
        };
        let iter: Box<dyn Iterator<Item = &DomainEvent>> =
            if filter.after_sequence.is_some() && filter.before_sequence.is_none() {
                Box::new(events.iter())
            } else {
                Box::new(events.iter().rev())
            };
        Ok(iter
            .filter(|event| {
                filter
                    .after_sequence
                    .is_none_or(|after| event.sequence > after)
                    && filter
                        .before_sequence
                        .is_none_or(|before| event.sequence < before)
                    && filter
                        .title_id
                        .as_ref()
                        .is_none_or(|title_id| event.title_id.as_deref() == Some(title_id.as_str()))
                    && filter
                        .facet
                        .as_ref()
                        .is_none_or(|facet| event.facet.as_ref() == Some(facet))
                    && filter.event_types.as_ref().is_none_or(|event_types| {
                        event_types
                            .iter()
                            .any(|event_type| &event.payload.event_type() == event_type)
                    })
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn count_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
    ) -> AppResult<i64> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter_map(crate::event_views::title_history_record_from_domain_event)
            .filter(|record| {
                event_types.is_none_or(|values| values.contains(&record.event_type))
                    && title_ids.is_none_or(|values| values.contains(&record.title_id))
                    && download_id.is_none_or(|value| record.download_id.as_deref() == Some(value))
            })
            .count() as i64)
    }

    async fn list_title_history_page_events(
        &self,
        event_types: Option<&[TitleHistoryEventType]>,
        title_ids: Option<&[String]>,
        download_id: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let page_size = if limit == 0 { usize::MAX } else { limit };
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .rev()
            .filter(|event| {
                crate::event_views::title_history_record_from_domain_event(event).is_some_and(
                    |record| {
                        event_types.is_none_or(|values| values.contains(&record.event_type))
                            && title_ids.is_none_or(|values| values.contains(&record.title_id))
                            && download_id
                                .is_none_or(|value| record.download_id.as_deref() == Some(value))
                    },
                )
            })
            .skip(offset)
            .take(page_size)
            .cloned()
            .collect())
    }

    async fn list_after_sequence(
        &self,
        after_sequence: i64,
        limit: usize,
    ) -> AppResult<Vec<DomainEvent>> {
        let events = self.events.lock().await;
        Ok(events
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn get_subscriber_offset(&self, subscriber: &str) -> AppResult<i64> {
        let offsets = self.subscriber_offsets.lock().await;
        Ok(*offsets.get(subscriber).unwrap_or(&0))
    }

    async fn set_subscriber_offset(&self, subscriber: &str, sequence: i64) -> AppResult<()> {
        let mut offsets = self.subscriber_offsets.lock().await;
        offsets.insert(subscriber.to_string(), sequence);
        Ok(())
    }
}

#[derive(Default)]
struct MockShowRepo {
    collections: Arc<Mutex<Vec<Collection>>>,
    episodes: Arc<Mutex<Vec<Episode>>>,
    collection_external_ids: Arc<Mutex<Vec<ScopedExternalId>>>,
    episode_external_ids: Arc<Mutex<Vec<ScopedExternalId>>>,
}

#[async_trait]
impl ShowRepository for MockShowRepo {
    async fn list_collections_for_title(&self, title_id: &str) -> AppResult<Vec<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .filter(|item| item.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_collection_external_ids(
        &self,
        collection_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let ids = self.collection_external_ids.lock().await;
        Ok(ids
            .iter()
            .filter(|item| item.scope_id == collection_id)
            .cloned()
            .collect())
    }

    async fn list_collections_for_titles(
        &self,
        title_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<Collection>>> {
        let collections = self.collections.lock().await;
        let wanted = title_ids.iter().cloned().collect::<HashSet<_>>();
        let mut grouped = HashMap::<String, Vec<Collection>>::new();
        for collection in collections.iter() {
            if wanted.contains(&collection.title_id) {
                grouped
                    .entry(collection.title_id.clone())
                    .or_default()
                    .push(collection.clone());
            }
        }
        Ok(grouped)
    }

    async fn get_collection_by_id(&self, collection_id: &str) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|item| item.id == collection_id)
            .cloned())
    }

    async fn get_collection_by_ordered_path(
        &self,
        ordered_path: &str,
    ) -> AppResult<Option<Collection>> {
        let collections = self.collections.lock().await;
        Ok(collections
            .iter()
            .find(|item| item.ordered_path.as_deref() == Some(ordered_path))
            .cloned())
    }

    async fn create_collection(&self, collection: Collection) -> AppResult<Collection> {
        self.collections.lock().await.push(collection.clone());
        Ok(collection)
    }

    async fn update_collection(
        &self,
        collection_id: &str,
        update: CollectionUpdate,
    ) -> AppResult<Collection> {
        let mut collections = self.collections.lock().await;
        let item = collections
            .iter_mut()
            .find(|entry| entry.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;

        if let Some(value) = update.collection_type {
            item.collection_type = value;
        }
        if let Some(value) = update.collection_index {
            item.collection_index = value;
        }
        if let Some(value) = update.label {
            item.label = Some(value);
        }
        if let Some(value) = update.ordered_path {
            item.ordered_path = Some(value);
        }
        if let Some(value) = update.first_episode_number {
            item.first_episode_number = Some(value);
        }
        if let Some(value) = update.last_episode_number {
            item.last_episode_number = Some(value);
        }
        if let Some(value) = update.monitored {
            item.monitored = value;
        }

        Ok(item.clone())
    }

    async fn update_collection_interstitial_movie(
        &self,
        collection_id: &str,
        interstitial_movie: scryer_domain::InterstitialMovieMetadata,
    ) -> AppResult<Collection> {
        let mut collections = self.collections.lock().await;
        let item = collections
            .iter_mut()
            .find(|entry| entry.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        item.interstitial_movie = Some(interstitial_movie);
        Ok(item.clone())
    }

    async fn update_collection_specials_movies(
        &self,
        collection_id: &str,
        specials_movies: Vec<scryer_domain::InterstitialMovieMetadata>,
    ) -> AppResult<Collection> {
        let mut collections = self.collections.lock().await;
        let item = collections
            .iter_mut()
            .find(|entry| entry.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        item.specials_movies = specials_movies;
        Ok(item.clone())
    }

    async fn update_interstitial_season_episode(
        &self,
        _collection_id: &str,
        _season_episode: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn set_collection_episodes_monitored(
        &self,
        collection_id: &str,
        monitored: bool,
    ) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.collection_id.as_deref() == Some(collection_id) {
                episode.monitored = monitored;
            }
        }
        Ok(())
    }

    async fn delete_collection(&self, collection_id: &str) -> AppResult<()> {
        let mut collections = self.collections.lock().await;
        let index = collections
            .iter()
            .position(|item| item.id == collection_id)
            .ok_or_else(|| AppError::NotFound(format!("collection {}", collection_id)))?;
        collections.remove(index);

        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.collection_id.as_deref() == Some(collection_id) {
                episode.collection_id = None;
            }
        }
        Ok(())
    }

    async fn delete_collections_for_title(&self, title_id: &str) -> AppResult<()> {
        let mut collections = self.collections.lock().await;
        collections.retain(|item| item.title_id != title_id);

        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if episode.title_id == title_id {
                episode.collection_id = None;
            }
        }
        Ok(())
    }

    async fn list_episodes_for_collection(&self, collection_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|item| item.collection_id.as_deref() == Some(collection_id))
            .cloned()
            .collect())
    }

    async fn list_episodes_for_title(&self, title_id: &str) -> AppResult<Vec<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .filter(|item| item.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn list_episode_external_ids(
        &self,
        episode_id: &str,
    ) -> AppResult<Vec<ScopedExternalId>> {
        let ids = self.episode_external_ids.lock().await;
        Ok(ids
            .iter()
            .filter(|item| item.scope_id == episode_id)
            .cloned()
            .collect())
    }

    async fn get_episode_by_id(&self, episode_id: &str) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes.iter().find(|item| item.id == episode_id).cloned())
    }

    async fn create_episode(&self, episode: Episode) -> AppResult<Episode> {
        self.episodes.lock().await.push(episode.clone());
        Ok(episode)
    }

    async fn update_episode(&self, episode_id: &str, update: EpisodeUpdate) -> AppResult<Episode> {
        let mut episodes = self.episodes.lock().await;
        let item = episodes
            .iter_mut()
            .find(|entry| entry.id == episode_id)
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;

        if let Some(value) = update.episode_type {
            item.episode_type = value;
        }
        if let Some(value) = update.episode_number {
            item.episode_number = Some(value);
        }
        if let Some(value) = update.season_number {
            item.season_number = Some(value);
        }
        if let Some(value) = update.episode_label {
            item.episode_label = Some(value);
        }
        if let Some(value) = update.title {
            item.title = Some(value);
        }
        if let Some(value) = update.air_date {
            item.air_date = Some(value);
        }
        if let Some(value) = update.duration_seconds {
            item.duration_seconds = Some(value);
        }
        if let Some(value) = update.has_multi_audio {
            item.has_multi_audio = value;
        }
        if let Some(value) = update.has_subtitle {
            item.has_subtitle = value;
        }
        if let Some(value) = update.monitored {
            item.monitored = value;
        }
        if let Some(value) = update.collection_id {
            item.collection_id = Some(value);
        }
        if let Some(value) = update.overview {
            item.overview = Some(value);
        }
        if let Some(value) = update.tvdb_id {
            item.tvdb_id = Some(value);
        }

        Ok(item.clone())
    }

    async fn delete_episode(&self, episode_id: &str) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        let index = episodes
            .iter()
            .position(|item| item.id == episode_id)
            .ok_or_else(|| AppError::NotFound(format!("episode {}", episode_id)))?;
        episodes.remove(index);
        Ok(())
    }

    async fn delete_episodes_for_title(&self, title_id: &str) -> AppResult<()> {
        let mut episodes = self.episodes.lock().await;
        episodes.retain(|item| item.title_id != title_id);
        Ok(())
    }

    async fn find_episode_by_title_and_numbers(
        &self,
        title_id: &str,
        season_number: &str,
        episode_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|ep| {
                ep.title_id == title_id
                    && ep.season_number.as_deref() == Some(season_number)
                    && ep.episode_number.as_deref() == Some(episode_number)
            })
            .cloned())
    }

    async fn find_episode_by_title_and_absolute_number(
        &self,
        title_id: &str,
        absolute_number: &str,
    ) -> AppResult<Option<Episode>> {
        let episodes = self.episodes.lock().await;
        Ok(episodes
            .iter()
            .find(|ep| {
                ep.title_id == title_id && ep.absolute_number.as_deref() == Some(absolute_number)
            })
            .cloned())
    }

    async fn list_primary_collection_summaries(
        &self,
        title_ids: &[String],
    ) -> AppResult<Vec<PrimaryCollectionSummary>> {
        let collections = self.collections.lock().await;
        let mut out = Vec::new();
        for tid in title_ids {
            if let Some(c) = collections
                .iter()
                .filter(|c| c.title_id == *tid)
                .filter(|c| c.collection_type == CollectionType::Movie || c.collection_index == "0")
                .min_by(|left, right| {
                    let left_key = (
                        left.collection_type != CollectionType::Movie,
                        left.ordered_path
                            .as_deref()
                            .is_none_or(|path| path.trim().is_empty()),
                        left.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                        left.collection_index.clone(),
                    );
                    let right_key = (
                        right.collection_type != CollectionType::Movie,
                        right
                            .ordered_path
                            .as_deref()
                            .is_none_or(|path| path.trim().is_empty()),
                        right.collection_index.parse::<u32>().unwrap_or(u32::MAX),
                        right.collection_index.clone(),
                    );
                    left_key.cmp(&right_key)
                })
            {
                out.push(PrimaryCollectionSummary {
                    title_id: tid.clone(),
                    label: c.label.clone(),
                    ordered_path: c.ordered_path.clone(),
                });
            }
        }
        Ok(out)
    }

    async fn list_episodes_in_date_range(
        &self,
        _start_date: &str,
        _end_date: &str,
    ) -> AppResult<Vec<CalendarEpisode>> {
        Ok(vec![])
    }

    async fn replace_anibridge_scoped_external_ids_for_title(
        &self,
        _title_id: &str,
        collection_ids: Vec<ScopedExternalId>,
        episode_ids: Vec<ScopedExternalId>,
    ) -> AppResult<()> {
        *self.collection_external_ids.lock().await = collection_ids;
        *self.episode_external_ids.lock().await = episode_ids;
        Ok(())
    }
}

#[derive(Default)]
struct MockIndexerClient;

#[async_trait]
impl IndexerClient for MockIndexerClient {
    async fn search(
        &self,
        query: String,
        ids: std::collections::HashMap<String, String>,
        category: Option<String>,
        _facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        if let Some(tvdb) = ids.get("tvdb_id") {
            tracing::info!(tvdb_id = %tvdb, category = ?category, "mock nzbgeek search");
        }
        if let Some(imdb) = ids.get("imdb_id") {
            tracing::info!(imdb_id = %imdb, category = ?category, "mock nzbgeek search");
        }
        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: format!("match for {query}"),
                link: None,
                download_url: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: None,
                quality_profile_decision: None,
                extra: Default::default(),
                guid: None,
                info_url: None,
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedIndexerSearch {
    query: String,
    season: Option<u32>,
    episode: Option<u32>,
}

#[derive(Default, Clone)]
struct TrackingIndexerClient {
    searches: Arc<Mutex<Vec<RecordedIndexerSearch>>>,
}

#[async_trait]
impl IndexerClient for TrackingIndexerClient {
    async fn search(
        &self,
        query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        season: Option<u32>,
        episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        self.searches.lock().await.push(RecordedIndexerSearch {
            query: query.clone(),
            season,
            episode,
        });

        let release_title = match (season, episode) {
            (Some(season), Some(episode)) => {
                format!("{query}.S{season:02}E{episode:02}.1080p.WEB-DL")
            }
            (Some(season), None) => format!("{query}.S{season:02}.1080p.WEB-DL"),
            (None, _) => format!("{query}.2024.1080p.WEB-DL"),
        };
        let release_slug = release_title.replace([' ', '/'], ".");

        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: release_title.clone(),
                link: Some(format!("https://example.invalid/info/{release_slug}")),
                download_url: Some(format!(
                    "https://example.invalid/download/{release_slug}.nzb"
                )),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some(format!("guid-{release_slug}")),
                info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
struct FixedReleaseIndexerClient {
    release_title: String,
    indexer_languages: Option<Vec<String>>,
}

impl FixedReleaseIndexerClient {
    fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            indexer_languages: None,
        }
    }
}

#[async_trait]
impl IndexerClient for FixedReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: vec![IndexerSearchResult {
                source: "nzbgeek".into(),
                title: self.release_title.clone(),
                link: Some("https://example.invalid/info".to_string()),
                download_url: Some("https://example.invalid/download.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                size_bytes: None,
                published_at: Some("1970-01-01T00:00:00Z".into()),
                thumbs_up: None,
                thumbs_down: None,
                indexer_languages: self.indexer_languages.clone(),
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some("guid-fixed-release".to_string()),
                info_url: Some("https://example.invalid/info".to_string()),
                provenance: None,
                auto_eligible: None,
                auto_decision_code: None,
                auto_decision_summary: None,
                candidate_token: None,
                queue_scope: None,
            }],
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

#[derive(Clone)]
struct MultiReleaseIndexerClient {
    release_titles: Vec<String>,
}

impl MultiReleaseIndexerClient {
    fn new(release_titles: Vec<&str>) -> Self {
        Self {
            release_titles: release_titles.into_iter().map(str::to_string).collect(),
        }
    }
}

#[async_trait]
impl IndexerClient for MultiReleaseIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        _facet: Option<String>,
        _newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        Ok(IndexerSearchResponse {
            results: self
                .release_titles
                .iter()
                .enumerate()
                .map(|(index, release_title)| IndexerSearchResult {
                    source: "nzbgeek".into(),
                    title: release_title.clone(),
                    link: Some(format!("https://example.invalid/info/{index}")),
                    download_url: Some(format!("https://example.invalid/download/{index}.nzb")),
                    source_kind: Some(DownloadSourceKind::NzbUrl),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: Some(crate::parse_release_metadata(release_title)),
                    quality_profile_decision: None,
                    extra: Default::default(),
                    guid: Some(format!("guid-multi-release-{index}")),
                    info_url: Some(format!("https://example.invalid/info/{index}")),
                    provenance: None,
                    auto_eligible: None,
                    auto_decision_code: None,
                    auto_decision_summary: None,
                    candidate_token: None,
                    queue_scope: None,
                })
                .collect(),
            api_current: None,
            api_max: None,
            grab_current: None,
            grab_max: None,
        })
    }
}

struct MockMetadataGateway {
    movies: HashMap<i64, MovieMetadata>,
}

#[async_trait]
impl MetadataGateway for MockMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_batch(
        &self,
        _queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_movie(&self, tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        self.movies
            .get(&tvdb_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound(format!("movie {tvdb_id}")))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::Repository("not implemented in tests".into()))
    }

    async fn get_metadata_bulk(
        &self,
        movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        let movies = movie_tvdb_ids
            .iter()
            .filter_map(|tvdb_id| {
                self.movies
                    .get(tvdb_id)
                    .cloned()
                    .map(|movie| (*tvdb_id, movie))
            })
            .collect();
        Ok(BulkMetadataResult {
            movies,
            series: HashMap::new(),
        })
    }
}

#[derive(Default)]
struct MockIndexerConfigRepo {
    store: Arc<Mutex<Vec<IndexerConfig>>>,
}

#[derive(Default)]
struct MockSettingsRepo;

#[async_trait]
impl SettingsRepository for MockSettingsRepo {
    async fn get_setting_json(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    async fn upsert_setting_json(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
        _value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default, Clone)]
struct StoredSettingsRepo {
    values: StoredSettingValues,
}

type StoredSettingValues = Arc<Mutex<HashMap<(String, String, Option<String>), String>>>;

impl StoredSettingsRepo {
    async fn set_value(&self, scope: &str, key_name: &str, value: &str) {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), None),
            value.to_string(),
        );
    }

    async fn set_scoped_value(&self, scope: &str, key_name: &str, scope_id: &str, value: &str) {
        self.values.lock().await.insert(
            (
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ),
            value.to_string(),
        );
    }

    async fn get_scoped_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: &str,
    ) -> Option<String> {
        self.values
            .lock()
            .await
            .get(&(
                scope.to_string(),
                key_name.to_string(),
                Some(scope_id.to_string()),
            ))
            .cloned()
    }
}

#[async_trait]
impl SettingsRepository for StoredSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id))
            .cloned())
    }

    async fn upsert_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
        value_json: String,
        _source: &str,
        _updated_by_user_id: Option<String>,
    ) -> AppResult<()> {
        self.values.lock().await.insert(
            (scope.to_string(), key_name.to_string(), scope_id),
            value_json,
        );
        Ok(())
    }
}

#[derive(Default, Clone)]
struct MutableLibraryScanner {
    library_files: Arc<Mutex<Vec<LibraryFile>>>,
}

impl MutableLibraryScanner {
    async fn set_library_files(&self, files: Vec<LibraryFile>) {
        *self.library_files.lock().await = files;
    }
}

#[async_trait]
impl LibraryScanner for MutableLibraryScanner {
    async fn scan_library(&self, _root: &str) -> AppResult<Vec<LibraryFile>> {
        Ok(self.library_files.lock().await.clone())
    }

    async fn scan_library_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let files = self.library_files.lock().await.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        tx.send(Ok(files))
            .await
            .map_err(|err| AppError::Repository(err.to_string()))?;
        Ok(rx)
    }

    async fn scan_directory_batched(
        &self,
        _root: &str,
        _batch_size: usize,
    ) -> AppResult<LibraryFileBatchReceiver> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }
}

#[derive(Default, Clone)]
struct EmptySearchMetadataGateway;

#[async_trait]
impl MetadataGateway for EmptySearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, Vec::new()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

#[derive(Clone)]
struct BlockingBatchMetadataGateway {
    batch_search_calls: Arc<AtomicUsize>,
    batch_search_started: Arc<Notify>,
    blocked_calls: Arc<Vec<usize>>,
    released_through: Arc<AtomicUsize>,
    release_notify: Arc<Notify>,
}

impl BlockingBatchMetadataGateway {
    fn blocking_calls(blocked_calls: &[usize]) -> Self {
        Self {
            batch_search_calls: Arc::new(AtomicUsize::new(0)),
            batch_search_started: Arc::new(Notify::new()),
            blocked_calls: Arc::new(blocked_calls.to_vec()),
            released_through: Arc::new(AtomicUsize::new(0)),
            release_notify: Arc::new(Notify::new()),
        }
    }

    async fn wait_for_batch_search(&self) {
        self.wait_for_batch_search_calls(1).await;
    }

    async fn wait_for_batch_search_calls(&self, expected_calls: usize) {
        if self.batch_search_calls.load(Ordering::SeqCst) >= expected_calls {
            return;
        }

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.batch_search_calls.load(Ordering::SeqCst) >= expected_calls {
                    break;
                }
                self.batch_search_started.notified().await;
            }
        })
        .await
        .expect("timed out waiting for metadata search to start");
    }

    fn release_through(&self, call_number: usize) {
        self.released_through.store(call_number, Ordering::SeqCst);
        self.release_notify.notify_waiters();
    }

    fn release(&self) {
        self.release_through(usize::MAX);
    }
}

impl Default for BlockingBatchMetadataGateway {
    fn default() -> Self {
        Self::blocking_calls(&[1])
    }
}

#[async_trait]
impl MetadataGateway for BlockingBatchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        let call_number = self.batch_search_calls.fetch_add(1, Ordering::SeqCst) + 1;
        self.batch_search_started.notify_waiters();

        if self.blocked_calls.contains(&call_number) {
            while self.released_through.load(Ordering::SeqCst) < call_number {
                self.release_notify.notified().await;
            }
        }

        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, Vec::new()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult::default())
    }
}

#[derive(Default, Clone)]
struct TrackingLibraryScanUnmatchedItemRepo {
    items: Arc<Mutex<Vec<LibraryScanUnmatchedItem>>>,
}

impl TrackingLibraryScanUnmatchedItemRepo {
    async fn items(&self) -> Vec<LibraryScanUnmatchedItem> {
        self.items.lock().await.clone()
    }
}

#[async_trait]
impl LibraryScanUnmatchedItemRepository for TrackingLibraryScanUnmatchedItemRepo {
    async fn upsert_library_scan_unmatched_item(
        &self,
        item: &LibraryScanUnmatchedItem,
    ) -> AppResult<String> {
        let mut items = self.items.lock().await;
        if let Some(existing) = items.iter_mut().find(|existing| existing.id == item.id) {
            let mut updated = item.clone();
            updated.created_at = existing.created_at.clone();
            if existing.status == PendingImportStatus::Ignored
                && updated.status == PendingImportStatus::Pending
            {
                updated.status = PendingImportStatus::Ignored;
            }
            *existing = updated;
        } else {
            items.push(item.clone());
        }

        Ok(item.id.clone())
    }

    async fn get_library_scan_unmatched_item(
        &self,
        id: &str,
    ) -> AppResult<Option<LibraryScanUnmatchedItem>> {
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .find(|item| item.id == id)
            .cloned())
    }

    async fn delete_library_scan_unmatched_item(
        &self,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        self.items
            .lock()
            .await
            .retain(|item| !(item.facet == facet && item.item_path == item_path));
        Ok(())
    }

    async fn list_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<LibraryScanUnmatchedItem>> {
        let offset = offset.max(0) as usize;
        let limit = limit.max(0) as usize;
        let mut items: Vec<_> = self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &item.facet == expected)
            })
            .filter(|item| {
                scan_root
                    .as_ref()
                    .is_none_or(|expected| item.scan_root == *expected)
            })
            .filter(|item| status.is_none_or(|expected| item.status == expected))
            .cloned()
            .collect();
        items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
        status: Option<PendingImportStatus>,
    ) -> AppResult<i64> {
        Ok(self
            .items
            .lock()
            .await
            .iter()
            .filter(|item| {
                facet
                    .as_ref()
                    .is_none_or(|expected| &item.facet == expected)
            })
            .filter(|item| {
                scan_root
                    .as_ref()
                    .is_none_or(|expected| item.scan_root == *expected)
            })
            .filter(|item| status.is_none_or(|expected| item.status == expected))
            .count() as i64)
    }
}

#[derive(Default)]
struct MockQualityProfileRepo;

#[async_trait]
impl QualityProfileRepository for MockQualityProfileRepo {
    async fn list_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        Ok(vec![])
    }

    async fn replace_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
        _profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        Ok(())
    }
}

#[derive(Default, Clone)]
struct StoredQualityProfileRepo {
    profiles: Arc<Mutex<Vec<QualityProfile>>>,
}

impl StoredQualityProfileRepo {
    async fn set_profiles(&self, profiles: Vec<QualityProfile>) {
        *self.profiles.lock().await = profiles;
    }
}

#[async_trait]
impl QualityProfileRepository for StoredQualityProfileRepo {
    async fn list_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
    ) -> AppResult<Vec<QualityProfile>> {
        Ok(self.profiles.lock().await.clone())
    }

    async fn replace_quality_profiles(
        &self,
        _scope: &str,
        _scope_id: Option<String>,
        profiles: Vec<QualityProfile>,
    ) -> AppResult<()> {
        *self.profiles.lock().await = profiles;
        Ok(())
    }
}

#[async_trait]
impl IndexerConfigRepository for MockIndexerConfigRepo {
    async fn list(&self, provider_filter: Option<String>) -> AppResult<Vec<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| {
                provider_filter
                    .as_ref()
                    .is_none_or(|provider| provider == &entry.provider_type)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<IndexerConfig>> {
        let entries = self.store.lock().await;
        Ok(entries.iter().find(|entry| entry.id == id).cloned())
    }

    async fn touch_last_error(&self, provider_type: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let now = Utc::now();
        for entry in entries.iter_mut() {
            if entry.provider_type == provider_type {
                entry.last_error_at = Some(now);
                entry.updated_at = now;
            }
        }
        Ok(())
    }

    async fn create(&self, config: IndexerConfig) -> AppResult<IndexerConfig> {
        let mut entries = self.store.lock().await;
        entries.push(config.clone());
        Ok(config)
    }

    async fn update(&self, update: crate::IndexerConfigUpdate) -> AppResult<IndexerConfig> {
        let crate::IndexerConfigUpdate {
            id,
            name,
            provider_type,
            base_url,
            api_key_encrypted,
            rate_limit_seconds,
            rate_limit_burst,
            is_enabled,
            enable_interactive_search,
            enable_auto_search,
            config_json,
        } = update;
        let mut entries = self.store.lock().await;
        let item = entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("indexer config {}", id)))?;

        if let Some(name) = name {
            item.name = name;
        }
        if let Some(provider_type) = provider_type {
            item.provider_type = provider_type;
        }
        if let Some(base_url) = base_url {
            item.base_url = base_url;
        }
        if let Some(api_key_encrypted) = api_key_encrypted {
            item.api_key_encrypted = Some(api_key_encrypted);
        }
        if let Some(rate_limit_seconds) = rate_limit_seconds {
            item.rate_limit_seconds = Some(rate_limit_seconds);
        }
        if let Some(rate_limit_burst) = rate_limit_burst {
            item.rate_limit_burst = Some(rate_limit_burst);
        }
        if let Some(is_enabled) = is_enabled {
            item.is_enabled = is_enabled;
        }
        if let Some(enable_interactive_search) = enable_interactive_search {
            item.enable_interactive_search = enable_interactive_search;
        }
        if let Some(enable_auto_search) = enable_auto_search {
            item.enable_auto_search = enable_auto_search;
        }
        if let Some(config_json) = config_json {
            item.config_json = Some(config_json);
        }
        item.updated_at = Utc::now();

        Ok(item.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let position = entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("indexer config {}", id)))?;
        entries.remove(position);
        Ok(())
    }
}

#[derive(Default)]
struct MockDownloadClientConfigRepo {
    store: Arc<Mutex<Vec<DownloadClientConfig>>>,
}

#[async_trait]
impl DownloadClientConfigRepository for MockDownloadClientConfigRepo {
    async fn list(&self, client_type: Option<String>) -> AppResult<Vec<DownloadClientConfig>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| {
                client_type
                    .as_ref()
                    .is_none_or(|client_type| client_type == &entry.client_type)
            })
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<DownloadClientConfig>> {
        let entries = self.store.lock().await;
        Ok(entries.iter().find(|entry| entry.id == id).cloned())
    }

    async fn create(&self, config: DownloadClientConfig) -> AppResult<DownloadClientConfig> {
        let mut entries = self.store.lock().await;
        entries.push(config.clone());
        Ok(config)
    }

    async fn update(
        &self,
        update: crate::DownloadClientConfigUpdate,
    ) -> AppResult<DownloadClientConfig> {
        let crate::DownloadClientConfigUpdate {
            id,
            name,
            client_type,
            config_json,
            is_enabled,
        } = update;
        let mut entries = self.store.lock().await;
        let item = entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("download client config {id}")))?;

        if let Some(name) = name {
            item.name = name;
        }
        if let Some(client_type) = client_type {
            item.client_type = client_type;
        }
        if let Some(config_json) = config_json {
            item.config_json = config_json;
        }
        if let Some(is_enabled) = is_enabled {
            item.is_enabled = is_enabled;
        }
        item.updated_at = Utc::now();

        Ok(item.clone())
    }

    async fn delete(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let position = entries
            .iter()
            .position(|entry| entry.id == id)
            .ok_or_else(|| AppError::NotFound(format!("download client config {id}")))?;
        entries.remove(position);
        Ok(())
    }

    async fn reorder(&self, ordered_ids: Vec<String>) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        for (index, id) in ordered_ids.iter().enumerate() {
            if let Some(entry) = entries.iter_mut().find(|e| &e.id == id) {
                entry.client_priority = index as i64;
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct MockReleaseAttemptRecord {
    title_id: Option<String>,
    source_hint: Option<String>,
    source_title: Option<String>,
    outcome: ReleaseDownloadAttemptOutcome,
    error_message: Option<String>,
    source_password: Option<String>,
    attempted_at: String,
}

#[derive(Default)]
struct MockReleaseAttemptRepo {
    attempts: Arc<Mutex<Vec<MockReleaseAttemptRecord>>>,
}

#[async_trait]
impl ReleaseAttemptRepository for MockReleaseAttemptRepo {
    async fn record_release_attempt(
        &self,
        title_id: Option<String>,
        source_hint: Option<String>,
        source_title: Option<String>,
        outcome: ReleaseDownloadAttemptOutcome,
        error_message: Option<String>,
        source_password: Option<String>,
    ) -> AppResult<()> {
        self.attempts.lock().await.push(MockReleaseAttemptRecord {
            title_id,
            source_hint,
            source_title,
            outcome,
            error_message,
            source_password,
            attempted_at: Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    async fn list_failed_release_signatures(
        &self,
        limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        let mut attempts: Vec<_> = self
            .attempts
            .lock()
            .await
            .iter()
            .filter(|attempt| attempt.outcome == ReleaseDownloadAttemptOutcome::Failed)
            .cloned()
            .collect();
        attempts.sort_by(|left, right| right.attempted_at.cmp(&left.attempted_at));
        attempts.truncate(limit);

        Ok(attempts
            .into_iter()
            .map(|attempt| ReleaseDownloadFailureSignature {
                source_hint: attempt.source_hint,
                source_title: attempt.source_title,
            })
            .collect())
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        let mut attempts: Vec<_> = self
            .attempts
            .lock()
            .await
            .iter()
            .filter(|attempt| {
                attempt.outcome == ReleaseDownloadAttemptOutcome::Failed
                    && attempt.title_id.as_deref() == Some(title_id)
            })
            .cloned()
            .collect();
        attempts.sort_by(|left, right| right.attempted_at.cmp(&left.attempted_at));
        attempts.truncate(limit);

        Ok(attempts
            .into_iter()
            .map(|attempt| TitleReleaseBlocklistEntry {
                source_hint: attempt.source_hint,
                source_title: attempt.source_title,
                error_message: attempt.error_message,
                attempted_at: attempt.attempted_at,
            })
            .collect())
    }

    async fn get_latest_source_password(
        &self,
        title_id: Option<&str>,
        source_hint: Option<&str>,
        source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        Ok(self
            .attempts
            .lock()
            .await
            .iter()
            .rev()
            .find(|attempt| {
                title_id.is_none_or(|title_id| attempt.title_id.as_deref() == Some(title_id))
                    && source_hint.is_none_or(|source_hint| {
                        attempt.source_hint.as_deref() == Some(source_hint)
                    })
                    && source_title.is_none_or(|source_title| {
                        attempt.source_title.as_deref() == Some(source_title)
                    })
            })
            .and_then(|attempt| attempt.source_password.clone()))
    }
}

#[derive(Default, Clone)]
struct TrackingDownloadSubmissionRepo {
    store: Arc<Mutex<Vec<DownloadSubmission>>>,
    deleted_title_ids: Arc<Mutex<Vec<String>>>,
    list_for_title_calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Default, Clone)]
struct TrackingWantedItemRepo {
    store: Arc<Mutex<Vec<WantedItem>>>,
    release_decisions: Arc<Mutex<Vec<ReleaseDecision>>>,
    title_facets: Arc<Mutex<HashMap<String, MediaFacet>>>,
    upsert_calls: Arc<AtomicUsize>,
}

impl TrackingWantedItemRepo {
    async fn remember_title_facet(&self, title_id: &str, facet: MediaFacet) {
        self.title_facets
            .lock()
            .await
            .insert(title_id.to_string(), facet);
    }

    fn upsert_call_count(&self) -> usize {
        self.upsert_calls.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
struct TrackingAcquisitionStateRepo {
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    wanted_items: Arc<TrackingWantedItemRepo>,
}

#[async_trait]
impl WantedItemRepository for TrackingWantedItemRepo {
    async fn upsert_wanted_item(&self, item: &WantedItem) -> AppResult<String> {
        self.upsert_calls.fetch_add(1, Ordering::SeqCst);
        let mut store = self.store.lock().await;
        if let Some(existing) = store.iter_mut().find(|existing| existing.id == item.id) {
            *existing = item.clone();
        } else {
            store.push(item.clone());
        }
        Ok(item.id.clone())
    }

    async fn list_due_wanted_items(
        &self,
        now: &str,
        batch_limit: i64,
        excluded_facets: &[MediaFacet],
    ) -> AppResult<Vec<WantedItem>> {
        let now = chrono::DateTime::parse_from_rfc3339(now)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let title_facets = self.title_facets.lock().await.clone();
        let mut items: Vec<WantedItem> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                item.status == WantedStatus::Wanted
                    && title_facets
                        .get(&item.title_id)
                        .is_none_or(|facet| !excluded_facets.contains(facet))
                    && item
                        .next_search_at
                        .as_deref()
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&Utc) <= now)
                        .unwrap_or(true)
            })
            .cloned()
            .collect();
        items.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        items.truncate(batch_limit.max(0) as usize);
        Ok(items)
    }

    async fn update_wanted_item_status(
        &self,
        id: &str,
        status: &str,
        next_search_at: Option<&str>,
        last_search_at: Option<&str>,
        search_count: i64,
        current_score: Option<i32>,
        grabbed_release: Option<&str>,
    ) -> AppResult<()> {
        let mut store = self.store.lock().await;
        let item = store
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::NotFound(format!("wanted item {id}")))?;
        item.status = WantedStatus::parse(status)
            .ok_or_else(|| AppError::Repository(format!("invalid wanted status {status}")))?;
        item.next_search_at = next_search_at.map(str::to_string);
        item.last_search_at = last_search_at.map(str::to_string);
        item.search_count = search_count;
        item.current_score = current_score;
        item.grabbed_release = grabbed_release.map(str::to_string);
        item.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn get_wanted_item_for_title(
        &self,
        title_id: &str,
        episode_id: Option<&str>,
    ) -> AppResult<Option<WantedItem>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|item| item.title_id == title_id && item.episode_id.as_deref() == episode_id)
            .cloned())
    }

    async fn delete_wanted_items_for_title(&self, title_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.title_id != title_id);
        Ok(())
    }

    async fn delete_wanted_items_for_collection(&self, collection_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.collection_id.as_deref() != Some(collection_id));
        Ok(())
    }

    async fn delete_wanted_items_for_episode(&self, episode_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|item| item.episode_id.as_deref() != Some(episode_id));
        Ok(())
    }

    async fn reset_fruitless_wanted_items(&self, _now: &str) -> AppResult<u64> {
        Ok(0)
    }

    async fn insert_release_decision(&self, decision: &ReleaseDecision) -> AppResult<String> {
        self.release_decisions.lock().await.push(decision.clone());
        Ok(decision.id.clone())
    }

    async fn get_wanted_item_by_id(&self, id: &str) -> AppResult<Option<WantedItem>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|item| item.id == id)
            .cloned())
    }

    async fn list_wanted_items(&self, query: WantedItemsQuery) -> AppResult<Vec<WantedItem>> {
        let WantedItemsQuery {
            status,
            media_type,
            title_id,
            title_search,
            latest_decision_code,
            limit,
            offset,
        } = query;
        let latest_decisions = self.release_decisions.lock().await.clone();
        let normalized_title_search = title_search.as_deref().map(str::to_lowercase);
        let items: Vec<WantedItem> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                let latest_decision = latest_decisions
                    .iter()
                    .filter(|decision| decision.wanted_item_id == item.id)
                    .max_by(|left, right| left.created_at.cmp(&right.created_at));
                status
                    .as_deref()
                    .is_none_or(|status| item.status.as_str() == status)
                    && media_type
                        .as_deref()
                        .is_none_or(|media_type| item.media_type == media_type)
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && latest_decision_code.as_deref().is_none_or(|code| {
                        latest_decision
                            .as_ref()
                            .is_some_and(|decision| decision.decision_code == code)
                    })
            })
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect();
        Ok(items)
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        let WantedItemsQuery {
            status,
            media_type,
            title_id,
            title_search,
            latest_decision_code,
            ..
        } = query;
        let latest_decisions = self.release_decisions.lock().await.clone();
        let normalized_title_search = title_search.as_deref().map(str::to_lowercase);
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                let latest_decision = latest_decisions
                    .iter()
                    .filter(|decision| decision.wanted_item_id == item.id)
                    .max_by(|left, right| left.created_at.cmp(&right.created_at));
                status
                    .as_deref()
                    .is_none_or(|status| item.status.as_str() == status)
                    && media_type
                        .as_deref()
                        .is_none_or(|media_type| item.media_type == media_type)
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && latest_decision_code.as_deref().is_none_or(|code| {
                        latest_decision
                            .as_ref()
                            .is_some_and(|decision| decision.decision_code == code)
                    })
            })
            .count() as i64)
    }

    async fn list_release_decisions_for_title(
        &self,
        title_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.title_id == title_id)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }

    async fn list_release_decisions_for_wanted_item(
        &self,
        wanted_item_id: &str,
        limit: i64,
    ) -> AppResult<Vec<ReleaseDecision>> {
        Ok(self
            .release_decisions
            .lock()
            .await
            .iter()
            .filter(|decision| decision.wanted_item_id == wanted_item_id)
            .take(limit.max(0) as usize)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl AcquisitionStateRepository for TrackingAcquisitionStateRepo {
    async fn commit_successful_grab(&self, commit: &SuccessfulGrabCommit) -> AppResult<()> {
        self.download_submissions
            .record_submission(commit.download_submission.clone())
            .await?;

        let mut covered_wanted_item_ids = commit.covered_wanted_item_ids.clone();
        if !covered_wanted_item_ids
            .iter()
            .any(|id| id == &commit.wanted_item_id)
        {
            covered_wanted_item_ids.push(commit.wanted_item_id.clone());
        }
        covered_wanted_item_ids.sort();
        covered_wanted_item_ids.dedup();
        for wanted_item_id in &covered_wanted_item_ids {
            self.wanted_items
                .update_wanted_item_status(
                    wanted_item_id,
                    WantedStatus::Grabbed.as_str(),
                    None,
                    commit.last_search_at.as_deref(),
                    commit.search_count,
                    commit.current_score,
                    Some(&commit.grabbed_release),
                )
                .await?;
        }

        if let Some(pending_release_id) = commit.grabbed_pending_release_id.as_deref() {
            self.pending_releases
                .update_pending_release_status(
                    pending_release_id,
                    PendingReleaseStatus::Grabbed,
                    commit.grabbed_at.as_deref(),
                )
                .await?;
        }

        let mut store = self.pending_releases.store.lock().await;
        for release in store.iter_mut() {
            let is_sibling = covered_wanted_item_ids
                .iter()
                .any(|wanted_item_id| wanted_item_id == &release.wanted_item_id)
                && commit
                    .grabbed_pending_release_id
                    .as_deref()
                    .is_none_or(|pending_release_id| release.id != pending_release_id);
            let should_supersede = matches!(
                release.status,
                PendingReleaseStatus::Waiting | PendingReleaseStatus::Standby
            );
            if is_sibling && should_supersede {
                release.status = PendingReleaseStatus::Superseded;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl DownloadSubmissionRepository for TrackingDownloadSubmissionRepo {
    async fn record_submission(&self, submission: DownloadSubmission) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        if let Some(existing) = entries.iter_mut().find(|entry| {
            entry.download_client_id == submission.download_client_id
                && entry.download_client_type == submission.download_client_type
                && entry.download_client_item_id == submission.download_client_item_id
        }) {
            *existing = submission;
        } else {
            entries.push(submission);
        }
        Ok(())
    }

    async fn find_by_client_item_id(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .find(|entry| {
                entry.download_client_id.as_deref().unwrap_or("").trim()
                    == identity.client_id.as_deref().unwrap_or("")
                    && entry.download_client_type == identity.client_type.as_str()
                    && entry.download_client_item_id == identity.item_id.as_str()
            })
            .cloned())
    }

    async fn list_for_client_items(
        &self,
        client_items: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| {
                client_items.iter().any(|identity| {
                    entry.download_client_id.as_deref().unwrap_or("").trim()
                        == identity.client_id.as_deref().unwrap_or("")
                        && entry.download_client_type == identity.client_type.as_str()
                        && entry.download_client_item_id == identity.item_id.as_str()
                })
            })
            .cloned()
            .collect())
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        self.list_for_title_calls
            .lock()
            .await
            .push(title_id.to_string());
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn find_by_title_and_request_signature(
        &self,
        title_id: &str,
        request_signature: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .find(|entry| {
                entry.title_id == title_id
                    && entry.request_signature.as_deref() == Some(request_signature)
            })
            .cloned())
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.deleted_title_ids
            .lock()
            .await
            .push(title_id.to_string());
        self.store
            .lock()
            .await
            .retain(|entry| entry.title_id != title_id);
        Ok(())
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        self.store.lock().await.retain(|entry| {
            entry.download_client_id.as_deref().unwrap_or("").trim()
                != identity.client_id.as_deref().unwrap_or("")
                || entry.download_client_type != identity.client_type.as_str()
                || entry.download_client_item_id != identity.item_id.as_str()
        });
        Ok(())
    }

    async fn update_tracked_state(&self, _: &DownloadSourceIdentity, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn get_tracked_state(&self, _: &DownloadSourceIdentity) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Default, Clone)]
struct TrackingPendingReleaseRepo {
    store: Arc<Mutex<Vec<PendingRelease>>>,
    deleted_title_ids: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl PendingReleaseRepository for TrackingPendingReleaseRepo {
    async fn insert_pending_release(&self, release: &PendingRelease) -> AppResult<String> {
        self.store.lock().await.push(release.clone());
        Ok(release.id.clone())
    }

    async fn list_expired_pending_releases(&self, _: &str) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }

    async fn list_waiting_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(vec![])
    }

    async fn get_pending_release(&self, id: &str) -> AppResult<Option<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .find(|release| release.id == id)
            .cloned())
    }

    async fn list_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| {
                release.wanted_item_id == wanted_item_id
                    && release.status == PendingReleaseStatus::Waiting
            })
            .cloned()
            .collect())
    }

    async fn list_pending_releases_for_title(
        &self,
        title_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.title_id == title_id)
            .cloned()
            .collect())
    }

    async fn update_pending_release_status(
        &self,
        id: &str,
        status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<()> {
        if let Some(release) = self
            .store
            .lock()
            .await
            .iter_mut()
            .find(|release| release.id == id)
        {
            release.status = status;
            release.grabbed_at = grabbed_at.map(str::to_string);
        }
        Ok(())
    }

    async fn list_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| {
                release.wanted_item_id == wanted_item_id
                    && release.status == PendingReleaseStatus::Standby
            })
            .cloned()
            .collect())
    }

    async fn delete_standby_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
    ) -> AppResult<()> {
        self.store.lock().await.retain(|release| {
            !(release.wanted_item_id == wanted_item_id
                && release.status == PendingReleaseStatus::Standby)
        });
        Ok(())
    }

    async fn list_all_standby_pending_releases(&self) -> AppResult<Vec<PendingRelease>> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|release| release.status == PendingReleaseStatus::Standby)
            .cloned()
            .collect())
    }

    async fn compare_and_set_pending_release_status(
        &self,
        id: &str,
        current_status: PendingReleaseStatus,
        next_status: PendingReleaseStatus,
        grabbed_at: Option<&str>,
    ) -> AppResult<bool> {
        let mut store = self.store.lock().await;
        let Some(release) = store.iter_mut().find(|release| release.id == id) else {
            return Ok(false);
        };
        if release.status != current_status {
            return Ok(false);
        }
        release.status = next_status;
        release.grabbed_at = grabbed_at.map(str::to_string);
        Ok(true)
    }

    async fn supersede_pending_releases_for_wanted_item(
        &self,
        wanted_item_id: &str,
        except_id: &str,
    ) -> AppResult<()> {
        for release in self.store.lock().await.iter_mut() {
            if release.wanted_item_id == wanted_item_id
                && release.id != except_id
                && release.status == PendingReleaseStatus::Waiting
            {
                release.status = PendingReleaseStatus::Superseded;
            }
        }
        Ok(())
    }

    async fn delete_pending_releases_for_title(&self, title_id: &str) -> AppResult<()> {
        self.deleted_title_ids
            .lock()
            .await
            .push(title_id.to_string());
        Ok(())
    }
}

#[derive(Default, Clone)]
struct StubDownloadClient {
    queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    history_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    deleted_items: Arc<Mutex<Vec<(String, bool)>>>,
    delete_error: Arc<Mutex<Option<String>>>,
    submitted_release_titles: Arc<Mutex<Vec<String>>>,
    queue_calls: Arc<Mutex<usize>>,
    queue_for_title_calls: Arc<Mutex<Vec<String>>>,
    history_calls: Arc<Mutex<usize>>,
    recent_activity_calls: Arc<Mutex<Vec<usize>>>,
    recent_activity_for_title_calls: Arc<Mutex<Vec<(String, usize)>>>,
}

impl StubDownloadClient {
    async fn set_delete_error(&self, error: Option<&str>) {
        *self.delete_error.lock().await = error.map(str::to_string);
    }
}

#[async_trait]
impl DownloadClient for StubDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        let job_id = format!("job-for-{}", request.title.id);
        self.submitted_release_titles.lock().await.push(
            request
                .release_title
                .clone()
                .unwrap_or_else(|| request.title.name.clone()),
        );
        let mut queue_items = self.queue_items.lock().await;
        if !queue_items
            .iter()
            .any(|item| item.download_client_item_id == job_id)
        {
            let mut queued = queue_history_fixture_item(&job_id, DownloadQueueState::Queued, 0);
            queued.title_id = Some(request.title.id.clone());
            queued.title_name = request.title.name.clone();
            queued.facet = Some(request.title.facet.as_str().to_string());
            queue_items.push(queued);
        }
        Ok(DownloadGrabResult {
            job_id,
            client_id: None,
            client_type: "nzbget".to_string(),
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        *self.queue_calls.lock().await += 1;
        Ok(self.queue_items.lock().await.clone())
    }

    async fn list_queue_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadQueueItem>> {
        self.queue_for_title_calls
            .lock()
            .await
            .push(title_id.to_string());
        Ok(self.queue_items.lock().await.clone())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        *self.history_calls.lock().await += 1;
        Ok(self.history_items.lock().await.clone())
    }

    async fn list_recent_activity(&self, limit: usize) -> AppResult<Vec<DownloadQueueItem>> {
        self.recent_activity_calls.lock().await.push(limit);
        Ok(self
            .history_items
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_recent_activity_for_title(
        &self,
        title_id: &str,
        limit: usize,
    ) -> AppResult<Vec<DownloadQueueItem>> {
        self.recent_activity_for_title_calls
            .lock()
            .await
            .push((title_id.to_string(), limit));
        Ok(self
            .history_items
            .lock()
            .await
            .iter()
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_completed_downloads(&self) -> AppResult<Vec<CompletedDownload>> {
        Ok(self.completed_downloads.lock().await.clone())
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        if let Some(error) = self.delete_error.lock().await.clone() {
            return Err(AppError::Repository(error));
        }
        self.deleted_items
            .lock()
            .await
            .push((id.to_string(), is_history));
        Ok(())
    }
}

#[derive(Default)]
struct TrackingDownloadQueueCommandRepo {
    queued: Arc<Mutex<Vec<DownloadQueueCommandRecord>>>,
    recovered_count: Arc<Mutex<u64>>,
}

impl TrackingDownloadQueueCommandRepo {
    async fn seed_pending(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
    ) -> String {
        let id = format!("delete-command-{download_client_item_id}");
        self.queued.lock().await.push(DownloadQueueCommandRecord {
            id: id.clone(),
            action: scryer_domain::DownloadQueueCommandAction::Delete,
            client_id: client_id.map(str::to_string),
            client_type: client_type.to_string(),
            download_client_item_id: download_client_item_id.to_string(),
            is_history,
            status: scryer_domain::DownloadQueueDeleteStatus::Queued,
            error_text: None,
            requested_by_user_id: Some("admin".to_string()),
            started_at: None,
            finished_at: None,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        });
        id
    }

    async fn get(&self, id: &str) -> Option<DownloadQueueCommandRecord> {
        self.queued
            .lock()
            .await
            .iter()
            .find(|record| record.id == id)
            .cloned()
    }
}

#[async_trait]
impl DownloadQueueCommandRepository for TrackingDownloadQueueCommandRepo {
    async fn queue_delete_command(
        &self,
        client_id: Option<&str>,
        client_type: &str,
        download_client_item_id: &str,
        is_history: bool,
        requested_by_user_id: Option<&str>,
    ) -> AppResult<DownloadQueueCommandRecord> {
        let id = self
            .seed_pending(client_id, client_type, download_client_item_id, is_history)
            .await;
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .expect("seeded queued delete command");
        record.requested_by_user_id = requested_by_user_id.map(str::to_string);
        Ok(record.clone())
    }

    async fn recover_stale_running_delete_commands(&self, _stale_seconds: i64) -> AppResult<u64> {
        Ok(*self.recovered_count.lock().await)
    }

    async fn list_pending_delete_commands(&self) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        Ok(self
            .queued
            .lock()
            .await
            .iter()
            .filter(|record| record.status == scryer_domain::DownloadQueueDeleteStatus::Queued)
            .cloned()
            .collect())
    }

    async fn mark_delete_command_running(&self, id: &str) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Running;
        record.started_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn mark_delete_command_completed(&self, id: &str) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Completed;
        record.finished_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn mark_delete_command_failed(
        &self,
        id: &str,
        error_text: Option<&str>,
    ) -> AppResult<()> {
        let mut queued = self.queued.lock().await;
        let record = queued
            .iter_mut()
            .find(|record| record.id == id)
            .ok_or_else(|| AppError::NotFound(format!("queued delete {}", id)))?;
        record.status = scryer_domain::DownloadQueueDeleteStatus::Failed;
        record.error_text = error_text.map(str::to_string);
        record.finished_at = Some(Utc::now().to_rfc3339());
        record.updated_at = Utc::now().to_rfc3339();
        Ok(())
    }

    async fn list_latest_delete_commands_for_sources(
        &self,
        sources: &[(Option<String>, String, String, bool)],
    ) -> AppResult<Vec<DownloadQueueCommandRecord>> {
        let queued = self.queued.lock().await;
        Ok(sources
            .iter()
            .filter_map(|(client_id, client_type, item_id, is_history)| {
                queued
                    .iter()
                    .find(|record| {
                        record.client_id.as_deref() == client_id.as_deref()
                            && record.client_type == *client_type
                            && record.download_client_item_id == *item_id
                            && record.is_history == *is_history
                    })
                    .cloned()
            })
            .collect())
    }

    async fn prune_terminal_delete_commands_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }
}

pub(crate) fn bootstrap() -> (AppUseCase, User) {
    bootstrap_with_user_repo(Arc::new(MockUserRepo::default()))
}

async fn title_updated_events(app: &AppUseCase, title_id: &str) -> Vec<DomainEvent> {
    app.services
        .events
        .domain_events
        .list(&DomainEventFilter {
            event_types: Some(vec![DomainEventType::TitleUpdated]),
            title_id: Some(title_id.to_string()),
            facet: None,
            after_sequence: Some(0),
            before_sequence: None,
            limit: 100,
        })
        .await
        .expect("title updated events should load")
}

fn bootstrap_with_user_repo(users: Arc<MockUserRepo>) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .build_partial_for_tests();
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"))
}

fn bootstrap_with_metadata_gateway_and_titles(
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_metadata_gateway(metadata_gateway)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"), titles)
}

fn make_due_hydration_title(id: &str, facet: MediaFacet, tvdb_id: i64) -> Title {
    Title {
        id: id.to_string(),
        name: format!("Title {id}"),
        facet,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: tvdb_id.to_string(),
        }],
        created_by: None,
        created_at: chrono::Utc::now(),
        year: Some(2026),
        overview: None,
        poster_url: None,
        poster_source_url: None,
        banner_url: None,
        banner_source_url: None,
        background_url: None,
        background_source_url: None,
        sort_title: None,
        slug: None,
        imdb_id: None,
        runtime_minutes: None,
        genres: vec![],
        content_status: None,
        language: None,
        first_aired: None,
        network: None,
        studio: None,
        country: None,
        aliases: vec![],
        tagged_aliases: vec![],
        metadata_language: None,
        metadata_fetched_at: None,
        min_availability: None,
        digital_release_date: None,
        folder_path: None,
    }
}

fn make_movie_metadata(tvdb_id: i64, name: &str) -> MovieMetadata {
    MovieMetadata {
        tvdb_id,
        name: name.to_string(),
        slug: name.to_ascii_lowercase().replace(' ', "-"),
        year: Some(2026),
        content_status: "Released".to_string(),
        overview: format!("{name} overview"),
        poster_url: format!("https://example.com/{tvdb_id}.jpg"),
        banner_url: None,
        background_url: None,
        language: "eng".to_string(),
        runtime_minutes: 100,
        sort_title: name.to_string(),
        imdb_id: format!("tt{tvdb_id:07}"),
        anidb_id: None,
        genres: vec!["Drama".to_string()],
        studio: "Test Studio".to_string(),
        tmdb_release_date: Some("2026-01-01".to_string()),
    }
}

fn bootstrap_with_cleanup_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
) -> (AppUseCase, User) {
    bootstrap_with_cleanup_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        Arc::new(MockIndexerClient),
    )
}

fn bootstrap_with_cleanup_tracking_and_tracked_handle(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    tracked_download_handle: crate::tracked_downloads::TrackedDownloadHandle,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        Arc::new(MockIndexerClient),
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions)
    .with_pending_releases(pending_releases)
    .with_tracked_download_handle(tracked_download_handle)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"))
}

fn bootstrap_with_cleanup_tracking_and_indexer(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_submissions(download_submissions)
    .with_pending_releases(pending_releases)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"))
}

fn bootstrap_with_search_settings_and_indexer(
    settings: Arc<StoredSettingsRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"))
}

#[tokio::test]
async fn remove_completed_download_defaults_true_when_scope_has_no_saved_entry() {
    // Legacy-compat coverage: a stored scope JSON exists but does not include
    // an entry for "weaver". Read path must fall back to the canonical
    // defaults (`removeCompleted=true`, `removeFailed=false`). New installs
    // converge on fully-materialized entries via `normalize_routing_settings`.
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "other-client": {
                    "enabled": true,
                    "removeCompleted": false,
                    "removeFailed": true
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings, Arc::new(MockIndexerClient));

    assert!(
        app.should_remove_completed_download(&MediaFacet::Movie, "weaver")
            .await
    );
    assert!(
        !app.should_remove_failed_download(&MediaFacet::Movie, "weaver")
            .await
    );
}

#[tokio::test]
async fn ensure_download_client_routing_entry_for_client_writes_full_default_entry() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.ensure_download_client_routing_entry_for_client(&actor, "weaver")
        .await
        .expect("ensure routing entry");

    for scope_id in ["movie", "series", "anime"] {
        let raw = settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                scope_id,
            )
            .await
            .unwrap_or_else(|| panic!("expected routing JSON for scope {scope_id}"));
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
        let entry = parsed
            .get("weaver")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("expected weaver entry for scope {scope_id}"));
        assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("category"), Some(&serde_json::json!("")));
        assert_eq!(
            entry.get("recentQueuePriority"),
            Some(&serde_json::json!(""))
        );
        assert_eq!(
            entry.get("olderQueuePriority"),
            Some(&serde_json::json!(""))
        );
        assert_eq!(entry.get("removeCompleted"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(false)));
        assert!(entry.contains_key("priority"));
    }
}

#[tokio::test]
async fn normalize_routing_settings_backfills_partial_legacy_download_client_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "category": "",
                    "priority": 1
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize routing settings");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
        )
        .await
        .expect("routing JSON present after normalize");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("weaver")
        .and_then(|v| v.as_object())
        .expect("weaver entry");
    assert_eq!(entry.get("removeCompleted"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(false)));
    assert_eq!(
        entry.get("recentQueuePriority"),
        Some(&serde_json::json!(""))
    );
    assert_eq!(
        entry.get("olderQueuePriority"),
        Some(&serde_json::json!(""))
    );
    // Existing explicit values must not be overwritten.
    assert_eq!(entry.get("priority"), Some(&serde_json::json!(1)));
    assert_eq!(entry.get("category"), Some(&serde_json::json!("")));
}

#[tokio::test]
async fn normalize_routing_settings_is_idempotent_for_complete_entries() {
    // Pre-seed a fully-normalized entry with non-default values. The normalize
    // pass must not overwrite explicit values back to canonical defaults.
    let original = serde_json::json!({
        "weaver": {
            "enabled": false,
            "category": "movies",
            "recentQueuePriority": "high",
            "olderQueuePriority": "low",
            "removeCompleted": false,
            "removeFailed": true,
            "priority": 7
        }
    })
    .to_string();
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &original,
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("first normalize");
    app.normalize_routing_settings()
        .await
        .expect("second normalize");

    let after = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
        )
        .await
        .expect("routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&after).expect("routing JSON parses");
    let entry = parsed
        .get("weaver")
        .and_then(|v| v.as_object())
        .expect("weaver entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(false)));
    assert_eq!(entry.get("category"), Some(&serde_json::json!("movies")));
    assert_eq!(
        entry.get("recentQueuePriority"),
        Some(&serde_json::json!("high"))
    );
    assert_eq!(
        entry.get("olderQueuePriority"),
        Some(&serde_json::json!("low"))
    );
    assert_eq!(
        entry.get("removeCompleted"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(entry.get("removeFailed"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("priority"), Some(&serde_json::json!(7)));
}

#[tokio::test]
async fn ensure_indexer_routing_entry_for_indexer_writes_full_default_entry() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.ensure_indexer_routing_entry_for_indexer(&actor, "indexer-1")
        .await
        .expect("ensure indexer routing entry");

    for (scope_id, expected_categories) in [
        ("movie", serde_json::json!(["2000"])),
        ("series", serde_json::json!(["5000"])),
        ("anime", serde_json::json!(["5070"])),
    ] {
        let raw = settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                INDEXER_ROUTING_SETTINGS_KEY,
                scope_id,
            )
            .await
            .unwrap_or_else(|| panic!("expected indexer routing JSON for scope {scope_id}"));
        let parsed: serde_json::Value =
            serde_json::from_str(&raw).expect("indexer routing JSON parses");
        let entry = parsed
            .get("indexer-1")
            .and_then(|v| v.as_object())
            .unwrap_or_else(|| panic!("expected indexer-1 entry for scope {scope_id}"));
        assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(entry.get("categories"), Some(&expected_categories));
        assert!(entry.contains_key("priority"));
    }
}

#[tokio::test]
async fn create_indexer_config_writes_default_routing_entries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, actor) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    let created = app
        .create_indexer_config(
            &actor,
            NewIndexerConfig {
                name: "NZBGeek".to_string(),
                provider_type: "nzbgeek".to_string(),
                base_url: "https://api.nzbgeek.info".to_string(),
                api_key_encrypted: Some("0123456789abcdef".to_string()),
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                config_json: None,
            },
        )
        .await
        .expect("create indexer config");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
        )
        .await
        .expect("series indexer routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get(&created.id)
        .and_then(|value| value.as_object())
        .expect("created indexer routing entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5000"])));
    assert!(entry.contains_key("priority"));
}

#[tokio::test]
async fn ensure_indexer_routing_entries_for_existing_indexers_backfills_missing_rows() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.services
        .integrations
        .indexer_configs
        .create(IndexerConfig {
            id: "existing-indexer".to_string(),
            name: "NZBGeek".to_string(),
            provider_type: "nzbgeek".to_string(),
            base_url: "https://api.nzbgeek.info".to_string(),
            api_key_encrypted: Some("0123456789abcdef".to_string()),
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            last_health_status: None,
            last_error_at: None,
            config_json: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("seed existing indexer config");

    app.ensure_indexer_routing_entries_for_existing_indexers()
        .await
        .expect("ensure existing indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "anime")
        .await
        .expect("anime indexer routing JSON present");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("existing-indexer")
        .and_then(|value| value.as_object())
        .expect("existing indexer routing entry");
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5070"])));
}

#[tokio::test]
async fn normalize_routing_settings_backfills_missing_indexer_categories_from_scope() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "anime",
            &serde_json::json!({
                "indexer-1": {
                    "enabled": true
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "anime")
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("routing JSON parses");
    let entry = parsed
        .get("indexer-1")
        .and_then(|value| value.as_object())
        .expect("indexer-1 entry");
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["5070"])));
    assert!(entry.contains_key("priority"));
}

#[tokio::test]
async fn normalize_routing_settings_backfills_partial_legacy_indexer_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "indexer-1": {
                    "categories": ["2000"]
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(SETTINGS_SCOPE_SYSTEM, INDEXER_ROUTING_SETTINGS_KEY, "movie")
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("indexer routing JSON parses");
    let entry = parsed
        .get("indexer-1")
        .and_then(|v| v.as_object())
        .expect("indexer-1 entry");
    assert_eq!(entry.get("enabled"), Some(&serde_json::json!(true)));
    assert!(entry.contains_key("priority"));
    // Existing categories must not be overwritten.
    assert_eq!(entry.get("categories"), Some(&serde_json::json!(["2000"])));
}

#[tokio::test]
async fn normalize_routing_settings_assigns_distinct_priorities_to_multiple_indexers() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
            &serde_json::json!({
                "indexer-1": {
                    "enabled": true,
                    "categories": ["5000"]
                },
                "indexer-2": {
                    "enabled": true,
                    "categories": ["5000"]
                }
            })
            .to_string(),
        )
        .await;

    let (app, _) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));

    app.normalize_routing_settings()
        .await
        .expect("normalize indexer routing");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
        )
        .await
        .expect("indexer routing JSON present after normalize");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("indexer routing JSON parses");
    let first_priority = parsed["indexer-1"]["priority"]
        .as_i64()
        .expect("indexer-1 priority");
    let second_priority = parsed["indexer-2"]["priority"]
        .as_i64()
        .expect("indexer-2 priority");

    assert_ne!(first_priority, second_priority);
}

fn bootstrap_with_cutoff_projection_state(
    settings: Arc<StoredSettingsRepo>,
    quality_profiles: Arc<StoredQualityProfileRepo>,
    media_files: Arc<MockMediaFileRepo>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_media_files(media_files)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"), titles)
}

fn bootstrap_with_delete_queue(
    download_client: Arc<StubDownloadClient>,
    download_queue_commands: Arc<TrackingDownloadQueueCommandRepo>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let indexer_client = Arc::new(MockIndexerClient);

    let services = AppServices::builder(
        titles,
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_download_queue_commands(download_queue_commands)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));
    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"))
}

fn bootstrap_with_acquisition_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    wanted_items: Arc<TrackingWantedItemRepo>,
) -> (AppUseCase, User) {
    bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items,
        Arc::new(MockIndexerClient),
    )
}

fn bootstrap_with_acquisition_tracking_and_indexer(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    wanted_items: Arc<TrackingWantedItemRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases.clone(),
        indexer_client,
    );
    let app = app.with_test_overrides(|services| {
        services
            .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
                download_submissions,
                pending_releases,
                wanted_items: wanted_items.clone(),
            }))
            .with_wanted_items(wanted_items)
    });
    (app, user)
}

fn bootstrap_with_scan_unmatched_tracking(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
) -> (AppUseCase, User) {
    let (app, user, _) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    );
    (app, user)
}

fn bootstrap_with_scan_unmatched_and_metadata_tracking(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User) {
    let (app, user, _) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items,
        metadata_gateway,
    );
    (app, user)
}

fn bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User, Arc<MockTitleRepo>) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let media_files = Arc::new(MockMediaFileRepo::default());

    let services = AppServices::builder(
        titles.clone(),
        shows,
        users,
        indexer_configs,
        indexer_client,
        download_client,
        download_client_configs,
        release_attempts,
        settings,
        quality_profiles,
        String::new(),
    )
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_metadata_gateway(metadata_gateway)
    .with_library_scanner(library_scanner)
    .with_media_files(media_files)
    .with_library_scan_unmatched_items(unmatched_items)
    .build_partial_for_tests();

    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    let app = AppUseCase::new(
        services,
        JwtAuthConfig {
            issuer: "scryer-test".to_string(),
            access_ttl_seconds: 3600,
            jwt_signing_salt: "test-salt".to_string(),
        },
        Arc::new(registry),
    );

    (app, User::new_admin("admin"), titles)
}

struct FixedBatchSearchMetadataGateway {
    results: Vec<MetadataSearchItem>,
}

#[async_trait]
impl MetadataGateway for FixedBatchSearchMetadataGateway {
    async fn search_tvdb(
        &self,
        _query: &str,
        _type_hint: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<MetadataSearchItem>> {
        Ok(self.results.clone())
    }

    async fn search_tvdb_batch(
        &self,
        queries: &[MetadataSearchQuery],
        _language: &str,
    ) -> AppResult<HashMap<MetadataSearchQuery, Vec<MetadataSearchItem>>> {
        Ok(queries
            .iter()
            .cloned()
            .map(|query| (query, self.results.clone()))
            .collect())
    }

    async fn search_tvdb_rich(
        &self,
        _query: &str,
        _type_hint: &str,
        _limit: i32,
        _language: &str,
        _year: Option<i32>,
    ) -> AppResult<Vec<RichMetadataSearchItem>> {
        Ok(Vec::new())
    }

    async fn search_tvdb_multi(
        &self,
        _query: &str,
        _limit: i32,
        _language: &str,
    ) -> AppResult<MultiMetadataSearchResult> {
        Ok(MultiMetadataSearchResult::default())
    }

    async fn get_movie(&self, _tvdb_id: i64, _language: &str) -> AppResult<MovieMetadata> {
        Err(AppError::NotFound(
            "movie metadata unavailable in test".into(),
        ))
    }

    async fn get_series(&self, _tvdb_id: i64, _language: &str) -> AppResult<SeriesMetadata> {
        Err(AppError::NotFound(
            "series metadata unavailable in test".into(),
        ))
    }

    async fn get_metadata_bulk(
        &self,
        _movie_tvdb_ids: &[i64],
        _series_tvdb_ids: &[i64],
        _language: &str,
    ) -> AppResult<BulkMetadataResult> {
        Ok(BulkMetadataResult {
            movies: HashMap::new(),
            series: HashMap::new(),
        })
    }
}

fn build_test_library_file(path: &str) -> LibraryFile {
    LibraryFile {
        path: path.to_string(),
        display_name: Path::new(path)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        nfo_path: None,
        size_bytes: None,
        source_signature_scheme: None,
        source_signature_value: None,
    }
}

fn build_test_unmatched_item(
    id: &str,
    facet: MediaFacet,
    scan_root: &str,
    item_path: &str,
    display_name: &str,
    query: &str,
    year_hint: Option<i32>,
) -> LibraryScanUnmatchedItem {
    let timestamp = chrono::Utc::now().to_rfc3339();
    LibraryScanUnmatchedItem {
        id: id.to_string(),
        facet,
        status: PendingImportStatus::Pending,
        title_id: None,
        scan_session_id: "scan-session-1".to_string(),
        scan_root: scan_root.to_string(),
        item_path: item_path.to_string(),
        display_name: display_name.to_string(),
        query: query.to_string(),
        year_hint,
        reason_code: "no_metadata_match".to_string(),
        error_message: None,
        search_attempts: vec![],
        created_at: timestamp.clone(),
        updated_at: timestamp,
    }
}

fn build_root_folder_entry(path: &Path, is_default: bool) -> RootFolderEntry {
    RootFolderEntry {
        path: path.to_string_lossy().to_string(),
        is_default,
    }
}

async fn wait_for_projected_library_scan_session_matching<F>(
    app: &AppUseCase,
    session_id: &str,
    predicate: F,
) -> LibraryScanSession
where
    F: Fn(&LibraryScanSession) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        if let Some(session) =
            crate::library_scan_coordinator::load_projected_library_scan_session(app, session_id)
                .await
                .expect("projected library scan session")
            && predicate(&session)
        {
            return session;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for projected session {session_id} to satisfy predicate",
        );
        sleep(Duration::from_millis(10)).await;
    }
}

fn empty_update_media_settings_with_roots(
    root_folders: Vec<RootFolderEntry>,
) -> UpdateMediaSettings {
    UpdateMediaSettings {
        library_path: None,
        root_folders: Some(root_folders),
        required_audio_languages: None,
        rename_template: None,
        rename_collision_policy: None,
        rename_missing_metadata_policy: None,
        filler_policy: None,
        recap_policy: None,
        monitor_specials: None,
        inter_season_movies: None,
        monitor_filler_movies: None,
        nfo_write_on_import: None,
        plexmatch_write_on_import: None,
    }
}

#[tokio::test]
async fn movie_full_scan_persists_and_reconciles_unmatched_items() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let first_path = tempdir.path().join("Unknown.One.2020.1080p.WEB-DL.mkv");
    std::fs::write(&first_path, b"movie").expect("write first movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        library_scanner.clone(),
        unmatched_items.clone(),
    );

    let first_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("first movie scan");
    assert_eq!(first_summary.scanned, 1);
    assert_eq!(first_summary.unmatched, 1);

    let first_items = unmatched_items.items().await;
    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].facet, MediaFacet::Movie);
    assert_eq!(first_items[0].item_path, first_path.to_string_lossy());
    let first_session_id = first_items[0].scan_session_id.clone();

    let second_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("second movie scan");
    assert_eq!(second_summary.unmatched, 1);

    let second_items = unmatched_items.items().await;
    assert_eq!(second_items.len(), 1);
    assert_ne!(second_items[0].scan_session_id, first_session_id);

    std::fs::remove_file(&first_path).expect("remove first movie file");
    let second_path = tempdir.path().join("Unknown.Two.2021.2160p.BluRay.mkv");
    std::fs::write(&second_path, b"movie").expect("write second movie file");
    let third_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("third movie scan");
    assert_eq!(third_summary.scanned, 1);
    assert_eq!(third_summary.unmatched, 1);

    let third_items = unmatched_items.items().await;
    assert_eq!(third_items.len(), 1);
    assert_eq!(third_items[0].item_path, second_path.to_string_lossy());
}

#[tokio::test]
async fn movie_title_scan_removes_missing_tracked_movie_file() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Titanic (1997) - 2160p.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Titanic".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                year: Some(1997),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    app.services
        .catalog
        .titles
        .set_folder_path(&title.id, tempdir.path().to_string_lossy().as_ref())
        .await
        .expect("set movie folder path");

    let movie_path_string = movie_path.to_string_lossy().to_string();
    app.services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Movie,
            collection_index: "1".to_string(),
            label: Some("2160p".to_string()),
            ordered_path: Some(movie_path_string.clone()),
            narrative_order: None,
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: title.monitored,
            created_at: Utc::now(),
        })
        .await
        .expect("seed movie collection");
    app.services
        .library
        .media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: movie_path_string,
            size_bytes: 5,
            quality_label: Some("2160p".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed movie media file");

    std::fs::remove_file(&movie_path).expect("remove movie file externally");

    let summary = app
        .scan_title_library(&user, &title.id)
        .await
        .expect("movie title scan should succeed");

    assert_eq!(summary.imported, 0);
    assert_eq!(summary.skipped, 0);
    assert!(
        app.services
            .library
            .media_files
            .list_media_files_for_title(&title.id)
            .await
            .expect("list media files")
            .is_empty()
    );
    assert!(
        app.services
            .catalog
            .shows
            .list_collections_for_title(&title.id)
            .await
            .expect("list collections")
            .is_empty()
    );
}

#[tokio::test]
async fn movie_full_scan_title_create_failure_from_nfo_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Broken.Movie.2020.mkv");
    let nfo_path = tempdir.path().join("movie.nfo");
    std::fs::write(&movie_path, b"movie").expect("write movie file");
    std::fs::write(
        &nfo_path,
        r#"<movie><title>Broken Movie</title><tvdbid>123456</tvdbid></movie>"#,
    )
    .expect("write nfo");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![LibraryFile {
            path: movie_path.to_string_lossy().to_string(),
            display_name: "Broken.Movie.2020".to_string(),
            nfo_path: Some(nfo_path.to_string_lossy().to_string()),
            size_bytes: None,
            source_signature_scheme: None,
            source_signature_value: None,
        }])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    titles
        .fail_create_or_get_existing("forced movie title creation failure from nfo")
        .await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles(&user, Some(MediaFacet::Movie), None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "title_create_from_nfo_failed");
    assert_eq!(
        items[0].error_message.as_deref(),
        Some("repository: forced movie title creation failure from nfo")
    );
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn movie_full_scan_title_create_failure_from_search_persists_unmatched_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Matched.Movie.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("write movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user, titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(FixedBatchSearchMetadataGateway {
            results: vec![MetadataSearchItem {
                tvdb_id: "123456".to_string(),
                name: "Matched Movie".to_string(),
                year: Some(2020),
            }],
        }),
    );
    titles
        .fail_create_or_get_existing("forced movie title creation failure from search")
        .await;

    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles(&user, Some(MediaFacet::Movie), None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "title_create_from_search_failed");
    assert_eq!(
        items[0].error_message.as_deref(),
        Some("repository: forced movie title creation failure from search")
    );
    assert_eq!(items[0].item_path, movie_path.to_string_lossy());
}

#[tokio::test]
async fn series_full_scan_persists_unmatched_folders() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tempdir.path().join("Unknown Show (2020)"))
        .expect("create unknown show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series scan");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].display_name, "Unknown Show (2020)");
    assert_eq!(
        items[0].scan_root,
        tempdir.path().to_string_lossy().to_string()
    );
    assert_eq!(
        items[0].item_path,
        tempdir
            .path()
            .join("Unknown Show (2020)")
            .to_string_lossy()
            .to_string()
    );
}

#[tokio::test]
async fn movie_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    std::fs::write(root_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(root_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-full-scan";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
    assert_eq!(projected.status, LibraryScanStatus::Completed);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn series_full_scan_scans_all_configured_roots_in_one_session() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first show folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second show folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let summary = app
        .scan_library(&user, MediaFacet::Series)
        .await
        .expect("series full scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 2);
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_one.to_string_lossy())
    );
    assert!(
        items
            .iter()
            .any(|item| item.scan_root == root_two.to_string_lossy())
    );
}

#[tokio::test]
async fn movie_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_root = tempdir.path().join("movies");
    std::fs::create_dir_all(&movie_root).expect("create movie root");
    let movie_path = movie_root.join("Unknown.One.2020.mkv");
    std::fs::write(&movie_path, b"movie").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            movie_root.to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    let session_id = "movie-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Movie,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join movie full scan task")
        .expect("movie full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn series_full_scan_marks_title_match_total_known_before_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    let session_id = "series-title-match-known-before-complete";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1 && session.title_match_total_known
    })
    .await;
    assert_eq!(projected.title_match_progress.total, 1);
    assert_eq!(projected.title_match_progress.completed, 0);
    assert!(projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join series full scan task")
        .expect("series full scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn multi_root_full_scan_waits_for_final_root_to_mark_title_match_total_known() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("series-a");
    let root_two = tempdir.path().join("series-b");
    std::fs::create_dir_all(root_one.join("Unknown Show One (2020)"))
        .expect("create first series folder");
    std::fs::create_dir_all(root_two.join("Unknown Show Two (2021)"))
        .expect("create second series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::blocking_calls(&[1, 2]));
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store series roots");

    let session_id = "series-multi-root-title-match-known";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search_calls(1).await;

    let first_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 1
        })
        .await;
    assert!(!first_root_projected.title_match_total_known);

    metadata_gateway.release_through(1);
    metadata_gateway.wait_for_batch_search_calls(2).await;

    let final_root_projected =
        wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
            session.found_titles == 2 && session.title_match_total_known
        })
        .await;
    assert_eq!(final_root_projected.title_match_progress.total, 2);
    assert!(final_root_projected.summary.is_none());

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join multi-root full scan task")
        .expect("multi-root full scan should complete");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);
}

#[tokio::test]
async fn additive_scan_keeps_title_match_total_unknown_until_completion() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    let session_id = "series-additive-title-match-stays-unknown";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let projected = wait_for_projected_library_scan_session_matching(&app, session_id, |session| {
        session.found_titles == 1
    })
    .await;
    assert!(!projected.title_match_total_known);

    metadata_gateway.release();

    let summary = handle
        .await
        .expect("join additive scan task")
        .expect("additive scan should complete");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
}

#[tokio::test]
async fn movie_full_scan_skips_invalid_roots_and_finishes_warning() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let valid_root = tempdir.path().join("movies-valid");
    let invalid_root = tempdir.path().join("movies-missing");
    std::fs::create_dir_all(&valid_root).expect("create valid movie root");
    std::fs::write(valid_root.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&valid_root, true),
            build_root_folder_entry(&invalid_root, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-invalid-root-warning";
    let summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Movie,
            Some(session_id.to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("movie full scan with invalid root");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert_eq!(summary.skipped, 1);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 1);
    assert_eq!(projected.status, LibraryScanStatus::Warning);
}

#[tokio::test]
async fn background_refresh_movies_scans_all_configured_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create movie root one");
    std::fs::create_dir_all(&root_two).expect("create movie root two");
    std::fs::write(root_one.join("Unknown.One.2020.mkv"), b"movie-one").expect("seed movie one");
    std::fs::write(root_two.join("Unknown.Two.2021.mkv"), b"movie-two").expect("seed movie two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("store movie roots");

    let session_id = "movie-multi-root-refresh";
    let summary = app
        .background_library_refresh_with_tracking(&user, MediaFacet::Movie, session_id)
        .await
        .expect("movie background refresh");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.unmatched, 2);

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected session")
            .expect("session snapshot");
    assert_eq!(projected.found_titles, 2);
}

#[tokio::test]
async fn cancel_full_library_scan_marks_session_canceled_and_allows_restart() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    let session_id = "cancel-full-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .scan_library_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                Some(session_id.to_string()),
                LibraryScanMode::Full,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let cancel_result = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect("cancel full library scan");
    assert!(cancel_result.accepted);
    assert_eq!(cancel_result.session_id, session_id);

    metadata_gateway.release();

    handle
        .await
        .expect("join canceled scan task")
        .expect("canceled scan task should not error");

    let projected =
        crate::library_scan_coordinator::load_projected_library_scan_session(&app, session_id)
            .await
            .expect("projected canceled session")
            .expect("canceled session snapshot");
    assert_eq!(projected.status, LibraryScanStatus::Canceled);
    assert_eq!(projected.found_titles, 1);
    assert!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .get(session_id)
            .is_none(),
        "cancellation token should be cleared after terminal cancel",
    );

    let retry_summary = app
        .scan_library_with_tracking(
            &user,
            MediaFacet::Series,
            Some("cancel-full-library-scan-retry".to_string()),
            LibraryScanMode::Full,
        )
        .await
        .expect("retry full scan after cancel");
    assert_eq!(retry_summary.unmatched, 1);
}

#[tokio::test]
async fn cancel_library_scan_rejects_additive_sessions() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let series_root = tempdir.path().join("series");
    std::fs::create_dir_all(series_root.join("Unknown Show (2020)")).expect("create series folder");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "series.path",
            series_root.to_string_lossy().as_ref(),
        )
        .await;

    let metadata_gateway = Arc::new(BlockingBatchMetadataGateway::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
        metadata_gateway.clone(),
    );

    let session_id = "cancel-additive-library-scan";
    let app_for_scan = app.clone();
    let user_for_scan = user.clone();
    let handle = tokio::spawn(async move {
        app_for_scan
            .background_library_refresh_with_tracking(
                &user_for_scan,
                MediaFacet::Series,
                session_id,
            )
            .await
    });

    metadata_gateway.wait_for_batch_search().await;

    let error = app
        .cancel_library_scan(&user, session_id)
        .await
        .expect_err("additive scan should not be cancelable");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("only full library scans")),
        "unexpected cancel error: {error:?}"
    );

    metadata_gateway.release();

    handle
        .await
        .expect("join additive scan task")
        .expect("background refresh should complete");
}

#[tokio::test]
async fn ensure_library_scan_cancellation_token_reuses_existing_token() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, _user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let first = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("first full-scan cancel token");
    let second = app
        .ensure_library_scan_cancellation_token("reused-library-scan-token", LibraryScanMode::Full)
        .await
        .expect("second full-scan cancel token");

    first.cancel();

    assert!(
        second.is_cancelled(),
        "subsequent ensure should reuse the existing cancellation token",
    );
    assert_eq!(
        app.runtime
            .library
            .library_scan_cancellation_tokens
            .lock()
            .await
            .len(),
        1,
        "reusing a cancellation token should not create duplicate map entries",
    );
}

#[tokio::test]
async fn pending_import_counts_and_items_are_facet_scoped() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());
    let known_series_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Known Show".to_string(),
                facet: MediaFacet::Series,
                monitored: true,
                ..NewTitle::default()
            },
        )
        .await
        .expect("seed known series title");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed movie item");
    let mut series_item = build_test_unmatched_item(
        "series-1",
        MediaFacet::Series,
        "/series",
        "/series/Unknown Show (2020)",
        "Unknown Show (2020)",
        "Unknown Show",
        Some(2020),
    );
    series_item.title_id = Some(known_series_title.title.id.clone());
    unmatched_items
        .upsert_library_scan_unmatched_item(&series_item)
        .await
        .expect("seed series item");
    let mut ignored_movie = build_test_unmatched_item(
        "movie-ignored-1",
        MediaFacet::Movie,
        "/movies",
        "/movies/Ignored.Movie.2020.mkv",
        "Ignored Movie",
        "Ignored Movie",
        Some(2020),
    );
    ignored_movie.status = PendingImportStatus::Ignored;
    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_movie)
        .await
        .expect("seed ignored movie item");

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts");
    assert_eq!(counts.movie, 1);
    assert_eq!(counts.series, 1);
    assert_eq!(counts.anime, 0);

    let movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("movie pending imports");
    assert_eq!(movie_items.total, 1);
    assert_eq!(movie_items.items.len(), 1);
    assert_eq!(movie_items.items[0].display_name, "Unknown Movie");
    assert_eq!(movie_items.items[0].path, "/movies/Unknown.Movie.2020.mkv");
    assert_eq!(movie_items.items[0].folder_path, None);

    let ignored_movie_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports");
    assert_eq!(ignored_movie_items.total, 1);
    assert_eq!(ignored_movie_items.items.len(), 1);
    assert_eq!(ignored_movie_items.items[0].display_name, "Ignored Movie");
    assert_eq!(
        ignored_movie_items.items[0].status,
        PendingImportStatus::Ignored
    );

    let series_items = app
        .pending_imports(
            &user,
            MediaFacet::Series,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("series pending imports");
    assert_eq!(series_items.total, 1);
    assert_eq!(series_items.items.len(), 1);
    assert_eq!(
        series_items.items[0].folder_path.as_deref(),
        Some("/series/Unknown Show (2020)")
    );
    assert_eq!(
        series_items.items[0].title_id.as_deref(),
        Some(known_series_title.title.id.as_str())
    );
    assert_eq!(
        series_items.items[0].title_name.as_deref(),
        Some(known_series_title.title.name.as_str())
    );
    assert_eq!(series_items.items[0].title_slug, known_series_title.title.slug);
}

#[tokio::test]
async fn ignore_pending_import_moves_item_out_of_pending_counts() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-ignore-1",
            MediaFacet::Movie,
            "/movies",
            "/movies/Needs.Ignore.2020.mkv",
            "Needs Ignore",
            "Needs Ignore",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let result = app
        .ignore_pending_import(&user, "movie-ignore-1")
        .await
        .expect("ignore pending import");
    assert_eq!(result.status, PendingImportStatus::Ignored);

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts after ignore");
    assert_eq!(counts.movie, 0);

    let pending_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            PendingImportStatus::Pending,
            50,
            0,
        )
        .await
        .expect("pending movie imports after ignore");
    assert_eq!(pending_items.total, 0);

    let ignored_items = app
        .pending_imports(
            &user,
            MediaFacet::Movie,
            PendingImportStatus::Ignored,
            50,
            0,
        )
        .await
        .expect("ignored movie imports after ignore");
    assert_eq!(ignored_items.total, 1);
    assert_eq!(ignored_items.items[0].id, "movie-ignore-1");
    assert_eq!(ignored_items.items[0].status, PendingImportStatus::Ignored);
}

#[tokio::test]
async fn update_media_settings_removing_root_clears_pending_imports_for_removed_root() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");
    std::fs::create_dir_all(&root_one).expect("create root one");
    std::fs::create_dir_all(&root_two).expect("create root two");

    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![
            build_root_folder_entry(&root_one, true),
            build_root_folder_entry(&root_two, false),
        ]),
    )
    .await
    .expect("seed movie roots");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-one",
            MediaFacet::Movie,
            root_one.to_string_lossy().as_ref(),
            root_one
                .join("Unknown.One.2020.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown One",
            "Unknown One",
            Some(2020),
        ))
        .await
        .expect("seed first pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-root-two",
            MediaFacet::Movie,
            root_two.to_string_lossy().as_ref(),
            root_two
                .join("Unknown.Two.2021.mkv")
                .to_string_lossy()
                .as_ref(),
            "Unknown Two",
            "Unknown Two",
            Some(2021),
        ))
        .await
        .expect("seed second pending import");

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        empty_update_media_settings_with_roots(vec![build_root_folder_entry(&root_one, true)]),
    )
    .await
    .expect("remove second movie root");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].scan_root, root_one.to_string_lossy());
}

#[tokio::test]
async fn update_library_paths_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "series-root",
            MediaFacet::Series,
            "/series",
            "/series/Unknown Show (2020)",
            "Unknown Show",
            "Unknown Show",
            Some(2020),
        ))
        .await
        .expect("seed kept pending import");

    app.update_library_paths(
        &user,
        UpdateLibraryPaths {
            movie_path: "/movies-new".to_string(),
            series_path: "/series".to_string(),
            anime_path: Some("/anime".to_string()),
        },
    )
    .await
    .expect("update library paths");

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Series);
    assert_eq!(items[0].scan_root, "/series");
}

#[tokio::test]
async fn update_library_paths_allows_partial_wizard_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime-old")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings.clone(),
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items,
    );

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "".to_string(),
                series_path: "/series-new".to_string(),
                anime_path: None,
            },
        )
        .await
        .expect("update partial library paths");

    assert_eq!(updated.movie_path, "/movies-old");
    assert_eq!(updated.series_path, "/series-new");
    assert_eq!(updated.anime_path, "/anime-old");
}

#[tokio::test]
async fn save_external_import_library_paths_removing_root_clears_pending_imports_for_removed_root()
{
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/movies-old")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "series.path", "/series")
        .await;
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "anime.path", "/anime")
        .await;

    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-external",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "anime-root-external",
            MediaFacet::Anime,
            "/anime",
            "/anime/Unknown Anime",
            "Unknown Anime",
            "Unknown Anime",
            Some(2021),
        ))
        .await
        .expect("seed kept pending import");

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec!["/movies-new".to_string()],
                series_paths: vec![],
                anime_paths: vec![],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);
    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].facet, MediaFacet::Anime);
    assert_eq!(items[0].scan_root, "/anime");
}

#[tokio::test]
async fn save_external_import_library_paths_persists_multiple_root_folders_per_facet() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/movies-primary".to_string(),
                    "/movies-secondary".to_string(),
                ],
                series_paths: vec!["/series-main".to_string(), "/series-archive".to_string()],
                anime_paths: vec!["/anime".to_string()],
            },
        )
        .await
        .expect("save external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/movies-primary");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/movies-primary".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/movies-secondary".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/series-main");
    assert_eq!(
        series_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/series-main".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/series-archive".to_string(),
                is_default: false,
            },
        ]
    );
}

#[tokio::test]
async fn save_external_import_library_paths_accepts_custom_selected_paths() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        Arc::new(TrackingLibraryScanUnmatchedItemRepo::default()),
    );

    let saved = app
        .save_external_import_library_paths(
            &user,
            ExternalImportLibraryPathsSelection {
                movie_paths: vec![
                    "/custom/movies".to_string(),
                    "/custom/movies-archive".to_string(),
                ],
                series_paths: vec!["/custom/series".to_string()],
                anime_paths: vec!["/custom/anime".to_string()],
            },
        )
        .await
        .expect("save custom external import paths");

    assert!(saved);

    let movie_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(movie_settings.library_path, "/custom/movies");
    assert_eq!(
        movie_settings.root_folders,
        vec![
            RootFolderEntry {
                path: "/custom/movies".to_string(),
                is_default: true,
            },
            RootFolderEntry {
                path: "/custom/movies-archive".to_string(),
                is_default: false,
            },
        ]
    );

    let series_settings = app
        .get_media_settings(&user, MediaFacet::Series)
        .await
        .expect("series settings");
    assert_eq!(series_settings.library_path, "/custom/series");
    assert_eq!(
        series_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/series".to_string(),
            is_default: true,
        }]
    );

    let anime_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(anime_settings.library_path, "/custom/anime");
    assert_eq!(
        anime_settings.root_folders,
        vec![RootFolderEntry {
            path: "/custom/anime".to_string(),
            is_default: true,
        }]
    );
}

#[tokio::test]
async fn resolve_pending_import_creates_unmonitored_movie_title_and_clears_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    anidb_id: None,
                    genres: vec!["Drama".into()],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                },
            )]),
        }),
    );

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let result = app
        .resolve_pending_import(&user, "movie-resolve-1", "123456")
        .await
        .expect("resolve pending import");

    assert!(result.created);
    assert!(!result.title.monitored);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(
        result.library_scan.scanned
            + result.library_scan.matched
            + result.library_scan.imported
            + result.library_scan.skipped
            + result.library_scan.unmatched
            > 0
    );
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_ignored_pending_import_creates_unmonitored_movie_title_and_clears_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Ignored.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "Matched Movie".into(),
                    slug: "matched-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Matched overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 101,
                    sort_title: "Matched Movie".into(),
                    imdb_id: "tt0123456".into(),
                    anidb_id: None,
                    genres: vec!["Drama".into()],
                    studio: "Test Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                },
            )]),
        }),
    );

    let mut ignored_item = build_test_unmatched_item(
        "movie-resolve-ignored-1",
        MediaFacet::Movie,
        tempdir.path().to_string_lossy().as_ref(),
        movie_path.to_string_lossy().as_ref(),
        "Ignored Movie",
        "Matched Movie",
        Some(2020),
    );
    ignored_item.status = PendingImportStatus::Ignored;

    unmatched_items
        .upsert_library_scan_unmatched_item(&ignored_item)
        .await
        .expect("seed ignored import");

    let result = app
        .resolve_pending_import(&user, "movie-resolve-ignored-1", "123456")
        .await
        .expect("resolve ignored pending import");

    assert!(result.created);
    assert_eq!(result.title.name, "Matched Movie");
    assert!(unmatched_items.items().await.is_empty());
}

#[tokio::test]
async fn resolve_pending_import_failure_keeps_pending_item() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Unknown.Movie.2020.mkv");
    std::fs::write(&movie_path, b"fake-video").expect("seed movie file");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner
        .set_library_files(vec![build_test_library_file(
            movie_path.to_string_lossy().as_ref(),
        )])
        .await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Matched Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(&user, "movie-resolve-failure-1", "999999")
        .await
        .expect_err("resolution should fail without metadata");
    assert!(!error.to_string().trim().is_empty());
    assert_eq!(unmatched_items.items().await.len(), 1);
    assert!(
        app.list_titles(&user, Some(MediaFacet::Movie), None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn hydrate_titles_bulk_updates_title_name_for_selected_metadata_language() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, METADATA_LANGUAGE_KEY, "jpn")
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "デューン".into(),
                    slug: "dune".into(),
                    year: Some(2021),
                    content_status: "Released".into(),
                    overview: "日本語概要".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "jpn".into(),
                    runtime_minutes: 155,
                    sort_title: "デューン".into(),
                    imdb_id: "tt1160419".into(),
                    anidb_id: None,
                    genres: vec!["Science Fiction".into()],
                    studio: "Legendary".into(),
                    tmdb_release_date: Some("2021-10-22".into()),
                },
            )]),
        }),
    );

    let created = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Dune".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                min_availability: None,
                poster_url: None,
                year: None,
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed untranslated title");
    let created_title = created.title;

    let mut outcome = app
        .hydrate_titles_bulk(vec![crate::catalog_workflow::HydrationTarget {
            title: created_title.clone(),
            requested_tvdb_id: None,
            sync_wanted_after_completion: false,
            source: crate::catalog_workflow::HydrationSource::Interactive,
        }])
        .await
        .expect("hydrate title");

    let hydrated = outcome
        .hydrated_titles
        .remove(&created_title.id)
        .expect("hydrated title should be returned");
    assert_eq!(hydrated.name, "デューン");
    assert_eq!(hydrated.metadata_language.as_deref(), Some("jpn"));
    assert_eq!(hydrated.overview.as_deref(), Some("日本語概要"));

    let persisted = app
        .list_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("list titles");
    assert_eq!(persisted[0].name, "デューン");
    assert_eq!(persisted[0].metadata_language.as_deref(), Some("jpn"));
}

#[tokio::test]
async fn background_title_hydrator_skips_full_scan_owned_facets_and_hydrates_other_due_titles() {
    let metadata_gateway = Arc::new(MockMetadataGateway {
        movies: HashMap::from([(101, make_movie_metadata(101, "Eligible Movie"))]),
    });
    let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);

    TitleRepository::create(
        &*titles,
        make_due_hydration_title("movie-due", MediaFacet::Movie, 101),
    )
    .await
    .expect("seed due movie title");
    TitleRepository::create(
        &*titles,
        make_due_hydration_title("series-due", MediaFacet::Series, 202),
    )
    .await
    .expect("seed due series title");

    app.runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "series-scan-owned".to_string(),
            MediaFacet::Series,
            LibraryScanMode::Full,
        )
        .await
        .expect("start series scan");

    let token = tokio_util::sync::CancellationToken::new();
    let handle = tokio::spawn(start_background_title_hydration_loop(
        app.clone(),
        token.child_token(),
    ));

    let hydrated_movie = timeout(Duration::from_secs(1), async {
        loop {
            let title = app
                .services
                .catalog
                .titles
                .get_by_id("movie-due")
                .await
                .expect("load movie title")
                .expect("movie title should exist");
            if title.metadata_fetched_at.is_some() {
                break title;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("movie due title should hydrate");

    let skipped_series = app
        .services
        .catalog
        .titles
        .get_by_id("series-due")
        .await
        .expect("load series title")
        .expect("series title should exist");

    token.cancel();
    handle
        .await
        .expect("title hydration loop should stop cleanly");

    assert!(hydrated_movie.metadata_fetched_at.is_some());
    assert!(
        skipped_series.metadata_fetched_at.is_none(),
        "background worker should not hydrate titles for the facet owned by the active scan"
    );
}

#[tokio::test]
async fn background_title_hydrator_retries_scan_owned_movie_titles_after_scan_clears() {
    for mode in [LibraryScanMode::Full, LibraryScanMode::Additive] {
        let metadata_gateway = Arc::new(MockMetadataGateway {
            movies: HashMap::from([(303, make_movie_metadata(303, "Recovered Movie"))]),
        });
        let (app, _user, titles) = bootstrap_with_metadata_gateway_and_titles(metadata_gateway);

        let title_id = format!("movie-due-{}", mode.as_str());
        let session_id = format!("scan-owned-{}", mode.as_str());
        TitleRepository::create(
            &*titles,
            make_due_hydration_title(&title_id, MediaFacet::Movie, 303),
        )
        .await
        .expect("seed due movie title");

        app.runtime
            .library
            .library_scan_tracker
            .start_session_with_id(session_id.clone(), MediaFacet::Movie, mode.clone())
            .await
            .expect("start movie scan");

        let token = tokio_util::sync::CancellationToken::new();
        let handle = tokio::spawn(start_background_title_hydration_loop(
            app.clone(),
            token.child_token(),
        ));

        let premature_hydration = timeout(Duration::from_millis(250), async {
            loop {
                let title = app
                    .services
                    .catalog
                    .titles
                    .get_by_id(&title_id)
                    .await
                    .expect("load movie title")
                    .expect("movie title should exist");
                if title.metadata_fetched_at.is_some() {
                    break title;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            premature_hydration.is_err(),
            "background worker should not hydrate a scan-owned movie title while the scan is active"
        );

        app.runtime
            .library
            .library_scan_tracker
            .cancel_session(&session_id)
            .await
            .expect("clear scan-owned session");

        let hydrated = timeout(Duration::from_secs(1), async {
            loop {
                let title = app
                    .services
                    .catalog
                    .titles
                    .get_by_id(&title_id)
                    .await
                    .expect("load movie title after scan clear")
                    .expect("movie title should still exist");
                if title.metadata_fetched_at.is_some() {
                    break title;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("movie due title should hydrate after scan clears");

        token.cancel();
        handle
            .await
            .expect("title hydration loop should stop cleanly");

        assert!(
            hydrated.metadata_fetched_at.is_some(),
            "scan-owned movie title should remain due and hydrate after the scan clears"
        );
    }
}

#[tokio::test]
async fn resolve_pending_import_failure_restores_existing_title_folder_path() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let movie_path = tempdir.path().join("Missing.Movie.2020.mkv");

    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            tempdir.path().to_string_lossy().as_ref(),
        )
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    library_scanner.set_library_files(vec![]).await;
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

    let existing_title = app
        .create_title_without_hydration(
            &user,
            NewTitle {
                name: "Existing Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "123456".to_string(),
                }],
                min_availability: None,
                poster_url: None,
                year: Some(2020),
                overview: None,
                sort_title: None,
                slug: None,
                runtime_minutes: None,
                language: None,
                content_status: None,
            },
        )
        .await
        .expect("seed existing title");
    let existing_title = existing_title.title;
    app.services
        .catalog
        .titles
        .set_folder_path(&existing_title.id, "/existing/movies/Existing Movie")
        .await
        .expect("set original folder path");

    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-resolve-existing-failure-1",
            MediaFacet::Movie,
            tempdir.path().to_string_lossy().as_ref(),
            movie_path.to_string_lossy().as_ref(),
            "Unknown Movie",
            "Existing Movie",
            Some(2020),
        ))
        .await
        .expect("seed pending import");

    let error = app
        .resolve_pending_import(&user, "movie-resolve-existing-failure-1", "123456")
        .await
        .expect_err("resolution should fail when scan finds no files");
    assert!(!error.to_string().trim().is_empty());
    assert_eq!(unmatched_items.items().await.len(), 1);

    let refreshed_title = app
        .services
        .catalog
        .titles
        .get_by_id(&existing_title.id)
        .await
        .expect("load existing title")
        .expect("existing title should still exist");
    assert_eq!(
        refreshed_title.folder_path.as_deref(),
        Some("/existing/movies/Existing Movie")
    );
}

#[tokio::test]
async fn add_title_and_queue_sends_download_job() {
    let (app, user) = bootstrap();
    let (title, job_id) = app
        .add_title_and_queue_download(
            &user,
            NewTitle {
                name: "Show One".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
            QueuedReleaseSelection::default(),
        )
        .await
        .expect("title + queue should succeed");

    assert_eq!(job_id, format!("job-for-{}", title.id));
}

#[tokio::test]
async fn add_title_with_outcome_returns_pending_and_reuses_existing_tvdb_title() {
    let (app, user) = bootstrap();
    let request = NewTitle {
        name: "Slow Hydration Movie".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "123456".to_string(),
        }],
        min_availability: None,
        ..Default::default()
    };

    let first = app
        .add_title_with_outcome(&user, request.clone())
        .await
        .expect("first add should succeed");
    assert_eq!(
        first.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );
    assert!(!first.reused_existing_title);

    let second = app
        .add_title_with_outcome(&user, request)
        .await
        .expect("duplicate add should reuse existing title");
    assert_eq!(second.title.id, first.title.id);
    assert_eq!(
        second.metadata_hydration_state,
        AddTitleHydrationState::Pending
    );
    assert!(second.reused_existing_title);

    let titles = app
        .list_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("titles should load");
    assert_eq!(titles.len(), 1);
}

#[tokio::test]
async fn add_title_and_queue_download_with_outcome_reuses_matching_queue_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let request = NewTitle {
        name: "Queued Once".into(),
        facet: MediaFacet::Movie,
        monitored: true,
        tags: vec![],
        external_ids: vec![ExternalId {
            source: "tvdb".to_string(),
            value: "654321".to_string(),
        }],
        min_availability: None,
        ..Default::default()
    };
    let queued_release = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/releases/queued-once.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Queued.Once.2026.1080p.WEB-DL".to_string()),
    };

    let first = app
        .add_title_and_queue_download_with_outcome(&user, request.clone(), queued_release.clone())
        .await
        .expect("first queued add should succeed");
    assert!(!first.reused_existing_title);
    assert!(!first.reused_queued_download);

    let second = app
        .add_title_and_queue_download_with_outcome(&user, request, queued_release)
        .await
        .expect("duplicate queued add should reuse existing queue submission");
    assert_eq!(second.title.id, first.title.id);
    assert_eq!(second.download_job_id, first.download_job_id);
    assert!(second.reused_existing_title);
    assert!(second.reused_queued_download);

    let submissions = download_submissions.store.lock().await.clone();
    let expected_signature = normalize_release_selection_signature(
        Some("https://example.invalid/releases/queued-once.nzb"),
        Some("Queued.Once.2026.1080p.WEB-DL"),
        Some(DownloadSourceKind::NzbUrl),
    );
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].request_signature, expected_signature);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Queued Once".to_string()]
    );
}

#[tokio::test]
async fn queue_existing_title_download_reuses_matching_queue_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Existing Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "7654321".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let queued_release = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/releases/existing-queue.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Existing.Queue.2026.1080p.WEB-DL".to_string()),
    };

    let first = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release.clone(),
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("first queue should succeed");
    let QueueDownloadOutcome::Queued(first) = first else {
        panic!("first queue should not conflict");
    };
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        &first.job_id,
        DownloadQueueState::Queued,
        0,
    )];
    let second = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("second queue should reuse submission");
    let QueueDownloadOutcome::Queued(second) = second else {
        panic!("second queue should not conflict");
    };

    assert_eq!(second.job_id, first.job_id);
    assert!(second.reused_existing);

    let submissions = download_submissions.store.lock().await.clone();
    let expected_signature = normalize_release_selection_signature(
        Some("https://example.invalid/releases/existing-queue.nzb"),
        Some("Existing.Queue.2026.1080p.WEB-DL"),
        Some(DownloadSourceKind::NzbUrl),
    );
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].request_signature, expected_signature);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Existing Queue".to_string()]
    );
}

#[tokio::test]
async fn queue_existing_title_download_ignores_stale_matching_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let queued_release = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/releases/stale-queue.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Stale.Queue.2026.1080p.WEB-DL".to_string()),
    };

    app.queue_existing_title_download(
        &user,
        &title.id,
        queued_release.clone(),
        SubmissionScope::Title,
        SubmissionConflictPolicy::Abort,
    )
    .await
    .expect("first queue should succeed");
    download_client.queue_items.lock().await.clear();

    let second = app
        .queue_existing_title_download(
            &user,
            &title.id,
            queued_release,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("stale signature should not be reused");
    let QueueDownloadOutcome::Queued(second) = second else {
        panic!("stale signature should queue again");
    };

    assert!(!second.reused_existing);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Stale Queue".to_string(), "Stale Queue".to_string()]
    );
}

#[tokio::test]
async fn queue_existing_title_download_reports_scope_conflict() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "existing-job".to_string(),
            source_hint: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Queue.2026.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "existing-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/replacement.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Blocked.Queue.Replacement.2026.1080p.WEB-DL".to_string()),
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("conflict should be returned as outcome");

    let QueueDownloadOutcome::Conflict(conflict) = outcome else {
        panic!("queue should conflict");
    };
    assert_eq!(conflict.download_client_item_id, "existing-job");
    assert!(conflict.replaceable);
    assert!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn queue_existing_title_download_replace_early_deletes_old_submission() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Replace Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "old-job".to_string(),
            source_hint: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Replace.Queue.2026.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "old-job",
        DownloadQueueState::Queued,
        0,
    )];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/new.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Replace.Queue.New.2026.1080p.WEB-DL".to_string()),
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect("replacement should succeed");

    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("replacement should queue");
    };
    assert_eq!(outcome.job_id, format!("job-for-{}", title.id));
    assert_eq!(
        download_client.deleted_items.lock().await.as_slice(),
        &[("old-job".to_string(), false)]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_client_item_id, outcome.job_id);
}

#[tokio::test]
async fn queue_existing_title_download_replace_early_deletes_all_blockers() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Replace All Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    for job_id in ["old-job-a", "old-job-b"] {
        download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: "movie".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "nzbget".to_string(),
                download_client_item_id: job_id.to_string(),
                source_hint: None,
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some(format!("Replace.All.Queue.{job_id}.2026.1080p.WEB-DL")),
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record submission");
    }
    *download_client.queue_items.lock().await = vec![
        queue_history_fixture_item("old-job-a", DownloadQueueState::Queued, 0),
        queue_history_fixture_item("old-job-b", DownloadQueueState::Downloading, 0),
    ];

    let outcome = app
        .queue_existing_title_download(
            &user,
            &title.id,
            QueuedReleaseSelection {
                source_hint: Some("https://example.invalid/new-all.nzb".to_string()),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                source_title: Some("Replace.All.Queue.New.2026.1080p.WEB-DL".to_string()),
            },
            SubmissionScope::Title,
            SubmissionConflictPolicy::ReplaceEarly,
        )
        .await
        .expect("replacement should succeed");

    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("replacement should queue");
    };
    let mut deleted_items = download_client.deleted_items.lock().await.clone();
    deleted_items.sort();
    assert_eq!(
        deleted_items,
        vec![
            ("old-job-a".to_string(), false),
            ("old-job-b".to_string(), false),
        ]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].download_client_item_id, outcome.job_id);
}

#[tokio::test]
async fn commit_successful_grab_marks_covered_wanted_set_and_supersedes_pending_releases() {
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let repo = TrackingAcquisitionStateRepo {
        download_submissions,
        pending_releases: pending_releases.clone(),
        wanted_items: wanted_items.clone(),
    };
    let now = Utc::now().to_rfc3339();
    let title_id = "covered-title";
    let wanted_a = WantedItem {
        id: "wanted-a".to_string(),
        title_id: title_id.to_string(),
        title_name: Some("Covered Title".to_string()),
        title_slug: None,
        title_facet: None,
        episode_id: Some("episode-a".to_string()),
        collection_id: Some("season-1".to_string()),
        season_number: Some("1".to_string()),
        episode_number: None,
        media_type: "series".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 1,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: now.clone(),
        updated_at: now.clone(),
    };
    let wanted_b = WantedItem {
        id: "wanted-b".to_string(),
        episode_id: Some("episode-b".to_string()),
        ..wanted_a.clone()
    };
    let wanted_c = WantedItem {
        id: "wanted-c".to_string(),
        episode_id: Some("episode-c".to_string()),
        ..wanted_a.clone()
    };
    for wanted in [&wanted_a, &wanted_b, &wanted_c] {
        wanted_items
            .upsert_wanted_item(wanted)
            .await
            .expect("seed wanted item");
    }

    for (id, wanted_item_id, status) in [
        ("pending-grabbed", "wanted-a", PendingReleaseStatus::Waiting),
        (
            "pending-a-sibling",
            "wanted-a",
            PendingReleaseStatus::Waiting,
        ),
        (
            "pending-b-waiting",
            "wanted-b",
            PendingReleaseStatus::Waiting,
        ),
        (
            "pending-b-standby",
            "wanted-b",
            PendingReleaseStatus::Standby,
        ),
        (
            "pending-c-uncovered",
            "wanted-c",
            PendingReleaseStatus::Waiting,
        ),
    ] {
        pending_releases
            .insert_pending_release(&PendingRelease {
                id: id.to_string(),
                wanted_item_id: wanted_item_id.to_string(),
                title_id: title_id.to_string(),
                release_title: format!("{id}.1080p.WEB-DL"),
                release_url: Some(format!("https://example.invalid/{id}.nzb")),
                source_kind: Some(DownloadSourceKind::NzbUrl),
                release_size_bytes: Some(1_000),
                release_score: 100,
                scoring_log_json: None,
                indexer_source: Some("test-indexer".to_string()),
                release_guid: Some(format!("guid-{id}")),
                added_at: now.clone(),
                delay_until: now.clone(),
                status,
                grabbed_at: None,
                source_password: None,
                published_at: Some(now.clone()),
                info_hash: None,
            })
            .await
            .expect("seed pending release");
    }

    repo.commit_successful_grab(&SuccessfulGrabCommit {
        wanted_item_id: wanted_a.id.clone(),
        covered_wanted_item_ids: vec![wanted_b.id.clone()],
        search_count: 2,
        current_score: Some(100),
        grabbed_release: "{\"title\":\"Covered.Release.1080p.WEB-DL\"}".to_string(),
        last_search_at: Some(now.clone()),
        download_submission: DownloadSubmission {
            title_id: title_id.to_string(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-covered".to_string(),
            source_hint: Some("https://example.invalid/grabbed.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Covered.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::EpisodeSet {
                episode_ids: vec!["episode-a".to_string(), "episode-b".to_string()],
            },
        },
        grabbed_pending_release_id: Some("pending-grabbed".to_string()),
        grabbed_at: Some(now),
    })
    .await
    .expect("commit successful grab");

    let wanted_store = wanted_items.store.lock().await.clone();
    let status_for = |id: &str| {
        wanted_store
            .iter()
            .find(|wanted| wanted.id == id)
            .map(|wanted| wanted.status)
            .expect("wanted item exists")
    };
    assert_eq!(status_for("wanted-a"), WantedStatus::Grabbed);
    assert_eq!(status_for("wanted-b"), WantedStatus::Grabbed);
    assert_eq!(status_for("wanted-c"), WantedStatus::Wanted);

    let pending_store = pending_releases.store.lock().await.clone();
    let pending_status_for = |id: &str| {
        pending_store
            .iter()
            .find(|release| release.id == id)
            .map(|release| release.status)
            .expect("pending release exists")
    };
    assert_eq!(
        pending_status_for("pending-grabbed"),
        PendingReleaseStatus::Grabbed
    );
    assert_eq!(
        pending_status_for("pending-a-sibling"),
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        pending_status_for("pending-b-waiting"),
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        pending_status_for("pending-b-standby"),
        PendingReleaseStatus::Superseded
    );
    assert_eq!(
        pending_status_for("pending-c-uncovered"),
        PendingReleaseStatus::Waiting
    );
}

#[tokio::test]
async fn trigger_title_wanted_search_conflicts_before_seeding_movie_wanted_item() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Wanted Movie".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "movie-job".to_string(),
            source_hint: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Wanted.Movie.2026.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "movie-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .trigger_title_wanted_search(&title.id, SubmissionConflictPolicy::Abort)
        .await
        .expect("wanted search should return conflict");

    assert_eq!(outcome.queued_count, 0);
    assert_eq!(outcome.skipped_in_progress_count, 0);
    assert_eq!(
        outcome
            .conflict
            .as_ref()
            .map(|conflict| conflict.download_client_item_id.as_str()),
        Some("movie-job")
    );
    assert!(
        app.services
            .workflow
            .wanted_items
            .list_wanted_items(WantedItemsQuery {
                title_id: Some(title.id.clone()),
                limit: 100,
                ..WantedItemsQuery::default()
            })
            .await
            .expect("list wanted items")
            .is_empty()
    );
}

#[tokio::test]
async fn trigger_title_wanted_search_skips_conflicted_first_seed_episode_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Blocked Wanted Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("1".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "episode-job".to_string(),
            source_hint: None,
            source_kind: Some(DownloadSourceKind::NzbUrl),
            source_title: Some("Blocked.Wanted.Series.S01E01.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode.id.clone(),
            },
        })
        .await
        .expect("record submission");
    *download_client.queue_items.lock().await = vec![queue_history_fixture_item(
        "episode-job",
        DownloadQueueState::Downloading,
        0,
    )];

    let outcome = app
        .trigger_title_wanted_search(&title.id, SubmissionConflictPolicy::Abort)
        .await
        .expect("wanted search should skip blocked episode");

    assert_eq!(outcome.queued_count, 0);
    assert_eq!(outcome.skipped_in_progress_count, 1);
    assert_eq!(
        outcome
            .conflict
            .as_ref()
            .map(|conflict| conflict.download_client_item_id.as_str()),
        Some("episode-job")
    );
    let wanted_items = app
        .services
        .workflow
        .wanted_items
        .list_wanted_items(WantedItemsQuery {
            title_id: Some(title.id.clone()),
            limit: 100,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("list wanted items");
    assert!(wanted_items.is_empty());
}

#[tokio::test]
async fn queue_existing_title_download_from_candidate_token_accepts_authenticated_actor() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, admin) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &admin,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &admin,
            NewTitle {
                name: "Token Queue".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "token_queue_user",
        "password123",
        vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    )
    .await;

    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/token-queue.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Token.Queue.2026.1080p.WEB-DL".to_string()),
    };
    let candidate_token = app
        .issue_release_candidate_token(
            &authenticated_user,
            &title.id,
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("issue candidate token");

    let outcome = app
        .queue_existing_title_download_from_candidate_token(
            &authenticated_user,
            &title.id,
            &candidate_token,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue existing title download from candidate token");
    let QueueDownloadOutcome::Queued(outcome) = outcome else {
        panic!("token queue should not conflict");
    };

    assert_eq!(outcome.job_id, format!("job-for-{}", title.id));
    assert_eq!(outcome.queued_release, selection);
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &["Token Queue".to_string()]
    );
}

#[tokio::test]
async fn queue_best_release_prefers_first_auto_eligible_candidate() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        "Wrong.Show.2026.1080p.WEB-DL",
        "Target.Show.2026.1080p.WEB-DL",
    ]));
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        indexer_client,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Target Show".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let job_id = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::Title,
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue best release");
    let QueueDownloadOutcome::Queued(job_id) = job_id else {
        panic!("best release should not conflict");
    };

    assert_eq!(job_id.job_id, format!("job-for-{}", title.id));
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Target Show".to_string()]
    );

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Target.Show.2026.1080p.WEB-DL")
    );
    assert!(
        submissions[0]
            .request_signature
            .as_deref()
            .is_some_and(|signature| signature.contains("Target.Show.2026.1080p.WEB-DL"))
    );
}

#[tokio::test]
async fn queue_best_release_supports_interstitial_movie_collection_scope() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let indexer_client = Arc::new(MultiReleaseIndexerClient::new(vec![
        "Wrong.Show.2024.1080p.WEB-DL",
        "Movie.1.2024.1080p.WEB-DL",
    ]));
    let (app, user) = bootstrap_with_cleanup_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases,
        indexer_client,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Parent Series".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Interstitial,
            collection_index: "1.1".to_string(),
            label: Some("Movie 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1.1".to_string()),
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "movie-1".to_string(),
                name: "Movie 1".to_string(),
                slug: "movie-1".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Interstitial movie".to_string(),
                poster_url: String::new(),
                language: "ja".to_string(),
                runtime_minutes: 110,
                sort_title: "Movie 1".to_string(),
                imdb_id: String::new(),
                genres: vec!["action".to_string()],
                studio: "Studio".to_string(),
                digital_release_date: Some("2024-02-01".to_string()),
                association_confidence: Some("high".to_string()),
                continuity_status: Some("canon".to_string()),
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: Some("between_seasons".to_string()),
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let job_id = app
        .queue_best_release(
            &user,
            &title.id,
            SubmissionScope::Collection {
                collection_id: collection.id.clone(),
            },
            SubmissionConflictPolicy::Abort,
        )
        .await
        .expect("queue best release for collection");
    let QueueDownloadOutcome::Queued(job_id) = job_id else {
        panic!("best release should not conflict");
    };

    assert_eq!(job_id.job_id, format!("job-for-{}", title.id));

    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].source_title.as_deref(),
        Some("Movie.1.2024.1080p.WEB-DL")
    );
    assert_eq!(
        submissions[0].scope,
        SubmissionScope::Collection {
            collection_id: collection.id
        }
    );
}

#[tokio::test]
async fn resolve_release_search_subject_for_collection_uses_interstitial_movie_metadata() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Mugen.Train.2020.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Wrong Show".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let collection = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Interstitial,
            collection_index: "1.1".to_string(),
            label: Some("Mugen Train".to_string()),
            ordered_path: None,
            narrative_order: Some("1.1".to_string()),
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "12345".to_string(),
                name: "Mugen Train".to_string(),
                slug: "mugen-train".to_string(),
                year: Some(2020),
                content_status: "released".to_string(),
                overview: "Interstitial movie".to_string(),
                poster_url: String::new(),
                language: "ja".to_string(),
                runtime_minutes: 110,
                sort_title: "Mugen Train".to_string(),
                imdb_id: "tt11032374".to_string(),
                genres: vec!["action".to_string()],
                studio: "Studio".to_string(),
                digital_release_date: Some("2024-02-01".to_string()),
                association_confidence: Some("high".to_string()),
                continuity_status: Some("canon".to_string()),
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: Some("between_seasons".to_string()),
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("create interstitial collection");

    let (search_title, subject) = app
        .resolve_release_search_subject_for_collection(&title, &collection)
        .await
        .expect("resolve interstitial movie subject");

    assert_eq!(search_title.name, "Mugen Train");
    assert_eq!(search_title.year, Some(2020));
    assert_eq!(search_title.imdb_id.as_deref(), Some("tt11032374"));
    assert_eq!(subject.queries, vec!["Mugen Train 2020".to_string()]);
    assert_eq!(subject.tvdb_id.as_deref(), Some("12345"));
    assert_eq!(subject.imdb_id.as_deref(), Some("tt11032374"));
    assert_eq!(
        subject.submission_scope,
        SubmissionScope::Collection {
            collection_id: collection.id,
        }
    );
}

fn cutoff_projection_test_profile(id: &str, cutoff_tier: &str) -> QualityProfile {
    QualityProfile {
        id: id.to_string(),
        name: format!("Profile {id}"),
        criteria: QualityProfileCriteria {
            quality_tiers: vec!["1080P".to_string(), "720P".to_string(), "480P".to_string()],
            archival_quality: Some("1080P".to_string()),
            allow_unknown_quality: false,
            source_allowlist: vec![],
            source_blocklist: vec![],
            video_codec_allowlist: vec![],
            video_codec_blocklist: vec![],
            audio_codec_allowlist: vec![],
            audio_codec_blocklist: vec![],
            atmos_preferred: false,
            dolby_vision_allowed: true,
            detected_hdr_allowed: true,
            prefer_remux: false,
            allow_bd_disk: false,
            allow_upgrades: true,
            prefer_dual_audio: false,
            required_audio_languages: vec![],
            scoring_persona: ScoringPersona::default(),
            scoring_overrides: ScoringOverrides::default(),
            cutoff_tier: Some(cutoff_tier.to_string()),
            min_score_to_grab: None,
            facet_persona_overrides: HashMap::new(),
        },
    }
}

#[tokio::test]
async fn list_cutoff_unmet_titles_normalizes_lowercase_cutoff_tier() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-lowercase""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-lowercase",
            "720p",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Case".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Case.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie))
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id, None);
    assert_eq!(items[0].current_tier, "480P");
    assert_eq!(items[0].target_tier, "720P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_returns_episode_scoped_rows_for_series() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-series""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-series",
            "1080P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Cutoff Episodes".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    let file_id = media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Cutoff Episodes/Season 01/Cutoff Episodes - S01E01.mkv"
                .to_string(),
            size_bytes: 1_000,
            quality_label: Some("720p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");
    media_files
        .link_file_to_episode(&file_id, &episode.id)
        .await
        .expect("link media file to episode");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Series))
        .await
        .expect("cutoff unmet query should succeed");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title_id, title.id);
    assert_eq!(items[0].episode_id.as_deref(), Some(episode.id.as_str()));
    assert_eq!(items[0].current_tier, "720P");
    assert_eq!(items[0].target_tier, "1080P");
}

#[tokio::test]
async fn list_cutoff_unmet_titles_falls_back_to_default_when_title_profile_tag_is_stale() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            r#""cutoff-global""#,
        )
        .await;
    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![cutoff_projection_test_profile(
            "cutoff-global",
            "720P",
        )])
        .await;
    let media_files = Arc::new(MockMediaFileRepo::default());
    let (app, user, _) =
        bootstrap_with_cutoff_projection_state(settings, quality_profiles, media_files.clone());

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Stale Tag".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["scryer:quality-profile:missing-profile".to_string()],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    media_files
        .insert_media_file(&InsertMediaFileInput {
            title_id: title.id.clone(),
            file_path: "/library/Stale Tag.mkv".to_string(),
            size_bytes: 1_000,
            quality_label: Some("480p".to_string()),
            ..Default::default()
        })
        .await
        .expect("insert media file");

    let items = app
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie))
        .await
        .expect("cutoff unmet query should succeed");

    assert!(items.is_empty());
}

#[tokio::test]
async fn search_titles_supports_facet_filter() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Movie A".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create movie");

    app.add_title(
        &user,
        NewTitle {
            name: "Show B".into(),
            facet: MediaFacet::Series,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,

            ..Default::default()
        },
    )
    .await
    .expect("create series");

    let tvs = app
        .list_titles(&user, Some(MediaFacet::Series), None)
        .await
        .expect("list titles");

    assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
}

#[tokio::test]
async fn search_indexers_anime_required_english_accepts_dual_audio_release() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Anime.Show.S01E01.1080p.WEB-DL.DUAL.H.265",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    app.set_facet_required_audio_languages(&user, "anime", vec!["English".to_string()])
        .await
        .expect("set anime required audio");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Anime Show".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let results = app
        .search_indexers_for_title(&user, title.id.clone())
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    let parsed = results[0]
        .parsed_release_metadata
        .as_ref()
        .expect("search result should be parsed");
    assert_eq!(
        parsed.languages_audio,
        vec!["eng".to_string(), "jpn".to_string()]
    );
    let decision = results[0]
        .quality_profile_decision
        .as_ref()
        .expect("search result should be scored");
    assert!(decision.allowed);
    assert!(
        decision
            .scoring_log
            .iter()
            .any(|entry| entry.code == "required_audio_languages_match")
    );
}

#[tokio::test]
async fn search_indexers_for_title_uses_tagged_aliases_for_auto_evaluation() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "BASTARD.Heavy.Metal.Dark.Fantasy.S01E01.1080p.NF.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let search_user = app
        .create_user(
            &user,
            "title_search_user".to_string(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
        )
        .await
        .expect("create search user");
    let search_token = app
        .issue_access_token(&search_user)
        .expect("issue search token");
    let authed_search_user = app
        .authenticate_token(&search_token)
        .await
        .expect("authenticate search user");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Bastard!!".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "1309".to_string(),
                }],
                year: Some(2022),
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.services
        .catalog
        .titles
        .update_title_hydrated_metadata(
            &title.id,
            TitleMetadataUpdate {
                tagged_aliases: vec![scryer_domain::TaggedAlias {
                    name: "Bastard Heavy Metal Dark Fantasy".to_string(),
                    language: "eng".to_string(),
                }],
                ..Default::default()
            },
        )
        .await
        .expect("persist tagged aliases");

    let results = app
        .search_indexers_for_title(&authed_search_user, title.id.clone())
        .await
        .expect("search indexers for title");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].auto_eligible, Some(true));
    assert_eq!(results[0].auto_decision_code.as_deref(), Some("eligible"));
    assert!(results[0].candidate_token.is_some());
}

#[tokio::test]
async fn search_indexers_for_title_returns_results_when_candidate_token_attachment_fails() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(
        "Failure.Recovery.2026.1080p.WEB-DL",
    ));
    let (app, user) = bootstrap_with_search_settings_and_indexer(settings, indexer_client);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let ghost_actor = User {
        id: "ghost-search-user".to_string(),
        username: "ghost".to_string(),
        password_hash: None,
        entitlements: vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    };

    let results = app
        .search_indexers_for_title(&ghost_actor, title.id.clone())
        .await
        .expect("search should still succeed without candidate signing key");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].candidate_token, None);
}

#[tokio::test]
async fn create_user_and_list_users() {
    let (app, user) = bootstrap();

    let created = app
        .create_user(
            &user,
            "editor".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
        )
        .await
        .expect("create user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(users.iter().any(|entry| entry.username == created.username));
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn get_user_by_id_returns_created_user() {
    let (app, user) = bootstrap();

    let created = app
        .create_user(
            &user,
            "viewer".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");

    let found = app.get_user(&user, &created.id).await.expect("get user");

    assert!(found.is_some());
    let found = found.expect("user should exist");
    assert_eq!(found.id, created.id);
    assert_eq!(found.username, "viewer");
}

#[tokio::test]
async fn create_user_rejects_duplicate_username() {
    let (app, user) = bootstrap();

    let _created = app
        .create_user(
            &user,
            "editor".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("first create");

    let second = app
        .create_user(
            &user,
            "editor".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await;

    assert!(second.is_err());
}

#[tokio::test]
async fn delete_title_removes_title_from_catalog() {
    let (app, user) = bootstrap();

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    let titles = app
        .list_titles(&user, Some(MediaFacet::Movie), None)
        .await
        .expect("list titles");
    assert!(titles.is_empty());
}

#[tokio::test]
async fn delete_title_cancels_queue_items_linked_via_submission_metadata() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
    );

    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Delete Me".into(),
                facet: MediaFacet::Movie,
                monitored: false,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: created.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "queue-fallback".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some(created.name.clone()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    *download_client.queue_items.lock().await = vec![
        DownloadQueueItem {
            id: "queue-direct".to_string(),
            title_id: Some(created.id.clone()),
            episode_id: None,
            title_name: created.name.clone(),
            facet: Some("movie".to_string()),
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "nzbget".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-direct".to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-fallback".to_string(),
            title_id: None,
            episode_id: None,
            title_name: created.name.clone(),
            facet: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "sabnzbd".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-fallback".to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-unrelated".to_string(),
            title_id: None,
            episode_id: None,
            title_name: "Other".to_string(),
            facet: None,
            client_id: "primary".to_string(),
            client_name: "Primary".to_string(),
            client_type: "sabnzbd".to_string(),
            state: DownloadQueueState::Queued,
            progress_percent: 0,
            size_bytes: None,
            remaining_seconds: None,
            queued_at: None,
            last_updated_at: None,
            attention_required: false,
            attention_reason: None,
            download_client_item_id: "queue-unrelated".to_string(),
            import_status: None,
            import_error_code: None,
            import_error_message: None,
            imported_at: None,
            delete_status: None,
            delete_error_message: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
    ];

    app.delete_title(&user, &created.id, false, None)
        .await
        .expect("delete title");

    let deleted_items = download_client.deleted_items.lock().await.clone();
    assert_eq!(
        deleted_items,
        vec![
            ("queue-direct".to_string(), false),
            ("queue-fallback".to_string(), false),
        ]
    );
    assert_eq!(
        pending_releases.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert_eq!(
        download_submissions.deleted_title_ids.lock().await.clone(),
        vec![created.id.clone()]
    );
    assert!(
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .all(|entry| entry.title_id != created.id)
    );
}

#[tokio::test]
async fn list_download_queue_does_not_treat_stub_submission_as_origin() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "SABnzbd".to_string(),
            client_type: "sabnzbd".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: String::new(),
            facet: String::new(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "foreign-stub".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Foreign Download".to_string()),
            request_signature: None,
            scope: SubmissionScope::Orphan,
        })
        .await
        .expect("record stub submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "foreign-stub".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Foreign Download".to_string(),
        facet: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "sabnzbd".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "foreign-stub".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue");

    assert_eq!(items.len(), 1);
    assert!(!items[0].is_scryer_origin);
    assert!(items[0].title_id.is_none());
    assert!(items[0].facet.is_none());
}

#[tokio::test]
async fn list_download_queue_uses_live_queue_only_for_all_activity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![DownloadQueueItem {
        id: "history-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "History Download".to_string(),
        facet: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Completed,
        progress_percent: 100,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: Some("100".to_string()),
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "history-1".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue(&user, true, false, false, DownloadActivityFilter::All)
        .await
        .expect("list queue should succeed");

    assert!(items.is_empty());
    assert_eq!(*download_client.history_calls.lock().await, 0);
    assert!(
        download_client
            .recent_activity_calls
            .lock()
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn list_download_queue_for_title_uses_title_scoped_client_query() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: "title-1".to_string(),
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "job-1".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Title Scoped Download".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "job-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Title Scoped Download".to_string(),
        facet: None,
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "job-1".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue_for_title(
            &user,
            "title-1",
            false,
            false,
            false,
            DownloadActivityFilter::All,
        )
        .await
        .expect("title-scoped queue should load");

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].download_client_item_id, "job-1");
    assert_eq!(items[0].title_id.as_deref(), Some("title-1"));
    assert_eq!(*download_client.queue_calls.lock().await, 0);
    assert_eq!(
        download_client
            .queue_for_title_calls
            .lock()
            .await
            .as_slice(),
        &["title-1".to_string()]
    );
    assert!(
        download_client
            .recent_activity_for_title_calls
            .lock()
            .await
            .is_empty()
    );
}

fn queue_history_fixture_item(
    download_client_item_id: &str,
    state: DownloadQueueState,
    last_updated_at: i64,
) -> DownloadQueueItem {
    DownloadQueueItem {
        id: download_client_item_id.to_string(),
        title_id: Some("title-1".to_string()),
        episode_id: None,
        title_name: format!("Fixture {download_client_item_id}"),
        facet: Some("movie".to_string()),
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state,
        progress_percent: 100,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: Some(last_updated_at.to_string()),
        attention_required: false,
        attention_reason: None,
        download_client_item_id: download_client_item_id.to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

fn completed_download_fixture_item(
    download_client_item_id: &str,
    title_id: &str,
    name: &str,
    dest_dir: &str,
) -> CompletedDownload {
    CompletedDownload {
        client_type: "nzbget".to_string(),
        client_id: "primary".to_string(),
        download_client_item_id: download_client_item_id.to_string(),
        name: name.to_string(),
        dest_dir: dest_dir.to_string(),
        category: Some("movie".to_string()),
        size_bytes: None,
        completed_at: Some(Utc::now()),
        parameters: vec![
            ("*scryer_title_id".to_string(), title_id.to_string()),
            ("*scryer_facet".to_string(), "movie".to_string()),
        ],
    }
}

#[tokio::test]
async fn list_download_import_page_returns_only_import_rows_for_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await =
        vec![completed, failed, blocked.clone(), pending, importing];

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::Blocked)
        .await
        .expect("import page should load");

    assert_eq!(page.total_count, 1);
    assert!(!page.has_more);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-1");
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::ImportBlocked
    );
}

#[tokio::test]
async fn count_download_import_items_matches_selected_filter() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Running);

    let mut pending =
        queue_history_fixture_item("pending-1", DownloadQueueState::ImportPending, 30);
    pending.tracked_state = Some(TrackedDownloadState::ImportPending);

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);

    let failed = queue_history_fixture_item("failed-1", DownloadQueueState::Failed, 10);
    let completed = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 5);

    *download_client.history_items.lock().await =
        vec![completed, failed, blocked, pending.clone(), importing];

    let all_page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("all import page");
    let all_count = app
        .count_download_import_items(&user, DownloadImportFilter::All)
        .await
        .expect("all import count");
    let pending_count = app
        .count_download_import_items(&user, DownloadImportFilter::Pending)
        .await
        .expect("pending import count");

    assert_eq!(all_count, all_page.total_count as i64);
    assert_eq!(pending_count, 1);
    assert_eq!(pending.download_client_item_id, "pending-1");
}

#[tokio::test]
async fn list_download_import_page_returns_promptly_when_tracked_snapshot_handle_never_replies() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![queue_history_fixture_item(
        "pending-1",
        DownloadQueueState::ImportPending,
        40,
    )];

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive even when the tracked snapshot handle is wedged")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "pending-1");
}

#[tokio::test]
async fn list_download_import_page_uses_runtime_tracked_snapshot_cache() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, _blocked_rx) = tokio::sync::mpsc::channel(1);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.history_items.lock().await = vec![queue_history_fixture_item(
        "completed-1",
        DownloadQueueState::Completed,
        40,
    )];

    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some("primary"), "nzbget", "completed-1");
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                title_id: Some("title-1".to_string()),
                facet: Some("series".to_string()),
                source_title: Some("Cached Release".to_string()),
                state: TrackedDownloadState::ImportBlocked,
                status: scryer_domain::TrackedDownloadStatus::Warning,
                status_messages: vec!["moving files to nas".to_string()],
                match_type: scryer_domain::TitleMatchType::Submission,
            },
        );

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 50, 0, DownloadImportFilter::All),
    )
    .await
    .expect("download import page should stay responsive with cached tracked metadata")
    .expect("download import page should load");

    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::ImportBlocked)
    );
    assert_eq!(
        page.items[0].tracked_status,
        Some(scryer_domain::TrackedDownloadStatus::Warning)
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["moving files to nas".to_string()]
    );
    assert_eq!(page.items[0].title_id.as_deref(), Some("title-1"));
}

#[tokio::test]
async fn list_download_import_page_degrades_promptly_for_limit_one_count_reads_when_snapshot_cache_is_contended()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut importing =
        queue_history_fixture_item("importing-1", DownloadQueueState::Completed, 40);
    importing.import_status = Some(ImportStatus::Processing);
    *download_client.history_items.lock().await = vec![importing];

    let _snapshot_guard = app
        .runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await;

    let page = timeout(
        Duration::from_millis(100),
        app.list_download_import_page(&user, 1, 0, DownloadImportFilter::All),
    )
    .await
    .expect("limit-one count-style download import read should degrade instead of blocking")
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "importing-1");
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(page.items[0].tracked_state, None);
}

#[tokio::test]
async fn download_import_page_stays_responsive_while_background_import_worker_is_blocked() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (tracked_download_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let (app, user) = bootstrap_with_cleanup_tracking_and_tracked_handle(
        download_client.clone(),
        download_submissions,
        pending_releases,
        crate::tracked_downloads::TrackedDownloadHandle::new(tracked_download_tx),
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let media_files = Arc::new(MockMediaFileRepo::default());
    let file_importer = Arc::new(BlockingFileImporter::default());
    let app = app.with_test_overrides(|services| {
        services
            .with_imports(import_repo.clone())
            .with_media_files(media_files)
            .with_file_importer(file_importer.clone())
    });

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Responsive Import Test".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create monitored movie title");

    let tempdir = tempfile::tempdir().expect("tempdir");
    let completed_dir = tempdir
        .path()
        .join("Responsive.Import.Test.2026.1080p.WEB-DL");
    std::fs::create_dir_all(&completed_dir).expect("create completed download dir");
    let source_video = completed_dir.join("Responsive.Import.Test.2026.1080p.WEB-DL.mkv");
    std::fs::write(&source_video, b"fake-video").expect("seed completed download video");

    let item_id = "blocked-worker-1";
    let release_name = "Responsive.Import.Test.2026.1080p.WEB-DL";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = release_name.to_string();
    history_item.facet = Some("movie".to_string());
    *download_client.history_items.lock().await = vec![history_item];
    *download_client.completed_downloads.lock().await = vec![completed_download_fixture_item(
        item_id,
        &title.id,
        release_name,
        completed_dir.to_string_lossy().as_ref(),
    )];

    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(crate::integration::start_download_queue_poller(
        app.clone(),
        token.child_token(),
        tracked_download_rx,
    ));

    timeout(Duration::from_secs(5), async {
        loop {
            if file_importer.call_count.load(Ordering::SeqCst) > 0 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background import worker should reach the file importer under parallel test load");

    let page = timeout(
        Duration::from_millis(150),
        app.list_download_import_page(&user, 1, 0, DownloadImportFilter::All),
    )
    .await
    .expect(
        "download import read should stay responsive while the background import worker is blocked",
    )
    .expect("download import page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, item_id);
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Importing)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Processing));
    assert_eq!(
        crate::integration::derive_download_queue_display_state(&page.items[0]),
        DownloadDisplayState::Importing
    );
    assert_eq!(
        page.items[0].tracked_status_messages,
        vec!["Moving files to library.".to_string()]
    );

    token.cancel();
    file_importer.release.notify_waiters();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn list_download_history_page_filters_terminal_rows_and_clamps_page_size_to_50() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut history_items = (0..120)
        .map(|index| {
            queue_history_fixture_item(
                &format!("completed-{index}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            )
        })
        .collect::<Vec<_>>();
    history_items.extend((0..5).map(|index| {
        queue_history_fixture_item(
            &format!("failed-{index}"),
            DownloadQueueState::Failed,
            20_000 - index as i64,
        )
    }));

    let mut blocked =
        queue_history_fixture_item("blocked-import", DownloadQueueState::Completed, 30_000);
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    history_items.push(blocked);

    *download_client.history_items.lock().await = history_items;

    let failed_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            None,
            false,
            None,
        )
        .await
        .expect("failed history page should load");
    assert_eq!(failed_page.total_count, 5);
    assert_eq!(failed_page.items.len(), 5);
    assert_eq!(failed_page.available_clients.len(), 1);
    assert!(
        failed_page
            .items
            .iter()
            .all(|item| item.state == DownloadQueueState::Failed)
    );
    assert!(!failed_page.has_more);

    let all_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("all history page should load");
    assert_eq!(all_page.total_count, 125);
    assert_eq!(all_page.items.len(), 50);
    assert!(all_page.has_more);
    assert_eq!(
        all_page.items[0].download_client_item_id, "failed-0",
        "newest terminal rows should be returned first"
    );
    assert!(all_page.items.iter().all(
        |item| crate::integration::derive_download_queue_display_state(item)
            != DownloadDisplayState::ImportBlocked
    ));

    let client_filtered_page = app
        .list_download_history_page(
            &user,
            250,
            0,
            Some(vec![DownloadHistoryFilter::Failed]),
            Some(vec!["primary".to_string()]),
            false,
            None,
        )
        .await
        .expect("client filtered history page should load");
    assert_eq!(client_filtered_page.total_count, 5);
    assert_eq!(client_filtered_page.available_clients.len(), 1);
}

#[tokio::test]
async fn list_download_history_page_sorts_before_paginating() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let history_items = (0..60)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("sort-{index:02}"),
                DownloadQueueState::Completed,
                10_000 - index as i64,
            );
            item.title_name = format!("Title {:02}", 59 - index);
            item
        })
        .collect::<Vec<_>>();

    *download_client.history_items.lock().await = history_items;

    let first_page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("sorted history page should load");

    let second_page = app
        .list_download_history_page(
            &user,
            50,
            50,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            Some(DownloadHistorySort {
                key: DownloadHistorySortKey::Title,
                direction: SortDirection::Asc,
            }),
        )
        .await
        .expect("second sorted history page should load");

    assert_eq!(first_page.items.len(), 50);
    assert_eq!(second_page.items.len(), 10);
    assert_eq!(first_page.items[0].title_name, "Title 00");
    assert_eq!(first_page.items[49].title_name, "Title 49");
    assert_eq!(second_page.items[0].title_name, "Title 50");
}

#[tokio::test]
async fn list_download_history_page_can_limit_to_scryer_submitted_rows() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut scryer_item =
        queue_history_fixture_item("scryer-item", DownloadQueueState::Completed, 100);
    scryer_item.client_id = "primary".to_string();
    scryer_item.client_name = "Primary".to_string();

    let mut external_item =
        queue_history_fixture_item("external-item", DownloadQueueState::Failed, 90);
    external_item.is_scryer_origin = false;
    external_item.client_id = "secondary".to_string();
    external_item.client_name = "Secondary".to_string();

    *download_client.history_items.lock().await = vec![scryer_item, external_item];

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            true,
            None,
        )
        .await
        .expect("scryer filtered history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert!(page.items.iter().all(|item| item.is_scryer_origin));
    assert_eq!(page.available_clients.len(), 1);
    assert_eq!(page.available_clients[0].client_id, "primary");
}

#[tokio::test]
async fn recent_activity_and_history_ignore_operational_domain_events() {
    let (app, user) = bootstrap();

    app.add_title(
        &user,
        NewTitle {
            name: "Activity Filter Fixture".to_string(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            poster_url: None,
            year: None,
            overview: None,
            sort_title: None,
            slug: None,
            runtime_minutes: None,
            language: None,
            content_status: None,
        },
    )
    .await
    .expect("title should be added");

    app.append_domain_event(crate::domain_events::new_global_domain_event(
        None,
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "job-run-1".to_string(),
            job_key: "rss_sync".to_string(),
            operation_type: "job".to_string(),
            trigger_source: "system_internal".to_string(),
        }),
    ))
    .await
    .expect("operational domain event should append");

    let activities = app
        .recent_activity(&user, 10, 0)
        .await
        .expect("recent activity should load");
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::TitleAdded);

    let history = app
        .recent_events(&user, None, 10, 0)
        .await
        .expect("recent events should load");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].event_type, EventType::TitleAdded);

    let after_sequence = app
        .list_activity_events_after_sequence(&user, 0, 10)
        .await
        .expect("activity replay should load");
    assert_eq!(after_sequence.len(), 1);
    assert_eq!(after_sequence[0].1.kind, ActivityKind::TitleAdded);
}

#[tokio::test]
async fn download_queue_subscription_bootstraps_from_live_queue_without_history_events() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "queue-1".to_string(),
        title_id: None,
        episode_id: None,
        title_name: "Foreign Queue Item".to_string(),
        facet: Some("movie".to_string()),
        client_id: "primary".to_string(),
        client_name: "NZBGet".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 10,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "queue-1".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].download_client_item_id, "queue-1");
    assert_eq!(snapshot[0].title_name, "Foreign Queue Item");
}

#[tokio::test]
async fn download_queue_subscription_sends_empty_bootstrap_snapshot() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) =
        bootstrap_with_cleanup_tracking(download_client, download_submissions, pending_releases);

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let mut receiver = app
        .subscribe_download_queue(&user)
        .expect("queue subscription should start");
    let snapshot = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial empty queue snapshot should arrive")
        .expect("queue subscription should stay open");

    assert!(snapshot.is_empty());
}

#[tokio::test]
async fn queued_delete_poller_executes_client_delete() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-1", true)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Completed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should complete");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Completed
    );
    assert_eq!(
        download_client.deleted_items.lock().await.clone(),
        vec![("job-1".to_string(), true)]
    );
}

#[tokio::test]
async fn queued_delete_poller_marks_failure_and_persists_error() {
    let download_client = Arc::new(StubDownloadClient::default());
    download_client
        .set_delete_error(Some("delete failed"))
        .await;
    let download_queue_commands = Arc::new(TrackingDownloadQueueCommandRepo::default());
    let command_id = download_queue_commands
        .seed_pending(None, "nzbget", "job-2", false)
        .await;
    let (app, _) =
        bootstrap_with_delete_queue(download_client.clone(), download_queue_commands.clone());
    let token = tokio_util::sync::CancellationToken::new();

    let handle = tokio::spawn(start_background_download_delete_poller(
        app,
        token.child_token(),
    ));

    let record = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(record) = download_queue_commands.get(&command_id).await
                && record.status == scryer_domain::DownloadQueueDeleteStatus::Failed
            {
                break record;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("queued delete should fail");

    token.cancel();
    handle.await.expect("delete poller should stop cleanly");

    assert_eq!(
        record.status,
        scryer_domain::DownloadQueueDeleteStatus::Failed
    );
    assert_eq!(
        record.error_text.as_deref(),
        Some("repository: delete failed")
    );
    assert!(download_client.deleted_items.lock().await.is_empty());
}

#[tokio::test]
async fn active_library_scans_and_subscription_use_runtime_tracker_state() {
    let (app, user) = bootstrap();

    let session = app
        .runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "scan-runtime-1".to_string(),
            MediaFacet::Movie,
            LibraryScanMode::Full,
        )
        .await
        .expect("library scan session should start");

    let active = app
        .active_library_scans(&user)
        .await
        .expect("active scans should load");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, session.session_id);

    let mut receiver = app
        .subscribe_library_scan_progress(&user)
        .expect("library scan subscription should start");
    let initial = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("initial library scan snapshot should arrive")
        .expect("library scan subscription should stay open");

    assert_eq!(initial.session_id, session.session_id);
    assert_eq!(initial.facet, session.facet);
}

#[tokio::test]
async fn notification_broadcast_ignores_operational_domain_events() {
    let (app, _) = bootstrap();
    let mut receiver = app.runtime.events.notification_event_broadcast.subscribe();

    app.append_domain_event(crate::domain_events::new_global_domain_event(
        None,
        DomainEventPayload::JobRunStarted(JobRunStartedEventData {
            run_id: "run-1".to_string(),
            job_key: "rss_sync".to_string(),
            operation_type: "job".to_string(),
            trigger_source: "system_internal".to_string(),
        }),
    ))
    .await
    .expect("operational event should append");

    assert!(
        matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "operational events should not wake notification dispatcher"
    );

    let notification = app
        .append_domain_event(crate::domain_events::new_global_domain_event(
            None,
            DomainEventPayload::TitleAdded(scryer_domain::TitleAddedEventData {
                title: scryer_domain::TitleContextSnapshot {
                    title_name: "Wake Fixture".to_string(),
                    facet: MediaFacet::Movie,
                    year: Some(2024),
                    poster_url: None,
                    external_ids: scryer_domain::DomainExternalIds::default(),
                },
            }),
        ))
        .await
        .expect("notification event should append");

    let wake = receiver
        .recv()
        .await
        .expect("notification wake should arrive after notification event");
    assert_eq!(wake, notification.sequence);
}

#[tokio::test]
async fn notification_broadcast_wakes_once_for_notification_batches() {
    let (app, _) = bootstrap();
    let mut receiver = app.runtime.events.notification_event_broadcast.subscribe();

    let stored = app
        .append_domain_events(vec![
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::JobRunStarted(JobRunStartedEventData {
                    run_id: "run-1".to_string(),
                    job_key: "rss_sync".to_string(),
                    operation_type: "job".to_string(),
                    trigger_source: "system_internal".to_string(),
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::TitleAdded(scryer_domain::TitleAddedEventData {
                    title: scryer_domain::TitleContextSnapshot {
                        title_name: "First Notification".to_string(),
                        facet: MediaFacet::Movie,
                        year: Some(2024),
                        poster_url: None,
                        external_ids: scryer_domain::DomainExternalIds::default(),
                    },
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::ImportRejected(scryer_domain::ImportRejectedEventData {
                    title: Some(scryer_domain::TitleContextSnapshot {
                        title_name: "Second Notification".to_string(),
                        facet: MediaFacet::Movie,
                        year: Some(2024),
                        poster_url: None,
                        external_ids: scryer_domain::DomainExternalIds::default(),
                    }),
                    status: ImportStatus::Failed,
                    import_id: None,
                    source_system: Some("download_client".to_string()),
                    source_ref: Some("queue-2".to_string()),
                    source_title: Some("Second.Notification.1080p".to_string()),
                    source_path: Some("/downloads/example.mkv".to_string()),
                    dest_path: None,
                    quality: Some("1080p".to_string()),
                    reason: Some("not parsable".to_string()),
                    skip_reason: None,
                    episode_ids: Vec::new(),
                }),
            ),
            crate::domain_events::new_global_domain_event(
                None,
                DomainEventPayload::JobRunCompleted(JobRunCompletedEventData {
                    run_id: "run-1".to_string(),
                    job_key: "rss_sync".to_string(),
                    summary_text: Some("done".to_string()),
                }),
            ),
        ])
        .await
        .expect("batch should append");

    let wake = tokio::time::timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("notification wake should arrive")
        .expect("notification broadcast should stay open");
    assert_eq!(
        wake,
        stored.last().expect("batch should have events").sequence,
        "mixed batches should publish a high-water wake hint"
    );

    assert!(
        matches!(
            receiver.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ),
        "a mixed batch should emit one notification wake, not one per notification event"
    );
}

fn failed_history_item(download_client_item_id: &str, title_name: &str) -> DownloadQueueItem {
    DownloadQueueItem {
        id: download_client_item_id.to_string(),
        title_id: None,
        episode_id: None,
        title_name: title_name.to_string(),
        facet: Some("movie".to_string()),
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Failed,
        progress_percent: 100,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: true,
        attention_reason: Some("corrupt archive".to_string()),
        download_client_item_id: download_client_item_id.to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }
}

#[tokio::test]
async fn acquisition_cycle_retries_standby_candidate_after_failed_grab() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let wanted = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(
            (Utc::now() - chrono::Duration::days(30))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        status: WantedStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    app.run_acquisition_cycle_once().await;

    let updated = wanted_items
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, WantedStatus::Grabbed);
    assert_eq!(updated.current_score, None);
    assert!(
        updated
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains("Standby.Release.1080p.WEB-DL")
    );

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
    assert!(pending_releases.store.lock().await.iter().any(|release| {
        release.release_title == "Standby.Release.1080p.WEB-DL"
            && release.status == PendingReleaseStatus::Grabbed
    }));

    let submissions = download_submissions.store.lock().await.clone();
    assert!(
        !submissions
            .iter()
            .any(|submission| submission.download_client_item_id == "failed-job")
    );
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref()
                == Some("nzb_url|https://example.com/standby.nzb|Standby.Release.1080p.WEB-DL")
    }));

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn tracked_download_failure_reuses_standby_recovery_policy() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked Failure Recovery".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let wanted = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(
            (Utc::now() - chrono::Duration::days(30))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        status: WantedStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "nzbget:failed-job".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item("failed-job", "Failed.Release.1080p.WEB-DL"),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        path_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    let updated = wanted_items
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted exists");
    assert_eq!(updated.status, WantedStatus::Grabbed);
    assert!(
        updated
            .grabbed_release
            .as_deref()
            .unwrap_or_default()
            .contains("Standby.Release.1080p.WEB-DL")
    );

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
    assert!(pending_releases.store.lock().await.iter().any(|release| {
        release.release_title == "Standby.Release.1080p.WEB-DL"
            && release.status == PendingReleaseStatus::Grabbed
    }));

    let submissions = download_submissions.store.lock().await.clone();
    assert!(
        !submissions
            .iter()
            .any(|submission| submission.download_client_item_id == "failed-job")
    );
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == format!("job-for-{}", title.id)
            && submission.source_title.as_deref() == Some("Standby.Release.1080p.WEB-DL")
            && submission.request_signature.as_deref()
                == Some("nzb_url|https://example.com/standby.nzb|Standby.Release.1080p.WEB-DL")
    }));

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn tracked_download_failure_requeues_episode_items_after_failed_season_pack() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Season Pack Failure Recovery".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "7".to_string(),
            label: Some("Season 7".to_string()),
            ordered_path: None,
            narrative_order: Some("7".to_string()),
            first_episode_number: Some("23".to_string()),
            last_episode_number: Some("24".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let original_next_search_at = (Utc::now() + chrono::Duration::hours(12)).to_rfc3339();
    let mut expected_wanted_ids = Vec::new();
    for (episode_number, label) in [("23", "S07E23"), ("24", "S07E24")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("7".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted = WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            season_number: Some("7".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(original_next_search_at.clone()),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(30)).to_rfc3339()),
            search_count: 1,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        };
        expected_wanted_ids.push(wanted.id.clone());
        wanted_items
            .upsert_wanted_item(&wanted)
            .await
            .expect("seed episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-season-pack".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed season pack submission");

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "nzbget:failed-season-pack".to_string(),
        client_id: "primary".to_string(),
        client_type: "nzbget".to_string(),
        client_item: failed_history_item(
            "failed-season-pack",
            "Season.Pack.Failure.Recovery.S07.1080p.WEB-DL",
        ),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("anime".to_string()),
        source_title: Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        path_missing_since: None,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    for wanted_id in expected_wanted_ids {
        let wanted = wanted_items
            .get_wanted_item_by_id(&wanted_id)
            .await
            .expect("get wanted item")
            .expect("wanted item exists");
        assert_eq!(wanted.status, WantedStatus::Wanted);
        assert!(wanted.grabbed_release.is_none());
        let next_search_at = wanted
            .next_search_at
            .as_deref()
            .and_then(crate::quality_profile::parse_published_at)
            .expect("next search should parse");
        let original_next_search_at =
            crate::quality_profile::parse_published_at(&original_next_search_at)
                .expect("original next search should parse");
        assert!(next_search_at < original_next_search_at);
        assert!(next_search_at <= Utc::now());
    }

    let history = app
        .list_title_history(
            &user,
            &TitleHistoryFilter {
                event_types: Some(vec![
                    TitleHistoryEventType::DownloadFailed,
                    TitleHistoryEventType::Blocklisted,
                ]),
                title_ids: Some(vec![title.id.clone()]),
                title_search: None,
                download_id: Some("failed-season-pack".to_string()),
                episode_id: None,
                group_by_event: false,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("list title history");

    assert_eq!(history.total_count, 4);
    assert!(history.records.iter().any(|record| {
        record.event_type == TitleHistoryEventType::DownloadFailed
            && record.collection_id.as_deref() == Some(season.id.as_str())
            && record.download_id.as_deref() == Some("failed-season-pack")
            && record.client_id.as_deref() == Some("primary")
            && record.client_name.as_deref() == Some("Primary")
            && record
                .failure_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("re-queuing season episodes"))
    }));
    assert!(history.records.iter().any(|record| {
        record.event_type == TitleHistoryEventType::Blocklisted
            && record.collection_id.as_deref() == Some(season.id.as_str())
            && record.download_id.as_deref() == Some("failed-season-pack")
            && record
                .blocklist_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("download client failure"))
    }));

    assert!(
        !download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.download_client_item_id == "failed-season-pack")
    );
}

#[tokio::test]
async fn acquisition_cycle_looks_up_submissions_once_per_title_for_grabbed_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Shared Grabbed Title".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    for (item_id, episode_id, release_title) in [
        ("wanted-1", "episode-1", "Shared.Release.S01E01"),
        ("wanted-2", "episode-2", "Shared.Release.S01E02"),
    ] {
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: item_id.to_string(),
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode_id.to_string()),
                collection_id: None,
                season_number: Some("1".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: None,
                last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
                search_count: 1,
                baseline_date: Some(
                    (Utc::now() - chrono::Duration::days(7))
                        .format("%Y-%m-%d")
                        .to_string(),
                ),
                status: WantedStatus::Grabbed,
                grabbed_release: Some(
                    serde_json::json!({
                        "title": release_title,
                        "score": 100,
                        "grabbed_at": Utc::now().to_rfc3339(),
                    })
                    .to_string(),
                ),
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed grabbed wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "shared-job".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Shared.Release".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record shared submission");

    app.run_acquisition_cycle_once().await;

    let calls = download_submissions
        .list_for_title_calls
        .lock()
        .await
        .clone();
    assert_eq!(calls, vec![title.id.clone()]);
}

#[tokio::test]
async fn acquisition_cycle_records_failed_collection_submission_once() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Shared Failed Season Pack".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let pack_title = "Shared Failed Season Pack.S01.1080p.WEB-DL";
    let grabbed_release = serde_json::json!({
        "title": pack_title,
        "score": 100,
        "grabbed_at": Utc::now().to_rfc3339(),
        "season_pack": true,
    })
    .to_string();

    let mut wanted_ids = Vec::new();
    for (episode_number, label) in [("1", "S01E01"), ("2", "S01E02")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted_id = Id::new().0;
        wanted_ids.push(wanted_id.clone());
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: wanted_id,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                season_number: Some("1".to_string()),
                episode_number: Some(episode_number.to_string()),
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: None,
                last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
                search_count: 1,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Grabbed,
                grabbed_release: Some(grabbed_release.clone()),
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed grabbed wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "shared-failed-season-pack".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some(pack_title.to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed collection submission");

    *download_client.history_items.lock().await = vec![DownloadQueueItem {
        title_id: Some(title.id.clone()),
        facet: Some("anime".to_string()),
        ..failed_history_item("shared-failed-season-pack", pack_title)
    }];

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(1) && search.episode.is_some())
    );

    let wanted_store = wanted_items.store.lock().await.clone();
    for wanted_id in wanted_ids {
        let wanted = wanted_store
            .iter()
            .find(|wanted| wanted.id == wanted_id)
            .expect("wanted item exists");
        assert_eq!(wanted.status, WantedStatus::Grabbed);
        assert!(wanted.grabbed_release.is_some());
    }

    let blocklist = app
        .list_title_release_blocklist(&user, &title.id, 10)
        .await
        .expect("list title release blocklist");
    assert_eq!(blocklist.len(), 1);
    assert!(
        blocklist[0]
            .source_title
            .as_deref()
            .is_some_and(|title| title.eq_ignore_ascii_case(pack_title))
    );
    assert!(
        blocklist[0]
            .error_message
            .as_deref()
            .is_some_and(|message| !message.trim().is_empty())
    );

    assert!(
        !download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| submission.download_client_item_id == "shared-failed-season-pack")
    );
}

#[tokio::test]
async fn acquisition_cycle_episode_submission_blocks_only_matching_episode() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Episode Blocking Scope".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season one");

    let season_two = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season two");

    let episode_one = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Season 1 Premiere".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode one");

    let episode_two = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_two.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E01".to_string()),
            title: Some("Season 2 Premiere".to_string()),
            air_date: Some("2025-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode two");

    for episode in [&episode_one, &episode_two] {
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                season_number: episode.season_number.clone(),
                episode_number: None,
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "episode-one-active".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Episode.Blocking.Scope.S01E01".to_string()),
            request_signature: None,
            scope: SubmissionScope::Episode {
                episode_id: episode_one.id.clone(),
            },
        })
        .await
        .expect("record active episode submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "episode-one-active".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: Some(episode_one.id.clone()),
        title_name: title.name.clone(),
        facet: Some("anime".to_string()),
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "episode-one-active".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    app.run_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(2) && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_collection_submission_blocks_same_season_only() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Season Pack Blocking Scope".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season one");

    let season_two = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "2".to_string(),
            label: Some("Season 2".to_string()),
            ordered_path: None,
            narrative_order: Some("2".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season two");

    for (collection, season_number, episode_number, label) in [
        (&season_one, "1", "1", "S01E01"),
        (&season_one, "1", "2", "S01E02"),
        (&season_two, "2", "1", "S02E01"),
    ] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(collection.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some(season_number.to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                season_number: Some(season_number.to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "anime".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "season-one-pack".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Season.Pack.Blocking.Scope.S01".to_string()),
            request_signature: None,
            scope: SubmissionScope::Collection {
                collection_id: season_one.id.clone(),
            },
        })
        .await
        .expect("record active season pack submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "season-one-pack".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: None,
        title_name: title.name.clone(),
        facet: Some("anime".to_string()),
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "season-one-pack".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    app.run_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.season == Some(2) && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_falls_back_to_episode_grabs_when_season_pack_is_not_selected() {
    struct AutoGrabSeasonPackIndexerClient {
        searches: Arc<Mutex<Vec<RecordedIndexerSearch>>>,
    }

    #[async_trait]
    impl IndexerClient for AutoGrabSeasonPackIndexerClient {
        async fn search(
            &self,
            query: String,
            _ids: std::collections::HashMap<String, String>,
            _category: Option<String>,
            _facet: Option<String>,
            _newznab_categories: Option<Vec<String>>,
            _indexer_routing: Option<IndexerRoutingPlan>,
            _mode: SearchMode,
            season: Option<u32>,
            episode: Option<u32>,
            _absolute_episode: Option<u32>,
            _tagged_aliases: Vec<TaggedAlias>,
        ) -> AppResult<IndexerSearchResponse> {
            self.searches.lock().await.push(RecordedIndexerSearch {
                query: query.clone(),
                season,
                episode,
            });

            let release_title = match (season, episode) {
                (Some(_season), Some(_episode)) => format!("{query}.1080p.WEB-DL"),
                (Some(season), None) => {
                    let season_token = format!(" S{season:02}");
                    let base_query = query
                        .strip_suffix(&season_token)
                        .unwrap_or(query.as_str());
                    format!("{base_query} Season {season} - (1 - 2) [Typis]")
                }
                (None, _) => format!("{query}.2024.1080p.WEB-DL"),
            };
            let parsed_release_metadata = match (season, episode) {
                (Some(season), None) => {
                    let mut parsed = crate::parse_release_metadata(&release_title);
                    let mut episode_metadata = parsed.episode.unwrap_or_default();
                    episode_metadata.season = Some(season);
                    episode_metadata.full_season = true;
                    episode_metadata.release_type =
                        crate::ParsedEpisodeReleaseType::SeasonPack;
                    parsed.episode = Some(crate::ParsedEpisodeMetadata {
                        ..episode_metadata
                    });
                    parsed
                }
                _ => crate::parse_release_metadata(&release_title),
            };
            let release_slug = release_title.replace([' ', '/'], ".");

            Ok(IndexerSearchResponse {
                results: vec![IndexerSearchResult {
                    source: "nzbgeek".into(),
                    title: release_title.clone(),
                    link: Some(format!("https://example.invalid/info/{release_slug}")),
                    download_url: Some(format!(
                        "https://example.invalid/download/{release_slug}.nzb"
                    )),
                    source_kind: Some(DownloadSourceKind::NzbFile),
                    size_bytes: None,
                    published_at: Some("1970-01-01T00:00:00Z".into()),
                    thumbs_up: None,
                    thumbs_down: None,
                    indexer_languages: None,
                    indexer_subtitles: None,
                    indexer_grabs: None,
                    password_hint: None,
                    parsed_release_metadata: Some(parsed_release_metadata),
                    quality_profile_decision: Some(
                        crate::quality::profile::QualityProfileDecision {
                            release_score: 100,
                            scoring_log: Vec::new(),
                            allowed: true,
                            block_codes: Vec::new(),
                            preference_score: 100,
                        },
                    ),
                    extra: Default::default(),
                    guid: Some(format!("guid-{release_slug}")),
                    info_url: Some(format!("https://example.invalid/info/{release_slug}")),
                    provenance: None,
                    auto_eligible: Some(true),
                    auto_decision_code: None,
                    auto_decision_summary: None,
                    candidate_token: None,
                    queue_scope: None,
                }],
                api_current: None,
                api_max: None,
                grab_current: None,
                grab_max: None,
            })
        }
    }

    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let recorded_searches = Arc::new(Mutex::new(Vec::new()));
    let indexer_client: Arc<dyn IndexerClient> = Arc::new(AutoGrabSeasonPackIndexerClient {
        searches: recorded_searches.clone(),
    });
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    app.create_download_client_config(
        &user,
        NewDownloadClientConfig {
            name: "NZBGet".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Bleach".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut wanted_ids = Vec::new();
    for (episode_number, label) in [("1", "S01E01"), ("2", "S01E02")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("1".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let wanted_id = Id::new().0;
        wanted_ids.push(wanted_id.clone());
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: wanted_id,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                season_number: Some("1".to_string()),
                episode_number: Some(episode_number.to_string()),
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due wanted item");
    }

    app.run_acquisition_cycle_once().await;

    let searches = recorded_searches.lock().await.clone();
    assert!(
        searches
            .iter()
            .any(|search| search.season == Some(1) && search.episode.is_none())
    );
    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec![
            "Bleach S01E01.1080p.WEB-DL".to_string(),
            "Bleach S01E02.1080p.WEB-DL".to_string(),
        ]
    );

    let submissions = download_submissions.store.lock().await.clone();
    assert!(!submissions.is_empty());

    let wanted_store = wanted_items.store.lock().await.clone();
    for wanted_id in wanted_ids {
        let wanted = wanted_store
            .iter()
            .find(|wanted| wanted.id == wanted_id)
            .expect("wanted item exists");
        assert_eq!(wanted.status, WantedStatus::Grabbed);
        let grabbed_release: serde_json::Value = serde_json::from_str(
            wanted
                .grabbed_release
                .as_deref()
                .expect("grabbed release recorded"),
        )
        .expect("grabbed release should parse");
        let expected_title = match wanted.episode_number.as_deref() {
            Some("1") => "Bleach S01E01.1080p.WEB-DL",
            Some("2") => "Bleach S01E02.1080p.WEB-DL",
            other => panic!("unexpected episode number: {other:?}"),
        };
        assert_eq!(
            grabbed_release["title"].as_str(),
            Some(expected_title)
        );
        assert_ne!(grabbed_release["season_pack"].as_bool(), Some(true));
    }
}

#[tokio::test]
async fn acquisition_cycle_skips_recently_failed_season_pack_and_searches_episodes() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Recent Failed Season Pack".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "7".to_string(),
            label: Some("Season 7".to_string()),
            ordered_path: None,
            narrative_order: Some("7".to_string()),
            first_episode_number: Some("23".to_string()),
            last_episode_number: Some("24".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    for (episode_number, label) in [("23", "S07E23"), ("24", "S07E24")] {
        let episode = app
            .services
            .catalog
            .shows
            .create_episode(Episode {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some(episode_number.to_string()),
                season_number: Some("7".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode.id.clone()),
                collection_id: None,
                season_number: Some("7".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed due episode wanted item");
    }

    app.services
        .workflow
        .release_attempts
        .record_release_attempt(
            Some(title.id.clone()),
            None,
            Some("Recent.Failed.Season.Pack.S07.1080p.WEB-DL".to_string()),
            ReleaseDownloadAttemptOutcome::Failed,
            Some("download client failure: corrupt archive".to_string()),
            None,
        )
        .await
        .expect("record failed season pack attempt");

    app.run_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| search.season == Some(7)));
    assert!(searches.iter().all(|search| search.episode.is_some()));
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(7) && search.episode.is_none())
    );
}

#[tokio::test]
async fn acquisition_cycle_title_submission_still_blocks_movie_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Movie Blocking Scope".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie");

    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "movie-active".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Movie.Blocking.Scope".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record active movie submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "movie-active".to_string(),
        title_id: Some(title.id.clone()),
        episode_id: None,
        title_name: title.name.clone(),
        facet: Some("movie".to_string()),
        client_id: "primary".to_string(),
        client_name: "Primary".to_string(),
        client_type: "nzbget".to_string(),
        state: DownloadQueueState::Queued,
        progress_percent: 0,
        size_bytes: None,
        remaining_seconds: None,
        queued_at: None,
        last_updated_at: None,
        attention_required: false,
        attention_reason: None,
        download_client_item_id: "movie-active".to_string(),
        import_status: None,
        import_error_code: None,
        import_error_message: None,
        imported_at: None,
        delete_status: None,
        delete_error_message: None,
        is_scryer_origin: true,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    app.run_acquisition_cycle_once().await;

    assert!(indexer_client.searches.lock().await.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_active_anime_scan_does_not_block_due_movie_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Movie Survives Anime Scan".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due movie wanted item");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    let searches = indexer_client.searches.lock().await.clone();
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].query, title.name);
    assert_eq!(searches[0].season, None);
    assert_eq!(searches[0].episode, None);
}

#[tokio::test]
async fn acquisition_cycle_active_movie_scan_does_not_block_due_series_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Survives Movie Scan".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Movie],
    )
    .await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(
        searches
            .iter()
            .all(|search| search.query.contains(&title.name)
                && search.season == Some(1)
                && search.episode == Some(1))
    );
}

#[tokio::test]
async fn acquisition_cycle_active_series_scan_defers_due_series_search() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Series Deferred By Series Scan".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("1".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let episode = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Pilot".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode");

    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: Id::new().0,
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: None,
            title_facet: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2024-01-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed due series wanted item");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Series],
    )
    .await;

    assert!(indexer_client.searches.lock().await.is_empty());
}

#[tokio::test]
async fn acquisition_cycle_retries_standby_candidate_during_unrelated_active_scan() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failure Recovery During Scan".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;

    let wanted = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(5)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(
            (Utc::now() - chrono::Duration::days(30))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        status: WantedStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Failed.Release.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Standby.Release.1080p.WEB-DL".to_string(),
            release_url: Some("https://example.com/standby.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: Some(1_000),
            release_score: 150,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-standby".to_string()),
            added_at: Utc::now().to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: Some(Utc::now().to_rfc3339()),
            info_hash: None,
        })
        .await
        .expect("seed standby");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    *download_client.history_items.lock().await = vec![failed_history_item(
        "failed-job",
        "Failed.Release.1080p.WEB-DL",
    )];

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .clone(),
        vec!["Standby.Release.1080p.WEB-DL".to_string()]
    );
}

#[tokio::test]
async fn acquisition_cycle_prunes_stale_standby_rows_during_unrelated_active_scan() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Prune During Scan".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let wanted = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Stale.Standby.Release".to_string(),
            release_url: Some("https://example.com/stale.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: None,
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed stale standby");

    app.runtime
        .library
        .library_scan_tracker
        .start_session_with_id(
            "anime-scan-during-prune".to_string(),
            MediaFacet::Anime,
            LibraryScanMode::Full,
        )
        .await
        .expect("start anime scan");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(
        &app,
        &[MediaFacet::Anime],
    )
    .await;

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
}

#[tokio::test]
async fn trigger_title_mismatch_recovery_search_requeues_only_mismatch_only_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Mismatch Recovery".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let original_due_at = "2099-01-01T00:00:00Z".to_string();
    let recovery_item = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: Some(original_due_at.clone()),
        last_search_at: Some("2026-04-21T00:00:00Z".to_string()),
        search_count: 2,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let untouched_item = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: Some("episode-2".to_string()),
        collection_id: None,
        season_number: Some("1".to_string()),
        episode_number: None,
        media_type: "episode".to_string(),
        search_phase: "primary".to_string(),
        next_search_at: Some(original_due_at.clone()),
        last_search_at: Some("2026-04-21T00:00:00Z".to_string()),
        search_count: 2,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&recovery_item)
        .await
        .expect("seed recovery item");
    wanted_items
        .upsert_wanted_item(&untouched_item)
        .await
        .expect("seed untouched item");

    for suffix in 0..3 {
        wanted_items
            .insert_release_decision(&ReleaseDecision {
                id: format!("decision-recovery-{suffix}"),
                wanted_item_id: recovery_item.id.clone(),
                title_id: title.id.clone(),
                release_title: format!("Mismatch.Release.{suffix}"),
                release_url: None,
                release_size_bytes: None,
                decision_code: "title_mismatch".to_string(),
                candidate_score: 100,
                current_score: None,
                score_delta: None,
                explanation_json: None,
                created_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed mismatch decision");
    }
    wanted_items
        .insert_release_decision(&ReleaseDecision {
            id: "decision-untouched-1".to_string(),
            wanted_item_id: untouched_item.id.clone(),
            title_id: title.id.clone(),
            release_title: "Mixed.Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "title_mismatch".to_string(),
            candidate_score: 100,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed mixed decision");
    wanted_items
        .insert_release_decision(&ReleaseDecision {
            id: "decision-untouched-2".to_string(),
            wanted_item_id: untouched_item.id.clone(),
            title_id: title.id.clone(),
            release_title: "Eligible.Release".to_string(),
            release_url: None,
            release_size_bytes: None,
            decision_code: "eligible".to_string(),
            candidate_score: 120,
            current_score: None,
            score_delta: None,
            explanation_json: None,
            created_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed non-mismatch decision");

    let queued = app
        .trigger_title_mismatch_recovery_search(&title.id)
        .await
        .expect("trigger mismatch recovery");

    assert_eq!(queued, 1);

    let updated_recovery = wanted_items
        .get_wanted_item_by_id(&recovery_item.id)
        .await
        .expect("load recovery item")
        .expect("recovery item exists");
    let updated_untouched = wanted_items
        .get_wanted_item_by_id(&untouched_item.id)
        .await
        .expect("load untouched item")
        .expect("untouched item exists");

    assert_ne!(
        updated_recovery.next_search_at.as_deref(),
        Some(original_due_at.as_str())
    );
    assert_eq!(
        updated_untouched.next_search_at.as_deref(),
        Some(original_due_at.as_str())
    );
}

#[tokio::test]
async fn acquisition_cycle_prunes_stale_standby_rows_for_non_grabbed_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases.clone(),
        wanted_items.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Prune Me".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let wanted = WantedItem {
        id: Id::new().0,
        title_id: title.id.clone(),
        title_name: Some(title.name.clone()),
        title_slug: None,
        title_facet: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    pending_releases
        .insert_pending_release(&PendingRelease {
            id: Id::new().0,
            wanted_item_id: wanted.id.clone(),
            title_id: title.id.clone(),
            release_title: "Stale.Standby.Release".to_string(),
            release_url: Some("https://example.com/stale.nzb".to_string()),
            source_kind: Some(DownloadSourceKind::NzbUrl),
            release_size_bytes: None,
            release_score: 100,
            scoring_log_json: None,
            indexer_source: Some("nzbgeek".to_string()),
            release_guid: Some("guid-stale".to_string()),
            added_at: (Utc::now() - chrono::Duration::hours(30)).to_rfc3339(),
            delay_until: Utc::now().to_rfc3339(),
            status: PendingReleaseStatus::Standby,
            grabbed_at: None,
            source_password: None,
            published_at: None,
            info_hash: None,
        })
        .await
        .expect("seed stale standby");

    app.run_acquisition_cycle_once().await;

    assert!(
        pending_releases
            .list_all_standby_pending_releases()
            .await
            .expect("list standby")
            .is_empty()
    );
}

#[tokio::test]
async fn monitoring_interstitial_collection_reconciles_stale_episode_wanted_items() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );

    let title = app
        .services
        .catalog
        .titles
        .create(Title {
            id: Id::new().0,
            name: "Interstitial Only".into(),
            facet: MediaFacet::Anime,
            monitored: false,
            tags: vec!["scryer:monitor-type:none".into()],
            external_ids: vec![],
            created_by: Some(user.id.clone()),
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
            banner_url: None,
            banner_source_url: None,
            background_url: None,
            background_source_url: None,
            sort_title: None,
            slug: None,
            imdb_id: None,
            runtime_minutes: None,
            genres: vec![],
            content_status: None,
            language: None,
            first_aired: None,
            network: None,
            studio: None,
            country: None,
            aliases: vec![],
            tagged_aliases: vec![],
            metadata_language: None,
            metadata_fetched_at: None,
            min_availability: None,
            digital_release_date: None,
            folder_path: None,
        })
        .await
        .expect("create title");

    let season_one = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "1".to_string(),
            label: Some("Season 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1".to_string()),
            first_episode_number: Some("1".to_string()),
            last_episode_number: Some("2".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("create season one");

    let episode_one = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("1".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E01".to_string()),
            title: Some("Episode 1".to_string()),
            air_date: Some("2024-01-01".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode one");

    let episode_two = app
        .services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season_one.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("2".to_string()),
            season_number: Some("1".to_string()),
            episode_label: Some("S01E02".to_string()),
            title: Some("Episode 2".to_string()),
            air_date: Some("2024-01-08".to_string()),
            duration_seconds: Some(1_440),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("create episode two");

    let interstitial = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Interstitial,
            collection_index: "1.1".to_string(),
            label: Some("Movie 1".to_string()),
            ordered_path: None,
            narrative_order: Some("1.1".to_string()),
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: Some(scryer_domain::InterstitialMovieMetadata {
                tvdb_id: "movie-1".to_string(),
                name: "Movie 1".to_string(),
                slug: "movie-1".to_string(),
                year: Some(2024),
                content_status: "released".to_string(),
                overview: "Interstitial movie".to_string(),
                poster_url: String::new(),
                language: "ja".to_string(),
                runtime_minutes: 110,
                sort_title: "Movie 1".to_string(),
                imdb_id: String::new(),
                genres: vec!["action".to_string()],
                studio: "Studio".to_string(),
                digital_release_date: Some("2024-02-01".to_string()),
                association_confidence: Some("high".to_string()),
                continuity_status: Some("canon".to_string()),
                movie_form: None,
                confidence: None,
                signal_summary: None,
                placement: Some("between_seasons".to_string()),
                movie_tmdb_id: None,
                movie_mal_id: None,
                movie_anidb_id: None,
            }),
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("create interstitial");

    for episode_id in [&episode_one.id, &episode_two.id] {
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: None,
                title_slug: None,
                title_facet: None,
                episode_id: Some(episode_id.clone()),
                collection_id: None,
                season_number: Some("1".to_string()),
                episode_number: None,
                media_type: "episode".to_string(),
                search_phase: "primary".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: None,
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            })
            .await
            .expect("seed stale episode wanted item");
    }

    app.set_collection_monitored(&user, &interstitial.id, true)
        .await
        .expect("monitor interstitial collection");

    let wanted = wanted_items
        .list_wanted_items(WantedItemsQuery {
            title_id: Some(title.id.clone()),
            limit: 50,
            ..WantedItemsQuery::default()
        })
        .await
        .expect("list wanted items");
    assert_eq!(wanted.len(), 1);
    assert_eq!(wanted[0].media_type, "interstitial_movie");
    assert_eq!(
        wanted[0].collection_id.as_deref(),
        Some(interstitial.id.as_str())
    );
    assert!(wanted.iter().all(|item| item.episode_id.is_none()));
}

#[tokio::test]
async fn update_user_entitlements_changes_permissions() {
    let (app, user) = bootstrap();

    let created = app
        .create_user(
            &user,
            "editor".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");

    let updated = app
        .set_user_entitlements(
            &user,
            &created.id,
            vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
        )
        .await
        .expect("update entitlements");

    assert!(updated.entitlements.contains(&Entitlement::ManageTitle));
}

#[tokio::test]
async fn update_user_password_is_hashed() {
    let (app, user) = bootstrap();

    let created = app
        .create_user(
            &user,
            "password-user".into(),
            "before-pass".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");

    let updated = app
        .set_user_password(&user, &created.id, "after-pass".to_string())
        .await
        .expect("update password");

    assert!(updated.password_hash.is_some());
    assert_ne!(
        updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(updated.password_hash, Some("after-pass".to_string()));
}

#[tokio::test]
async fn self_password_change_is_hashed() {
    let (app, admin) = bootstrap();

    let created = app
        .create_user(
            &admin,
            "self-password-user".into(),
            "before-pass".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");

    let updated = app
        .change_own_password(
            &created,
            "after-pass".to_string(),
            "before-pass".to_string(),
        )
        .await
        .expect("update own password");

    assert!(updated.password_hash.is_some());
    assert_ne!(
        updated.password_hash, created.password_hash,
        "password hash should change when password is updated"
    );
    assert_ne!(updated.password_hash, Some("after-pass".to_string()));
}

#[tokio::test]
async fn delete_other_user_removes_user() {
    let (app, user) = bootstrap();

    let created = app
        .create_user(
            &user,
            "removable".into(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");

    app.delete_user(&user, &created.id)
        .await
        .expect("delete user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(!users.iter().any(|entry| entry.id == created.id));
}

#[tokio::test]
async fn update_title_metadata_changes_name_and_tags() {
    let (app, user) = bootstrap();
    let created = app
        .add_title(
            &user,
            NewTitle {
                name: "Original".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec!["SciFi".into()],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let updated = app
        .update_title_metadata(
            &user,
            &created.id,
            Some("Updated Name".into()),
            None,
            Some(vec!["Action".into(), "Drama".into(), "Action".into()]),
        )
        .await
        .expect("update title metadata");

    assert_eq!(updated.name, "Updated Name");
    assert_eq!(
        updated.tags,
        vec!["action".to_string(), "drama".to_string()]
    );
    let events = title_updated_events(&app, &created.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
    assert!(matches!(
        &events[0].payload,
        DomainEventPayload::TitleUpdated(_)
    ));
}

async fn create_series_with_collection_and_episode(
    app: &AppUseCase,
    user: &User,
    name: &str,
) -> (Title, Collection, Episode) {
    let title = app
        .add_title(
            user,
            NewTitle {
                name: name.into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    (title, collection, episode)
}

#[tokio::test]
async fn set_title_monitored_emits_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Monitor Fixture".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let updated = app
        .set_title_monitored(&user, &title.id, false)
        .await
        .expect("update monitored");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_collection_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, collection, _) =
        create_series_with_collection_and_episode(&app, &user, "Collection Monitor Fixture").await;

    let updated = app
        .set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("update collection monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn set_episode_monitored_emits_one_title_updated_with_actor() {
    let (app, user) = bootstrap();
    let (title, _, episode) =
        create_series_with_collection_and_episode(&app, &user, "Episode Monitor Fixture").await;

    let updated = app
        .set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("update episode monitoring");

    assert!(!updated.monitored);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].actor_user_id.as_deref(), Some(user.id.as_str()));
}

#[tokio::test]
async fn external_import_monitor_snapshot_emits_title_updated_without_actor() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Monitor Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "4242".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");
    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    app.save_external_import_monitor_snapshot(
        &user,
        MediaFacet::Series,
        ExternalImportMonitorSnapshotPayload::Series {
            entries: vec![ExternalImportMonitorSeriesEntry {
                root_path: "/media/series".to_string(),
                tvdb_id: Some("4242".to_string()),
                monitored: false,
                seasons: vec![],
                episodes: vec![],
            }],
        },
    )
    .await
    .expect("save monitor snapshot");

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| event.actor_user_id.is_none()));

    app.save_external_import_monitor_snapshot(
        &user,
        MediaFacet::Series,
        ExternalImportMonitorSnapshotPayload::Series {
            entries: vec![ExternalImportMonitorSeriesEntry {
                root_path: "/media/series".to_string(),
                tvdb_id: Some("4242".to_string()),
                monitored: false,
                seasons: vec![],
                episodes: vec![],
            }],
        },
    )
    .await
    .expect("save unchanged monitor snapshot");

    let reapplied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("reapply monitor snapshot");

    assert!(reapplied);
    let replay_events = title_updated_events(&app, &title.id).await;
    assert_eq!(replay_events.len(), 3);
    assert!(
        replay_events
            .iter()
            .all(|event| event.actor_user_id.is_none())
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_syncs_wanted_state_once_per_title() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items.clone(),
    );
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Sync Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "5150".to_string(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");
    app.create_episode(
        &user,
        title.id.clone(),
        Some(collection.id),
        "standard".into(),
        Some("1".into()),
        Some("1".into()),
        Some("Pilot".into()),
        Some("Pilot".into()),
        None,
        Some(1_200),
        false,
        false,
    )
    .await
    .expect("create episode");

    app.save_external_import_monitor_snapshot(
        &user,
        MediaFacet::Series,
        ExternalImportMonitorSnapshotPayload::Series {
            entries: vec![ExternalImportMonitorSeriesEntry {
                root_path: "/media/series".to_string(),
                tvdb_id: Some("5150".to_string()),
                monitored: true,
                seasons: vec![ExternalImportMonitorSeasonEntry {
                    season_number: 1,
                    monitored: true,
                }],
                episodes: vec![ExternalImportMonitorEpisodeEntry {
                    tvdb_id: None,
                    season_number: 1,
                    episode_number: 1,
                    monitored: true,
                }],
            }],
        },
    )
    .await
    .expect("save monitor snapshot");

    let upserts_before_apply = wanted_items.upsert_call_count();
    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let upserts_after_apply = wanted_items.upsert_call_count();
    assert_eq!(upserts_after_apply - upserts_before_apply, 1);
}

#[tokio::test]
async fn create_collection_and_episode() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "The Odes".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let episodes = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");

    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].id, collection.id);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].id, episode.id);
}

#[tokio::test]
async fn anime_hybrid_movie_mapping_creates_interstitial_collection() {
    let (app, user) = bootstrap();
    let app = app.with_test_overrides(|services| {
        services.with_metadata_gateway(Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                131_963,
                MovieMetadata {
                    tvdb_id: 131_963,
                    name: "Mugen Train".into(),
                    slug: "mugen-train".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "A train mission.".into(),
                    poster_url: "https://example.com/mugen-train.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 117,
                    sort_title: "Mugen Train".into(),
                    imdb_id: "tt11032374".into(),
                    anidb_id: None,
                    genres: vec!["Action".into(), "Anime".into()],
                    studio: "ufotable".into(),
                    tmdb_release_date: Some("2020-10-16".into()),
                },
            )]),
        }))
    });
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Demon Slayer".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "348545".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 1001,
            episode_number: 1,
            name: "Cruelty".into(),
            aired: "2019-04-06".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
        },
        EpisodeMetadata {
            tvdb_id: 1002,
            episode_number: 26,
            name: "New Mission".into(),
            aired: "2019-09-28".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 26".into(),
            absolute_number: "26".into(),
            season_number: 1,
        },
        EpisodeMetadata {
            tvdb_id: 2001,
            episode_number: 1,
            name: "Mugen Train".into(),
            aired: "2020-10-10".into(),
            runtime_minutes: 117,
            is_filler: false,
            is_recap: false,
            overview: "Special cut".into(),
            absolute_number: String::new(),
            season_number: 0,
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(40456),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(348545),
        themoviedb_id: Some(438759),
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: Some(131_963),
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];
    let anime_movies = vec![AnimeMovie {
        movie_tvdb_id: Some(131_963),
        movie_tmdb_id: Some(438759),
        movie_imdb_id: Some("tt11032374".into()),
        movie_mal_id: Some(40456),
        movie_anidb_id: None,
        name: "Mugen Train".into(),
        slug: "mugen-train".into(),
        year: Some(2020),
        content_status: "released".into(),
        overview: "Demon Slayer: Mugen Train".into(),
        poster_url: "poster".into(),
        language: "eng".into(),
        runtime_minutes: 117,
        sort_title: "Mugen Train".into(),
        imdb_id: "tt11032374".into(),
        genres: vec!["Action".into()],
        studio: "ufotable".into(),
        digital_release_date: Some("2020-10-16".into()),
        association_confidence: "high".into(),
        continuity_status: "canon".into(),
        movie_form: "movie".into(),
        placement: "ordered".into(),
        confidence: "high".into(),
        signal_summary: "TVDB marked special as critical to story".into(),
    }];

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        &anime_movies,
    )
    .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let interstitial = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Interstitial)
        .expect("interstitial collection should exist");
    assert_eq!(interstitial.collection_index, "1.1");
    assert!(!interstitial.monitored);
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.tvdb_id.as_str()),
        Some("131963")
    );
    assert_eq!(interstitial.label.as_deref(), Some("Mugen Train"));

    let interstitial_episodes = app
        .list_episodes(&user, &interstitial.id)
        .await
        .expect("list interstitial episodes");
    assert_eq!(interstitial_episodes.len(), 1);
    assert_eq!(
        interstitial_episodes[0].title.as_deref(),
        Some("Mugen Train")
    );
}

#[tokio::test]
async fn series_season_zero_creates_canonical_specials_collection() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Arrested Development".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 80,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 81,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 8001,
            episode_number: 1,
            name: "Special Episode".into(),
            aired: "2003-11-01".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: "Special".into(),
            absolute_number: String::new(),
            season_number: 0,
        },
        EpisodeMetadata {
            tvdb_id: 8101,
            episode_number: 1,
            name: "Pilot".into(),
            aired: "2003-11-02".into(),
            runtime_minutes: 22,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let specials = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Specials
                || (collection.collection_type == CollectionType::Season
                    && collection.collection_index == "0")
        })
        .expect("specials collection should exist");
    assert_eq!(specials.collection_type, CollectionType::Specials);
    assert_eq!(specials.collection_index, "0");
    assert!(!specials.monitored);
}

#[tokio::test]
async fn series_rollout_reuses_legacy_season_zero_specials_collection() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Legacy Specials Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let legacy_specials = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "0".into(),
            Some("Season 0".into()),
            None,
            None,
            None,
        )
        .await
        .expect("create legacy season zero collection");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 90,
        number: 0,
        label: "Specials".into(),
        episode_type: "special".into(),
    }];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 9001,
        episode_number: 1,
        name: "Pilot Special".into(),
        aired: "2004-01-01".into(),
        runtime_minutes: 22,
        is_filler: false,
        is_recap: false,
        overview: "Legacy special".into(),
        absolute_number: String::new(),
        season_number: 0,
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let logical_specials: Vec<&Collection> = collections
        .iter()
        .filter(|collection| {
            collection.collection_type == CollectionType::Specials
                || (collection.collection_type == CollectionType::Season
                    && collection.collection_index == "0")
        })
        .collect();
    assert_eq!(logical_specials.len(), 1);
    assert_eq!(logical_specials[0].id, legacy_specials.id);
    assert_eq!(logical_specials[0].collection_type, CollectionType::Season);

    let episodes = app
        .list_episodes(&user, &legacy_specials.id)
        .await
        .expect("list legacy season zero episodes");
    assert_eq!(episodes.len(), 1);
}

#[tokio::test]
async fn anime_mapping_without_movie_link_does_not_create_interstitial_collection() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Given".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "361218".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 20,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 21,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 3001,
            episode_number: 1,
            name: "Boys in the Band".into(),
            aired: "2019-07-12".into(),
            runtime_minutes: 23,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
        },
        EpisodeMetadata {
            tvdb_id: 3002,
            episode_number: 1,
            name: "OVA".into(),
            aired: "2020-02-01".into(),
            runtime_minutes: 23,
            is_filler: false,
            is_recap: false,
            overview: "Special".into(),
            absolute_number: String::new(),
            season_number: 0,
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(40421),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(361218),
        themoviedb_id: None,
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: None,
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &anime_mappings, &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    assert!(
        collections
            .iter()
            .all(|collection| collection.collection_type != CollectionType::Interstitial),
        "unexpected interstitial collection created"
    );
}

#[tokio::test]
async fn anime_hydration_persists_scoped_anibridge_ids_for_episode_and_full_season() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "The Apothecary Diaries".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "431162".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 4_311_622,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];
    let episodes = (1..=24)
        .map(|episode_number| EpisodeMetadata {
            tvdb_id: 431_162_200 + i64::from(episode_number),
            episode_number,
            name: format!("Episode {episode_number}"),
            aired: "2025-01-10".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: String::new(),
            absolute_number: episode_number.to_string(),
            season_number: 2,
        })
        .collect::<Vec<_>>();
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(58514),
        mal_dub_id: Some(999_58514),
        anilist_id: Some(176301),
        anidb_id: Some(18562),
        kitsu_id: Some(48924),
        simkl_id: Some(231_001),
        thetvdb_id: Some(431162),
        themoviedb_id: Some(156_067),
        imdb_id: Some(2_024_544),
        trakt_id: Some(314_159),
        alt_tvdb_id: None,
        thetvdb_season: Some(2),
        thetvdb_part: Some(1),
        score: Some(1.0),
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "current".into(),
        mapping_type: "R".into(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 2,
            episode_start: 1,
            episode_end: 24,
        }],
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &anime_mappings, &[])
        .await;

    let collections = app
        .services
        .catalog
        .shows
        .list_collections_for_title(&title.id)
        .await
        .expect("list collections");
    let season_two = collections
        .iter()
        .find(|collection| collection.collection_index == "2")
        .expect("season two collection");
    let collection_ids = app
        .services
        .catalog
        .shows
        .list_collection_external_ids(&season_two.id)
        .await
        .expect("list collection external ids");
    assert!(
        collection_ids.iter().any(|id| {
            id.source == "anilist"
                && id.external_id == "176301"
                && id.source_scope.as_deref() == Some("R")
        }),
        "expected full-season scoped AniList ID"
    );
    assert!(
        collection_ids
            .iter()
            .any(|id| id.source == "simkl" && id.external_id == "231001"),
        "expected all available AniBridge ID sources to be persisted"
    );

    let episodes = app
        .services
        .catalog
        .shows
        .list_episodes_for_title(&title.id)
        .await
        .expect("list episodes");
    let episode_23 = episodes
        .iter()
        .find(|episode| {
            episode.season_number.as_deref() == Some("2")
                && episode.episode_number.as_deref() == Some("23")
        })
        .expect("season two episode 23");
    let episode_ids = app
        .services
        .catalog
        .shows
        .list_episode_external_ids(&episode_23.id)
        .await
        .expect("list episode external ids");
    assert!(
        episode_ids.iter().any(|id| {
            id.source == "anilist"
                && id.external_id == "176301"
                && id.source_scope.as_deref() == Some("R")
        }),
        "expected episode-scoped AniList ID"
    );
}

#[tokio::test]
async fn anime_specials_movies_attach_to_specials_collection_and_keep_ordered_movies_separate() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Attack on Titan".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec!["scryer:monitor-specials:false".into()],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "267440".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 50,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 51,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
        SeasonMetadata {
            tvdb_id: 52,
            number: 2,
            label: "Season 2".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 5001,
            episode_number: 1,
            name: "To You, in 2000 Years".into(),
            aired: "2013-04-07".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
        },
        EpisodeMetadata {
            tvdb_id: 6001,
            episode_number: 1,
            name: "Beast Titan".into(),
            aired: "2017-04-01".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "26".into(),
            season_number: 2,
        },
    ];

    let anime_movies = vec![
        AnimeMovie {
            movie_tvdb_id: Some(379088),
            movie_tmdb_id: Some(379088),
            movie_imdb_id: Some("tt3865768".into()),
            movie_mal_id: Some(23775),
            movie_anidb_id: None,
            name: "Attack on Titan: Crimson Bow and Arrow".into(),
            slug: "crimson-bow-and-arrow".into(),
            year: Some(2014),
            content_status: "released".into(),
            overview: "Recap of episodes 1-13.".into(),
            poster_url: "poster-aot".into(),
            language: "eng".into(),
            runtime_minutes: 120,
            sort_title: "Crimson Bow and Arrow".into(),
            imdb_id: "tt3865768".into(),
            genres: vec!["Action".into()],
            studio: "WIT Studio".into(),
            digital_release_date: Some("2014-11-22".into()),
            association_confidence: "high".into(),
            continuity_status: "unknown".into(),
            movie_form: "recap".into(),
            placement: "specials".into(),
            confidence: "high".into(),
            signal_summary: "TVDB special category marks this as a recap".into(),
        },
        AnimeMovie {
            movie_tvdb_id: Some(131963),
            movie_tmdb_id: Some(438759),
            movie_imdb_id: Some("tt11032374".into()),
            movie_mal_id: Some(40456),
            movie_anidb_id: None,
            name: "Mugen Train".into(),
            slug: "mugen-train".into(),
            year: Some(2020),
            content_status: "released".into(),
            overview: "Canon bridge movie".into(),
            poster_url: "poster-ds".into(),
            language: "eng".into(),
            runtime_minutes: 117,
            sort_title: "Mugen Train".into(),
            imdb_id: "tt11032374".into(),
            genres: vec!["Action".into()],
            studio: "ufotable".into(),
            digital_release_date: Some("2020-10-16".into()),
            association_confidence: "high".into(),
            continuity_status: "canon".into(),
            movie_form: "movie".into(),
            placement: "ordered".into(),
            confidence: "high".into(),
            signal_summary: "TVDB marked special as critical to story".into(),
        },
    ];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &anime_movies)
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let specials = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Specials)
        .expect("specials collection should exist");
    assert!(!specials.monitored);
    assert_eq!(specials.specials_movies.len(), 1);
    assert_eq!(
        specials.specials_movies[0].movie_form.as_deref(),
        Some("recap")
    );

    let interstitial = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Interstitial)
        .expect("ordered movie collection should exist");
    assert!(!interstitial.monitored);
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .and_then(|movie| movie.continuity_status.as_deref()),
        Some("canon")
    );
}

#[tokio::test]
async fn anime_interstitial_refresh_updates_localized_collection_metadata() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "My Hero Academia".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![
        EpisodeMetadata {
            tvdb_id: 1001,
            episode_number: 1,
            name: "Episode 1".into(),
            aired: "2018-04-03".into(),
            runtime_minutes: 24,
            is_filler: false,
            is_recap: false,
            overview: "Episode 1".into(),
            absolute_number: "1".into(),
            season_number: 1,
        },
        EpisodeMetadata {
            tvdb_id: 2001,
            episode_number: 1,
            name: "Two Heroes".into(),
            aired: "2018-08-03".into(),
            runtime_minutes: 96,
            is_filler: false,
            is_recap: false,
            overview: "Movie special".into(),
            absolute_number: String::new(),
            season_number: 0,
        },
    ];
    let anime_mappings = vec![AnimeMapping {
        mal_id: Some(36665),
        mal_dub_id: None,
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        simkl_id: None,
        thetvdb_id: Some(305074),
        themoviedb_id: Some(505262),
        imdb_id: None,
        trakt_id: None,
        alt_tvdb_id: Some(149921),
        thetvdb_season: Some(0),
        thetvdb_part: None,
        score: None,
        anime_media_type: "TV".into(),
        global_media_type: "series".into(),
        status: "finished".into(),
        mapping_type: String::new(),
        episode_mappings: vec![AnimeEpisodeMapping {
            tvdb_season: 0,
            episode_start: 1,
            episode_end: 1,
        }],
    }];

    let japanese_movie = AnimeMovie {
        movie_tvdb_id: Some(149921),
        movie_tmdb_id: Some(505262),
        movie_imdb_id: Some("tt5626028".into()),
        movie_mal_id: Some(36665),
        movie_anidb_id: None,
        name: "僕のヒーローアカデミア THE MOVIE ～2人の英雄～".into(),
        slug: "my-hero-academia-the-movie-two-heroes".into(),
        year: Some(2018),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 96,
        sort_title: "僕のヒーローアカデミア THE MOVIE ～2人の英雄～".into(),
        imdb_id: "tt5626028".into(),
        genres: vec!["Action".into()],
        studio: "Bones".into(),
        digital_release_date: Some("2018-08-03".into()),
        association_confidence: "high".into(),
        continuity_status: "canon".into(),
        movie_form: "movie".into(),
        placement: "ordered".into(),
        confidence: "high".into(),
        signal_summary: "TVDB special linked to movie".into(),
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        std::slice::from_ref(&japanese_movie),
    )
    .await;

    let english_movie = AnimeMovie {
        name: "My Hero Academia: Two Heroes".into(),
        overview: "English overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "My Hero Academia: Two Heroes".into(),
        ..japanese_movie.clone()
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &anime_mappings,
        std::slice::from_ref(&english_movie),
    )
    .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let interstitial = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Interstitial)
        .expect("interstitial collection should exist");

    assert_eq!(
        interstitial.label.as_deref(),
        Some("My Hero Academia: Two Heroes")
    );
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.as_str()),
        Some("My Hero Academia: Two Heroes")
    );
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.overview.as_str()),
        Some("English overview")
    );
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.language.as_str()),
        Some("eng")
    );
}

#[tokio::test]
async fn anime_specials_refresh_updates_localized_specials_movie_metadata() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Attack on Titan".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![
        SeasonMetadata {
            tvdb_id: 10,
            number: 0,
            label: "Specials".into(),
            episode_type: "special".into(),
        },
        SeasonMetadata {
            tvdb_id: 11,
            number: 1,
            label: "Season 1".into(),
            episode_type: "official".into(),
        },
    ];
    let episodes = vec![EpisodeMetadata {
        tvdb_id: 1001,
        episode_number: 1,
        name: "Episode 1".into(),
        aired: "2013-04-07".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: "Episode 1".into(),
        absolute_number: "1".into(),
        season_number: 1,
    }];

    let japanese_special = AnimeMovie {
        movie_tvdb_id: Some(379088),
        movie_tmdb_id: Some(379088),
        movie_imdb_id: Some("tt3865768".into()),
        movie_mal_id: Some(23775),
        movie_anidb_id: None,
        name: "進撃の巨人 前編～紅蓮の弓矢～".into(),
        slug: "crimson-bow-and-arrow".into(),
        year: Some(2014),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 120,
        sort_title: "進撃の巨人 前編～紅蓮の弓矢～".into(),
        imdb_id: "tt3865768".into(),
        genres: vec!["Action".into()],
        studio: "WIT Studio".into(),
        digital_release_date: Some("2014-11-22".into()),
        association_confidence: "high".into(),
        continuity_status: "unknown".into(),
        movie_form: "recap".into(),
        placement: "specials".into(),
        confidence: "high".into(),
        signal_summary: "TVDB special category marks this as a recap".into(),
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &[],
        std::slice::from_ref(&japanese_special),
    )
    .await;

    let english_special = AnimeMovie {
        name: "Attack on Titan: Crimson Bow and Arrow".into(),
        overview: "English recap overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "Attack on Titan: Crimson Bow and Arrow".into(),
        ..japanese_special.clone()
    };

    app.create_series_seasons_and_episodes(
        &title,
        &seasons,
        &episodes,
        &[],
        std::slice::from_ref(&english_special),
    )
    .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let specials = collections
        .iter()
        .find(|collection| collection.collection_type == CollectionType::Specials)
        .expect("specials collection should exist");

    assert_eq!(specials.specials_movies.len(), 1);
    assert_eq!(
        specials.specials_movies[0].name,
        "Attack on Titan: Crimson Bow and Arrow"
    );
    assert_eq!(
        specials.specials_movies[0].overview,
        "English recap overview"
    );
    assert_eq!(specials.specials_movies[0].language, "eng");
}

#[tokio::test]
async fn read_collection_by_id_returns_item() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Read Collection".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let found = app
        .get_collection(&user, &collection.id)
        .await
        .expect("get collection")
        .expect("found collection");

    assert_eq!(found.id, collection.id);
    assert_eq!(found.collection_index, collection.collection_index);
}

#[tokio::test]
async fn read_episode_by_id_returns_item() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Read Episode".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    let found = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("found episode");

    assert_eq!(found.id, episode.id);
    assert_eq!(found.episode_number, episode.episode_number);
}

#[tokio::test]
async fn delete_collection_removes_collection_entry() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Collection Delete".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    app.delete_collection(&user, &collection.id)
        .await
        .expect("delete collection");

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    assert!(collections.is_empty());
}

#[tokio::test]
async fn delete_episode_removes_episode_entry() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Episode Delete".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    app.delete_episode(&user, &episode.id)
        .await
        .expect("delete episode");

    let episodes = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");
    assert!(episodes.is_empty(), "expected episode to be deleted");
}

#[tokio::test]
async fn update_collection_changes_fields() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Update Collection".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            Some("s1".into()),
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let updated = app
        .update_collection(
            &user,
            collection.id.clone(),
            Some("arc".into()),
            None,
            Some("Arc One".into()),
            Some("arc-one".into()),
            None,
            Some("13".into()),
            None,
        )
        .await
        .expect("update collection");

    assert_eq!(updated.collection_type, CollectionType::Arc);
    assert_eq!(updated.label, Some("Arc One".into()));
    assert_eq!(updated.ordered_path, Some("arc-one".into()));
    assert_eq!(updated.last_episode_number, Some("13".into()));
    assert_eq!(updated.collection_index, "1");
}

#[tokio::test]
async fn update_episode_changes_fields() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Update Episode".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,

                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let collection = app
        .create_collection(
            &user,
            title.id.clone(),
            "season".into(),
            "1".into(),
            Some("Season One".into()),
            None,
            Some("1".into()),
            Some("12".into()),
        )
        .await
        .expect("create collection");

    let episode = app
        .create_episode(
            &user,
            title.id.clone(),
            Some(collection.id.clone()),
            "standard".into(),
            Some("1".into()),
            Some("1".into()),
            Some("Pilot".into()),
            Some("Pilot".into()),
            None,
            Some(1_200),
            false,
            false,
        )
        .await
        .expect("create episode");

    let updated = app
        .update_episode(
            &user,
            episode.id.clone(),
            Some("special".into()),
            Some("E01".into()),
            None,
            None,
            Some("Pilot Updated".into()),
            Some("2026-01-01".into()),
            Some(1_800),
            Some(true),
            None,
            None,
            Some(collection.id.clone()),
            Some("Updated overview".into()),
        )
        .await
        .expect("update episode");

    assert_eq!(updated.episode_type, scryer_domain::EpisodeType::Special);
    assert_eq!(updated.episode_number, Some("E01".into()));
    assert_eq!(updated.title, Some("Pilot Updated".into()));
    assert_eq!(updated.air_date, Some("2026-01-01".into()));
    assert_eq!(updated.overview, Some("Updated overview".into()));
    assert_eq!(updated.duration_seconds, Some(1_800));
    assert!(updated.has_multi_audio);
    assert!(!updated.has_subtitle);
}

#[test]
fn hash_and_validate_password_round_trip() {
    let (app, _user) = bootstrap();
    let hashed = app
        .hash_password("P@ssw0rd")
        .expect("hash should be generated");
    assert!(
        app.validate_password("P@ssw0rd", &hashed)
            .expect("hash should be valid")
    );
    assert!(
        !app.validate_password("wrong", &hashed)
            .expect("hash should validate")
    );
}

#[test]
fn hash_version_is_explicit() {
    let (app, _user) = bootstrap();

    assert!(app.hash_password("abc").expect("hash").starts_with("v2$"));
}

#[test]
fn v1_password_still_validates() {
    let (app, _user) = bootstrap();
    // Simulate a legacy v1 hash
    let salt = "abcdef0123456789abcdef0123456789";
    let digest = sha256_hex(format!("{salt}legacy-pass"));
    let v1_hash = format!("v1${salt}${digest}");
    assert!(
        app.validate_password("legacy-pass", &v1_hash)
            .expect("v1 should validate")
    );
    assert!(
        !app.validate_password("wrong", &v1_hash)
            .expect("v1 should reject wrong password")
    );
}

// ── password edge cases ───────────────────────────────────────────────────

#[test]
fn hash_password_empty_returns_error() {
    let (app, _) = bootstrap();
    assert!(app.hash_password("").is_err());
    assert!(app.hash_password("   ").is_err());
}

#[test]
fn validate_password_v1_malformed_no_salt_separator() {
    let (app, _) = bootstrap();
    // Only "v1" prefix, no $ after it
    let bad_hash = "v1nope";
    let result = app.validate_password("anything", bad_hash);
    assert!(
        result.is_err(),
        "malformed v1 hash (no $) should return Err"
    );
}

#[test]
fn validate_password_v1_malformed_no_hash_component() {
    let (app, _) = bootstrap();
    // Has v1$salt but no third segment
    let bad_hash = "v1$somesalt";
    let result = app.validate_password("anything", bad_hash);
    assert!(
        result.is_err(),
        "malformed v1 hash (no hash segment) should return Err"
    );
}

#[test]
fn validate_password_unknown_version_returns_error() {
    let (app, _) = bootstrap();
    let result = app.validate_password("pass", "v99$somehash");
    assert!(result.is_err(), "unknown hash version should return Err");
}

// ── JWT round-trip ────────────────────────────────────────────────────────

/// Derive a per-user JWT signing key (mirrors `AppUseCase::derive_jwt_key`).
fn test_derive_jwt_key(salt: &str, password_hash: &str, entitlements: &[Entitlement]) -> Vec<u8> {
    use ring::hmac;
    let mut entitlement_claims = entitlements
        .iter()
        .map(AppUseCase::entitlement_claim_string)
        .map(str::to_string)
        .collect::<Vec<_>>();
    entitlement_claims.sort();
    entitlement_claims.dedup();
    let entitlement_fingerprint = sha256_hex(entitlement_claims.join("\n"));
    let signing_material = format!("{password_hash}\n{entitlement_fingerprint}");
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, salt.as_bytes());
    hmac::sign(&hmac_key, signing_material.as_bytes())
        .as_ref()
        .to_vec()
}

const TEST_PASSWORD_HASH: &str = "v2$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQ$dGVzdGhhc2g";

async fn create_authenticated_user(
    app: &AppUseCase,
    admin: &User,
    username: &str,
    password: &str,
    entitlements: Vec<Entitlement>,
) -> (User, User) {
    let created = app
        .create_user(
            admin,
            username.to_string(),
            password.to_string(),
            entitlements,
        )
        .await
        .expect("create user");
    let token = app.issue_access_token(&created).expect("issue token");
    let authenticated = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");

    (created, authenticated)
}

#[tokio::test]
async fn issue_and_authenticate_token_round_trips() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-1".to_string(),
        username: "jwt_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![Entitlement::ViewCatalog],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let token = app.issue_access_token(&user).expect("issue token");
    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");
    assert_eq!(decoded.id, user.id);
    assert_eq!(decoded.username, user.username);
}

#[tokio::test]
async fn entitlements_survive_token_round_trip() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-2".to_string(),
        username: "ent_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let token = app.issue_access_token(&user).expect("issue token");
    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");
    assert!(decoded.entitlements.contains(&Entitlement::ViewCatalog));
    assert!(decoded.entitlements.contains(&Entitlement::ManageTitle));
}

#[tokio::test]
async fn release_candidate_token_round_trips_for_matching_actor_title_and_scope() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user",
        "password123",
        vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-1",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let decoded = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-1",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .expect("candidate token should verify");

    assert_eq!(decoded.source_hint, selection.source_hint);
    assert_eq!(decoded.source_kind, selection.source_kind);
    assert_eq!(decoded.source_title, selection.source_title);
}

#[tokio::test]
async fn release_candidate_token_round_trips_episode_set_scope() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_episode_set_user",
        "password123",
        vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/range-pack.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.S01E01-E03.1080p.WEB-DL".to_string()),
    };
    let scope = SubmissionScope::EpisodeSet {
        episode_ids: vec![
            "episode-1".to_string(),
            "episode-2".to_string(),
            "episode-3".to_string(),
        ],
    };

    let token = app
        .issue_release_candidate_token(&authenticated_user, "title-1", &scope, &selection)
        .await
        .expect("candidate token should issue");
    let (decoded, signed_scope) = app
        .verify_release_candidate_token_for_signed_scope(&authenticated_user, "title-1", &token)
        .await
        .expect("candidate token should verify");

    assert_eq!(decoded.source_hint, selection.source_hint);
    assert_eq!(signed_scope, scope);
}

#[tokio::test]
async fn release_candidate_token_rejects_tampering() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_2",
        "password123",
        vec![Entitlement::ViewCatalog],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-2",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");
    let tampered = format!("{token}x");

    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "title-2",
            &SubmissionScope::Title,
            &tampered,
        )
        .await
        .is_err(),
        "tampered token should be rejected"
    );
}

#[tokio::test]
async fn release_candidate_token_rejects_actor_title_and_scope_mismatch() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_3",
        "password123",
        vec![Entitlement::ViewCatalog],
    )
    .await;
    let (_other_created, other_authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_4",
        "password123",
        vec![Entitlement::ViewCatalog],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
    };

    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-3",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    assert!(
        app.verify_release_candidate_token(
            &other_authenticated_user,
            "title-3",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .is_err(),
        "actor mismatch should be rejected"
    );
    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "other-title",
            &SubmissionScope::Title,
            &token,
        )
        .await
        .is_err(),
        "title mismatch should be rejected"
    );
    assert!(
        app.verify_release_candidate_token(
            &authenticated_user,
            "title-3",
            &SubmissionScope::Episode {
                episode_id: "episode-1".to_string(),
            },
            &token,
        )
        .await
        .is_err(),
        "scope mismatch should be rejected"
    );
}

#[tokio::test]
async fn release_candidate_token_is_invalidated_by_password_rotation() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "candidate_pw_rotate",
        "before-pass",
        vec![Entitlement::ViewCatalog],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
    };
    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-password-rotate",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    app.set_user_password(&admin, &created.id, "after-pass".to_string())
        .await
        .expect("rotate password");

    let result = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-password-rotate",
            &SubmissionScope::Title,
            &token,
        )
        .await;
    assert!(
        result.is_err(),
        "candidate token should be rejected after password rotation"
    );
}

#[tokio::test]
async fn release_candidate_token_is_invalidated_by_entitlement_change() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "candidate_ent_rotate",
        "same-pass",
        vec![Entitlement::ViewCatalog],
    )
    .await;
    let selection = QueuedReleaseSelection {
        source_hint: Some("https://example.invalid/download.nzb".to_string()),
        source_kind: Some(DownloadSourceKind::NzbUrl),
        source_title: Some("Example.Release.1080p.WEB-DL".to_string()),
    };
    let token = app
        .issue_release_candidate_token(
            &authenticated_user,
            "title-entitlement-rotate",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    app.set_user_entitlements(
        &admin,
        &created.id,
        vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
    )
    .await
    .expect("update entitlements");

    let result = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-entitlement-rotate",
            &SubmissionScope::Title,
            &token,
        )
        .await;
    assert!(
        result.is_err(),
        "candidate token should be rejected after entitlement change"
    );
}

#[tokio::test]
async fn expired_token_returns_unauthorized() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-3".to_string(),
        username: "exp_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    // Encode a token with an exp 100 seconds in the past
    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() - 100,
        iat: Utc::now().timestamp() - 200,
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        entitlements: vec![],
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let expired_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
    let result = app.authenticate_token(&expired_token).await;
    assert!(result.is_err(), "expired token should be rejected");
}

#[tokio::test]
async fn wrong_issuer_token_returns_unauthorized() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-4".to_string(),
        username: "iss_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![Entitlement::ViewCatalog],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: "wrong-issuer".to_string(),
        username: user.username.clone(),
        entitlements: vec!["view_catalog".to_string()],
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &user.entitlements,
    );
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let bad_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
    let result = app.authenticate_token(&bad_token).await;
    assert!(
        result.is_err(),
        "token with wrong issuer should be rejected"
    );
}

#[tokio::test]
async fn authenticate_token_warms_cache_without_get_by_id_round_trip() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let user = User {
        id: "user-jwt-cache-1".to_string(),
        username: "cache_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![Entitlement::ViewCatalog],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let token = app.issue_access_token(&user).expect("issue token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token from warm cache");

    assert_eq!(users.get_by_id_call_count(), 0);
    assert_eq!(users.list_all_call_count(), 1);
}

#[tokio::test]
async fn password_change_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = app
        .create_user(
            &admin,
            "pw_rotate".to_string(),
            "before-pass".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");
    let token = app.issue_access_token(&created).expect("issue token");

    app.set_user_password(&admin, &created.id, "after-pass".to_string())
        .await
        .expect("rotate password");

    let result = app.authenticate_token(&token).await;
    assert!(
        result.is_err(),
        "old token should be rejected after password change"
    );
}

#[tokio::test]
async fn entitlement_change_invalidates_existing_token_and_relogin_works() {
    let (app, admin) = bootstrap();
    let created = app
        .create_user(
            &admin,
            "ent_rotate".to_string(),
            "same-pass".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");
    let old_token = app.issue_access_token(&created).expect("issue token");

    let updated = app
        .set_user_entitlements(
            &admin,
            &created.id,
            vec![Entitlement::ViewCatalog, Entitlement::ManageTitle],
        )
        .await
        .expect("update entitlements");

    let old_result = app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "old token should be rejected after entitlement change"
    );

    let relogged = app
        .authenticate_credentials("ent_rotate", "same-pass")
        .await
        .expect("re-login after entitlement change");
    let new_token = app
        .issue_access_token(&relogged)
        .expect("issue refreshed token");
    let decoded = app
        .authenticate_token(&new_token)
        .await
        .expect("authenticate refreshed token");

    assert_eq!(decoded.id, updated.id);
    assert!(decoded.entitlements.contains(&Entitlement::ManageTitle));
}

#[tokio::test]
async fn deleting_user_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = app
        .create_user(
            &admin,
            "gone_user".to_string(),
            "password123".to_string(),
            vec![Entitlement::ViewCatalog],
        )
        .await
        .expect("create user");
    let token = app.issue_access_token(&created).expect("issue token");

    app.delete_user(&admin, &created.id)
        .await
        .expect("delete user");

    let result = app.authenticate_token(&token).await;
    assert!(result.is_err(), "deleted user token should be rejected");
}

#[test]
fn jwt_key_derivation_is_stable_across_entitlement_order() {
    let (app, _) = bootstrap();
    let key_a = app.derive_jwt_key(
        TEST_PASSWORD_HASH,
        &[Entitlement::ManageTitle, Entitlement::ViewCatalog],
    );
    let key_b = app.derive_jwt_key(
        TEST_PASSWORD_HASH,
        &[Entitlement::ViewCatalog, Entitlement::ManageTitle],
    );

    assert_eq!(key_a, key_b);
}

#[tokio::test]
async fn malformed_entitlement_claims_are_rejected() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-malformed".to_string(),
        username: "jwt_claims".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        entitlements: vec![Entitlement::ViewCatalog],
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    app.ensure_jwt_signing_keys_loaded()
        .await
        .expect("seed signing key cache");

    let claims = JwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        entitlements: vec!["definitely_not_real".to_string()],
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &user.entitlements,
    );
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");

    let result = app.authenticate_token(&token).await;
    assert!(
        result.is_err(),
        "malformed entitlement claims should be rejected"
    );
}
