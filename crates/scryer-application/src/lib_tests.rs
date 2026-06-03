use super::*;
use async_trait::async_trait;
use scryer_domain::{
    Collection, CollectionType, DomainEventFilter, DomainEventPayload, DomainEventType, Episode,
    EpisodeType, EventType, ImportType, JobRunCompletedEventData, JobRunStartedEventData,
    MediaRequestRequester, MediaRequestStatus, RootFolderEntry, TrackedDownloadState,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, Notify};
use tokio::time::{Duration, Instant, sleep, timeout};

type DeleteOperationLog = Arc<Mutex<Vec<String>>>;
type OptionalDeleteOperationLog = Arc<Mutex<Option<DeleteOperationLog>>>;
type TrackedDownloadStateKey = (String, String, String);
type TrackedDownloadStates = Arc<Mutex<HashMap<TrackedDownloadStateKey, String>>>;
type DeletedDownloadRequest = (Option<String>, Option<String>, String, bool);
type DeletedDownloadRequests = Arc<Mutex<Vec<DeletedDownloadRequest>>>;

#[derive(Default)]
struct MockTitleRepo {
    store: Arc<Mutex<Vec<Title>>>,
    create_or_get_existing_error: Arc<Mutex<Option<String>>>,
    delete_operation_log: OptionalDeleteOperationLog,
}

impl MockTitleRepo {
    async fn fail_create_or_get_existing(&self, message: &str) {
        *self.create_or_get_existing_error.lock().await = Some(message.to_string());
    }

    async fn set_delete_operation_log(&self, operation_log: Arc<Mutex<Vec<String>>>) {
        *self.delete_operation_log.lock().await = Some(operation_log);
    }
}

#[derive(Default)]
struct BlockingTitleImageRepo {
    clear_calls: AtomicUsize,
    release_clear: Notify,
}

#[async_trait]
impl TitleImageRepository for BlockingTitleImageRepo {
    async fn list_title_image_refresh_work(
        &self,
        _limit: usize,
        _skipped: &[TitleImageSyncTask],
    ) -> AppResult<Vec<TitleImageSyncTask>> {
        Ok(Vec::new())
    }

    async fn clear_title_image_cache(&self) -> AppResult<()> {
        self.clear_calls.fetch_add(1, Ordering::SeqCst);
        self.release_clear.notified().await;
        Ok(())
    }

    async fn upsert_title_image_source_result(
        &self,
        _title_id: &str,
        _result: TitleImageSourceResult,
        _event: Option<NewDomainEvent>,
    ) -> AppResult<Option<DomainEvent>> {
        Ok(None)
    }

    async fn get_title_image_blob(
        &self,
        _title_id: &str,
        _kind: TitleImageKind,
        _variant_key: &str,
    ) -> AppResult<Option<TitleImageBlob>> {
        Ok(None)
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
        if let Some(operation_log) = self.delete_operation_log.lock().await.clone() {
            operation_log
                .lock()
                .await
                .push(format!("delete_title:{id}"));
        }
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
            video_codec_parsed: input.video_codec_parsed,
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
        source_identity: DownloadSourceIdentity,
        import_type: String,
        payload_json: String,
    ) -> AppResult<String> {
        let id = Id::new().0;
        let now = Utc::now().to_rfc3339();
        self.records.lock().await.push(ImportRecord {
            id: id.clone(),
            source_client_id: source_identity.client_id.clone(),
            source_system: source_identity.client_type,
            source_ref: source_identity.item_id,
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

    async fn list_imports_for_identities(
        &self,
        identities: &[DownloadSourceIdentity],
    ) -> AppResult<Vec<ImportRecord>> {
        let records = self.records.lock().await;
        Ok(identities
            .iter()
            .filter_map(|identity| {
                records
                    .iter()
                    .rev()
                    .find(|record| {
                        record.source_client_id.as_deref().unwrap_or("")
                            == identity.client_id_or_empty()
                            && record.source_system == identity.client_type
                            && record.source_ref == identity.item_id
                    })
                    .cloned()
            })
            .collect())
    }

    async fn is_already_imported(&self, identity: &DownloadSourceIdentity) -> AppResult<bool> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .rev()
            .find(|record| {
                record.source_client_id.as_deref().unwrap_or("") == identity.client_id_or_empty()
                    && record.source_system == identity.client_type
                    && record.source_ref == identity.item_id
            })
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
        _mode: scryer_domain::ImportMode,
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
            source_cleanup: None,
        })
    }

    async fn remove_import_source_after_verified_import(
        &self,
        _guard: scryer_domain::ImportSourceCleanupGuard,
        _final_dest_path: &Path,
    ) -> AppResult<()> {
        Ok(())
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

    async fn auth_session_version(&self, _user_id: &str) -> AppResult<Option<String>> {
        Ok(None)
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
    delete_operation_log: OptionalDeleteOperationLog,
}

impl MockDomainEventRepo {
    async fn set_delete_operation_log(&self, operation_log: Arc<Mutex<Vec<String>>>) {
        *self.delete_operation_log.lock().await = Some(operation_log);
    }
}

#[derive(Default)]
struct MockExternalImportMonitorSnapshotRepo {
    chunks: Arc<Mutex<Vec<ExternalImportMonitorSnapshotChunk>>>,
}

#[async_trait]
impl ExternalImportMonitorSnapshotRepository for MockExternalImportMonitorSnapshotRepo {
    async fn append_external_import_monitor_snapshot_chunk(
        &self,
        chunk: &ExternalImportMonitorSnapshotChunk,
    ) -> AppResult<()> {
        self.chunks.lock().await.push(chunk.clone());
        Ok(())
    }

    async fn list_external_import_monitor_snapshot_chunk_batch(
        &self,
        facet: MediaFacet,
        entry_kind: ExternalImportMonitorSnapshotEntryKind,
        after_chunk_index: Option<i32>,
        limit: i32,
    ) -> AppResult<Vec<ExternalImportMonitorSnapshotChunk>> {
        let chunks = self.chunks.lock().await;
        let mut matched = chunks
            .iter()
            .filter(|chunk| {
                chunk.facet == facet
                    && chunk.entry_kind == entry_kind
                    && after_chunk_index
                        .map(|after| chunk.chunk_index > after)
                        .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        matched.sort_by_key(|chunk| chunk.chunk_index);
        matched.truncate(limit.max(0) as usize);
        Ok(matched)
    }

    async fn delete_external_import_monitor_snapshot_chunks(
        &self,
        facet: MediaFacet,
    ) -> AppResult<()> {
        let mut chunks = self.chunks.lock().await;
        chunks.retain(|chunk| chunk.facet != facet);
        Ok(())
    }
}

async fn append_series_monitor_snapshot_chunk(
    app: &AppUseCase,
    user: &User,
    facet: MediaFacet,
    entries: Vec<ExternalImportMonitorSeriesEntry>,
) {
    let payload_ndjson = entries
        .into_iter()
        .map(|entry| serde_json::to_string(&entry).expect("serialize series snapshot entry"))
        .collect::<Vec<_>>()
        .join("\n");
    app.append_external_import_monitor_snapshot_chunk(
        user,
        ExternalImportMonitorSnapshotChunk {
            facet,
            entry_kind: ExternalImportMonitorSnapshotEntryKind::Series,
            chunk_index: 0,
            payload_ndjson,
            created_at: Utc::now().to_rfc3339(),
        },
    )
    .await
    .expect("append monitor snapshot chunk");
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

    async fn delete_for_title_ids(&self, title_ids: &[String]) -> AppResult<u32> {
        if let Some(operation_log) = self.delete_operation_log.lock().await.clone() {
            operation_log
                .lock()
                .await
                .push("delete_domain_events".to_string());
        }
        let mut events = self.events.lock().await;
        let before = events.len();
        events.retain(|event| {
            event
                .title_id
                .as_ref()
                .is_none_or(|title_id| !title_ids.iter().any(|candidate| candidate == title_id))
        });
        Ok((before - events.len()) as u32)
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
struct MockMediaRequestRepo {
    requests: Arc<Mutex<Vec<MediaRequest>>>,
    domain_events: Option<Arc<MockDomainEventRepo>>,
}

impl MockMediaRequestRepo {
    fn with_domain_events(domain_events: Arc<MockDomainEventRepo>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            domain_events: Some(domain_events),
        }
    }
}

#[async_trait]
impl MediaRequestRepository for MockMediaRequestRepo {
    async fn submit(
        &self,
        request: NewMediaRequest,
        requester: &User,
        submitted_event: NewDomainEvent,
    ) -> AppResult<MediaRequest> {
        let mut requests = self.requests.lock().await;
        let now = Utc::now();
        let stored = MediaRequest {
            id: request.id,
            library_id: request.library_id,
            facet: request.facet,
            status: MediaRequestStatus::Pending,
            identity_fingerprint: request.identity_fingerprint,
            title: request.title,
            sort_title: request.sort_title,
            slug: request.slug,
            poster_url: request.poster_url,
            year: request.year,
            overview: request.overview,
            runtime_minutes: request.runtime_minutes,
            language: request.language,
            content_status: request.content_status,
            requested_quality_profile_id: request.requested_quality_profile_id,
            requested_quality_profile_name: request.requested_quality_profile_name,
            requested_monitor_type: request.requested_monitor_type,
            external_ids: request.external_ids,
            requesters: vec![MediaRequestRequester {
                user_id: requester.id.clone(),
                username: requester.username.clone(),
                avatar_url: None,
                requested_at: now,
            }],
            created_by_user_id: request.created_by_user_id,
            resolved_by_user_id: None,
            resolved_at: None,
            created_title_id: None,
            approved_quality_profile_id: None,
            approved_quality_profile_name: None,
            created_at: now,
            updated_at: now,
        };
        requests.push(stored.clone());
        drop(requests);
        if let Some(domain_events) = &self.domain_events {
            domain_events.append(submitted_event).await?;
        }
        Ok(stored)
    }

    async fn get(&self, request_id: &str) -> AppResult<Option<MediaRequest>> {
        let requests = self.requests.lock().await;
        Ok(requests
            .iter()
            .find(|request| request.id == request_id)
            .cloned())
    }

    async fn resolve_pending_overlapping(
        &self,
        request: &MediaRequest,
        resolution: MediaRequestResolution,
    ) -> AppResult<u64> {
        let mut requests = self.requests.lock().await;
        let mut updated = 0;
        for candidate in requests.iter_mut().filter(|candidate| {
            candidate.status == MediaRequestStatus::Pending
                && candidate.library_id == request.library_id
                && candidate.facet == request.facet
                && candidate.external_ids.iter().any(|candidate_id| {
                    request.external_ids.iter().any(|request_id| {
                        candidate_id.source == request_id.source
                            && candidate_id.value == request_id.value
                    })
                })
        }) {
            candidate.status = resolution.status;
            candidate.resolved_by_user_id = Some(resolution.resolved_by_user_id.clone());
            candidate.resolved_at = Some(resolution.resolved_at);
            candidate.created_title_id = resolution.created_title_id.clone();
            candidate.approved_quality_profile_id = resolution.approved_quality_profile_id.clone();
            candidate.approved_quality_profile_name =
                resolution.approved_quality_profile_name.clone();
            candidate.updated_at = resolution.resolved_at;
            updated += 1;
        }
        drop(requests);

        if updated > 0
            && let Some(domain_events) = &self.domain_events
        {
            domain_events.append(resolution.event).await?;
        }

        Ok(updated)
    }

    async fn resolve_pending(
        &self,
        request_id: &str,
        resolution: MediaRequestResolution,
    ) -> AppResult<u64> {
        let mut requests = self.requests.lock().await;
        let mut updated = 0;
        for candidate in requests.iter_mut().filter(|candidate| {
            candidate.id == request_id && candidate.status == MediaRequestStatus::Pending
        }) {
            candidate.status = resolution.status;
            candidate.resolved_by_user_id = Some(resolution.resolved_by_user_id.clone());
            candidate.resolved_at = Some(resolution.resolved_at);
            candidate.created_title_id = resolution.created_title_id.clone();
            candidate.approved_quality_profile_id = resolution.approved_quality_profile_id.clone();
            candidate.approved_quality_profile_name =
                resolution.approved_quality_profile_name.clone();
            candidate.updated_at = resolution.resolved_at;
            updated += 1;
        }
        drop(requests);

        if updated > 0
            && let Some(domain_events) = &self.domain_events
        {
            domain_events.append(resolution.event).await?;
        }

        Ok(updated)
    }

    async fn update_pending_request_preferences(
        &self,
        request_id: &str,
        requested_quality_profile_id: String,
        requested_quality_profile_name: String,
        requested_monitor_type: Option<String>,
        updated_event: NewDomainEvent,
    ) -> AppResult<MediaRequest> {
        let mut requests = self.requests.lock().await;
        let now = Utc::now();
        let Some(request) = requests.iter_mut().find(|request| {
            request.id == request_id && request.status == MediaRequestStatus::Pending
        }) else {
            return Err(AppError::Validation(
                "media request is no longer pending".into(),
            ));
        };
        request.requested_quality_profile_id = Some(requested_quality_profile_id);
        request.requested_quality_profile_name = Some(requested_quality_profile_name);
        request.requested_monitor_type = requested_monitor_type;
        request.updated_at = now;
        let updated = request.clone();
        drop(requests);

        if let Some(domain_events) = &self.domain_events {
            domain_events.append(updated_event).await?;
        }

        Ok(updated)
    }

    async fn count_pending_by_facet(
        &self,
        library_ids: &[String],
    ) -> AppResult<MediaRequestCounts> {
        let requests = self.requests.lock().await;
        let mut counts = MediaRequestCounts::default();
        let mut seen = HashSet::new();
        for request in requests.iter().filter(|request| {
            request.status == MediaRequestStatus::Pending
                && library_ids
                    .iter()
                    .any(|library_id| library_id == &request.library_id)
        }) {
            if !seen.insert((
                request.library_id.clone(),
                request.identity_fingerprint.clone(),
            )) {
                continue;
            }
            match request.facet {
                MediaFacet::Movie => counts.movie += 1,
                MediaFacet::Series => counts.series += 1,
                MediaFacet::Anime => counts.anime += 1,
            }
        }
        Ok(counts)
    }

    async fn list(&self, query: MediaRequestQuery) -> AppResult<Vec<MediaRequest>> {
        let requests = self.requests.lock().await;
        Ok(requests
            .iter()
            .filter(|request| {
                query
                    .facet
                    .as_ref()
                    .is_none_or(|facet| &request.facet == facet)
                    && query.status.is_none_or(|status| request.status == status)
                    && query.library_ids.as_ref().is_none_or(|library_ids| {
                        library_ids.iter().any(|id| id == &request.library_id)
                    })
                    && query.requester_user_id.as_ref().is_none_or(|user_id| {
                        request
                            .requesters
                            .iter()
                            .any(|requester| &requester.user_id == user_id)
                    })
            })
            .cloned()
            .collect())
    }
}

struct MockLibraryRepo {
    libraries: Arc<Mutex<Vec<Library>>>,
    app_permissions: Arc<Mutex<HashMap<String, AppPermissionMask>>>,
    grants: Arc<Mutex<HashMap<String, Vec<LibraryGrant>>>>,
}

impl MockLibraryRepo {
    fn with_libraries(libraries: Vec<Library>) -> Self {
        Self {
            libraries: Arc::new(Mutex::new(libraries)),
            app_permissions: Arc::new(Mutex::new(HashMap::new())),
            grants: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn empty() -> Self {
        Self::with_libraries(Vec::new())
    }
}

impl Default for MockLibraryRepo {
    fn default() -> Self {
        Self::with_libraries(
            [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
                .into_iter()
                .map(mock_default_library)
                .collect(),
        )
    }
}

fn mock_default_library(facet: MediaFacet) -> Library {
    let now = Utc::now();
    Library {
        id: scryer_domain::default_library_id_for_facet(&facet),
        facet: facet.clone(),
        name: format!("Default {}", facet.as_str()),
        slug: scryer_domain::default_library_slug_for_facet(&facet).to_string(),
        is_default: true,
        roots: Vec::new(),
        created_at: now,
        updated_at: now,
    }
}

fn mock_library_roots(
    library_id: &str,
    roots: Vec<LibraryRootDraft>,
) -> Vec<scryer_domain::LibraryRoot> {
    let now = Utc::now();
    roots
        .into_iter()
        .map(|root| scryer_domain::LibraryRoot {
            id: Id::new().0,
            library_id: library_id.to_string(),
            path: root.path,
            is_default: root.is_default,
            created_at: now,
            updated_at: now,
        })
        .collect()
}

#[async_trait]
impl LibraryRepository for MockLibraryRepo {
    async fn list(&self, facet: Option<MediaFacet>) -> AppResult<Vec<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .filter(|library| facet.as_ref().is_none_or(|facet| &library.facet == facet))
            .cloned()
            .collect())
    }

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .find(|library| library.id == id)
            .cloned())
    }

    async fn default_for_facet(&self, facet: MediaFacet) -> AppResult<Option<Library>> {
        Ok(self
            .libraries
            .lock()
            .await
            .iter()
            .find(|library| library.facet == facet && library.is_default)
            .cloned())
    }

    async fn create(
        &self,
        mut library: Library,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        library.roots = mock_library_roots(&library.id, roots);
        self.libraries.lock().await.push(library.clone());
        Ok(library)
    }

    async fn update(
        &self,
        library_id: &str,
        name: String,
        slug: String,
        roots: Vec<LibraryRootDraft>,
    ) -> AppResult<Library> {
        let mut libraries = self.libraries.lock().await;
        let library = libraries
            .iter_mut()
            .find(|library| library.id == library_id)
            .ok_or_else(|| AppError::NotFound(format!("library {library_id}")))?;
        library.name = name;
        library.slug = slug;
        library.roots = mock_library_roots(library_id, roots);
        library.updated_at = Utc::now();
        Ok(library.clone())
    }

    async fn delete_library(&self, library_id: &str) -> AppResult<bool> {
        let mut libraries = self.libraries.lock().await;
        let before = libraries.len();
        libraries.retain(|library| library.id != library_id || library.is_default);
        let deleted = libraries.len() != before;
        drop(libraries);
        if deleted {
            let mut grants = self.grants.lock().await;
            for user_grants in grants.values_mut() {
                user_grants.retain(|grant| grant.library_id != library_id);
            }
        }
        Ok(deleted)
    }

    async fn app_permission_mask_for_user(&self, user_id: &str) -> AppResult<AppPermissionMask> {
        Ok(self
            .app_permissions
            .lock()
            .await
            .get(user_id)
            .copied()
            .unwrap_or(AppPermissionMask::NONE))
    }

    async fn set_app_permission_mask_for_user(
        &self,
        user_id: &str,
        permissions: AppPermissionMask,
    ) -> AppResult<()> {
        self.app_permissions
            .lock()
            .await
            .insert(user_id.to_string(), permissions);
        Ok(())
    }

    async fn permission_masks_for_user(&self, user_id: &str) -> AppResult<Vec<LibraryGrant>> {
        Ok(self
            .grants
            .lock()
            .await
            .get(user_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn set_grants_for_user(
        &self,
        user_id: &str,
        mut grants: Vec<LibraryGrant>,
    ) -> AppResult<()> {
        for grant in &mut grants {
            grant.user_id = user_id.to_string();
        }
        self.grants.lock().await.insert(user_id.to_string(), grants);
        Ok(())
    }

    async fn title_library_id(&self, _title_id: &str) -> AppResult<Option<String>> {
        Ok(Some(scryer_domain::default_library_id_for_facet(
            &MediaFacet::Movie,
        )))
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

    async fn set_collections_monitored(
        &self,
        collection_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = collection_ids.iter().cloned().collect::<HashSet<_>>();
        let mut collections = self.collections.lock().await;
        for collection in collections.iter_mut() {
            if wanted.contains(&collection.id) {
                collection.monitored = monitored;
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
        if update.clear_image_url {
            item.image_url = None;
        } else if let Some(value) = update.image_url {
            item.image_url = Some(value);
        }

        Ok(item.clone())
    }

    async fn set_episodes_monitored(
        &self,
        episode_ids: &[String],
        monitored: bool,
    ) -> AppResult<()> {
        let wanted = episode_ids.iter().cloned().collect::<HashSet<_>>();
        let mut episodes = self.episodes.lock().await;
        for episode in episodes.iter_mut() {
            if wanted.contains(&episode.id) {
                episode.monitored = monitored;
            }
        }
        Ok(())
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

struct MockIndexerPluginProvider {
    client: Arc<dyn IndexerClient>,
}

impl IndexerPluginProvider for MockIndexerPluginProvider {
    fn client_for_provider(&self, _config: &IndexerConfig) -> Option<Arc<dyn IndexerClient>> {
        Some(Arc::clone(&self.client))
    }

    fn available_provider_types(&self) -> Vec<String> {
        vec!["nzbgeek".to_string(), "torrent_rss".to_string()]
    }

    fn scoring_policies(&self) -> Vec<scryer_rules::UserPolicy> {
        vec![]
    }

    fn config_fields_for_provider(
        &self,
        provider_type: &str,
    ) -> Vec<scryer_domain::ConfigFieldDef> {
        let connection_key = match provider_type {
            "torrent_rss" => "feed_url",
            _ => "base_url",
        };
        let mut fields = vec![scryer_domain::ConfigFieldDef {
            key: connection_key.to_string(),
            label: "Base URL".to_string(),
            field_type: scryer_domain::ConfigFieldType::String,
            required: true,
            default_value: None,
            value_source: scryer_domain::ConfigFieldValueSource::User,
            role: Some(scryer_domain::ConfigFieldRole::ConnectionUrl),
            host_binding: None,
            options: vec![],
            help_text: None,
        }];
        if provider_type != "torrent_rss" {
            fields.push(scryer_domain::ConfigFieldDef {
                key: "api_key".to_string(),
                label: "API Key".to_string(),
                field_type: scryer_domain::ConfigFieldType::Password,
                required: true,
                default_value: None,
                value_source: scryer_domain::ConfigFieldValueSource::User,
                role: None,
                host_binding: None,
                options: vec![],
                help_text: None,
            });
        }
        fields
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedSearchCall {
    facet: Option<String>,
    newznab_categories: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedStructuredQueryCall {
    query: String,
    season: Option<u32>,
    episode: Option<u32>,
    absolute_episode: Option<u32>,
}

#[derive(Clone)]
struct RecordingCategoriesIndexerClient {
    release_title: String,
    calls: Arc<Mutex<Vec<RecordedSearchCall>>>,
}

impl RecordingCategoriesIndexerClient {
    fn new(release_title: impl Into<String>) -> Self {
        Self {
            release_title: release_title.into(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[derive(Clone, Default)]
struct RecordingStructuredQueryIndexerClient {
    calls: Arc<Mutex<Vec<RecordedStructuredQueryCall>>>,
}

#[async_trait]
impl IndexerClient for RecordingCategoriesIndexerClient {
    async fn search(
        &self,
        _query: String,
        _ids: std::collections::HashMap<String, String>,
        _category: Option<String>,
        facet: Option<String>,
        newznab_categories: Option<Vec<String>>,
        _indexer_routing: Option<IndexerRoutingPlan>,
        _mode: SearchMode,
        _season: Option<u32>,
        _episode: Option<u32>,
        _absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedSearchCall {
            facet,
            newznab_categories,
        });

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
                indexer_languages: None,
                indexer_subtitles: None,
                indexer_grabs: None,
                password_hint: None,
                parsed_release_metadata: Some(crate::parse_release_metadata(&self.release_title)),
                quality_profile_decision: None,
                extra: Default::default(),
                guid: Some("guid-recording-release".to_string()),
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

#[async_trait]
impl IndexerClient for RecordingStructuredQueryIndexerClient {
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
        absolute_episode: Option<u32>,
        _tagged_aliases: Vec<TaggedAlias>,
    ) -> AppResult<IndexerSearchResponse> {
        self.calls.lock().await.push(RecordedStructuredQueryCall {
            query,
            season,
            episode,
            absolute_episode,
        });

        Ok(IndexerSearchResponse {
            results: vec![],
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

    async fn delete_setting_value(
        &self,
        _scope: &str,
        _key_name: &str,
        _scope_id: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, _scope_id: &str) -> AppResult<u32> {
        Ok(0)
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

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.values
            .lock()
            .await
            .remove(&(scope.to_string(), key_name.to_string(), scope_id));
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let mut values = self.values.lock().await;
        let before = values.len();
        values.retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
        Ok((before - values.len()) as u32)
    }
}

#[derive(Default, Clone)]
struct CoalescingSettingsRepo {
    values: StoredSettingValues,
}

impl CoalescingSettingsRepo {
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

    fn implicit_default(key_name: &str) -> Option<&'static str> {
        match key_name {
            QUALITY_PROFILE_ID_KEY => Some("\"4k\""),
            SCORING_PERSONA_KEY => Some("\"Balanced\""),
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY | INDEXER_ROUTING_SETTINGS_KEY => Some("{}"),
            _ => None,
        }
    }
}

#[async_trait]
impl SettingsRepository for CoalescingSettingsRepo {
    async fn get_setting_json(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<Option<String>> {
        if let Some(value) = self
            .values
            .lock()
            .await
            .get(&(scope.to_string(), key_name.to_string(), scope_id.clone()))
            .cloned()
        {
            return Ok(Some(value));
        }

        Ok(Self::implicit_default(key_name).map(str::to_string))
    }

    async fn get_setting_json_explicit(
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

    async fn delete_setting_value(
        &self,
        scope: &str,
        key_name: &str,
        scope_id: Option<String>,
    ) -> AppResult<()> {
        self.values
            .lock()
            .await
            .remove(&(scope.to_string(), key_name.to_string(), scope_id));
        Ok(())
    }

    async fn delete_values_for_scope_id(&self, scope_id: &str) -> AppResult<u32> {
        let mut values = self.values.lock().await;
        let before = values.len();
        values.retain(|(_, _, stored_scope_id), _| stored_scope_id.as_deref() != Some(scope_id));
        Ok((before - values.len()) as u32)
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
        library_id: &str,
        facet: MediaFacet,
        item_path: &str,
    ) -> AppResult<()> {
        self.items.lock().await.retain(|item| {
            !(item.library_id == library_id && item.facet == facet && item.item_path == item_path)
        });
        Ok(())
    }

    async fn delete_for_library(&self, library_id: &str) -> AppResult<u32> {
        let mut items = self.items.lock().await;
        let before = items.len();
        items.retain(|item| item.library_id != library_id);
        Ok((before - items.len()) as u32)
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

    async fn touch_last_error(&self, id: &str) -> AppResult<()> {
        let mut entries = self.store.lock().await;
        let now = Utc::now();
        for entry in entries.iter_mut() {
            if entry.id == id {
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
            derived_base_url,
            rate_limit_seconds,
            rate_limit_burst,
            is_enabled,
            enable_interactive_search,
            enable_auto_search,
            managed_parent_config_id,
            managed_child_key,
            managed_metadata_json,
            caps_snapshot_json,
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
        if let Some(base_url) = derived_base_url {
            item.base_url = base_url;
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
        if let Some(managed_parent_config_id) = managed_parent_config_id {
            item.managed_parent_config_id = managed_parent_config_id;
        }
        if let Some(managed_child_key) = managed_child_key {
            item.managed_child_key = managed_child_key;
        }
        if let Some(managed_metadata_json) = managed_metadata_json {
            item.managed_metadata_json = managed_metadata_json;
        }
        if let Some(caps_snapshot_json) = caps_snapshot_json {
            item.caps_snapshot_json = caps_snapshot_json;
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

#[derive(Default)]
struct MockBlocklistRepo {
    entries: Arc<Mutex<Vec<BlocklistEntry>>>,
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
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for attempt in attempts {
            let Some(normalized_title) =
                crate::normalize_release_attempt_title(attempt.source_title.as_deref())
            else {
                continue;
            };
            if seen.insert(normalized_title) {
                deduped.push(ReleaseDownloadFailureSignature {
                    source_hint: attempt.source_hint,
                    source_title: attempt.source_title,
                });
            }
            if deduped.len() >= limit {
                break;
            }
        }

        Ok(deduped)
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
        let mut seen = HashSet::new();
        let mut deduped = Vec::new();
        for attempt in attempts {
            let Some(normalized_title) =
                crate::normalize_release_attempt_title(attempt.source_title.as_deref())
            else {
                continue;
            };
            if seen.insert(normalized_title) {
                deduped.push(TitleReleaseBlocklistEntry {
                    id: format!(
                        "failed-attempt:{}:{}:{}",
                        attempt.attempted_at,
                        attempt.source_title.as_deref().unwrap_or_default(),
                        attempt.source_hint.as_deref().unwrap_or_default(),
                    ),
                    source_hint: attempt.source_hint,
                    source_title: attempt.source_title,
                    error_message: attempt.error_message,
                    attempted_at: attempt.attempted_at,
                    episode_ids: Vec::new(),
                });
            }
            if deduped.len() >= limit {
                break;
            }
        }

        Ok(deduped)
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

#[async_trait]
impl BlocklistRepository for MockBlocklistRepo {
    async fn add(&self, entry: &NewBlocklistEntry) -> AppResult<String> {
        let id = Id::new().0;
        let data_json = if entry.data.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&entry.data)
                    .map_err(|err| AppError::Repository(err.to_string()))?,
            )
        };
        self.entries.lock().await.push(BlocklistEntry {
            id: id.clone(),
            title_id: entry.title_id.clone(),
            source_title: entry.source_title.clone(),
            source_hint: entry.source_hint.clone(),
            quality: entry.quality.clone(),
            download_id: entry.download_id.clone(),
            reason: entry.reason.clone(),
            data_json,
            created_at: Utc::now().to_rfc3339(),
        });
        Ok(id)
    }

    async fn list_for_title(&self, title_id: &str, limit: usize) -> AppResult<Vec<BlocklistEntry>> {
        let mut entries: Vec<_> = self
            .entries
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect();
        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        entries.truncate(limit);
        Ok(entries)
    }

    async fn list_all(&self, limit: usize, offset: usize) -> AppResult<(Vec<BlocklistEntry>, i64)> {
        let mut entries = self.entries.lock().await.clone();
        entries.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        let total = entries.len() as i64;
        let page = entries.into_iter().skip(offset).take(limit).collect();
        Ok((page, total))
    }

    async fn has_recorded_download_failure(
        &self,
        title_id: &str,
        source_title: Option<&str>,
    ) -> AppResult<bool> {
        let entries = self.entries.lock().await;
        let normalized_source_title = source_title
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        if normalized_source_title.is_none() {
            return Ok(false);
        }

        Ok(entries.iter().any(|entry| {
            entry.title_id == title_id && entry.source_title == normalized_source_title
        }))
    }

    async fn remove(&self, id: &str) -> AppResult<()> {
        self.entries.lock().await.retain(|entry| entry.id != id);
        Ok(())
    }

    async fn is_blocklisted(&self, title_id: &str, source_title: &str) -> AppResult<bool> {
        Ok(self.entries.lock().await.iter().any(|entry| {
            entry.title_id == title_id
                && entry
                    .source_title
                    .as_deref()
                    .is_some_and(|value| value == source_title)
        }))
    }

    async fn delete_for_title(&self, title_id: &str) -> AppResult<()> {
        self.entries
            .lock()
            .await
            .retain(|entry| entry.title_id != title_id);
        Ok(())
    }
}

#[derive(Default, Clone)]
struct TrackingDownloadSubmissionRepo {
    store: Arc<Mutex<Vec<DownloadSubmission>>>,
    tracked_states: TrackedDownloadStates,
    deleted_title_ids: Arc<Mutex<Vec<String>>>,
    list_for_title_calls: Arc<Mutex<Vec<String>>>,
}

#[derive(Default, Clone)]
struct TrackingWantedItemRepo {
    store: Arc<Mutex<Vec<WantedItem>>>,
    release_decisions: Arc<Mutex<Vec<ReleaseDecision>>>,
    title_facets: Arc<Mutex<HashMap<String, MediaFacet>>>,
    status_update_calls: Arc<Mutex<Vec<String>>>,
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

    async fn status_update_call_count_for(&self, id: &str) -> usize {
        self.status_update_calls
            .lock()
            .await
            .iter()
            .filter(|existing| existing.as_str() == id)
            .count()
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
                        .unwrap_or(false)
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
        drop(store);
        self.status_update_calls.lock().await.push(id.to_string());
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
            statuses,
            media_types,
            title_id,
            title_search,
            latest_decision_codes,
            limit,
            offset,
            library_ids: _,
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
                (statuses.is_empty()
                    || statuses.iter().any(|status| item.status.as_str() == status))
                    && (media_types.is_empty() || media_types.contains(&item.media_type))
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && (latest_decision_codes.is_empty()
                        || latest_decision_codes.iter().any(|code| {
                            latest_decision
                                .as_ref()
                                .is_some_and(|decision| decision.decision_code == *code)
                        }))
            })
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect();
        Ok(items)
    }

    async fn count_wanted_items(&self, query: WantedItemsQuery) -> AppResult<i64> {
        let WantedItemsQuery {
            statuses,
            media_types,
            title_id,
            title_search,
            latest_decision_codes,
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
                (statuses.is_empty()
                    || statuses.iter().any(|status| item.status.as_str() == status))
                    && (media_types.is_empty() || media_types.contains(&item.media_type))
                    && title_id
                        .as_deref()
                        .is_none_or(|title_id| item.title_id == title_id)
                    && normalized_title_search.as_ref().is_none_or(|title_search| {
                        item.title_name.as_deref().is_some_and(|title_name| {
                            title_name.to_lowercase().contains(title_search)
                        })
                    })
                    && (latest_decision_codes.is_empty()
                        || latest_decision_codes.iter().any(|code| {
                            latest_decision
                                .as_ref()
                                .is_some_and(|decision| decision.decision_code == *code)
                        }))
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
        let removed_keys: Vec<_> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .map(|entry| {
                (
                    entry.download_client_id.clone().unwrap_or_default(),
                    entry.download_client_type.clone(),
                    entry.download_client_item_id.clone(),
                )
            })
            .collect();
        self.store
            .lock()
            .await
            .retain(|entry| entry.title_id != title_id);
        self.tracked_states
            .lock()
            .await
            .retain(|key, _| !removed_keys.iter().any(|removed| removed == key));
        Ok(())
    }

    async fn delete_by_client_item_id(&self, identity: &DownloadSourceIdentity) -> AppResult<()> {
        let key = (
            identity.client_id.as_deref().unwrap_or("").to_string(),
            identity.client_type.clone(),
            identity.item_id.clone(),
        );
        self.store.lock().await.retain(|entry| {
            entry.download_client_id.as_deref().unwrap_or("").trim()
                != identity.client_id.as_deref().unwrap_or("")
                || entry.download_client_type != identity.client_type.as_str()
                || entry.download_client_item_id != identity.item_id.as_str()
        });
        self.tracked_states.lock().await.remove(&key);
        Ok(())
    }

    async fn update_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
        tracked_state: &str,
    ) -> AppResult<()> {
        let key = (
            identity.client_id.as_deref().unwrap_or("").to_string(),
            identity.client_type.clone(),
            identity.item_id.clone(),
        );
        self.tracked_states
            .lock()
            .await
            .insert(key, tracked_state.to_string());

        let mut entries = self.store.lock().await;
        if !entries.iter().any(|entry| {
            entry.download_client_id.as_deref().unwrap_or("").trim()
                == identity.client_id.as_deref().unwrap_or("")
                && entry.download_client_type == identity.client_type.as_str()
                && entry.download_client_item_id == identity.item_id.as_str()
        }) {
            entries.push(DownloadSubmission {
                title_id: String::new(),
                facet: String::new(),
                download_client_id: identity.client_id.clone(),
                download_client_type: identity.client_type.clone(),
                download_client_item_id: identity.item_id.clone(),
                source_hint: None,
                source_kind: None,
                source_title: None,
                request_signature: None,
                scope: SubmissionScope::Orphan,
            });
        }
        Ok(())
    }

    async fn get_tracked_state(
        &self,
        identity: &DownloadSourceIdentity,
    ) -> AppResult<Option<String>> {
        Ok(self
            .tracked_states
            .lock()
            .await
            .get(&(
                identity.client_id.as_deref().unwrap_or("").to_string(),
                identity.client_type.clone(),
                identity.item_id.clone(),
            ))
            .cloned())
    }
}

#[derive(Default, Clone)]
struct TrackingPendingReleaseRepo {
    store: Arc<Mutex<Vec<PendingRelease>>>,
    deleted_title_ids: Arc<Mutex<Vec<String>>>,
    delete_error: Arc<Mutex<Option<String>>>,
}

impl TrackingPendingReleaseRepo {
    async fn fail_delete_for_title(&self, message: &str) {
        *self.delete_error.lock().await = Some(message.to_string());
    }
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
        if let Some(message) = self.delete_error.lock().await.clone() {
            return Err(AppError::Repository(message));
        }
        self.deleted_title_ids
            .lock()
            .await
            .push(title_id.to_string());
        self.store
            .lock()
            .await
            .retain(|release| release.title_id != title_id);
        Ok(())
    }
}

#[derive(Default, Clone)]
struct TrackingHousekeepingRepo {
    operation_log: Arc<Mutex<Vec<String>>>,
}

impl TrackingHousekeepingRepo {
    fn with_operation_log(operation_log: Arc<Mutex<Vec<String>>>) -> Self {
        Self { operation_log }
    }
}

#[async_trait]
impl HousekeepingRepository for TrackingHousekeepingRepo {
    async fn delete_release_decisions_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_release_attempts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_dispatched_event_outboxes_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_history_events_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_domain_events_older_than_for_types(
        &self,
        _days: i64,
        _event_types: &[DomainEventType],
    ) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_title_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_download_import_artifacts_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_terminal_imports_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_terminal_download_queue_commands_older_than(
        &self,
        _days: i64,
    ) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_rule_set_history_older_than(&self, _days: i64) -> AppResult<u32> {
        Ok(0)
    }

    async fn delete_history_events_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_history_events".to_string());
        Ok(0)
    }

    async fn delete_download_import_artifacts_for_title_ids(
        &self,
        _title_ids: &[String],
    ) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_download_import_artifacts".to_string());
        Ok(0)
    }

    async fn delete_release_attempts_for_title_ids(&self, _title_ids: &[String]) -> AppResult<u32> {
        self.operation_log
            .lock()
            .await
            .push("delete_release_attempts".to_string());
        Ok(0)
    }

    async fn list_all_media_file_paths(&self) -> AppResult<Vec<(String, String)>> {
        Ok(Vec::new())
    }

    async fn delete_media_files_by_ids(&self, _ids: &[String]) -> AppResult<u32> {
        Ok(0)
    }
}

#[derive(Default, Clone)]
struct StubDownloadClient {
    queue_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    history_items: Arc<Mutex<Vec<DownloadQueueItem>>>,
    completed_downloads: Arc<Mutex<Vec<CompletedDownload>>>,
    deleted_items: Arc<Mutex<Vec<(String, bool)>>>,
    deleted_requests: DeletedDownloadRequests,
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

    async fn record_delete(
        &self,
        client_id: Option<&str>,
        client_type: Option<&str>,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        if let Some(error) = self.delete_error.lock().await.clone() {
            return Err(AppError::Repository(error));
        }
        self.deleted_items
            .lock()
            .await
            .push((id.to_string(), is_history));
        self.deleted_requests.lock().await.push((
            client_id.map(str::to_string),
            client_type.map(str::to_string),
            id.to_string(),
            is_history,
        ));
        Ok(())
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
        self.record_delete(None, None, id, is_history).await
    }

    async fn delete_queue_item_for_client_id(
        &self,
        client_id: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.record_delete(Some(client_id), None, id, is_history)
            .await
    }

    async fn delete_queue_item_for_client(
        &self,
        client_type: &str,
        id: &str,
        is_history: bool,
    ) -> AppResult<()> {
        self.record_delete(None, Some(client_type), id, is_history)
            .await
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

#[tokio::test]
async fn create_library_rejects_root_used_by_other_facet_library() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let series_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&series_library_id)
        .await
        .expect("series library should load")
        .expect("series library should exist");

    app.services
        .catalog
        .libraries
        .update(
            &series_library_id,
            series_library.name.clone(),
            series_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("series library roots should update");

    let error = app
        .create_library(
            &user,
            MediaFacet::Anime,
            "Anime2".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect_err("duplicate cross-facet root should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("/Volumes/Media/TV"));
            assert!(message.contains(&series_library.name));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn update_library_rejects_root_used_by_other_facet_library() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let series_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&series_library_id)
        .await
        .expect("series library should load")
        .expect("series library should exist");
    let anime_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&anime_library_id)
        .await
        .expect("anime library should load")
        .expect("anime library should exist");

    app.services
        .catalog
        .libraries
        .update(
            &series_library_id,
            series_library.name.clone(),
            series_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("series library roots should update");

    let error = app
        .update_library(
            &user,
            &anime_library_id,
            Some(anime_library.name.clone()),
            Some(vec![LibraryRootDraft {
                path: "/Volumes/Media/TV".to_string(),
                is_default: true,
            }]),
            None,
        )
        .await
        .expect_err("duplicate cross-facet root should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("/Volumes/Media/TV"));
            assert!(message.contains(&series_library.name));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn delete_library_rejects_default_library() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let error = app
        .delete_library(&user, &movie_library_id)
        .await
        .expect_err("default library delete should be rejected");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("default libraries cannot be deleted")),
        "unexpected delete error: {error:?}"
    );
}

#[tokio::test]
async fn delete_library_purges_library_state_for_non_default_library() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, mut user) = bootstrap_with_scan_unmatched_tracking(
        settings.clone(),
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    settings
        .set_scoped_value("system", "quality.profile", &library.id, "\"kids\"")
        .await;

    let created = app
        .create_title_without_hydration_in_library(
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
            library.id.clone(),
        )
        .await
        .expect("title should be created");

    let mut pending_item = build_test_unmatched_item(
        "library-delete-unmatched",
        MediaFacet::Movie,
        "/Volumes/Media/Kids",
        "/Volumes/Media/Kids/Delete.Me.2026.mkv",
        "Delete Me",
        "Delete Me",
        Some(2026),
    );
    pending_item.library_id = library.id.clone();
    unmatched_items
        .upsert_library_scan_unmatched_item(&pending_item)
        .await
        .expect("pending import should be stored");

    let deleted = app
        .delete_library(&user, &library.id)
        .await
        .expect("library delete should succeed");
    assert!(deleted);

    assert!(
        app.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await
            .expect("library lookup should succeed")
            .is_none()
    );
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&created.title.id)
            .await
            .expect("title lookup should succeed")
            .is_none()
    );
    assert!(
        settings
            .get_scoped_value("system", "quality.profile", &library.id)
            .await
            .is_none()
    );
    assert!(
        unmatched_items
            .items()
            .await
            .iter()
            .all(|item| item.library_id != library.id)
    );
    assert!(
        app.services
            .catalog
            .libraries
            .permission_masks_for_user(&user.id)
            .await
            .expect("grant lookup should succeed")
            .iter()
            .all(|grant| grant.library_id != library.id)
    );
}

#[tokio::test]
async fn delete_library_purges_history_before_deleting_title_rows() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let titles = Arc::new(MockTitleRepo::default());
    let operation_log = Arc::new(Mutex::new(Vec::new()));
    titles.set_delete_operation_log(operation_log.clone()).await;

    let domain_events = Arc::new(MockDomainEventRepo::default());
    domain_events
        .set_delete_operation_log(operation_log.clone())
        .await;

    let (app, mut user) = bootstrap_with_library_delete_repositories(
        titles,
        settings,
        unmatched_items,
        domain_events,
        Arc::new(TrackingHousekeepingRepo::with_operation_log(
            operation_log.clone(),
        )),
        Arc::new(TrackingPendingReleaseRepo::default()),
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    app.create_title_without_hydration_in_library(
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
        library.id.clone(),
    )
    .await
    .expect("title should be created");

    let deleted = app
        .delete_library(&user, &library.id)
        .await
        .expect("library delete should succeed");
    assert!(deleted);

    let operations = operation_log.lock().await.clone();
    let delete_title_index = operations
        .iter()
        .position(|entry| entry.starts_with("delete_title:"))
        .expect("title delete should be recorded");

    assert!(operations[..delete_title_index].contains(&"delete_domain_events".to_string()));
    assert!(operations[..delete_title_index].contains(&"delete_history_events".to_string()));
    assert!(
        operations[..delete_title_index].contains(&"delete_download_import_artifacts".to_string())
    );
    assert!(operations[..delete_title_index].contains(&"delete_release_attempts".to_string()));
}

#[tokio::test]
async fn delete_library_returns_error_when_title_dependency_cleanup_fails() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let titles = Arc::new(MockTitleRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    pending_releases
        .fail_delete_for_title("pending release cleanup failed")
        .await;

    let (app, mut user) = bootstrap_with_library_delete_repositories(
        titles,
        settings.clone(),
        unmatched_items,
        Arc::new(MockDomainEventRepo::default()),
        Arc::new(TrackingHousekeepingRepo::default()),
        pending_releases,
    );

    let library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("library should be created");

    app.services
        .catalog
        .libraries
        .set_grants_for_user(
            &user.id,
            vec![scryer_domain::LibraryGrant {
                user_id: user.id.clone(),
                library_id: library.id.clone(),
                permissions: scryer_domain::LibraryPermissionMask::from_permissions([
                    scryer_domain::LibraryPermission::View,
                    scryer_domain::LibraryPermission::ManageTitles,
                    scryer_domain::LibraryPermission::ManageLibrary,
                ]),
            }],
        )
        .await
        .expect("library grants should be stored");
    user.authorization.loaded = false;

    settings
        .set_scoped_value("system", "quality.profile", &library.id, "\"kids\"")
        .await;

    let created = app
        .create_title_without_hydration_in_library(
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
            library.id.clone(),
        )
        .await
        .expect("title should be created");

    let error = app
        .delete_library(&user, &library.id)
        .await
        .expect_err("library delete should fail");

    assert!(
        matches!(error, AppError::Repository(ref message) if message.contains("pending release cleanup failed")),
        "unexpected delete error: {error:?}"
    );
    assert!(
        app.services
            .catalog
            .libraries
            .get_by_id(&library.id)
            .await
            .expect("library lookup should succeed")
            .is_some()
    );
    assert!(
        app.services
            .catalog
            .titles
            .get_by_id(&created.title.id)
            .await
            .expect("title lookup should succeed")
            .is_some()
    );
    assert!(
        settings
            .get_scoped_value("system", "quality.profile", &library.id)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn update_default_library_preserves_default_slug() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let updated = app
        .update_library(
            &user,
            &movie_library_id,
            Some("Main Movies".to_string()),
            None,
            None,
        )
        .await
        .expect("default library rename should succeed");

    assert_eq!(updated.name, "Main Movies");
    assert_eq!(
        updated.slug,
        scryer_domain::default_library_slug_for_facet(&MediaFacet::Movie)
    );
}

#[tokio::test]
async fn update_non_default_library_rederives_slug_from_name() {
    let (app, user) = bootstrap();
    let created = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids Movies".to_string(),
            vec![LibraryRootDraft {
                path: "/Volumes/Media/Kids".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("custom library should be created");

    let updated = app
        .update_library(
            &user,
            &created.id,
            Some("Adult Movies".to_string()),
            None,
            None,
        )
        .await
        .expect("custom library rename should succeed");

    assert_eq!(updated.name, "Adult Movies");
    assert_eq!(updated.slug, "adult-movies");
}

#[tokio::test]
async fn library_sidecar_settings_resolve_facet_defaults_and_library_overrides() {
    let (app, user) = bootstrap();
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    app.update_media_settings(
        &user,
        MediaFacet::Series,
        UpdateMediaSettings {
            nfo_write_on_import: Some(true),
            plexmatch_write_on_import: Some(true),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("series media settings should update");

    let baseline = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("series library settings should load");
    assert_eq!(baseline.nfo_write_on_import_override, None);
    assert!(baseline.nfo_write_on_import);
    assert_eq!(baseline.plexmatch_write_on_import_override, None);
    assert_eq!(baseline.plexmatch_write_on_import, Some(true));

    app.update_library_settings(
        &user,
        &series_library_id,
        LibrarySettingsOverrideDraft {
            nfo_write_on_import: Some(false),
            plexmatch_write_on_import: Some(false),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("series library overrides should save");

    let overridden = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("series library settings should reload");
    assert_eq!(overridden.nfo_write_on_import_override, Some(false));
    assert!(!overridden.nfo_write_on_import);
    assert_eq!(overridden.plexmatch_write_on_import_override, Some(false));
    assert_eq!(overridden.plexmatch_write_on_import, Some(false));
}

#[tokio::test]
async fn import_mode_settings_resolve_default_facet_override_and_library_override() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let default_media_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie media settings should load");
    assert_eq!(
        default_media_settings.import_mode,
        ImportMode::HardlinkOrCopy
    );

    app.update_media_settings(
        &user,
        MediaFacet::Movie,
        UpdateMediaSettings {
            import_mode: Some(ImportMode::Move),
            ..empty_update_media_settings()
        },
    )
    .await
    .expect("movie import mode should update");

    let facet_override = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should load");
    assert_eq!(facet_override.import_mode_override, None);
    assert_eq!(facet_override.import_mode, ImportMode::Move);

    app.update_library_settings(
        &user,
        &movie_library_id,
        LibrarySettingsOverrideDraft {
            import_mode: Some(ImportMode::HardlinkOrCopy),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("movie library import mode override should save");

    let library_override = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should reload");
    assert_eq!(
        library_override.import_mode_override,
        Some(ImportMode::HardlinkOrCopy)
    );
    assert_eq!(library_override.import_mode, ImportMode::HardlinkOrCopy);

    app.update_library_settings(&user, &movie_library_id, empty_library_settings_override())
        .await
        .expect("movie library import mode override should clear");

    let inherited_again = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("movie library settings should reload after reset");
    assert_eq!(inherited_again.import_mode_override, None);
    assert_eq!(inherited_again.import_mode, ImportMode::Move);
}

#[tokio::test]
async fn import_mode_settings_reject_invalid_stored_value() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_scoped_value(SETTINGS_SCOPE_SYSTEM, IMPORT_MODE_KEY, "movie", "\"auto\"")
        .await;
    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(StoredQualityProfileRepo::default()),
        Arc::new(MockIndexerClient),
    );

    let error = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect_err("invalid import mode should be rejected");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("invalid import.mode setting value"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

fn test_quality_profile(id: &str) -> QualityProfile {
    QualityProfile {
        id: id.to_string(),
        name: id.to_string(),
        criteria: QualityProfileCriteria::default(),
    }
}

#[tokio::test]
async fn resolve_quality_profile_uses_facet_settings_when_library_scope_only_coalesces_defaults() {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "movie",
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "series",
            "\"wizard-series\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "anime",
            "\"wizard-anime\"",
        )
        .await;

    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            test_quality_profile("4k"),
            test_quality_profile("wizard-movie"),
            test_quality_profile("wizard-series"),
            test_quality_profile("wizard-anime"),
        ])
        .await;

    let (app, _) = bootstrap_with_settings_repo_and_profiles(
        settings,
        quality_profiles,
        Arc::new(MockIndexerClient),
    );

    for (facet, category_hint, expected_profile_id) in [
        (MediaFacet::Movie, "movie", "wizard-movie"),
        (MediaFacet::Series, "series", "wizard-series"),
        (MediaFacet::Anime, "anime", "wizard-anime"),
    ] {
        let library_id = scryer_domain::default_library_id_for_facet(&facet);
        let resolved = app
            .resolve_quality_profile(crate::app_usecase_discovery::QualityProfileLookup {
                title_tags: &[],
                library_id: Some(library_id.as_str()),
                imdb_id: None,
                tvdb_id: None,
                category_hint: Some(category_hint),
            })
            .await
            .expect("quality profile should resolve");

        assert_eq!(resolved.id, expected_profile_id);
    }
}

#[tokio::test]
async fn library_settings_inherit_facet_quality_and_persona_when_library_scope_only_coalesces_defaults()
 {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    settings
        .set_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "\"wizard-movie\"",
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            QUALITY_PROFILE_ID_KEY,
            "series",
            "\"wizard-series\"",
        )
        .await;
    settings
        .set_value(SETTINGS_SCOPE_SYSTEM, SCORING_PERSONA_KEY, "\"Compatible\"")
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            SCORING_PERSONA_KEY,
            "series",
            "\"Audiophile\"",
        )
        .await;

    let quality_profiles = Arc::new(StoredQualityProfileRepo::default());
    quality_profiles
        .set_profiles(vec![
            test_quality_profile("4k"),
            test_quality_profile("wizard-movie"),
            test_quality_profile("wizard-series"),
        ])
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        quality_profiles,
        Arc::new(MockIndexerClient),
    );

    let library_settings = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("library settings should load");

    assert_eq!(library_settings.quality_profile_id_override, None);
    assert_eq!(library_settings.quality_profile_id, "wizard-series");
    assert_eq!(library_settings.scoring_persona_override, None);
    assert_eq!(library_settings.scoring_persona, ScoringPersona::Audiophile);
}

#[tokio::test]
async fn library_settings_inherit_facet_routing_when_library_scope_only_coalesces_defaults() {
    let settings = Arc::new(CoalescingSettingsRepo::default());
    let series_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "series",
            r#"{"weaver":{"enabled":true,"category":"tv"}}"#,
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            INDEXER_ROUTING_SETTINGS_KEY,
            "series",
            r#"{"nzbgeek":{"enabled":true,"categories":["5000"],"priority":7}}"#,
        )
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );

    let download_client_routing = app
        .get_download_client_routing(&user, "series")
        .await
        .expect("download client routing should load");
    assert_eq!(download_client_routing.len(), 1);
    assert_eq!(download_client_routing[0].client_id, "weaver");

    let indexer_routing = app
        .get_indexer_routing(&user, "series")
        .await
        .expect("indexer routing should load");
    assert_eq!(indexer_routing.len(), 1);
    assert_eq!(indexer_routing[0].indexer_id, "nzbgeek");

    let library_settings = app
        .get_library_settings(&user, &series_library_id)
        .await
        .expect("library settings should load");

    assert_eq!(library_settings.download_client_routing_override, None);
    assert_eq!(library_settings.indexer_routing_override, None);
}

#[tokio::test]
async fn movie_library_rejects_plexmatch_override() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let error = app
        .update_library_settings(
            &user,
            &movie_library_id,
            LibrarySettingsOverrideDraft {
                plexmatch_write_on_import: Some(true),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect_err("movie library should reject plexmatch override");

    match error {
        AppError::Validation(message) => {
            assert!(message.contains("plexmatch_write_on_import"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }
}

fn test_admin_user() -> User {
    let mut user = User::new_admin("admin");
    user.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::from_permissions([
            AppPermission::ManageUsers,
            AppPermission::ManagePermissions,
            AppPermission::ManageSystemSettings,
            AppPermission::ManageCatalogSettings,
        ]),
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
            scryer_domain::LibraryPermission::ResolveImports,
            scryer_domain::LibraryPermission::ManageLibrary,
            scryer_domain::LibraryPermission::Request,
            scryer_domain::LibraryPermission::AutoApproveRequests,
        ]),
        loaded: true,
        ..Default::default()
    };
    user
}

fn test_user_with_app_permissions(username: &str, app_permissions: AppPermissionMask) -> User {
    let mut user = User {
        id: Id::new().0,
        username: username.to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    user.authorization.app = app_permissions;
    user.authorization.loaded = true;
    user
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
    let settings = Arc::new(StoredSettingsRepo::default());
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
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
}

struct MediaRequestTestHarness {
    app: AppUseCase,
    user: User,
    titles: Arc<MockTitleRepo>,
    libraries: Arc<MockLibraryRepo>,
    media_requests: Arc<MockMediaRequestRepo>,
    domain_events: Arc<MockDomainEventRepo>,
}

fn bootstrap_media_request_app() -> MediaRequestTestHarness {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let libraries = Arc::new(MockLibraryRepo::default());
    let domain_events = Arc::new(MockDomainEventRepo::default());
    let media_requests = Arc::new(MockMediaRequestRepo::with_domain_events(
        domain_events.clone(),
    ));
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());

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
    .with_domain_events(domain_events.clone())
    .with_libraries(libraries.clone())
    .with_media_requests(media_requests.clone())
    .with_wanted_items(wanted_items.clone())
    .with_pending_releases(pending_releases.clone())
    .with_download_submissions(download_submissions.clone())
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
        download_submissions,
        pending_releases,
        wanted_items,
    }))
    .build_partial_for_tests();
    let mut registry = FacetRegistry::new();
    registry.register(Arc::new(MovieFacetHandler));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Series,
    )));
    registry.register(Arc::new(SeriesFacetHandler::new(
        scryer_domain::MediaFacet::Anime,
    )));

    MediaRequestTestHarness {
        app: AppUseCase::new(
            services,
            JwtAuthConfig {
                issuer: "scryer-test".to_string(),
                access_ttl_seconds: 3600,
                jwt_signing_salt: "test-salt".to_string(),
            },
            Arc::new(registry),
        ),
        user: test_admin_user(),
        titles,
        libraries,
        media_requests,
        domain_events,
    }
}

fn media_request_input(library_id: impl Into<String>, tvdb_id: i64) -> SubmitMediaRequestInput {
    SubmitMediaRequestInput {
        library_id: library_id.into(),
        facet: MediaFacet::Movie,
        title: "Glass Harbor".to_string(),
        sort_title: Some("Glass Harbor".to_string()),
        slug: Some("glass-harbor".to_string()),
        poster_url: Some("https://example.test/glass-harbor.jpg".to_string()),
        year: Some(2026),
        overview: Some("A test request subject".to_string()),
        runtime_minutes: Some(101),
        language: Some("en".to_string()),
        content_status: Some("Released".to_string()),
        requested_quality_profile_id: None,
        requested_monitor_type: None,
        external_ids: vec![
            ExternalId {
                source: "TVDB".to_string(),
                value: tvdb_id.to_string(),
            },
            ExternalId {
                source: "imdb".to_string(),
                value: "tt1234567".to_string(),
            },
        ],
    }
}

fn library_permission_user(
    username: &str,
    library_id: &str,
    permissions: &[scryer_domain::LibraryPermission],
) -> User {
    let mut user = User::new_admin(username);
    user.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::NONE,
        libraries: HashMap::from([(
            library_id.to_string(),
            scryer_domain::LibraryPermissionMask::from_permissions(permissions.iter().copied()),
        )]),
        default_library: scryer_domain::LibraryPermissionMask::NONE,
        loaded: true,
    };
    user
}

fn custom_movie_library(id: &str, name: &str) -> Library {
    let mut library = mock_default_library(MediaFacet::Movie);
    library.id = id.to_string();
    library.name = name.to_string();
    library.slug = name.to_ascii_lowercase().replace(' ', "-");
    library.is_default = false;
    library
}

#[tokio::test]
async fn submit_media_request_creates_request_requester_and_domain_event() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let outcome = harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9010))
        .await
        .expect("request submission should succeed");

    assert!(outcome.accepted);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.library_id, library_id);
    assert_eq!(request.status, MediaRequestStatus::Pending);
    assert_eq!(request.requested_quality_profile_id.as_deref(), Some("4k"));
    assert_eq!(
        request.requested_quality_profile_name.as_deref(),
        Some("4K")
    );
    assert!(request.requested_monitor_type.is_none());
    assert_eq!(request.created_by_user_id, harness.user.id);
    assert_eq!(request.requesters.len(), 1);
    assert_eq!(request.requesters[0].user_id, harness.user.id);
    assert_eq!(
        request.external_ids,
        vec![
            ExternalId {
                source: "imdb".to_string(),
                value: "tt1234567".to_string(),
            },
            ExternalId {
                source: "tvdb".to_string(),
                value: "9010".to_string(),
            },
        ]
    );

    let events = harness.domain_events.events.lock().await;
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].actor_user_id.as_deref(),
        Some(harness.user.id.as_str())
    );
    match &events[0].payload {
        DomainEventPayload::MediaRequestSubmitted(data) => {
            assert_eq!(data.request_id, request.id);
            assert_eq!(data.library_id, library_id);
            assert_eq!(data.title_name, "Glass Harbor");
            assert_eq!(data.external_ids, request.external_ids);
            assert_eq!(data.requested_quality_profile_id.as_deref(), Some("4k"));
            assert_eq!(data.requested_quality_profile_name.as_deref(), Some("4K"));
            assert!(data.requested_monitor_type.is_none());
        }
        other => panic!("unexpected event payload: {other:?}"),
    }
}

#[tokio::test]
async fn submit_media_request_uses_library_request_quality_profile_allowlist() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    let mut input = media_request_input(library_id.clone(), 9026);
    input.requested_quality_profile_id = Some("1080p".to_string());
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("allowlisted request profile should be accepted");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
    assert_eq!(
        requests[0].requested_quality_profile_name.as_deref(),
        Some("1080P")
    );
}

#[tokio::test]
async fn submit_media_request_rejects_profiles_outside_library_request_allowlist() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    let mut input = media_request_input(library_id, 9027);
    input.requested_quality_profile_id = Some("4k".to_string());
    let error = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("request profile outside allowlist should fail");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("not allowed")),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_defaults_missing_profile_to_library_request_default() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9028))
        .await
        .expect("missing profile should use request default");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(
        requests[0].requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
}

#[tokio::test]
async fn media_request_activity_is_visible_to_library_viewers() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9021))
        .await
        .expect("request submission should succeed");

    let viewer = library_permission_user(
        "request-activity-viewer",
        &library_id,
        &[scryer_domain::LibraryPermission::View],
    );
    let activities = harness
        .app
        .recent_activity(&viewer, 10, 0)
        .await
        .expect("request activity should be visible");

    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].kind, ActivityKind::SystemNotice);
    assert!(
        activities[0].message.contains("Requested 'Glass Harbor'"),
        "unexpected activity message: {}",
        activities[0].message
    );
}

#[tokio::test]
async fn submit_media_request_duplicate_same_user_creates_separate_submission_and_event() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9011);

    let first = harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    let second = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed opaquely");

    assert!(first.accepted);
    assert!(second.accepted);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.requesters.len() == 1));
    let request_ids = requests
        .iter()
        .map(|request| request.id.clone())
        .collect::<HashSet<_>>();
    assert_eq!(request_ids.len(), 2);
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|event| matches!(event.payload, DomainEventPayload::MediaRequestSubmitted(_)))
    );
}

#[tokio::test]
async fn submit_media_request_second_user_creates_private_submission_without_exposing_prior_request()
 {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id.clone(), 9012);
    let second_user = library_permission_user(
        "requester-two",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );

    let first = harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    let second = harness
        .app
        .submit_media_request(&second_user, input)
        .await
        .expect("second request should attach opaquely");

    assert!(first.accepted);
    assert!(second.accepted);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| request.requesters.len() == 1));
    let requester_ids = requests
        .iter()
        .flat_map(|request| request.requesters.iter().map(|entry| entry.user_id.clone()))
        .collect::<HashSet<_>>();
    assert!(requester_ids.contains(&harness.user.id));
    assert!(requester_ids.contains(&second_user.id));
    assert_eq!(requester_ids.len(), 2);
    drop(requests);

    let events = harness.domain_events.events.lock().await;
    let request_ids = events
        .iter()
        .map(|event| match &event.payload {
            DomainEventPayload::MediaRequestSubmitted(data) => data.request_id.clone(),
            other => panic!("unexpected event payload: {other:?}"),
        })
        .collect::<HashSet<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(request_ids.len(), 2);
}

#[tokio::test]
async fn submit_media_request_accepts_search_correlation_id_without_tvdb() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9019);
    input.external_ids = vec![ExternalId {
        source: "imdb".to_string(),
        value: "tt7654321".to_string(),
    }];

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("imdb-backed search request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].external_ids,
        vec![ExternalId {
            source: "imdb".to_string(),
            value: "tt7654321".to_string(),
        }]
    );
}

#[tokio::test]
async fn submit_media_request_rejects_ids_that_cannot_correlate_to_smg_search() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let mut input = media_request_input(library_id, 9020);
    input.external_ids = vec![ExternalId {
        source: "unknown".to_string(),
        value: "opaque".to_string(),
    }];

    let error = harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect_err("unsupported identity should fail");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("searchable SMG identifier")),
        "unexpected error: {error:?}"
    );
}

#[tokio::test]
async fn submit_media_request_allows_same_identity_in_different_libraries() {
    let harness = bootstrap_media_request_app();
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let alternate_library_id = "movie-library-alt".to_string();
    harness
        .libraries
        .libraries
        .lock()
        .await
        .push(custom_movie_library(&alternate_library_id, "Movie Alt"));

    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(default_library_id.clone(), 9013),
        )
        .await
        .expect("default library request should succeed");
    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(alternate_library_id.clone(), 9013),
        )
        .await
        .expect("alternate library request should succeed");

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .any(|request| request.library_id == default_library_id)
    );
    assert!(
        requests
            .iter()
            .any(|request| request.library_id == alternate_library_id)
    );
}

#[tokio::test]
async fn submit_media_request_blocks_existing_title_in_target_library() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    harness
        .titles
        .store
        .lock()
        .await
        .push(make_due_hydration_title(
            "existing-movie",
            MediaFacet::Movie,
            9014,
        ));

    let error = harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9014))
        .await
        .expect_err("existing title identity should block request");

    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("already exists")),
        "unexpected error: {error:?}"
    );
    assert!(harness.media_requests.requests.lock().await.is_empty());
    assert!(harness.domain_events.events.lock().await.is_empty());
}

#[tokio::test]
async fn submit_media_request_requires_request_permission_and_matching_facet() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let requestless_user = library_permission_user("viewer", &library_id, &[]);

    let permission_error = harness
        .app
        .submit_media_request(
            &requestless_user,
            media_request_input(library_id.clone(), 9015),
        )
        .await
        .expect_err("request permission should be required");
    assert!(
        matches!(permission_error, AppError::Unauthorized(_)),
        "unexpected permission error: {permission_error:?}"
    );

    let mut mismatched = media_request_input(library_id, 9016);
    mismatched.facet = MediaFacet::Series;
    let facet_error = harness
        .app
        .submit_media_request(&harness.user, mismatched)
        .await
        .expect_err("facet mismatch should fail");
    assert!(
        matches!(facet_error, AppError::Validation(ref message) if message.contains("facet")),
        "unexpected facet error: {facet_error:?}"
    );
}

#[tokio::test]
async fn list_media_requests_filters_by_facet_and_manageable_libraries() {
    let harness = bootstrap_media_request_app();
    let default_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let alternate_library_id = "movie-library-queue".to_string();
    harness
        .libraries
        .libraries
        .lock()
        .await
        .push(custom_movie_library(&alternate_library_id, "Movie Queue"));

    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(default_library_id.clone(), 9017),
        )
        .await
        .expect("default library request should succeed");
    harness
        .app
        .submit_media_request(
            &harness.user,
            media_request_input(alternate_library_id.clone(), 9018),
        )
        .await
        .expect("alternate library request should succeed");

    let queue_manager = library_permission_user(
        "queue-manager",
        &alternate_library_id,
        &[scryer_domain::LibraryPermission::ManageTitles],
    );
    let requests = harness
        .app
        .list_media_requests(
            &queue_manager,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: Some(vec![default_library_id, alternate_library_id.clone()]),
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("request list should load");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].library_id, alternate_library_id);
    assert_eq!(requests[0].requesters.len(), 1);
}

#[tokio::test]
async fn list_my_media_requests_filters_to_requester_owned_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let second_user = library_permission_user(
        "requester-owned-list",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9031))
        .await
        .expect("first user's request should succeed");
    harness
        .app
        .submit_media_request(&second_user, media_request_input(library_id, 9032))
        .await
        .expect("second user's request should succeed");

    let requests = harness
        .app
        .list_my_media_requests(
            &second_user,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: None,
                status: None,
            },
        )
        .await
        .expect("own requests should load");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].created_by_user_id, second_user.id);
    assert!(
        requests[0]
            .requesters
            .iter()
            .any(|requester| requester.user_id == second_user.id)
    );
}

#[tokio::test]
async fn request_only_user_can_list_submitted_bluey_series_request() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let requester = library_permission_user(
        "bluey-requester-owned-list",
        &library_id,
        &[scryer_domain::LibraryPermission::Request],
    );
    let mut input = media_request_input(library_id.clone(), 353546);
    input.facet = MediaFacet::Series;
    input.title = "Bluey".to_string();
    input.sort_title = Some("Bluey".to_string());
    input.slug = Some("bluey".to_string());
    input.year = Some(2018);
    input.content_status = Some("Continuing".to_string());
    input.requested_monitor_type = Some("allEpisodes".to_string());
    input.external_ids = vec![
        ExternalId {
            source: "tvdb".to_string(),
            value: "353546".to_string(),
        },
        ExternalId {
            source: "imdb".to_string(),
            value: "tt7678620".to_string(),
        },
    ];

    harness
        .app
        .submit_media_request(&requester, input)
        .await
        .expect("request-only Bluey submission should succeed");

    let requests = harness
        .app
        .list_my_media_requests(
            &requester,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Series),
                library_ids: None,
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("requester should list own Bluey request");

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].library_id, library_id);
    assert_eq!(requests[0].facet, MediaFacet::Series);
    assert_eq!(requests[0].title, "Bluey");
    assert_eq!(requests[0].status, MediaRequestStatus::Pending);
    assert_eq!(requests[0].created_by_user_id, requester.id);
    assert_eq!(
        requests[0].requested_monitor_type.as_deref(),
        Some("allepisodes")
    );
    assert!(
        requests[0]
            .requesters
            .iter()
            .any(|entry| entry.user_id == requester.id)
    );
}

#[tokio::test]
async fn requester_can_update_pending_request_preferences() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    harness
        .app
        .update_library_settings(
            &harness.user,
            &library_id,
            LibrarySettingsOverrideDraft {
                request_quality_profile_ids: Some(vec!["1080p".to_string(), "4k".to_string()]),
                ..empty_library_settings_override()
            },
        )
        .await
        .expect("request profile allowlist should save");
    let mut input = media_request_input(library_id, 9033);
    input.facet = MediaFacet::Series;

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();

    let updated = harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id,
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: Some("allEpisodes".to_string()),
            },
        )
        .await
        .expect("requester should update pending request");

    assert_eq!(
        updated.requested_quality_profile_id.as_deref(),
        Some("1080p")
    );
    assert_eq!(
        updated.requested_quality_profile_name.as_deref(),
        Some("1080P")
    );
    assert_eq!(
        updated.requested_monitor_type.as_deref(),
        Some("allepisodes")
    );

    let events = harness.domain_events.events.lock().await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event.payload, DomainEventPayload::MediaRequestUpdated(_)))
    );
}

#[tokio::test]
async fn requester_can_cancel_pending_request_without_resolving_overlapping_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9034);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();

    let canceled = harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect("requester should cancel pending request");

    assert_eq!(canceled, 1);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].status, MediaRequestStatus::Canceled);
    assert_eq!(requests[1].status, MediaRequestStatus::Pending);
}

#[tokio::test]
async fn requester_cannot_update_or_cancel_after_manager_resolution() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id, 9035))
        .await
        .expect("request should succeed");
    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    harness
        .app
        .dismiss_media_request(&harness.user, &request_id)
        .await
        .expect("manager should reject request");

    let update_error = harness
        .app
        .update_my_media_request(
            &harness.user,
            UpdateMediaRequestInput {
                request_id: request_id.clone(),
                requested_quality_profile_id: "1080p".to_string(),
                requested_monitor_type: None,
            },
        )
        .await
        .expect_err("resolved request cannot be updated");
    assert!(
        matches!(update_error, AppError::Validation(ref message) if message.contains("no longer pending")),
        "unexpected update error: {update_error:?}"
    );

    let cancel_error = harness
        .app
        .cancel_my_media_request(&harness.user, &request_id)
        .await
        .expect_err("resolved request cannot be canceled");
    assert!(
        matches!(cancel_error, AppError::Validation(ref message) if message.contains("no longer pending")),
        "unexpected cancel error: {cancel_error:?}"
    );
}

#[tokio::test]
async fn approve_media_request_creates_title_and_resolves_overlapping_pending_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id.clone(), 9022);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    let outcome = harness
        .app
        .approve_media_request(&harness.user, &request_id, "1080p", None)
        .await
        .expect("approval should create the title");

    assert!(outcome.accepted);
    assert!(outcome.search_error.is_none());
    let titles = harness.titles.store.lock().await;
    assert_eq!(titles.len(), 1);
    let title = &titles[0];
    assert_eq!(outcome.title_id, title.id);
    assert_eq!(title.name, "Glass Harbor");
    assert_eq!(title.library_id, library_id);
    assert_eq!(title.year, Some(2026));
    assert_eq!(
        title.poster_url.as_deref(),
        Some("https://example.test/glass-harbor.jpg")
    );
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:quality-profile:1080p")
    );
    drop(titles);

    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.status == MediaRequestStatus::Approved)
    );
    assert!(requests.iter().all(|request| {
        request.created_title_id.as_deref() == Some(outcome.title_id.as_str())
            && request.approved_quality_profile_id.as_deref() == Some("1080p")
            && request.approved_quality_profile_name.as_deref() == Some("1080P")
            && request.resolved_by_user_id.as_deref() == Some(harness.user.id.as_str())
            && request.resolved_at.is_some()
    }));
}

#[tokio::test]
async fn approve_series_media_request_applies_requested_monitor_type() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut input = media_request_input(library_id.clone(), 9030);
    input.facet = MediaFacet::Series;
    input.requested_monitor_type = Some("missingAndFutureEpisodes".to_string());

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("series request should succeed");

    let request = harness.media_requests.requests.lock().await[0].clone();
    assert_eq!(
        request.requested_monitor_type.as_deref(),
        Some("missingandfutureepisodes")
    );

    let outcome = harness
        .app
        .approve_media_request(&harness.user, &request.id, "1080p", None)
        .await
        .expect("approval should create the series title");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == outcome.title_id)
        .expect("approved title should be stored");
    assert!(title.monitored);
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:monitor-type:missingandfutureepisodes")
    );
}

#[tokio::test]
async fn approve_series_media_request_can_override_requested_monitor_type() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Series);
    let mut input = media_request_input(library_id, 9036);
    input.facet = MediaFacet::Series;
    input.requested_monitor_type = Some("allEpisodes".to_string());

    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("series request should succeed");

    let request = harness.media_requests.requests.lock().await[0].clone();
    let outcome = harness
        .app
        .approve_media_request(
            &harness.user,
            &request.id,
            "1080p",
            Some("none".to_string()),
        )
        .await
        .expect("approval should create the series title");

    let titles = harness.titles.store.lock().await;
    let title = titles
        .iter()
        .find(|title| title.id == outcome.title_id)
        .expect("approved title should be stored");
    assert!(!title.monitored);
    assert!(
        title
            .tags
            .iter()
            .any(|tag| tag == "scryer:monitor-type:none")
    );
}

#[tokio::test]
async fn dismiss_media_request_resolves_overlapping_pending_requests_without_title() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9023);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let request_id = harness.media_requests.requests.lock().await[0].id.clone();
    let removed = harness
        .app
        .dismiss_media_request(&harness.user, &request_id)
        .await
        .expect("dismiss should remove the request group");

    assert_eq!(removed, 2);
    let requests = harness.media_requests.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.status == MediaRequestStatus::Rejected)
    );
    assert!(requests.iter().all(|request| {
        request.created_title_id.is_none()
            && request.approved_quality_profile_id.is_none()
            && request.resolved_by_user_id.as_deref() == Some(harness.user.id.as_str())
            && request.resolved_at.is_some()
    }));
    drop(requests);
    assert!(harness.titles.store.lock().await.is_empty());
}

#[tokio::test]
async fn pending_media_request_counts_deduplicate_duplicate_identity_requests() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let input = media_request_input(library_id, 9024);

    harness
        .app
        .submit_media_request(&harness.user, input.clone())
        .await
        .expect("first request should succeed");
    harness
        .app
        .submit_media_request(&harness.user, input)
        .await
        .expect("duplicate request should succeed");

    let counts = harness
        .app
        .pending_media_request_counts(&harness.user)
        .await
        .expect("request counts should load");

    assert_eq!(counts.movie, 1);
    assert_eq!(counts.series, 0);
    assert_eq!(counts.anime, 0);
}

#[tokio::test]
async fn media_request_admin_surfaces_require_manage_titles_library_permission() {
    let harness = bootstrap_media_request_app();
    let library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    harness
        .app
        .submit_media_request(&harness.user, media_request_input(library_id.clone(), 9025))
        .await
        .expect("request submission should succeed");

    let config_admin = test_user_with_app_permissions(
        "catalog-config-admin",
        AppPermissionMask::MANAGE_CATALOG_SETTINGS,
    );

    let listed = harness
        .app
        .list_media_requests(
            &config_admin,
            ListMediaRequestsInput {
                facet: Some(MediaFacet::Movie),
                library_ids: None,
                status: Some(MediaRequestStatus::Pending),
            },
        )
        .await
        .expect("request list should load");
    assert!(listed.is_empty());

    let counts = harness
        .app
        .pending_media_request_counts(&config_admin)
        .await
        .expect("request counts should load");
    assert_eq!(counts.movie, 0);
    assert_eq!(counts.series, 0);
    assert_eq!(counts.anime, 0);
    assert!(
        !harness
            .app
            .can_manage_media_requests(&config_admin)
            .await
            .expect("permission check should load")
    );

    let events = harness
        .app
        .list_media_request_lifecycle_events_for_manager(&config_admin, 0, 10)
        .await
        .expect("request event list should load");
    assert!(events.is_empty());

    let request_manager = library_permission_user(
        "request-manager",
        &library_id,
        &[scryer_domain::LibraryPermission::ManageTitles],
    );
    let manager_events = harness
        .app
        .list_media_request_lifecycle_events_for_manager(&request_manager, 0, 10)
        .await
        .expect("manager request events should load");
    assert_eq!(manager_events.len(), 1);
}

async fn wait_for_title_image_clear_calls(repo: &BlockingTitleImageRepo, expected: usize) {
    timeout(Duration::from_secs(1), async {
        loop {
            if repo.clear_calls.load(Ordering::SeqCst) >= expected {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("title image clear call count should reach expected value");
}

async fn wait_for_title_image_cache_clear_idle(app: &AppUseCase) {
    timeout(Duration::from_secs(1), async {
        loop {
            if !app
                .runtime
                .catalog
                .title_image_cache_clear_scheduled
                .load(Ordering::Acquire)
            {
                return;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("title image cache clear should become idle");
}

#[tokio::test]
async fn clear_title_image_cache_collapses_duplicate_requests_and_waits_for_scans() {
    let (app, admin) = bootstrap_with_user_repo(Arc::new(MockUserRepo::default()));
    let title_images = Arc::new(BlockingTitleImageRepo::default());
    let app = app.with_test_overrides(|services| services.with_title_images(title_images.clone()));

    let scan = app
        .runtime
        .library
        .library_scan_tracker
        .start_session(MediaFacet::Movie)
        .await
        .expect("scan should start");

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset")
    );
    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse queued reset")
    );

    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        0,
        "cache clear should wait behind active library scans"
    );

    app.runtime
        .library
        .library_scan_tracker
        .fail_session(&scan.session_id)
        .await
        .expect("scan should finish");
    wait_for_title_image_clear_calls(&title_images, 1).await;

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("collapse running reset")
    );
    sleep(Duration::from_millis(50)).await;
    assert_eq!(
        title_images.clear_calls.load(Ordering::SeqCst),
        1,
        "running cache clear should collapse duplicate requests"
    );

    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;

    assert!(
        app.clear_title_image_cache(&admin)
            .await
            .expect("queue reset after previous reset completes")
    );
    wait_for_title_image_clear_calls(&title_images, 2).await;
    title_images.release_clear.notify_waiters();
    wait_for_title_image_cache_clear_idle(&app).await;
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
    let settings = Arc::new(StoredSettingsRepo::default());
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
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user(), titles)
}

fn make_due_hydration_title(id: &str, facet: MediaFacet, tvdb_id: i64) -> Title {
    Title {
        id: id.to_string(),
        name: format!("Title {id}"),
        library_id: scryer_domain::default_library_id_for_facet(&facet),
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
    let settings = Arc::new(StoredSettingsRepo::default());
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
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_tracked_download_handle(tracked_download_handle)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
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
    let settings = Arc::new(StoredSettingsRepo::default());
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
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
}

fn bootstrap_with_search_settings_and_indexer(
    settings: Arc<StoredSettingsRepo>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    bootstrap_with_settings_repo_and_profiles(
        settings,
        Arc::new(MockQualityProfileRepo),
        indexer_client,
    )
}

fn bootstrap_with_search_settings_indexer_and_configs(
    settings: Arc<StoredSettingsRepo>,
    indexer_client: Arc<dyn IndexerClient>,
    configs: Vec<IndexerConfig>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo {
        store: Arc::new(Mutex::new(configs)),
    });
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let plugin_provider = Arc::new(MockIndexerPluginProvider {
        client: Arc::clone(&indexer_client),
    });

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
        Arc::new(MockQualityProfileRepo),
        String::new(),
    )
    .with_plugin_provider(plugin_provider)
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
}

fn bootstrap_with_settings_repo_and_profiles(
    settings: Arc<dyn SettingsRepository>,
    quality_profiles: Arc<dyn QualityProfileRepository>,
    indexer_client: Arc<dyn IndexerClient>,
) -> (AppUseCase, User) {
    bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        quality_profiles,
        indexer_client,
        Arc::new(MockLibraryRepo::default()),
    )
}

fn bootstrap_with_settings_repo_and_profiles_and_libraries(
    settings: Arc<dyn SettingsRepository>,
    quality_profiles: Arc<dyn QualityProfileRepository>,
    indexer_client: Arc<dyn IndexerClient>,
    libraries: Arc<dyn LibraryRepository>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let download_client = Arc::new(StubDownloadClient::default());
    let plugin_provider = Arc::new(MockIndexerPluginProvider {
        client: Arc::clone(&indexer_client),
    });

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
    .with_plugin_provider(plugin_provider)
    .with_domain_events(Arc::new(MockDomainEventRepo::default()))
    .with_libraries(libraries)
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

    (app, test_admin_user())
}

fn synthetic_direct_nab_indexer_config(id: &str, provider_type: &str) -> IndexerConfig {
    IndexerConfig {
        id: id.to_string(),
        name: format!("Synthetic {provider_type}"),
        provider_type: provider_type.to_string(),
        base_url: "https://example.invalid".to_string(),
        api_key_encrypted: None,
        rate_limit_seconds: None,
        rate_limit_burst: None,
        disabled_until: None,
        is_enabled: true,
        enable_interactive_search: true,
        enable_auto_search: true,
        managed_parent_config_id: None,
        managed_child_key: None,
        managed_metadata_json: None,
        caps_snapshot_json: None,
        last_health_status: None,
        last_error_at: None,
        config_json: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
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
        app.should_remove_completed_download(None, &MediaFacet::Movie, "weaver")
            .await
    );
    assert!(
        !app.should_remove_failed_download(None, &MediaFacet::Movie, "weaver")
            .await
    );
}

#[tokio::test]
async fn library_cleanup_routing_override_beats_facet_cleanup_flags() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            "movie",
            &serde_json::json!({
                "weaver": {
                    "enabled": true,
                    "removeCompleted": true,
                    "removeFailed": false
                }
            })
            .to_string(),
        )
        .await;
    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            &serde_json::json!({
                "weaver": {
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
        !app.should_remove_completed_download(
            Some(movie_library_id.as_str()),
            &MediaFacet::Movie,
            "weaver"
        )
        .await
    );
    assert!(
        app.should_remove_failed_download(
            Some(movie_library_id.as_str()),
            &MediaFacet::Movie,
            "weaver"
        )
        .await
    );
}

#[tokio::test]
async fn library_settings_download_client_routing_override_normalizes_current_clients_and_hydrates_new_ones()
 {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let primary = create_enabled_download_client_config(&app, &user, "Primary", "weaver").await;
    let secondary =
        create_enabled_download_client_config(&app, &user, "Secondary", "sabnzbd").await;

    app.update_library_settings(
        &user,
        &movie_library_id,
        LibrarySettingsOverrideDraft {
            download_client_routing: Some(vec![DownloadClientRoutingSettingsEntry {
                client_id: primary.id.clone(),
                enabled: true,
                category: Some("movies".to_string()),
                recent_queue_priority: Some("high".to_string()),
                older_queue_priority: Some("low".to_string()),
                remove_completed: false,
                remove_failed: true,
            }]),
            ..empty_library_settings_override()
        },
    )
    .await
    .expect("library routing override should save");

    let raw = settings
        .get_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
        )
        .await
        .expect("saved library routing JSON");
    let parsed: serde_json::Value =
        serde_json::from_str(&raw).expect("saved library routing JSON should parse");
    assert_eq!(
        parsed[secondary.id.as_str()]["enabled"],
        serde_json::json!(false),
        "saving a library override should materialize current missing clients as disabled",
    );

    let tertiary = create_enabled_download_client_config(&app, &user, "Tertiary", "nzbget").await;
    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should reload");
    let routing = library_settings
        .download_client_routing_override
        .expect("library override should be present");

    assert_eq!(routing[0].client_id, primary.id);
    assert_eq!(routing[0].category.as_deref(), Some("movies"));
    assert_eq!(routing[0].recent_queue_priority.as_deref(), Some("high"));
    assert_eq!(routing[0].older_queue_priority.as_deref(), Some("low"));
    assert!(!routing[0].remove_completed);
    assert!(routing[0].remove_failed);

    let secondary_entry = routing
        .iter()
        .find(|entry| entry.client_id == secondary.id)
        .expect("secondary client should be present");
    assert!(!secondary_entry.enabled);
    assert_eq!(secondary_entry.category, None);
    assert_eq!(secondary_entry.recent_queue_priority, None);
    assert_eq!(secondary_entry.older_queue_priority, None);
    assert!(secondary_entry.remove_completed);
    assert!(!secondary_entry.remove_failed);

    let tertiary_entry = routing
        .iter()
        .find(|entry| entry.client_id == tertiary.id)
        .expect("newly added client should be hydrated as disabled");
    assert!(!tertiary_entry.enabled);
    assert_eq!(tertiary_entry.category, None);
    assert_eq!(tertiary_entry.recent_queue_priority, None);
    assert_eq!(tertiary_entry.older_queue_priority, None);
    assert!(tertiary_entry.remove_completed);
    assert!(!tertiary_entry.remove_failed);
}

#[tokio::test]
async fn library_settings_download_client_routing_override_reads_legacy_key_and_clears_it_when_reset()
 {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    let primary = create_enabled_download_client_config(&app, &user, "Primary", "weaver").await;
    let secondary =
        create_enabled_download_client_config(&app, &user, "Secondary", "sabnzbd").await;

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            &serde_json::json!({
                primary.id.as_str(): {
                    "enabled": true,
                    "category": "movies",
                    "recentQueuePriority": "high",
                    "olderQueuePriority": "low",
                    "removeCompleted": false,
                    "removeFailed": true
                }
            })
            .to_string(),
        )
        .await;

    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should read legacy routing override");
    let routing = library_settings
        .download_client_routing_override
        .expect("legacy routing override should be surfaced");

    let primary_entry = routing
        .iter()
        .find(|entry| entry.client_id == primary.id)
        .expect("primary client should be present");
    assert!(primary_entry.enabled);
    assert_eq!(primary_entry.category.as_deref(), Some("movies"));
    assert_eq!(primary_entry.recent_queue_priority.as_deref(), Some("high"));
    assert_eq!(primary_entry.older_queue_priority.as_deref(), Some("low"));
    assert!(!primary_entry.remove_completed);
    assert!(primary_entry.remove_failed);

    let secondary_entry = routing
        .iter()
        .find(|entry| entry.client_id == secondary.id)
        .expect("missing clients should hydrate as disabled");
    assert!(!secondary_entry.enabled);

    app.update_library_settings(&user, &movie_library_id, empty_library_settings_override())
        .await
        .expect("resetting library settings should succeed");

    assert!(
        settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
                &movie_library_id,
            )
            .await
            .is_none(),
        "resetting should remove the canonical library override",
    );
    assert!(
        settings
            .get_scoped_value(
                SETTINGS_SCOPE_SYSTEM,
                LEGACY_NZBGET_CLIENT_ROUTING_SETTINGS_KEY,
                &movie_library_id,
            )
            .await
            .is_none(),
        "resetting should remove the legacy library override too",
    );
}

#[tokio::test]
async fn library_settings_download_client_routing_override_ignores_invalid_json() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings.clone(), Arc::new(MockIndexerClient));
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);

    settings
        .set_scoped_value(
            SETTINGS_SCOPE_SYSTEM,
            DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY,
            &movie_library_id,
            "[]",
        )
        .await;

    let library_settings = app
        .get_library_settings(&user, &movie_library_id)
        .await
        .expect("library settings should load");

    assert!(
        library_settings.download_client_routing_override.is_none(),
        "invalid library routing JSON should be ignored instead of materialized as a disabled override",
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
                rate_limit_seconds: None,
                rate_limit_burst: None,
                is_enabled: true,
                enable_interactive_search: true,
                enable_auto_search: true,
                config_json: Some(
                    serde_json::json!({
                        "base_url": "https://api.nzbgeek.info",
                        "api_key": "0123456789abcdef"
                    })
                    .to_string(),
                ),
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
            api_key_encrypted: None,
            rate_limit_seconds: None,
            rate_limit_burst: None,
            disabled_until: None,
            is_enabled: true,
            enable_interactive_search: true,
            enable_auto_search: true,
            managed_parent_config_id: None,
            managed_child_key: None,
            managed_metadata_json: None,
            caps_snapshot_json: None,
            last_health_status: None,
            last_error_at: None,
            config_json: Some(
                serde_json::json!({
                    "base_url": "https://api.nzbgeek.info",
                    "api_key": "0123456789abcdef"
                })
                .to_string(),
            ),
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
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user(), titles)
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
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
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
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    download_client_configs
        .store
        .try_lock()
        .expect("download client config store should not be contended during bootstrap")
        .push(DownloadClientConfig {
            id: "background-search-default-client".to_string(),
            name: "Background Search Default Client".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 10_000,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let settings = Arc::new(StoredSettingsRepo::default());
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
    .with_download_submissions(download_submissions.clone())
    .with_pending_releases(pending_releases.clone())
    .with_blocklist_repo(Arc::new(MockBlocklistRepo::default()))
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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
    let app = app.with_test_overrides(|services| {
        services
            .with_acquisition_state(Arc::new(TrackingAcquisitionStateRepo {
                download_submissions,
                pending_releases,
                wanted_items: wanted_items.clone(),
            }))
            .with_wanted_items(wanted_items)
    });
    (app, test_admin_user())
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

fn bootstrap_with_library_delete_repositories(
    titles: Arc<MockTitleRepo>,
    settings: Arc<StoredSettingsRepo>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    domain_events: Arc<dyn DomainEventRepository>,
    housekeeping: Arc<dyn HousekeepingRepository>,
    pending_releases: Arc<dyn PendingReleaseRepository>,
) -> (AppUseCase, User) {
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    download_client_configs
        .store
        .try_lock()
        .expect("download client config store should not be contended during bootstrap")
        .push(DownloadClientConfig {
            id: "default-download-client".to_string(),
            name: "Default Download Client".to_string(),
            client_type: "nzbget".to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        });
    let release_attempts = Arc::new(MockReleaseAttemptRepo::default());
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
    let indexer_client = Arc::new(MockIndexerClient);
    let media_files = Arc::new(MockMediaFileRepo::default());

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
    .with_domain_events(domain_events)
    .with_metadata_gateway(Arc::new(EmptySearchMetadataGateway))
    .with_library_scanner(Arc::new(MutableLibraryScanner::default()))
    .with_media_files(media_files)
    .with_library_scan_unmatched_items(unmatched_items)
    .with_pending_releases(pending_releases)
    .with_housekeeping(housekeeping)
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user())
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
    .with_libraries(Arc::new(MockLibraryRepo::default()))
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

    (app, test_admin_user(), titles)
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
        library_id: scryer_domain::default_library_id_for_facet(&facet),
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
        folder_template: None,
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
        import_mode: None,
    }
}

fn empty_update_media_settings() -> UpdateMediaSettings {
    UpdateMediaSettings {
        library_path: None,
        root_folders: None,
        required_audio_languages: None,
        folder_template: None,
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
        import_mode: None,
    }
}

fn empty_library_settings_override() -> LibrarySettingsOverrideDraft {
    LibrarySettingsOverrideDraft {
        required_audio_languages: None,
        quality_profile_id: None,
        request_quality_profile_ids: None,
        scoring_persona: None,
        filler_policy: None,
        recap_policy: None,
        monitor_specials: None,
        inter_season_movies: None,
        monitor_filler_movies: None,
        nfo_write_on_import: None,
        plexmatch_write_on_import: None,
        import_mode: None,
        indexer_routing: None,
        download_client_routing: None,
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
async fn movie_full_scan_external_id_nfo_without_gateway_match_persists_unmatched_item() {
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
    let (app, user, _titles) = bootstrap_with_scan_unmatched_and_metadata_tracking_and_titles(
        settings,
        library_scanner,
        unmatched_items.clone(),
        Arc::new(EmptySearchMetadataGateway),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
    let summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("movie scan should continue");

    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.unmatched, 1);
    assert!(
        app.list_titles(&user, Some(MediaFacet::Movie), None, None)
            .await
            .expect("list titles")
            .is_empty()
    );

    let items = unmatched_items.items().await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].reason_code, "no_metadata_search_results");
    assert_eq!(items[0].error_message, None);
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
                auto_match_safe: true,
                auto_match_signals: vec!["exact_title".into(), "exact_year".into()],
            }],
        }),
    );
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");
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
        app.list_titles(&user, Some(MediaFacet::Movie), None, None)
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

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
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

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

    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
        None,
        Some(vec![
            LibraryRootDraft {
                path: root_one.to_string_lossy().to_string(),
                is_default: true,
            },
            LibraryRootDraft {
                path: root_two.to_string_lossy().to_string(),
                is_default: false,
            },
        ]),
        None,
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy series root");

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
    app.update_library(
        &user,
        &scryer_domain::default_library_id_for_facet(&MediaFacet::Series),
        None,
        Some(vec![LibraryRootDraft {
            path: series_root.to_string_lossy().to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("store series library roots");

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
            None,
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
            None,
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
            None,
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
    assert_eq!(
        series_items.items[0].title_slug,
        known_series_title.title.slug
    );
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
            None,
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
            None,
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
async fn update_media_settings_root_folders_sync_default_library_roots() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let root_one = tempdir.path().join("movies-a");
    let root_two = tempdir.path().join("movies-b");

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
    .expect("save movie roots");

    let library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            (root_one.to_string_lossy().to_string(), true),
            (root_two.to_string_lossy().to_string(), false),
        ]
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_backfills_legacy_root_folders_when_bootstrap() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let legacy_roots = vec![
        RootFolderEntry {
            path: "/mnt/anime-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/mnt/anime-archive".to_string(),
            is_default: false,
        },
    ];
    settings
        .set_value(
            SETTINGS_SCOPE_MEDIA,
            "anime.root_folders",
            &serde_json::to_string(&legacy_roots).expect("serialize legacy roots"),
        )
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let anime_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Anime);
    let anime_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&anime_library_id)
        .await
        .expect("library lookup")
        .expect("default anime library");
    app.services
        .catalog
        .libraries
        .update(
            &anime_library_id,
            anime_library.name.clone(),
            anime_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/data/anime".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed bootstrap root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Anime)
        .await
        .expect("anime settings");
    assert_eq!(media_settings.library_path, "/mnt/anime-main");
    assert_eq!(media_settings.root_folders, legacy_roots);
}

#[tokio::test]
async fn reconcile_default_library_roots_keeps_non_bootstrap_canonical_roots() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/legacy/movies")
        .await;

    let (app, user) = bootstrap_with_settings_repo_and_profiles(
        settings.clone(),
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let movie_library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup")
        .expect("default movie library");
    app.services
        .catalog
        .libraries
        .update(
            &movie_library_id,
            movie_library.name.clone(),
            movie_library.slug.clone(),
            vec![LibraryRootDraft {
                path: "/canonical/movies".to_string(),
                is_default: true,
            }],
        )
        .await
        .expect("seed canonical root");

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile roots");

    let paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(paths.movie_path, "/canonical/movies");
    assert_eq!(
        app.read_setting_string_value_for_scope_explicit(
            SETTINGS_SCOPE_MEDIA,
            "movies.path",
            None,
        )
        .await
        .expect("read mirror"),
        Some("/canonical/movies".to_string())
    );
}

#[tokio::test]
async fn reconcile_default_library_roots_repairs_missing_default_libraries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    app.reconcile_default_library_roots()
        .await
        .expect("reconcile missing defaults");

    let libraries = app
        .services
        .catalog
        .libraries
        .list(None)
        .await
        .expect("list repaired libraries");
    assert_eq!(libraries.len(), 3);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/data/movies");
    assert_eq!(library_paths.series_path, "/data/series");
    assert_eq!(library_paths.anime_path, "/data/anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/data/movies"),
        (MediaFacet::Series, "/data/series"),
        (MediaFacet::Anime, "/data/anime"),
    ] {
        let library = app
            .services
            .catalog
            .libraries
            .default_for_facet(facet.clone())
            .await
            .expect("lookup repaired library")
            .expect("default library should be recreated");
        assert_eq!(
            crate::settings::runtime::root_folder_entries_from_library_roots(&library.roots),
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn update_library_paths_repairs_missing_default_libraries_before_save() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::empty()),
    );

    let updated = app
        .update_library_paths(
            &user,
            UpdateLibraryPaths {
                movie_path: "/wizard-movies".to_string(),
                series_path: "/wizard-series".to_string(),
                anime_path: Some("/wizard-anime".to_string()),
            },
        )
        .await
        .expect("update repaired library paths");

    assert_eq!(updated.movie_path, "/wizard-movies");
    assert_eq!(updated.series_path, "/wizard-series");
    assert_eq!(updated.anime_path, "/wizard-anime");

    for (facet, expected_path) in [
        (MediaFacet::Movie, "/wizard-movies"),
        (MediaFacet::Series, "/wizard-series"),
        (MediaFacet::Anime, "/wizard-anime"),
    ] {
        let root_folders = app
            .root_folders_for_facet(&facet)
            .await
            .expect("repaired root folders");
        assert_eq!(
            root_folders,
            vec![RootFolderEntry {
                path: expected_path.to_string(),
                is_default: true,
            }]
        );
    }
}

#[tokio::test]
async fn find_or_create_default_user_dedupes_duplicate_default_library_grants() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let duplicate_movie_library = mock_default_library(MediaFacet::Movie);
    let libraries = vec![
        duplicate_movie_library.clone(),
        duplicate_movie_library,
        mock_default_library(MediaFacet::Series),
        mock_default_library(MediaFacet::Anime),
    ];
    let (app, user) = bootstrap_with_settings_repo_and_profiles_and_libraries(
        settings,
        Arc::new(MockQualityProfileRepo),
        Arc::new(MockIndexerClient),
        Arc::new(MockLibraryRepo::with_libraries(libraries)),
    );

    let admin = app
        .find_or_create_default_user()
        .await
        .expect("create default admin");
    assert_eq!(admin.username, user.username);

    let grants = app
        .services
        .catalog
        .libraries
        .permission_masks_for_user(&admin.id)
        .await
        .expect("load grants");
    let unique_library_ids = grants
        .iter()
        .map(|grant| grant.library_id.clone())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(grants.len(), 3);
    assert_eq!(unique_library_ids.len(), 3);
}

#[tokio::test]
async fn update_default_library_roots_updates_all_facet_root_read_paths() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    let expected_roots = vec![
        RootFolderEntry {
            path: "/library/movies-main".to_string(),
            is_default: true,
        },
        RootFolderEntry {
            path: "/library/movies-archive".to_string(),
            is_default: false,
        },
    ];

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(
            expected_roots
                .iter()
                .map(|root| LibraryRootDraft {
                    path: root.path.clone(),
                    is_default: root.is_default,
                })
                .collect(),
        ),
        None,
    )
    .await
    .expect("update canonical roots");

    let media_settings = app
        .get_media_settings(&user, MediaFacet::Movie)
        .await
        .expect("movie settings");
    assert_eq!(media_settings.library_path, "/library/movies-main");
    assert_eq!(media_settings.root_folders, expected_roots);

    let library_paths = app.get_library_paths(&user).await.expect("library paths");
    assert_eq!(library_paths.movie_path, "/library/movies-main");

    let root_folders = app
        .root_folders_for_facet(&MediaFacet::Movie)
        .await
        .expect("root folders");
    assert_eq!(root_folders, media_settings.root_folders);
}

#[tokio::test]
async fn title_root_resolution_uses_owning_library_roots() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/default-movies".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("default movie library roots should update");
    let kids_library = app
        .create_library(
            &user,
            MediaFacet::Movie,
            "Kids Movies".to_string(),
            vec![LibraryRootDraft {
                path: "/library/kids-movies".to_string(),
                is_default: true,
            }],
            None,
        )
        .await
        .expect("custom library should be created");
    let mut title = make_due_hydration_title("custom-library-title", MediaFacet::Movie, 42);
    title.library_id = kids_library.id.clone();

    let import_paths = crate::import_workflow::resolve_import_paths(&app, &title)
        .await
        .expect("import paths should resolve");
    assert_eq!(import_paths.media_root, "/library/kids-movies");

    let recycle_root = crate::recycle_bin::media_root_for_title(&app, &title).await;
    assert_eq!(recycle_root.as_deref(), Some("/library/kids-movies"));
}

#[tokio::test]
async fn update_default_library_rejects_empty_roots_without_persisting_them() {
    let (app, user) = bootstrap();
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/library/movies-main".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");

    let error = app
        .update_library(&user, &movie_library_id, None, Some(Vec::new()), None)
        .await
        .expect_err("empty default roots should be rejected");
    assert!(
        matches!(error, AppError::Validation(ref message) if message.contains("default libraries require at least one root folder")),
        "unexpected error: {error:?}"
    );

    let library = app
        .services
        .catalog
        .libraries
        .get_by_id(&movie_library_id)
        .await
        .expect("library lookup should succeed")
        .expect("movie library should exist");
    assert_eq!(library.roots.len(), 1);
    assert_eq!(library.roots[0].path, "/library/movies-main");
}

#[tokio::test]
async fn update_library_removing_root_clears_pending_imports_for_removed_root() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        Arc::new(MutableLibraryScanner::default()),
        unmatched_items.clone(),
    );
    let movie_library_id = scryer_domain::default_library_id_for_facet(&MediaFacet::Movie);
    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-old".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("initial default roots should update");
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "movie-old-root-canonical",
            MediaFacet::Movie,
            "/movies-old",
            "/movies-old/Unknown.Movie.2020.mkv",
            "Unknown Movie",
            "Unknown Movie",
            Some(2020),
        ))
        .await
        .expect("seed removed-root pending import");

    app.update_library(
        &user,
        &movie_library_id,
        None,
        Some(vec![LibraryRootDraft {
            path: "/movies-new".to_string(),
            is_default: true,
        }]),
        None,
    )
    .await
    .expect("canonical roots should update");

    let items = unmatched_items.items().await;
    assert!(items.is_empty());
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy roots");

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

    let movie_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Movie)
        .await
        .expect("lookup should succeed")
        .expect("default movie library");
    assert_eq!(
        movie_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/movies-primary".to_string(), true),
            ("/movies-secondary".to_string(), false),
        ]
    );

    let series_library = app
        .services
        .catalog
        .libraries
        .default_for_facet(MediaFacet::Series)
        .await
        .expect("lookup should succeed")
        .expect("default series library");
    assert_eq!(
        series_library
            .roots
            .iter()
            .map(|root| (root.path.clone(), root.is_default))
            .collect::<Vec<_>>(),
        vec![
            ("/series-main".to_string(), true),
            ("/series-archive".to_string(), false),
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
        app.list_titles(&user, Some(MediaFacet::Movie), None, None)
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
                name: "Glass Harbor".to_string(),
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
        .list_titles(&user, Some(MediaFacet::Movie), None, None)
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
    app.reconcile_default_library_roots()
        .await
        .expect("reconcile legacy movie root");

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
        .list_titles(&user, Some(MediaFacet::Movie), None, None)
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        .trigger_title_wanted_search(&user, &title.id, SubmissionConflictPolicy::Abort)
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
        .trigger_title_wanted_search(&user, &title.id, SubmissionConflictPolicy::Abort)
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
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
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
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
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
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Series), None)
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
        .list_cutoff_unmet_titles(&user, Some(MediaFacet::Movie), None)
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
        .list_titles(&user, Some(MediaFacet::Series), None, None)
        .await
        .expect("list titles");

    assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
}

#[tokio::test]
async fn search_indexers_for_title_keeps_direct_nab_searches_uncategorized_when_routing_is_empty() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingCategoriesIndexerClient::new(
        "Generic.Release.2026.1080p.WEB-DL",
    ));
    let (app, user) =
        bootstrap_with_search_settings_and_indexer(settings, recording_client.clone());

    let movie = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Movie".into(),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2026),
                ..Default::default()
            },
        )
        .await
        .expect("create movie title");
    let series = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Series".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");
    let anime = app
        .add_title(
            &user,
            NewTitle {
                name: "Default Category Anime".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    app.search_indexers_for_title(&user, movie.id.clone())
        .await
        .expect("movie search should succeed");
    app.search_indexers_for_title(&user, series.id.clone())
        .await
        .expect("series search should succeed");
    app.search_indexers_for_title(&user, anime.id.clone())
        .await
        .expect("anime search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].newznab_categories, None);
    assert_eq!(calls[1].newznab_categories, None);
    assert_eq!(calls[2].newznab_categories, None);
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_series_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-series", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Signal".into(),
                facet: MediaFacet::Series,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create series title");

    let season = app
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
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: None,
            overview: None,
            tvdb_id: Some("tvdb-series-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create series episode");

    app.search_indexers_for_episode(&user, title.id.clone(), "2".to_string(), "11".to_string())
        .await
        .expect("series episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Signal S02E11".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: None,
        }]
    );
}

#[tokio::test]
async fn search_indexers_for_episode_dedupes_equivalent_structured_anime_queries() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let recording_client = Arc::new(RecordingStructuredQueryIndexerClient::default());
    let (app, user) = bootstrap_with_search_settings_indexer_and_configs(
        settings,
        recording_client.clone(),
        vec![synthetic_direct_nab_indexer_config("idx-anime", "nzbgeek")],
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Synthetic Atlas".into(),
                facet: MediaFacet::Anime,
                monitored: true,
                ..Default::default()
            },
        )
        .await
        .expect("create anime title");

    let season = app
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
            first_episode_number: Some("11".to_string()),
            last_episode_number: Some("11".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime season");

    app.services
        .catalog
        .shows
        .create_episode(Episode {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_id: Some(season.id.clone()),
            episode_type: scryer_domain::EpisodeType::Standard,
            episode_number: Some("11".to_string()),
            season_number: Some("2".to_string()),
            episode_label: Some("S02E11".to_string()),
            title: Some("Episode 11".to_string()),
            air_date: Some("2026-01-01".to_string()),
            duration_seconds: Some(1_500),
            has_multi_audio: false,
            has_subtitle: false,
            is_filler: false,
            is_recap: false,
            absolute_number: Some("35".to_string()),
            overview: None,
            tvdb_id: Some("tvdb-anime-211".to_string()),
            image_url: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create anime episode");

    app.search_indexers_for_episode(&user, title.id.clone(), "2".to_string(), "11".to_string())
        .await
        .expect("anime episode search should succeed");

    let calls = recording_client.calls.lock().await.clone();
    assert_eq!(
        calls,
        vec![RecordedStructuredQueryCall {
            query: "Synthetic Atlas 035".to_string(),
            season: Some(2),
            episode: Some(11),
            absolute_episode: Some(35),
        }]
    );
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
        "Nightfall.Heavy.Metal.Dark.Fantasy.S01E01.1080p.NF.WEB-DL",
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

    let search_user = create_user_with_permissions(
        &app,
        &user,
        "title_search_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create search user");
    let search_token = app
        .issue_access_token(&search_user)
        .await
        .expect("issue search token");
    let authed_search_user = app
        .authenticate_token(&search_token)
        .await
        .expect("authenticate search user");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Nightfall!!".into(),
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
                    name: "Nightfall Heavy Metal Dark Fantasy".to_string(),
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

    let mut ghost_actor = User {
        id: "ghost-search-user".to_string(),
        username: "ghost".to_string(),
        password_hash: None,
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    ghost_actor.authorization = scryer_domain::UserAuthorization {
        default_library: scryer_domain::LibraryPermissionMask::from_permissions([
            scryer_domain::LibraryPermission::View,
            scryer_domain::LibraryPermission::ManageTitles,
        ]),
        loaded: true,
        ..Default::default()
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

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    )
    .await
    .expect("create user");

    let users = app.list_users(&user).await.expect("list users");
    assert!(users.iter().any(|entry| entry.username == created.username));
    assert_eq!(users.len(), 1);
}

#[tokio::test]
async fn create_user_without_permission_grants_allows_manage_users_only_actor() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let created = app
        .create_user(
            &actor,
            "plain-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            Vec::new(),
        )
        .await
        .expect("create user without grants");

    assert_eq!(created.username, "plain-user");
}

#[tokio::test]
async fn create_user_with_app_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "privileged-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::MANAGE_SYSTEM_SETTINGS,
            Vec::new(),
        )
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn create_user_with_library_permission_grants_requires_manage_permissions() {
    let (app, _) = bootstrap();
    let actor = test_user_with_app_permissions("user-admin", AppPermissionMask::MANAGE_USERS);

    let result = app
        .create_user(
            &actor,
            "library-user".to_string(),
            "password123".to_string(),
            AppPermissionMask::NONE,
            vec![scryer_domain::LibraryGrant {
                user_id: String::new(),
                library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Movie),
                permissions: scryer_domain::LibraryPermissionMask::VIEW,
            }],
        )
        .await;

    assert!(matches!(result, Err(AppError::Unauthorized(_))));
}

#[tokio::test]
async fn get_user_by_id_returns_created_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "viewer",
        "password123",
        vec![TestPermissionPreset::CatalogView],
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

    let _created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("first create");

    let second = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
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
        .list_titles(&user, Some(MediaFacet::Movie), None, None)
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

    app.services
        .catalog
        .titles
        .create(make_due_hydration_title("title-1", MediaFacet::Series, 1))
        .await
        .expect("seed title");

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

async fn create_enabled_download_client_config(
    app: &AppUseCase,
    user: &User,
    name: &str,
    client_type: &str,
) -> DownloadClientConfig {
    app.create_download_client_config(
        user,
        NewDownloadClientConfig {
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            client_priority: 1,
            is_enabled: true,
        },
    )
    .await
    .expect("create download client config")
}

async fn seed_download_client_config(
    app: &AppUseCase,
    id: &str,
    name: &str,
    client_type: &str,
) -> DownloadClientConfig {
    app.services
        .integrations
        .download_client_configs
        .create(DownloadClientConfig {
            id: id.to_string(),
            name: name.to_string(),
            client_type: client_type.to_string(),
            config_json: "{}".to_string(),
            is_enabled: true,
            status: scryer_domain::DownloadClientStatus::Healthy,
            last_error: None,
            last_seen_at: None,
            client_priority: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .expect("seed download client config")
}

async fn set_download_client_cleanup_routing(
    app: &AppUseCase,
    user: &User,
    facet: &str,
    client_id: &str,
    remove_completed: bool,
    remove_failed: bool,
) {
    app.update_download_client_routing(
        user,
        facet,
        vec![DownloadClientRoutingSettingsEntry {
            client_id: client_id.to_string(),
            enabled: true,
            category: None,
            recent_queue_priority: None,
            older_queue_priority: None,
            remove_completed,
            remove_failed,
        }],
    )
    .await
    .expect("update download client routing");
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
async fn find_download_queue_scope_ignores_stale_submission_titles() {
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

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Scope Regression Movie".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create visible title");

    let mut blocked = queue_history_fixture_item("blocked-1", DownloadQueueState::Completed, 20);
    blocked.title_id = Some(title.id.clone());
    blocked.title_name = title.name.clone();
    blocked.facet = Some("movie".to_string());
    blocked.tracked_state = Some(TrackedDownloadState::ImportBlocked);
    *download_client.history_items.lock().await = vec![blocked];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: "missing-title".to_string(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "blocked-1".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Fixture blocked-1".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record stale submission");

    let scope = app
        .find_download_queue_scope(&user, Some("primary"), "nzbget", "blocked-1")
        .await
        .expect("stale scope lookup should not fail");
    assert!(scope.is_none());

    let page = app
        .list_download_import_page(&user, 50, 0, DownloadImportFilter::All)
        .await
        .expect("download import page should still load");
    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "blocked-1");
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

    let history_item = queue_history_fixture_item("completed-1", DownloadQueueState::Completed, 40);
    *download_client.history_items.lock().await = vec![history_item.clone()];

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
                client_item: history_item,
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
async fn download_queue_poller_retries_imported_cleanup_from_facet_routing_until_delete_succeeds() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Imported Cleanup Retry".to_string(),
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

    let item_id = "imported-cleanup-1";
    let tracked_id =
        crate::tracked_downloads::tracked_download_id(Some(config.id.as_str()), "nzbget", item_id);
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    *download_client.history_items.lock().await = vec![history_item];

    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some(config.id.as_str()), "nzbget", item_id),
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("seed imported tracked state");

    download_client
        .set_delete_error(Some("repository: delete failed"))
        .await;

    let (_tx, tracked_download_rx) = tokio::sync::mpsc::channel(8);
    let token = tokio_util::sync::CancellationToken::new();
    let poller = tokio::spawn(crate::integration::start_download_queue_poller(
        app.clone(),
        token.child_token(),
        tracked_download_rx,
    ));

    timeout(Duration::from_secs(5), async {
        loop {
            if app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .contains_key(&tracked_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should stay visible after retryable delete failure");
    assert!(download_client.deleted_items.lock().await.is_empty());

    let mut pushed_out_history = (0..105)
        .map(|index| {
            let mut item = queue_history_fixture_item(
                &format!("recent-history-{index}"),
                DownloadQueueState::Completed,
                1_000 - index as i64,
            );
            item.client_id = config.id.clone();
            item.client_name = config.name.clone();
            item
        })
        .collect::<Vec<_>>();
    let mut hidden_target = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 1);
    hidden_target.client_id = config.id.clone();
    hidden_target.client_name = config.name.clone();
    hidden_target.title_id = Some(title.id.clone());
    hidden_target.title_name = title.name.clone();
    hidden_target.facet = Some("movie".to_string());
    pushed_out_history.push(hidden_target);
    *download_client.history_items.lock().await = pushed_out_history;

    download_client.set_delete_error(None).await;

    timeout(Duration::from_secs(5), async {
        loop {
            if !download_client.deleted_requests.lock().await.is_empty() {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("poller should retry imported cleanup on the next cycle");

    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if !app
                .runtime
                .acquisition
                .tracked_download_snapshot
                .read()
                .await
                .contains_key(&tracked_id)
            {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("tracked imported item should disappear once cleanup succeeds");

    token.cancel();
    poller
        .await
        .expect("download queue poller should stop cleanly");
}

#[tokio::test]
async fn failed_tracked_cleanup_uses_facet_routing_and_exact_client_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );

    let config =
        create_enabled_download_client_config(&app, &user, "Series NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, false, true).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Failed Cleanup".to_string(),
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

    let item_id = "failed-cleanup-1";
    let mut history_item = queue_history_fixture_item(item_id, DownloadQueueState::Failed, 40);
    history_item.client_id = config.id.clone();
    history_item.client_name = config.name.clone();
    history_item.title_id = Some(title.id.clone());
    history_item.title_name = title.name.clone();
    history_item.facet = Some("movie".to_string());
    let tracked = crate::tracked_downloads::TrackedDownload {
        id: crate::tracked_downloads::tracked_download_id(
            Some(config.id.as_str()),
            "nzbget",
            item_id,
        ),
        client_id: config.id.clone(),
        client_type: "nzbget".to_string(),
        client_item: history_item,
        state: TrackedDownloadState::Failed,
        status: scryer_domain::TrackedDownloadStatus::Ok,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("movie".to_string()),
        source_title: Some(title.name.clone()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: true,
        path_missing_since: None,
        skip_reacquire_on_failure: false,
    };

    let outcome = crate::import::import::reconcile_terminal_download_cleanup_for_tracked(
        &app,
        &tracked,
        TrackedDownloadState::Failed,
    )
    .await;

    assert_eq!(
        outcome,
        crate::import::import::TerminalDownloadCleanupOutcome::Removed
    );
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_removes_already_imported_history_with_exact_client_id() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        create_enabled_download_client_config(&app, &user, "Primary NZBGet", "nzbget").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Legacy Cleanup".to_string(),
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

    let now = Utc::now().to_rfc3339();
    let item_id = "legacy-completed-1";
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(config.id.clone()),
        source_system: "nzbget".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Legacy.Cleanup.2026.1080p.WEB-DL",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    *download_client.completed_downloads.lock().await = vec![completed];

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_leaves_already_imported_item_unprocessed_when_completed_download_is_missing()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions,
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let now = Utc::now().to_rfc3339();
    let item_id = "legacy-missing-completed-1";
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some("client-1".to_string()),
        source_system: "nzbget".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(!processed.contains(item_id));
    assert!(download_client.deleted_requests.lock().await.is_empty());
}

#[tokio::test]
async fn try_import_completed_downloads_uses_download_submission_fallback_for_untagged_qbittorrent_history()
 {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (base_app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );
    let import_repo = Arc::new(TrackingImportRepo::default());
    let app = base_app.with_test_overrides(|services| services.with_imports(import_repo.clone()));

    let config =
        seed_download_client_config(&app, "decypharr-qbit-cleanup", "Decypharr", "qbittorrent")
            .await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Decypharr Cleanup".to_string(),
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

    let now = Utc::now().to_rfc3339();
    let item_id = "decypharr-untagged-cleanup-1";
    import_repo.records.lock().await.push(ImportRecord {
        id: Id::new().0,
        source_client_id: Some(config.id.clone()),
        source_system: "qbittorrent".to_string(),
        source_ref: item_id.to_string(),
        import_type: ImportType::MovieDownload,
        status: ImportStatus::Completed,
        payload_json: String::new(),
        result_json: None,
        started_at: Some(now.clone()),
        finished_at: Some(now.clone()),
        created_at: now.clone(),
        updated_at: now,
    });

    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "qbittorrent".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "qbittorrent".to_string();
    completed.category = Some("radarr".to_string());
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some(title.name.clone()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
}

#[tokio::test]
async fn try_import_completed_downloads_retries_terminal_cleanup_for_untagged_qbittorrent_history()
{
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
    );

    let config =
        seed_download_client_config(&app, "decypharr-qbit-retry", "Decypharr", "qbittorrent").await;
    set_download_client_cleanup_routing(&app, &user, "movie", &config.id, true, false).await;

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Decypharr Retry".to_string(),
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

    let item_id = "decypharr-untagged-retry-1";
    let mut item = queue_history_fixture_item(item_id, DownloadQueueState::Completed, 40);
    item.client_id = config.id.clone();
    item.client_name = config.name.clone();
    item.client_type = "qbittorrent".to_string();
    item.title_id = Some(title.id.clone());
    item.title_name = title.name.clone();
    item.facet = Some("movie".to_string());
    item.is_scryer_origin = false;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut completed = completed_download_fixture_item(
        item_id,
        &title.id,
        "Harry.Potter.and.the.Prisoner.of.Azkaban.2004.BluRay.1080p.AV1.Opus-nAV1gator",
        dir.path().to_string_lossy().as_ref(),
    );
    completed.client_id = config.id.clone();
    completed.client_type = "qbittorrent".to_string();
    completed.category = Some("radarr".to_string());
    completed.parameters.clear();
    *download_client.completed_downloads.lock().await = vec![completed];

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some(config.id.clone()),
            download_client_type: "qbittorrent".to_string(),
            download_client_item_id: item_id.to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some(title.name.clone()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("seed download submission");
    download_submissions
        .update_tracked_state(
            &DownloadSourceIdentity::new(Some(config.id.as_str()), "qbittorrent", item_id),
            TrackedDownloadState::Imported.as_str(),
        )
        .await
        .expect("seed tracked state");

    let processed =
        crate::import::import::try_import_completed_downloads(&app, &user, &[item]).await;

    assert!(processed.contains(item_id));
    assert_eq!(
        download_client.deleted_requests.lock().await.clone(),
        vec![(Some(config.id.clone()), None, item_id.to_string(), true)]
    );
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
async fn list_download_history_page_includes_tracked_terminal_rows_when_client_history_is_empty() {
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

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Tracked History Fixture".to_string(),
                facet: MediaFacet::Movie,
                monitored: true,
                tags: vec![],
                external_ids: vec![],
                min_availability: None,
                poster_url: None,
                year: Some(2012),
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

    let mut tracked_history_item =
        queue_history_fixture_item("tracked-terminal-1", DownloadQueueState::Completed, 50);
    tracked_history_item.client_id = "primary".to_string();
    tracked_history_item.client_name = "NZBGet".to_string();
    tracked_history_item.client_type = "nzbget".to_string();
    tracked_history_item.title_id = Some(title.id.clone());
    tracked_history_item.title_name = "Paper Lantern".to_string();

    let tracked_id = crate::tracked_downloads::tracked_download_id(
        Some("primary"),
        "nzbget",
        "tracked-terminal-1",
    );
    app.runtime
        .acquisition
        .tracked_download_snapshot
        .write()
        .await
        .insert(
            tracked_id,
            crate::tracked_downloads::TrackedDownloadQueueMetadata {
                client_item: tracked_history_item,
                title_id: Some(title.id.clone()),
                facet: Some("movie".to_string()),
                source_title: Some("Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb".to_string()),
                state: TrackedDownloadState::Imported,
                status: scryer_domain::TrackedDownloadStatus::Ok,
                status_messages: Vec::new(),
                match_type: scryer_domain::TitleMatchType::Submission,
            },
        );

    let page = app
        .list_download_history_page(
            &user,
            50,
            0,
            Some(vec![DownloadHistoryFilter::All]),
            None,
            false,
            None,
        )
        .await
        .expect("tracked terminal history page should load");

    assert_eq!(page.total_count, 1);
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].download_client_item_id, "tracked-terminal-1");
    assert_eq!(
        page.items[0].tracked_state,
        Some(TrackedDownloadState::Imported)
    );
    assert_eq!(page.items[0].import_status, Some(ImportStatus::Completed));
    assert_eq!(
        page.items[0].title_name,
        "Paper.Lantern.2012.720p.WEB-DL.AV1.AAC2.0-NTb"
    );
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
        .await
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == "failed-job"
            && submission.source_title.as_deref() == Some("Failed.Release.1080p.WEB-DL")
    }));
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        skip_reacquire_on_failure: false,
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
    assert!(submissions.iter().any(|submission| {
        submission.download_client_item_id == "failed-job"
            && submission.source_title.as_deref() == Some("Failed.Release.1080p.WEB-DL")
    }));
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-job",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
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
async fn process_download_failure_returns_already_handled_for_duplicate_failed_download() {
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
                name: "Duplicate Failed Download".into(),
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
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: Some((Utc::now() + chrono::Duration::hours(6)).to_rfc3339()),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(
            (Utc::now() - chrono::Duration::days(14))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        status: WantedStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Duplicate.Failed.Release.1080p.WEB-DL",
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
    let wanted_id = wanted.id.clone();

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-duplicate".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Duplicate.Failed.Release.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted.clone()),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-duplicate".to_string(),
            release_title: "Duplicate.Failed.Release.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_ne!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let second = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-duplicate".to_string(),
            release_title: "Duplicate.Failed.Release.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        second,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    assert_eq!(
        wanted_items.status_update_call_count_for(&wanted_id).await,
        1,
        "duplicate failure should not reschedule the wanted item twice"
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].download_id.as_deref(),
        Some("failed-duplicate")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);

    let history = app
        .list_title_history(
            &user,
            &TitleHistoryFilter {
                event_types: Some(vec![
                    TitleHistoryEventType::DownloadFailed,
                    TitleHistoryEventType::Blocklisted,
                ]),
                title_ids: Some(vec![title.id.clone()]),
                library_ids: None,
                title_search: None,
                download_id: Some("failed-duplicate".to_string()),
                episode_id: None,
                group_by_event: false,
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("list title history");
    assert_eq!(history.total_count, 2);
}

#[tokio::test]
async fn process_download_failure_skip_reacquire_records_failure_without_due_search() {
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
                name: "Manual Failed Only".into(),
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
        library_id: None,
        library_name: None,
        library_slug: None,
        episode_id: None,
        collection_id: None,
        season_number: None,
        episode_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: Some(Utc::now().to_rfc3339()),
        last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
        search_count: 1,
        baseline_date: Some(
            (Utc::now() - chrono::Duration::days(14))
                .format("%Y-%m-%d")
                .to_string(),
        ),
        status: WantedStatus::Grabbed,
        grabbed_release: Some(
            serde_json::json!({
                "title": "Manual.Failed.Only.1080p.WEB-DL",
                "score": 100,
                "grabbed_at": Utc::now().to_rfc3339(),
            })
            .to_string(),
        ),
        current_score: Some(100),
        latest_release_decision: None,
        mismatch_recovery_eligible: false,
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    wanted_items
        .upsert_wanted_item(&wanted)
        .await
        .expect("seed wanted item");

    download_submissions
        .record_submission(DownloadSubmission {
            title_id: title.id.clone(),
            facet: "movie".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-only".to_string(),
            source_hint: None,
            source_kind: None,
            source_title: Some("Manual.Failed.Only.1080p.WEB-DL".to_string()),
            request_signature: None,
            scope: SubmissionScope::Title,
        })
        .await
        .expect("record failed submission");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(wanted.clone()),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-only".to_string(),
            release_title: "Manual.Failed.Only.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: true,
        },
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RecordedNoReacquire
    );

    let updated_wanted = wanted_items
        .get_wanted_item_by_id(&wanted.id)
        .await
        .expect("get wanted")
        .expect("wanted item");
    assert_eq!(updated_wanted.status, WantedStatus::Wanted);
    assert!(updated_wanted.next_search_at.is_none());
    assert!(updated_wanted.grabbed_release.is_none());

    let due = wanted_items
        .list_due_wanted_items(&Utc::now().to_rfc3339(), 10, &[])
        .await
        .expect("list due wanted");
    assert!(due.is_empty());

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(blocklist[0].download_id.as_deref(), Some("failed-only"));
}

#[tokio::test]
async fn process_download_failure_dedupes_same_release_title_across_client_item_ids() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
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

    for (item_id, source_hint, source_title) in [
        (
            "weaver-1",
            "weaver://job/weaver-1",
            "Friends.S05.720p.BluRay.DD5.1.x264-NTb",
        ),
        (
            "weaver-2",
            "weaver://job/weaver-2",
            " friends.s05.720p.bluray.dd5.1.x264-ntb ",
        ),
    ] {
        download_submissions
            .record_submission(DownloadSubmission {
                title_id: title.id.clone(),
                facet: "series".to_string(),
                download_client_id: Some("primary".to_string()),
                download_client_type: "weaver".to_string(),
                download_client_item_id: item_id.to_string(),
                source_hint: Some(source_hint.to_string()),
                source_kind: None,
                source_title: Some(source_title.to_string()),
                request_signature: None,
                scope: SubmissionScope::Title,
            })
            .await
            .expect("record failed submission");
    }

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-1".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_ne!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let second = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: None,
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-2".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        second,
        crate::acquisition_workflow::FailureHandlingOutcome::AlreadyHandled
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);
}

#[tokio::test]
async fn tracked_download_failure_prefers_tracked_source_title_for_blocklist_identity() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let (app, user) = bootstrap_with_acquisition_tracking(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
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

    let mut tracked_download = crate::tracked_downloads::TrackedDownload {
        id: "weaver:job-1".to_string(),
        client_id: "primary".to_string(),
        client_type: "weaver".to_string(),
        client_item: failed_history_item("job-1", "Friends"),
        state: scryer_domain::TrackedDownloadState::FailedPending,
        status: scryer_domain::TrackedDownloadStatus::Error,
        status_messages: Vec::new(),
        title_id: Some(title.id.clone()),
        facet: Some("series".to_string()),
        source_title: Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
        indexer: None,
        added_at: None,
        notified_manual_interaction: false,
        match_type: scryer_domain::TitleMatchType::Submission,
        is_trackable: true,
        import_attempted: false,
        path_missing_since: None,
        skip_reacquire_on_failure: false,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);
    assert_eq!(
        failed_attempts[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );
}

#[tokio::test]
async fn season_pack_failure_processed_twice_only_requeues_once_and_blocklists_once() {
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
                image_url: None,
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
            library_id: None,
            library_name: None,
            library_slug: None,
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

    let grabbed_wanted = wanted_items
        .get_wanted_item_by_id(
            expected_wanted_ids
                .first()
                .expect("expected wanted ids should contain seeded episodes"),
        )
        .await
        .expect("get grabbed wanted")
        .expect("grabbed wanted should exist");

    let first = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(grabbed_wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "nzbget".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "failed-season-pack".to_string(),
            release_title: "Season.Pack.Failure.Recovery.S07.1080p.WEB-DL".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;
    assert_eq!(
        first,
        crate::acquisition_workflow::FailureHandlingOutcome::RequeuedFreshSearch
    );

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
        skip_reacquire_on_failure: false,
    };

    crate::failed_download_handler::process_failed(&app, &mut tracked_download).await;

    assert_eq!(
        tracked_download.state,
        scryer_domain::TrackedDownloadState::Failed
    );

    for wanted_id in &expected_wanted_ids {
        let wanted = wanted_items
            .get_wanted_item_by_id(wanted_id)
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
        assert_eq!(
            wanted_items.status_update_call_count_for(wanted_id).await,
            1,
            "duplicate season-pack failure should only requeue each episode once"
        );
    }

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].download_id.as_deref(),
        Some("failed-season-pack")
    );

    let failed_attempts = app
        .services
        .workflow
        .release_attempts
        .list_failed_release_signatures_for_title(&title.id, 10)
        .await
        .expect("list failed release attempts");
    assert_eq!(failed_attempts.len(), 1);

    let history = app
        .list_title_history(
            &user,
            &TitleHistoryFilter {
                event_types: Some(vec![
                    TitleHistoryEventType::DownloadFailed,
                    TitleHistoryEventType::Blocklisted,
                ]),
                title_ids: Some(vec![title.id.clone()]),
                library_ids: None,
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
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| {
                submission.download_client_item_id == "failed-season-pack"
                    && submission.source_title.as_deref()
                        == Some("Season.Pack.Failure.Recovery.S07.1080p.WEB-DL")
            })
    );
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "failed-season-pack",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
                image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
        download_submissions
            .store
            .lock()
            .await
            .iter()
            .any(|submission| {
                submission.download_client_item_id == "shared-failed-season-pack"
                    && submission.source_title.as_deref() == Some(pack_title)
            })
    );
    assert_eq!(
        download_submissions
            .get_tracked_state(&DownloadSourceIdentity::new(
                Some("primary"),
                "nzbget",
                "shared-failed-season-pack",
            ))
            .await
            .expect("load tracked state")
            .as_deref(),
        Some("failed")
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
            image_url: None,
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
            image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
                image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
                    let base_query = query.strip_suffix(&season_token).unwrap_or(query.as_str());
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
                    episode_metadata.release_type = crate::ParsedEpisodeReleaseType::SeasonPack;
                    parsed.episode = Some(crate::ParsedEpisodeMetadata { ..episode_metadata });
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
                name: "Emberfall".into(),
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
                image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
            "Emberfall S01E01.1080p.WEB-DL".to_string(),
            "Emberfall S01E02.1080p.WEB-DL".to_string(),
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
            Some("1") => "Emberfall S01E01.1080p.WEB-DL",
            Some("2") => "Emberfall S01E02.1080p.WEB-DL",
            other => panic!("unexpected episode number: {other:?}"),
        };
        assert_eq!(grabbed_release["title"].as_str(), Some(expected_title));
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
                image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
async fn acquisition_cycle_skips_recently_failed_season_pack_from_submission_release_title() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client.clone(),
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Friends".into(),
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

    let season = app
        .services
        .catalog
        .shows
        .create_collection(Collection {
            id: Id::new().0,
            title_id: title.id.clone(),
            collection_type: CollectionType::Season,
            collection_index: "5".to_string(),
            label: Some("Season 5".to_string()),
            ordered_path: None,
            narrative_order: Some("5".to_string()),
            first_episode_number: Some("01".to_string()),
            last_episode_number: Some("02".to_string()),
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: true,
            created_at: Utc::now(),
        })
        .await
        .expect("create season");

    let mut expected_wanted_ids = Vec::new();
    for (episode_number, label) in [("01", "S05E01"), ("02", "S05E02")] {
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
                season_number: Some("5".to_string()),
                episode_label: Some(label.to_string()),
                title: Some(label.to_string()),
                air_date: Some("1998-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
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
            library_id: None,
            library_name: None,
            library_slug: None,
            episode_id: Some(episode.id.clone()),
            collection_id: None,
            season_number: Some("5".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: Some((Utc::now() - chrono::Duration::minutes(10)).to_rfc3339()),
            search_count: 1,
            baseline_date: Some("1998-01-01".to_string()),
            status: WantedStatus::Grabbed,
            grabbed_release: Some(
                serde_json::json!({
                    "title": "Friends.S05.720p.BluRay.DD5.1.x264-NTb",
                    "score": 100,
                    "grabbed_at": Utc::now().to_rfc3339(),
                    "season_pack": true,
                })
                .to_string(),
            ),
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
            facet: "series".to_string(),
            download_client_id: Some("primary".to_string()),
            download_client_type: "weaver".to_string(),
            download_client_item_id: "weaver-season-pack-1".to_string(),
            source_hint: Some("weaver://job/weaver-season-pack-1".to_string()),
            source_kind: None,
            source_title: Some("Friends.S05.720p.BluRay.DD5.1.x264-NTb".to_string()),
            request_signature: Some(
                "nzb_url|https://example.com/friends-s05.nzb|Friends.S05.720p.BluRay.DD5.1.x264-NTb"
                    .to_string(),
            ),
            scope: SubmissionScope::Collection {
                collection_id: season.id.clone(),
            },
        })
        .await
        .expect("record failed season pack submission");

    let grabbed_wanted = wanted_items
        .get_wanted_item_by_id(
            expected_wanted_ids
                .first()
                .expect("expected wanted ids should contain seeded episodes"),
        )
        .await
        .expect("get grabbed wanted")
        .expect("grabbed wanted should exist");

    let outcome = crate::acquisition_workflow::process_download_failure(
        &app,
        crate::acquisition_workflow::DownloadFailureContext {
            wanted_item: Some(grabbed_wanted),
            title_id: Some(title.id.clone()),
            client_id: "primary".to_string(),
            client_type: "weaver".to_string(),
            client_name: Some("Primary".to_string()),
            client_item_id: "weaver-season-pack-1".to_string(),
            release_title: "Friends".to_string(),
            reason: "download failed".to_string(),
            remove_from_client_if_configured: false,
            skip_reacquire: false,
        },
        None,
    )
    .await;

    assert_eq!(
        outcome,
        crate::acquisition_workflow::FailureHandlingOutcome::RequeuedFreshSearch
    );

    let blocklist = app
        .services
        .workflow
        .blocklist_repo
        .list_for_title(&title.id, 10)
        .await
        .expect("list blocklist");
    assert_eq!(blocklist.len(), 1);
    assert_eq!(
        blocklist[0].source_title.as_deref(),
        Some("friends.s05.720p.bluray.dd5.1.x264-ntb")
    );

    app.run_acquisition_cycle_once().await;

    let searches = indexer_client.searches.lock().await.clone();
    assert!(!searches.is_empty());
    assert!(searches.iter().all(|search| search.season == Some(5)));
    assert!(searches.iter().all(|search| search.episode.is_some()));
    assert!(
        !searches
            .iter()
            .any(|search| search.season == Some(5) && search.episode.is_none())
    );
}

#[tokio::test]
async fn acquisition_cycle_submits_paperman_media_request_candidate() {
    let release_title = "Paperman.2012.720p.WEB-DL.AV1.AAC2.0-NTb";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Paperman".into(),
                sort_title: Some("Paperman".into()),
                slug: Some("paperman".into()),
                facet: MediaFacet::Movie,
                monitored: true,
                year: Some(2012),
                external_ids: vec![
                    ExternalId {
                        source: "tvdb".to_string(),
                        value: "5890".to_string(),
                    },
                    ExternalId {
                        source: "imdb".to_string(),
                        value: "tt2388725".to_string(),
                    },
                ],
                content_status: Some("Released".to_string()),
                min_availability: Some("released".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create Paperman movie");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Movie)
        .await;
    let wanted_id = Id::new().0;
    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: wanted_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: title.slug.clone(),
            title_facet: Some(MediaFacet::Movie.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: Some("Movies".to_string()),
            library_slug: Some("movies".to_string()),
            episode_id: None,
            collection_id: None,
            season_number: None,
            episode_number: None,
            media_type: "movie".to_string(),
            search_phase: "initial".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2012-11-02".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Paperman wanted item");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(&app, &[]).await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_title));
    assert_eq!(submissions[0].scope, SubmissionScope::Title);

    let decisions = wanted_items.release_decisions.lock().await.clone();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].wanted_item_id, wanted_id);
    assert_eq!(decisions[0].release_title, release_title);
    assert_eq!(decisions[0].decision_code, "eligible");
}

#[tokio::test]
async fn acquisition_cycle_submits_bluey_episode_media_request_candidate() {
    let release_title = "Bluey.S01E01.720p.WEB-DL.AV1.AAC2.0-NTb";
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(FixedReleaseIndexerClient::new(release_title));
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client.clone(),
        download_submissions.clone(),
        pending_releases,
        wanted_items.clone(),
        indexer_client,
    );

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Bluey (2018)".into(),
                sort_title: Some("Bluey".into()),
                slug: Some("bluey-2018".into()),
                facet: MediaFacet::Series,
                monitored: true,
                year: Some(2018),
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "353546".to_string(),
                }],
                content_status: Some("Continuing".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("create Bluey series");
    wanted_items
        .remember_title_facet(&title.id, MediaFacet::Series)
        .await;

    let season = Collection {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_type: CollectionType::Season,
        collection_index: "1".to_string(),
        label: Some("Season 1".to_string()),
        ordered_path: Some("S01".to_string()),
        narrative_order: None,
        first_episode_number: Some("1".to_string()),
        last_episode_number: Some("1".to_string()),
        interstitial_movie: None,
        specials_movies: vec![],
        interstitial_season_episode: None,
        monitored: true,
        created_at: Utc::now(),
    };
    app.services
        .catalog
        .shows
        .create_collection(season.clone())
        .await
        .expect("create Bluey season");

    let episode = Episode {
        id: Id::new().0,
        title_id: title.id.clone(),
        collection_id: Some(season.id.clone()),
        episode_type: EpisodeType::Standard,
        episode_number: Some("1".to_string()),
        season_number: Some("1".to_string()),
        episode_label: Some("S01E01".to_string()),
        title: Some("The Magic Xylophone".to_string()),
        air_date: Some("2018-10-01".to_string()),
        duration_seconds: Some(420),
        has_multi_audio: false,
        has_subtitle: false,
        is_filler: false,
        is_recap: false,
        absolute_number: Some("1".to_string()),
        overview: None,
        tvdb_id: Some("7214505".to_string()),
        image_url: None,
        monitored: true,
        created_at: Utc::now(),
    };
    app.services
        .catalog
        .shows
        .create_episode(episode.clone())
        .await
        .expect("create Bluey episode");

    let wanted_id = Id::new().0;
    wanted_items
        .upsert_wanted_item(&WantedItem {
            id: wanted_id.clone(),
            title_id: title.id.clone(),
            title_name: Some(title.name.clone()),
            title_slug: title.slug.clone(),
            title_facet: Some(MediaFacet::Series.as_str().to_string()),
            library_id: Some(title.library_id.clone()),
            library_name: Some("Series".to_string()),
            library_slug: Some("series".to_string()),
            episode_id: Some(episode.id.clone()),
            collection_id: Some(season.id.clone()),
            season_number: Some("1".to_string()),
            episode_number: None,
            media_type: "episode".to_string(),
            search_phase: "long_tail".to_string(),
            next_search_at: Some(Utc::now().to_rfc3339()),
            last_search_at: None,
            search_count: 0,
            baseline_date: Some("2018-10-01".to_string()),
            status: WantedStatus::Wanted,
            grabbed_release: None,
            current_score: None,
            latest_release_decision: None,
            mismatch_recovery_eligible: false,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        })
        .await
        .expect("seed Bluey wanted item");

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(&app, &[]).await;

    assert_eq!(
        download_client
            .submitted_release_titles
            .lock()
            .await
            .as_slice(),
        &[release_title.to_string()]
    );
    let submissions = download_submissions.store.lock().await.clone();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].title_id, title.id);
    assert_eq!(submissions[0].source_title.as_deref(), Some(release_title));
    assert_eq!(
        submissions[0].scope,
        SubmissionScope::Episode {
            episode_id: episode.id.clone()
        }
    );

    let decisions = wanted_items.release_decisions.lock().await.clone();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].wanted_item_id, wanted_id);
    assert_eq!(decisions[0].release_title, release_title);
    assert_eq!(decisions[0].decision_code, "eligible");
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
            library_id: None,
            library_name: None,
            library_slug: None,
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
async fn acquisition_cycle_skips_due_search_when_no_download_clients_are_enabled() {
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

    let default_client = app
        .list_download_client_configs(&user, None)
        .await
        .expect("list download client configs")
        .into_iter()
        .next()
        .expect("default download client");
    app.update_download_client_config(
        &user,
        crate::DownloadClientConfigUpdate {
            id: default_client.id.clone(),
            is_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("disable default download client");

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "No Downloader Search Gate".into(),
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
            library_id: None,
            library_name: None,
            library_slug: None,
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

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(&app, &[]).await;

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
            library_id: None,
            library_name: None,
            library_slug: None,
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
async fn rss_sync_skips_indexer_search_when_no_download_clients_are_enabled() {
    let download_client = Arc::new(StubDownloadClient::default());
    let download_submissions = Arc::new(TrackingDownloadSubmissionRepo::default());
    let pending_releases = Arc::new(TrackingPendingReleaseRepo::default());
    let wanted_items = Arc::new(TrackingWantedItemRepo::default());
    let indexer_client = Arc::new(TrackingIndexerClient::default());
    let (app, user) = bootstrap_with_acquisition_tracking_and_indexer(
        download_client,
        download_submissions,
        pending_releases,
        wanted_items,
        indexer_client.clone(),
    );

    let default_client = app
        .list_download_client_configs(&user, None)
        .await
        .expect("list download client configs")
        .into_iter()
        .next()
        .expect("default download client");
    app.update_download_client_config(
        &user,
        crate::DownloadClientConfigUpdate {
            id: default_client.id.clone(),
            is_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .expect("disable default download client");

    app.add_title(
        &user,
        NewTitle {
            name: "RSS Skip Without Downloader".into(),
            facet: MediaFacet::Movie,
            monitored: true,
            tags: vec![],
            external_ids: vec![],
            min_availability: None,
            ..Default::default()
        },
    )
    .await
    .expect("create monitored movie");

    let report = app.run_scheduled_rss_sync().await.expect("run RSS sync");

    assert!(indexer_client.searches.lock().await.is_empty());
    assert_eq!(report.releases_fetched, 0);
    assert_eq!(report.releases_matched, 0);
    assert_eq!(report.releases_grabbed, 0);
    assert_eq!(report.releases_held, 0);
}

#[tokio::test]
async fn acquisition_cycle_limits_due_work_per_title_slice() {
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
                name: "Large Series Backlog".into(),
                facet: MediaFacet::Series,
                monitored: false,
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

    let now = Utc::now();
    for season_number in 1..=12 {
        let season = app
            .services
            .catalog
            .shows
            .create_collection(Collection {
                id: Id::new().0,
                title_id: title.id.clone(),
                collection_type: CollectionType::Season,
                collection_index: season_number.to_string(),
                label: Some(format!("Season {season_number}")),
                ordered_path: None,
                narrative_order: Some(season_number.to_string()),
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
                collection_id: Some(season.id.clone()),
                episode_type: scryer_domain::EpisodeType::Standard,
                episode_number: Some("1".to_string()),
                season_number: Some(season_number.to_string()),
                episode_label: Some(format!("S{season_number:02}E01")),
                title: Some(format!("Episode {season_number}")),
                air_date: Some("2024-01-01".to_string()),
                duration_seconds: Some(1_440),
                has_multi_audio: false,
                has_subtitle: false,
                is_filler: false,
                is_recap: false,
                absolute_number: None,
                overview: None,
                tvdb_id: None,
                image_url: None,
                monitored: true,
                created_at: Utc::now(),
            })
            .await
            .expect("create episode");

        let created_at = (now + chrono::Duration::seconds(i64::from(season_number))).to_rfc3339();
        wanted_items
            .upsert_wanted_item(&WantedItem {
                id: Id::new().0,
                title_id: title.id.clone(),
                title_name: Some(title.name.clone()),
                title_slug: None,
                title_facet: None,
                library_id: None,
                library_name: None,
                library_slug: None,
                episode_id: Some(episode.id.clone()),
                collection_id: Some(season.id.clone()),
                season_number: Some(season_number.to_string()),
                episode_number: Some("1".to_string()),
                media_type: "episode".to_string(),
                search_phase: "initial".to_string(),
                next_search_at: Some(now.to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: Some("2024-01-01".to_string()),
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
                latest_release_decision: None,
                mismatch_recovery_eligible: false,
                created_at: created_at.clone(),
                updated_at: created_at,
            })
            .await
            .expect("seed due episode wanted item");
    }

    crate::acquisition_workflow::process_due_wanted_items_with_blocked_facets(&app, &[]).await;

    let store = wanted_items.store.lock().await.clone();
    let processed = store
        .iter()
        .filter(|item| item.search_count > 0 && item.last_search_at.is_some())
        .count();
    let deferred = store
        .iter()
        .filter(|item| item.search_count == 0 && item.last_search_at.is_none())
        .count();

    assert_eq!(processed, 10);
    assert_eq!(deferred, 2);
    assert!(indexer_client.searches.lock().await.len() >= 10);
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
            image_url: None,
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
            library_id: None,
            library_name: None,
            library_slug: None,
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
            image_url: None,
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
            library_id: None,
            library_name: None,
            library_slug: None,
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
        .trigger_title_mismatch_recovery_search(&user, &title.id)
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
        library_id: None,
        library_name: None,
        library_slug: None,
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
            library_id: scryer_domain::default_library_id_for_facet(&MediaFacet::Anime),
            monitored: false,
            tags: vec!["scryer:monitor-type:none".into()],
            external_ids: vec![],
            created_by: Some(user.id.clone()),
            created_at: Utc::now(),
            year: None,
            overview: None,
            poster_url: None,
            poster_source_url: None,
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
            image_url: None,
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
            image_url: None,
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
                library_id: None,
                library_name: None,
                library_slug: None,
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
async fn update_user_library_permissions_changes_grants() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "editor",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    let updated = app
        .set_user_library_permissions(&user, &created.id, grants)
        .await
        .expect("update permissions");

    let authorization = app
        .load_user_authorization(&updated)
        .await
        .expect("load authorization");
    assert!(
        authorization.has_any_library_permission(scryer_domain::LibraryPermission::ManageTitles)
    );
}

#[tokio::test]
async fn update_user_password_is_hashed() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
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
async fn create_user_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let result = create_user_with_permissions(
        &app,
        &user,
        "short-password-user",
        "1234567",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_user_password_rejects_password_shorter_than_minimum() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "password-reset-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .set_user_password(&user, &created.id, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn self_password_change_is_hashed() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
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
async fn self_password_change_rejects_password_shorter_than_minimum() {
    let (app, admin) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &admin,
        "self-short-password-user",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");

    let result = app
        .change_own_password(&created, "1234567".to_string(), "before-pass".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn set_initial_own_password_rejects_password_shorter_than_minimum() {
    let (app, _) = bootstrap();
    let user =
        test_user_with_app_permissions("initial-short-password-user", AppPermissionMask::NONE);
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create passwordless user");

    let result = app
        .set_initial_own_password(&user, "1234567".to_string())
        .await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "password must be at least 8 characters");
        }
        Err(error) => panic!("expected password-length validation error, got {error}"),
        Ok(_) => panic!("expected password-length validation error"),
    }
}

#[tokio::test]
async fn delete_other_user_removes_user() {
    let (app, user) = bootstrap();

    let created = create_user_with_permissions(
        &app,
        &user,
        "removable",
        "password123",
        vec![TestPermissionPreset::CatalogView],
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

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let events = title_updated_events(&app, &title.id).await;
    assert_eq!(events.len(), 1);
    assert!(events.iter().all(|event| event.actor_user_id.is_none()));

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("4242".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![],
        }],
    )
    .await;

    let reapplied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("reapply monitor snapshot");

    assert!(reapplied);
    let replay_events = title_updated_events(&app, &title.id).await;
    assert_eq!(replay_events.len(), 1);
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
    app.set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("disable episode");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("5150".to_string()),
            path: None,
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
    )
    .await;

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
async fn external_import_monitor_snapshot_emits_title_updated_for_child_only_changes() {
    let (app, user) = bootstrap();
    let snapshots = Arc::new(MockExternalImportMonitorSnapshotRepo::default());
    let app = app.with_test_overrides(|services| {
        services.with_external_import_monitor_snapshots(snapshots.clone())
    });

    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Snapshot Child Activity Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "6262".to_string(),
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
    app.set_episode_monitored(&user, &episode.id, false)
        .await
        .expect("disable episode");

    let events_before_apply = title_updated_events(&app, &title.id).await.len();

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("6262".to_string()),
            path: None,
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
    )
    .await;

    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_episode.monitored);

    let events_after_apply = title_updated_events(&app, &title.id).await;
    assert_eq!(events_after_apply.len(), events_before_apply + 1);
    assert!(
        events_after_apply
            .last()
            .expect("latest event")
            .actor_user_id
            .is_none()
    );
}

#[tokio::test]
async fn external_import_monitor_snapshot_enables_collection_for_monitored_episode_override() {
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
                name: "Snapshot Episode Override Fixture".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb".to_string(),
                    value: "7373".to_string(),
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
    app.set_collection_monitored(&user, &collection.id, false)
        .await
        .expect("disable collection");

    append_series_monitor_snapshot_chunk(
        &app,
        &user,
        MediaFacet::Series,
        vec![ExternalImportMonitorSeriesEntry {
            tvdb_id: Some("7373".to_string()),
            path: None,
            monitored: false,
            seasons: vec![],
            episodes: vec![ExternalImportMonitorEpisodeEntry {
                tvdb_id: None,
                season_number: 1,
                episode_number: 1,
                monitored: true,
            }],
        }],
    )
    .await;

    let upserts_before_apply = wanted_items.upsert_call_count();
    let applied = app
        .apply_pending_external_import_monitor_snapshot_for_facet(&MediaFacet::Series)
        .await
        .expect("apply monitor snapshot");

    assert!(applied);
    let updated_collection = app
        .get_collection(&user, &collection.id)
        .await
        .expect("get collection")
        .expect("collection exists");
    let updated_episode = app
        .get_episode(&user, &episode.id)
        .await
        .expect("get episode")
        .expect("episode exists");
    assert!(updated_collection.monitored);
    assert!(updated_episode.monitored);

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
async fn series_hydration_persists_and_clears_episode_image_url() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Still Frames".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec![],
                external_ids: vec![ExternalId {
                    source: "tvdb_id".into(),
                    value: "880088".into(),
                }],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 880_001,
        number: 1,
        label: "Season 1".into(),
        episode_type: "official".into(),
    }];
    let mut episodes = vec![EpisodeMetadata {
        tvdb_id: 880_101,
        episode_number: 1,
        name: "A Still Frame".into(),
        aired: "2026-01-01".into(),
        runtime_minutes: 24,
        is_filler: false,
        is_recap: false,
        overview: "A frame is captured.".into(),
        absolute_number: "1".into(),
        season_number: 1,
        image_url: " https://image.tmdb.org/t/p/original/still-a.jpg ".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let collection = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections")
        .into_iter()
        .next()
        .expect("collection created");
    let hydrated = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes");
    assert_eq!(
        hydrated[0].image_url.as_deref(),
        Some("https://image.tmdb.org/t/p/original/still-a.jpg")
    );

    episodes[0].image_url = "https://image.tmdb.org/t/p/original/still-b.jpg".into();
    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let updated = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes after image update");
    assert_eq!(
        updated[0].image_url.as_deref(),
        Some("https://image.tmdb.org/t/p/original/still-b.jpg")
    );

    episodes[0].image_url = "not-a-url".into();
    app.create_series_seasons_and_episodes(&title, &seasons, &episodes, &[], &[])
        .await;
    let cleared = app
        .list_episodes(&user, &collection.id)
        .await
        .expect("list episodes after image clear");
    assert_eq!(cleared[0].image_url, None);
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
                name: "Blade Summit".into(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
        overview: "Blade Summit: Ember Rail".into(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
async fn new_regular_season_without_episodes_is_monitored_when_title_is_monitored() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Future Season Show".into(),
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

    let seasons = vec![SeasonMetadata {
        tvdb_id: 92,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert!(
        season.monitored,
        "new regular seasons should auto-monitor for monitored titles even before episodes exist"
    );
}

#[tokio::test]
async fn new_regular_season_without_episodes_is_not_monitored_when_monitor_type_is_none() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Manual Season Show".into(),
                facet: MediaFacet::Series,
                monitored: true,
                tags: vec!["scryer:monitor-type:none".into()],
                external_ids: vec![],
                min_availability: None,
                ..Default::default()
            },
        )
        .await
        .expect("create title");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 93,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert!(
        !season.monitored,
        "monitor-type:none should keep new empty regular seasons unmonitored"
    );
}

#[tokio::test]
async fn rehydrating_existing_regular_season_preserves_manual_unmonitored_state() {
    let (app, user) = bootstrap();
    let title = app
        .add_title(
            &user,
            NewTitle {
                name: "Existing Season Show".into(),
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

    let existing_collection = app
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
            first_episode_number: None,
            last_episode_number: None,
            interstitial_movie: None,
            specials_movies: vec![],
            interstitial_season_episode: None,
            monitored: false,
            created_at: Utc::now(),
        })
        .await
        .expect("seed existing season collection");

    let seasons = vec![SeasonMetadata {
        tvdb_id: 94,
        number: 2,
        label: "Season 2".into(),
        episode_type: "official".into(),
    }];

    app.create_series_seasons_and_episodes(&title, &seasons, &[], &[], &[])
        .await;

    let collections = app
        .list_collections(&user, &title.id)
        .await
        .expect("list collections");
    let season = collections
        .iter()
        .find(|collection| {
            collection.collection_type == CollectionType::Season
                && collection.collection_index == "2"
        })
        .expect("season two collection should exist");

    assert_eq!(season.id, existing_collection.id);
    assert!(
        !season.monitored,
        "rehydration should not retroactively flip existing manually unmonitored seasons"
    );
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
        image_url: String::new(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
                name: "Stoneguard".into(),
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
            image_url: String::new(),
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
            image_url: String::new(),
        },
    ];

    let anime_movies = vec![
        AnimeMovie {
            movie_tvdb_id: Some(379088),
            movie_tmdb_id: Some(379088),
            movie_imdb_id: Some("tt3865768".into()),
            movie_mal_id: Some(23775),
            movie_anidb_id: None,
            name: "Stoneguard: Crimson Bow and Arrow".into(),
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
                name: "Vanguard Academy".into(),
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
            image_url: String::new(),
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
            image_url: String::new(),
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
        name: "星界学園 THE MOVIE ～二人の英雄～".into(),
        slug: "my-hero-academia-the-movie-two-heroes".into(),
        year: Some(2018),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 96,
        sort_title: "星界学園 THE MOVIE ～二人の英雄～".into(),
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
        name: "Vanguard Academy: Two Heroes".into(),
        overview: "English overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "Vanguard Academy: Two Heroes".into(),
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
        Some("Vanguard Academy: Two Heroes")
    );
    assert_eq!(
        interstitial
            .interstitial_movie
            .as_ref()
            .map(|movie| movie.name.as_str()),
        Some("Vanguard Academy: Two Heroes")
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
                name: "Stoneguard".into(),
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
        image_url: String::new(),
    }];

    let japanese_special = AnimeMovie {
        movie_tvdb_id: Some(379088),
        movie_tmdb_id: Some(379088),
        movie_imdb_id: Some("tt3865768".into()),
        movie_mal_id: Some(23775),
        movie_anidb_id: None,
        name: "石衛 前編～紅蓮の弓矢～".into(),
        slug: "crimson-bow-and-arrow".into(),
        year: Some(2014),
        content_status: "released".into(),
        overview: "日本語概要".into(),
        poster_url: "poster-ja".into(),
        language: "jpn".into(),
        runtime_minutes: 120,
        sort_title: "石衛 前編～紅蓮の弓矢～".into(),
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
        name: "Stoneguard: Crimson Bow and Arrow".into(),
        overview: "English recap overview".into(),
        poster_url: "poster-en".into(),
        language: "eng".into(),
        sort_title: "Stoneguard: Crimson Bow and Arrow".into(),
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
        "Stoneguard: Crimson Bow and Arrow"
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

#[test]
fn login_failure_delay_targets_stay_in_configured_ranges() {
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            0,
        ),
        Duration::from_millis(400),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            300,
        ),
        Duration::from_millis(700),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 0,),
        Duration::from_millis(500),
    );
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 300,),
        Duration::from_millis(800),
    );
}

#[test]
fn login_failure_delay_ranges_overlap_and_do_not_go_negative() {
    assert_eq!(
        AppUseCase::login_failure_delay_target_for_random(
            LoginFailureTimingClass::PasswordBackedLocal,
            200,
        ),
        AppUseCase::login_failure_delay_target_for_random(LoginFailureTimingClass::FastMasked, 100),
    );
    assert_eq!(
        AppUseCase::login_failure_remaining_delay_for_elapsed(
            LoginFailureTimingClass::FastMasked,
            300,
            Duration::from_millis(900),
        ),
        None,
    );
}

#[tokio::test]
async fn empty_local_login_inputs_use_masked_failure_delay() {
    let (app, _) = bootstrap();
    let started = std::time::Instant::now();

    let result = app.authenticate_credentials("", "s3cr3t!!").await;

    assert!(result.is_err());
    assert!(
        started.elapsed() >= Duration::from_millis(400),
        "empty credential failure returned before the masked delay band"
    );
}

#[tokio::test]
async fn existing_short_password_remains_valid_after_minimum_is_raised() {
    let (app, admin) = bootstrap();
    let short_password = "short7!";
    let user = User {
        id: "existing-short-password-user".to_string(),
        username: "existing_short_password".to_string(),
        password_hash: Some(
            app.hash_password(short_password)
                .expect("hash short password"),
        ),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create short-password user");

    app.update_security_settings(
        &admin,
        UpdateSecuritySettings {
            form_login_enabled: false,
            password_min_length: 12,
            skip_login_for_local_ips: false,
            totp_require_config_step_up: false,
            totp_require_local_login: false,
            totp_require_jellyfin_login: false,
        },
    )
    .await
    .expect("raise password minimum");

    let authenticated = app
        .authenticate_credentials("existing_short_password", short_password)
        .await
        .expect("authenticate existing short password");
    assert_eq!(authenticated.id, user.id);
}

#[tokio::test]
async fn existing_short_v1_password_rehashes_after_minimum_is_raised() {
    let (app, admin) = bootstrap();
    let short_password = "short7!";
    let salt = "abcdef0123456789abcdef0123456789";
    let digest = sha256_hex(format!("{salt}{short_password}"));
    let legacy_hash = format!("v1${salt}${digest}");
    let user = User {
        id: "existing-short-v1-password-user".to_string(),
        username: "existing_short_v1_password".to_string(),
        password_hash: Some(legacy_hash.clone()),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create short-password legacy user");

    app.update_security_settings(
        &admin,
        UpdateSecuritySettings {
            form_login_enabled: false,
            password_min_length: 12,
            skip_login_for_local_ips: false,
            totp_require_config_step_up: false,
            totp_require_local_login: false,
            totp_require_jellyfin_login: false,
        },
    )
    .await
    .expect("raise password minimum");

    let authenticated = app
        .authenticate_credentials("existing_short_v1_password", short_password)
        .await
        .expect("authenticate existing short v1 password");
    assert!(
        authenticated
            .password_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("v2$"))
    );

    let stored = app
        .services
        .identity
        .users
        .get_by_id(&user.id)
        .await
        .expect("load migrated user")
        .expect("migrated user present");
    assert_ne!(stored.password_hash.as_deref(), Some(legacy_hash.as_str()));
    assert!(
        stored
            .password_hash
            .as_deref()
            .is_some_and(|hash| hash.starts_with("v2$"))
    );
}

#[tokio::test]
async fn local_password_login_requires_exact_spacing() {
    let (app, _) = bootstrap();
    let password = "  exact-pass  ";
    let user = User {
        id: "exact-spacing-login-user".to_string(),
        username: "exact_spacing_login".to_string(),
        password_hash: Some(app.hash_password(password).expect("hash spaced password")),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .expect("create spaced-password user");

    let authenticated = app
        .authenticate_credentials("exact_spacing_login", password)
        .await
        .expect("exact password should authenticate");
    assert_eq!(authenticated.id, user.id);

    let trimmed = app
        .authenticate_credentials("exact_spacing_login", password.trim())
        .await;
    assert!(trimmed.is_err(), "trimmed password must be rejected");
}

#[tokio::test]
async fn change_own_password_requires_exact_current_password_spacing() {
    let (app, _) = bootstrap();
    let old_password = "  old-pass  ";
    let user = User {
        id: "exact-current-password-user".to_string(),
        username: "exact_current_password".to_string(),
        password_hash: Some(app.hash_password(old_password).expect("hash old password")),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    let user = app
        .services
        .identity
        .users
        .create(user)
        .await
        .expect("create exact-current-password user");

    let trimmed_current = app
        .change_own_password(
            &user,
            "new-pass-1".to_string(),
            old_password.trim().to_string(),
        )
        .await;
    assert!(
        trimmed_current.is_err(),
        "trimmed current password must be rejected"
    );

    let changed = app
        .change_own_password(&user, "new-pass-1".to_string(), old_password.to_string())
        .await
        .expect("exact current password should succeed");
    assert!(
        app.validate_password(
            "new-pass-1",
            changed.password_hash.as_deref().expect("new password hash")
        )
        .expect("new password should validate")
    );
}

// ── password edge cases ───────────────────────────────────────────────────

#[test]
fn hash_password_empty_returns_error() {
    let (app, _) = bootstrap();
    assert!(app.hash_password("").is_err());
}

#[test]
fn hash_password_preserves_password_spacing() {
    let (app, _) = bootstrap();
    let password = "  P@ssw0rd  ";
    let hash = app.hash_password(password).expect("hash password");

    assert!(
        app.validate_password(password, &hash)
            .expect("exact password should validate")
    );
    assert!(
        !app.validate_password(password.trim(), &hash)
            .expect("trimmed password should be rejected")
    );
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestPermissionPreset {
    CatalogView,
    TitleManagement,
    UserManagement,
    ConfigManagement,
}

/// Derive a per-user JWT signing key (mirrors `AppUseCase::derive_jwt_key`).
fn test_derive_jwt_key(
    salt: &str,
    password_hash: &str,
    permissions: &[TestPermissionPreset],
) -> Vec<u8> {
    use aws_lc_rs::hmac;
    let app_permissions = test_app_permissions_from_presets(permissions);
    let library_grants = test_library_grants_from_presets(permissions);
    let mut app_claims = app_permissions
        .to_permissions()
        .into_iter()
        .map(AppUseCase::app_permission_claim_string)
        .map(str::to_string)
        .collect::<Vec<_>>();
    app_claims.sort();
    app_claims.dedup();
    let mut library_claims = library_grants
        .into_iter()
        .map(|grant| {
            let mut permissions = grant
                .permissions
                .to_permissions()
                .into_iter()
                .map(AppUseCase::library_permission_claim_string)
                .map(str::to_string)
                .collect::<Vec<_>>();
            permissions.sort();
            permissions.dedup();
            format!("{}:{}", grant.library_id, permissions.join(","))
        })
        .collect::<Vec<_>>();
    library_claims.sort();
    let authorization_fingerprint = sha256_hex(format!(
        "app\n{}\nlibrary\n{}",
        app_claims.join("\n"),
        library_claims.join("\n")
    ));
    let signing_material = format!("{password_hash}\n{authorization_fingerprint}");
    let hmac_key = hmac::Key::new(hmac::HMAC_SHA256, salt.as_bytes());
    hmac::sign(&hmac_key, signing_material.as_bytes())
        .as_ref()
        .to_vec()
}

const TEST_PASSWORD_HASH: &str = "v2$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQ$dGVzdGhhc2g";

fn test_app_permissions_from_presets(
    permissions: &[TestPermissionPreset],
) -> scryer_domain::AppPermissionMask {
    let mut mask = scryer_domain::AppPermissionMask::NONE;
    if permissions.contains(&TestPermissionPreset::UserManagement) {
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_USERS);
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_PERMISSIONS);
    }
    if permissions.contains(&TestPermissionPreset::ConfigManagement) {
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_SYSTEM_SETTINGS);
        mask.insert(scryer_domain::AppPermissionMask::MANAGE_CATALOG_SETTINGS);
    }
    mask
}

fn test_library_grants_from_presets(
    presets: &[TestPermissionPreset],
) -> Vec<scryer_domain::LibraryGrant> {
    let mut permissions = scryer_domain::LibraryPermissionMask::NONE;
    if presets.contains(&TestPermissionPreset::CatalogView) {
        permissions.insert(scryer_domain::LibraryPermissionMask::VIEW);
    }
    if presets.contains(&TestPermissionPreset::TitleManagement) {
        permissions.insert(scryer_domain::LibraryPermissionMask::VIEW);
        permissions.insert(scryer_domain::LibraryPermissionMask::MANAGE_TITLES);
        permissions.insert(scryer_domain::LibraryPermissionMask::RESOLVE_IMPORTS);
        permissions.insert(scryer_domain::LibraryPermissionMask::MANAGE_LIBRARY);
    }
    if permissions.is_empty() {
        return Vec::new();
    }
    [MediaFacet::Movie, MediaFacet::Series, MediaFacet::Anime]
        .into_iter()
        .map(|facet| scryer_domain::LibraryGrant {
            user_id: String::new(),
            library_id: scryer_domain::default_library_id_for_facet(&facet),
            permissions,
        })
        .collect()
}

async fn create_user_with_permissions(
    app: &AppUseCase,
    actor: &User,
    username: &str,
    password: &str,
    permissions: Vec<TestPermissionPreset>,
) -> AppResult<User> {
    app.create_user(
        actor,
        username.to_string(),
        password.to_string(),
        test_app_permissions_from_presets(&permissions),
        test_library_grants_from_presets(&permissions),
    )
    .await
}

async fn create_authenticated_user(
    app: &AppUseCase,
    admin: &User,
    username: &str,
    password: &str,
    permissions: Vec<TestPermissionPreset>,
) -> (User, User) {
    let created = create_user_with_permissions(app, admin, username, password, permissions)
        .await
        .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");
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
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let token = app.issue_access_token(&user).await.expect("issue token");
    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("authenticate token");
    assert_eq!(decoded.id, user.id);
    assert_eq!(decoded.username, user.username);
}

#[tokio::test]
async fn token_signed_without_auth_session_version_authenticates() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-no-session-version".to_string(),
        username: "jwt_no_session_version".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
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
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        auth_scope: JwtSessionScope::Full,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode token");

    let decoded = app
        .authenticate_token(&token)
        .await
        .expect("token without auth session version should authenticate");
    assert_eq!(decoded.id, user.id);
}

#[tokio::test]
async fn issue_mfa_enrollment_token_sets_enrollment_scope() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-mfa-enroll".to_string(),
        username: "jwt_mfa_enroll".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let token = app
        .issue_mfa_enrollment_token(&user)
        .await
        .expect("issue enrollment token");
    let (decoded, claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("authenticate enrollment token");

    assert_eq!(decoded.id, user.id);
    assert_eq!(claims.session_scope, JwtSessionScope::MfaEnrollment);
    assert_eq!(claims.mfa_verified_until, None);
}

#[tokio::test]
async fn legacy_token_without_scope_claim_defaults_to_full_scope() {
    #[derive(serde::Serialize)]
    struct LegacyJwtClaims {
        sub: String,
        exp: i64,
        iat: i64,
        iss: String,
        username: String,
        #[serde(rename = "appPermissions")]
        app_permissions: Vec<String>,
        #[serde(rename = "libraryPermissions")]
        library_permissions: Vec<serde_json::Value>,
        #[serde(rename = "mfaVerifiedUntil")]
        mfa_verified_until: Option<i64>,
    }

    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-legacy-scope".to_string(),
        username: "jwt_legacy_scope".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();
    let claims = LegacyJwtClaims {
        sub: user.id.clone(),
        exp: Utc::now().timestamp() + 3600,
        iat: Utc::now().timestamp(),
        iss: app.auth.issuer.clone(),
        username: user.username.clone(),
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode legacy token");

    let (decoded, token_claims) = app
        .authenticate_token_with_claims(&token)
        .await
        .expect("legacy token should authenticate");

    assert_eq!(decoded.id, user.id);
    assert_eq!(token_claims.session_scope, JwtSessionScope::Full);
}

#[tokio::test]
async fn permission_claims_survive_token_round_trip() {
    let (app, admin) = bootstrap();
    let user = create_user_with_permissions(
        &app,
        &admin,
        "permission_claims_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::UserManagement,
        ],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&user).await.expect("issue token");
    let decoded =
        jsonwebtoken::dangerous::insecure_decode::<JwtClaims>(&token).expect("token should decode");
    assert!(
        decoded
            .claims
            .app_permissions
            .contains(&"manageUsers".to_string())
    );
    assert!(
        decoded
            .claims
            .app_permissions
            .contains(&"managePermissions".to_string())
    );
    assert!(decoded.claims.library_permissions.iter().any(|grant| {
        grant.permissions.contains(&"view".to_string())
            && grant.permissions.contains(&"manageTitles".to_string())
    }));
}

#[tokio::test]
async fn release_candidate_token_round_trips_for_matching_actor_title_and_scope() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user",
        "password123",
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
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
        vec![
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
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
        vec![TestPermissionPreset::CatalogView],
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
        vec![TestPermissionPreset::CatalogView],
    )
    .await;
    let (_other_created, other_authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "release_user_4",
        "password123",
        vec![TestPermissionPreset::CatalogView],
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
        vec![TestPermissionPreset::CatalogView],
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
async fn release_candidate_token_is_invalidated_by_permission_change() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "candidate_permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
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
            "title-permission-rotate",
            &SubmissionScope::Title,
            &selection,
        )
        .await
        .expect("candidate token should issue");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    app.set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let result = app
        .verify_release_candidate_token(
            &authenticated_user,
            "title-permission-rotate",
            &SubmissionScope::Title,
            &token,
        )
        .await;
    assert!(
        result.is_err(),
        "candidate token should be rejected after permission change"
    );
}

#[tokio::test]
async fn backup_download_token_round_trips() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(&authenticated_user, "backup_20260515_abcd1234.tar.zst")
        .await
        .expect("backup download token should issue");

    app.verify_backup_download_token(
        &authenticated_user,
        "backup_20260515_abcd1234.tar.zst",
        &ticket.token,
    )
    .await
    .expect("backup download token should verify");
}

#[tokio::test]
async fn backup_download_token_rejects_tampering_and_filename_mismatch() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user_2",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(&authenticated_user, "backup_20260515_abcd1234.tar.zst")
        .await
        .expect("backup download token should issue");

    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_different.tar.zst",
            &ticket.token,
        )
        .await
        .is_err(),
        "filename mismatch should be rejected"
    );

    let tampered = format!("{}x", ticket.token);
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &tampered,
        )
        .await
        .is_err(),
        "tampered token should be rejected"
    );
}

#[tokio::test]
async fn backup_download_token_rejects_wrong_kind_and_expired_claims() {
    let (app, admin) = bootstrap();
    let (_created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_user_3",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let signing_key = app
        .backup_download_signing_key_for_actor(&authenticated_user)
        .await
        .expect("signing key should resolve");
    let now = Utc::now();
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);

    let wrong_kind = crate::types::BackupDownloadTokenClaims {
        sub: authenticated_user.id.clone(),
        exp: (now + chrono::Duration::minutes(5)).timestamp(),
        iat: now.timestamp(),
        iss: app.auth.issuer.clone(),
        kind: "wrong_backup_kind".to_string(),
        filename: "backup_20260515_abcd1234.tar.zst".to_string(),
    };
    let wrong_kind_token =
        jsonwebtoken::encode(&header, &wrong_kind, &key).expect("wrong kind token should encode");
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &wrong_kind_token,
        )
        .await
        .is_err(),
        "wrong kind token should be rejected"
    );

    let expired = crate::types::BackupDownloadTokenClaims {
        sub: authenticated_user.id.clone(),
        exp: (now - chrono::Duration::minutes(5)).timestamp(),
        iat: (now - chrono::Duration::minutes(10)).timestamp(),
        iss: app.auth.issuer.clone(),
        kind: "backup_download_v1".to_string(),
        filename: "backup_20260515_abcd1234.tar.zst".to_string(),
    };
    let expired_token =
        jsonwebtoken::encode(&header, &expired, &key).expect("expired token should encode");
    assert!(
        app.verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_abcd1234.tar.zst",
            &expired_token,
        )
        .await
        .is_err(),
        "expired token should be rejected"
    );
}

#[tokio::test]
async fn backup_download_token_is_invalidated_by_permission_change() {
    let (app, admin) = bootstrap();
    let (created, authenticated_user) = create_authenticated_user(
        &app,
        &admin,
        "backup_download_permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await;

    let ticket = app
        .issue_backup_download_token(
            &authenticated_user,
            "backup_20260515_permission_rotate.tar.zst",
        )
        .await
        .expect("backup download token should issue");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    app.set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let result = app
        .verify_backup_download_token(
            &authenticated_user,
            "backup_20260515_permission_rotate.tar.zst",
            &ticket.token,
        )
        .await;
    assert!(
        result.is_err(),
        "backup download token should be rejected after permission change"
    );
}

#[tokio::test]
async fn expired_token_returns_unauthorized() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-3".to_string(),
        username: "exp_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
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
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        auth_scope: JwtSessionScope::Full,
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
        account_kind: Default::default(),
        authorization: Default::default(),
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
        app_permissions: vec![],
        library_permissions: vec![],
        mfa_verified_until: None,
        auth_scope: JwtSessionScope::Full,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let bad_token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");
    let result = app.authenticate_token(&bad_token).await;
    assert!(
        result.is_err(),
        "token with wrong issuer should be rejected"
    );
}

#[tokio::test]
async fn authenticate_token_uses_cached_signing_key_and_loads_current_user() {
    let users = Arc::new(MockUserRepo::default());
    let (app, _) = bootstrap_with_user_repo(users.clone());
    let user = User {
        id: "user-jwt-cache-1".to_string(),
        username: "cache_user".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
    };
    app.services
        .identity
        .users
        .create(user.clone())
        .await
        .unwrap();

    let token = app.issue_access_token(&user).await.expect("issue token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token");
    app.authenticate_token(&token)
        .await
        .expect("authenticate token from warm cache");

    assert_eq!(users.get_by_id_call_count(), 3);
    assert_eq!(users.list_all_call_count(), 1);
}

#[tokio::test]
async fn passkey_registration_requires_password_backed_user() {
    let users = Arc::new(MockUserRepo::default());
    let user = test_user_with_app_permissions("jellyfin_user", AppPermissionMask::NONE);
    users.create(user.clone()).await.expect("create user");

    let (mut app, _) = bootstrap_with_user_repo(users);
    let origin = url::Url::parse("https://scryer.test").expect("valid WebAuthn origin");
    let webauthn = webauthn_rs::WebauthnBuilder::new("scryer.test", &origin)
        .expect("valid WebAuthn builder")
        .build()
        .expect("valid WebAuthn runtime");
    app.webauthn = services::RuntimeFeature::enabled(Arc::new(webauthn));

    let result = app.webauthn_register_start(&user, true).await;

    match result {
        Err(AppError::Validation(message)) => {
            assert_eq!(message, "passkeys require a password-backed account");
        }
        Err(error) => panic!("expected password-backed validation error, got {error}"),
        Ok(_) => panic!("expected password-backed validation error"),
    }
}

#[tokio::test]
async fn password_change_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "pw_rotate",
        "before-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");

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
async fn permission_change_invalidates_existing_token_and_relogin_works() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "permission_rotate",
        "same-pass",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let old_token = app.issue_access_token(&created).await.expect("issue token");

    let grants = test_library_grants_from_presets(&[
        TestPermissionPreset::CatalogView,
        TestPermissionPreset::TitleManagement,
    ]);
    let updated = app
        .set_user_library_permissions(&admin, &created.id, grants)
        .await
        .expect("update permissions");

    let old_result = app.authenticate_token(&old_token).await;
    assert!(
        old_result.is_err(),
        "old token should be rejected after permission change"
    );

    let relogged = app
        .authenticate_credentials("permission_rotate", "same-pass")
        .await
        .expect("re-login after permission change");
    let new_token = app
        .issue_access_token(&relogged)
        .await
        .expect("issue refreshed token");
    let decoded = app
        .authenticate_token(&new_token)
        .await
        .expect("authenticate refreshed token");

    assert_eq!(decoded.id, updated.id);
    let authorization = app
        .load_user_authorization(&decoded)
        .await
        .expect("load authorization");
    assert!(
        authorization.has_any_library_permission(scryer_domain::LibraryPermission::ManageTitles)
    );
}

#[tokio::test]
async fn deleting_user_invalidates_existing_token_immediately() {
    let (app, admin) = bootstrap();
    let created = create_user_with_permissions(
        &app,
        &admin,
        "gone_user",
        "password123",
        vec![TestPermissionPreset::CatalogView],
    )
    .await
    .expect("create user");
    let token = app.issue_access_token(&created).await.expect("issue token");

    app.delete_user(&admin, &created.id)
        .await
        .expect("delete user");

    let result = app.authenticate_token(&token).await;
    assert!(result.is_err(), "deleted user token should be rejected");
}

#[test]
fn jwt_key_derivation_is_stable_across_permission_order() {
    let (app, _) = bootstrap();
    let key_a = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &[
            TestPermissionPreset::TitleManagement,
            TestPermissionPreset::CatalogView,
        ],
    );
    let key_b = test_derive_jwt_key(
        &app.auth.jwt_signing_salt,
        TEST_PASSWORD_HASH,
        &[
            TestPermissionPreset::CatalogView,
            TestPermissionPreset::TitleManagement,
        ],
    );

    assert_eq!(key_a, key_b);
}

#[tokio::test]
async fn token_permission_claims_do_not_override_database_authorization() {
    let (app, _) = bootstrap();
    let user = User {
        id: "user-jwt-malformed".to_string(),
        username: "jwt_claims".to_string(),
        password_hash: Some(TEST_PASSWORD_HASH.to_string()),
        account_kind: Default::default(),
        authorization: Default::default(),
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
        app_permissions: vec!["manageSystemSettings".to_string()],
        library_permissions: vec![],
        mfa_verified_until: None,
        auth_scope: JwtSessionScope::Full,
    };
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
    let signing_key = test_derive_jwt_key(&app.auth.jwt_signing_salt, TEST_PASSWORD_HASH, &[]);
    let key = jsonwebtoken::EncodingKey::from_secret(&signing_key);
    let token = jsonwebtoken::encode(&header, &claims, &key).expect("encode");

    let authenticated = app
        .authenticate_token(&token)
        .await
        .expect("token identity should authenticate from DB permissions");
    assert_eq!(authenticated.id, user.id);
}
