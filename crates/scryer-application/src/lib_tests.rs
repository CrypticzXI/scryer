use super::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;

#[derive(Default)]
struct MockTitleRepo {
    store: Arc<Mutex<Vec<Title>>>,
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

    async fn get_by_id(&self, id: &str) -> AppResult<Option<Title>> {
        let list = self.store.lock().await;
        Ok(list.iter().find(|title| title.id == id).cloned())
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

    async fn create(&self, title: Title) -> AppResult<Title> {
        self.store.lock().await.push(title.clone());
        Ok(title)
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

    async fn list_title_media_size_summaries(
        &self,
        _title_ids: &[String],
    ) -> AppResult<Vec<TitleMediaSizeSummary>> {
        Ok(Vec::new())
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
            }],
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

    async fn anibridge_mappings_for_episode(
        &self,
        _tvdb_id: i64,
        _season: i32,
        _episode: i32,
    ) -> AppResult<Vec<crate::library_scan::AnibridgeSourceMapping>> {
        Ok(vec![])
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

    async fn anibridge_mappings_for_episode(
        &self,
        _tvdb_id: i64,
        _season: i32,
        _episode: i32,
    ) -> AppResult<Vec<AnibridgeSourceMapping>> {
        Ok(Vec::new())
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
            .cloned()
            .collect();
        items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));

        Ok(items.into_iter().skip(offset).take(limit).collect())
    }

    async fn count_library_scan_unmatched_items(
        &self,
        facet: Option<MediaFacet>,
        scan_root: Option<&str>,
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

#[derive(Default)]
struct MockReleaseAttemptRepo;

#[async_trait]
impl ReleaseAttemptRepository for MockReleaseAttemptRepo {
    async fn record_release_attempt(
        &self,
        _title_id: Option<String>,
        _source_hint: Option<String>,
        _source_title: Option<String>,
        _outcome: ReleaseDownloadAttemptOutcome,
        _error_message: Option<String>,
        _source_password: Option<String>,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn list_failed_release_signatures(
        &self,
        _limit: usize,
    ) -> AppResult<Vec<ReleaseDownloadFailureSignature>> {
        Ok(vec![])
    }

    async fn list_failed_release_signatures_for_title(
        &self,
        _title_id: &str,
        _limit: usize,
    ) -> AppResult<Vec<TitleReleaseBlocklistEntry>> {
        Ok(vec![])
    }

    async fn get_latest_source_password(
        &self,
        _title_id: Option<&str>,
        _source_hint: Option<&str>,
        _source_title: Option<&str>,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }
}

#[derive(Default, Clone)]
struct TrackingDownloadSubmissionRepo {
    store: Arc<Mutex<Vec<DownloadSubmission>>>,
    deleted_title_ids: Arc<Mutex<Vec<String>>>,
}

#[derive(Default, Clone)]
struct TrackingWantedItemRepo {
    store: Arc<Mutex<Vec<WantedItem>>>,
    release_decisions: Arc<Mutex<Vec<ReleaseDecision>>>,
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
    ) -> AppResult<Vec<WantedItem>> {
        let now = chrono::DateTime::parse_from_rfc3339(now)
            .map(|value| value.with_timezone(&Utc))
            .map_err(|err| AppError::Repository(err.to_string()))?;
        let mut items: Vec<WantedItem> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                item.status == WantedStatus::Wanted
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

    async fn list_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<WantedItem>> {
        let items: Vec<WantedItem> = self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                status.is_none_or(|status| item.status.as_str() == status)
                    && media_type.is_none_or(|media_type| item.media_type == media_type)
                    && title_id.is_none_or(|title_id| item.title_id == title_id)
            })
            .skip(offset.max(0) as usize)
            .take(limit.max(0) as usize)
            .cloned()
            .collect();
        Ok(items)
    }

    async fn count_wanted_items(
        &self,
        status: Option<&str>,
        media_type: Option<&str>,
        title_id: Option<&str>,
    ) -> AppResult<i64> {
        Ok(self
            .store
            .lock()
            .await
            .iter()
            .filter(|item| {
                status.is_none_or(|status| item.status.as_str() == status)
                    && media_type.is_none_or(|media_type| item.media_type == media_type)
                    && title_id.is_none_or(|title_id| item.title_id == title_id)
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

        self.wanted_items
            .update_wanted_item_status(
                &commit.wanted_item_id,
                WantedStatus::Grabbed.as_str(),
                None,
                commit.last_search_at.as_deref(),
                commit.search_count,
                commit.current_score,
                Some(&commit.grabbed_release),
            )
            .await?;

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
            let is_sibling = release.wanted_item_id == commit.wanted_item_id
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
            entry.download_client_type == submission.download_client_type
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
        download_client_type: &str,
        download_client_item_id: &str,
    ) -> AppResult<Option<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .find(|entry| {
                entry.download_client_type == download_client_type
                    && entry.download_client_item_id == download_client_item_id
            })
            .cloned())
    }

    async fn list_for_title(&self, title_id: &str) -> AppResult<Vec<DownloadSubmission>> {
        let entries = self.store.lock().await;
        Ok(entries
            .iter()
            .filter(|entry| entry.title_id == title_id)
            .cloned()
            .collect())
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

    async fn delete_by_client_item_id(&self, download_client_item_id: &str) -> AppResult<()> {
        self.store
            .lock()
            .await
            .retain(|entry| entry.download_client_item_id != download_client_item_id);
        Ok(())
    }

    async fn update_tracked_state(&self, _: &str, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }

    async fn get_tracked_state(&self, _: &str, _: &str) -> AppResult<Option<String>> {
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
    deleted_items: Arc<Mutex<Vec<(String, bool)>>>,
    submitted_release_titles: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl DownloadClient for StubDownloadClient {
    async fn submit_download(
        &self,
        request: &DownloadClientAddRequest,
    ) -> AppResult<DownloadGrabResult> {
        self.submitted_release_titles.lock().await.push(
            request
                .release_title
                .clone()
                .unwrap_or_else(|| request.title.name.clone()),
        );
        Ok(DownloadGrabResult {
            job_id: format!("job-for-{}", request.title.id),
            client_type: "nzbget".to_string(),
        })
    }

    async fn list_queue(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Ok(self.queue_items.lock().await.clone())
    }

    async fn list_history(&self) -> AppResult<Vec<DownloadQueueItem>> {
        Ok(self.history_items.lock().await.clone())
    }

    async fn delete_queue_item(&self, id: &str, is_history: bool) -> AppResult<()> {
        self.deleted_items
            .lock()
            .await
            .push((id.to_string(), is_history));
        Ok(())
    }
}

fn bootstrap() -> (AppUseCase, User) {
    bootstrap_with_user_repo(Arc::new(MockUserRepo::default()))
}

fn bootstrap_with_user_repo(users: Arc<MockUserRepo>) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo);
    let settings = Arc::new(MockSettingsRepo);
    let quality_profiles = Arc::new(MockQualityProfileRepo);
    let download_client = Arc::new(StubDownloadClient::default());
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

fn bootstrap_with_cleanup_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo);
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

fn bootstrap_with_acquisition_tracking(
    download_client: Arc<StubDownloadClient>,
    download_submissions: Arc<TrackingDownloadSubmissionRepo>,
    pending_releases: Arc<TrackingPendingReleaseRepo>,
    wanted_items: Arc<TrackingWantedItemRepo>,
) -> (AppUseCase, User) {
    let (app, user) = bootstrap_with_cleanup_tracking(
        download_client,
        download_submissions.clone(),
        pending_releases.clone(),
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
    bootstrap_with_scan_unmatched_and_metadata_tracking(
        settings,
        library_scanner,
        unmatched_items,
        Arc::new(EmptySearchMetadataGateway),
    )
}

fn bootstrap_with_scan_unmatched_and_metadata_tracking(
    settings: Arc<StoredSettingsRepo>,
    library_scanner: Arc<MutableLibraryScanner>,
    unmatched_items: Arc<TrackingLibraryScanUnmatchedItemRepo>,
    metadata_gateway: Arc<dyn MetadataGateway>,
) -> (AppUseCase, User) {
    let titles = Arc::new(MockTitleRepo::default());
    let shows = Arc::new(MockShowRepo::default());
    let users = Arc::new(MockUserRepo::default());
    let indexer_configs = Arc::new(MockIndexerConfigRepo::default());
    let download_client_configs = Arc::new(MockDownloadClientConfigRepo::default());
    let release_attempts = Arc::new(MockReleaseAttemptRepo);
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

    (app, User::new_admin("admin"))
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

#[tokio::test]
async fn movie_full_scan_persists_and_reconciles_unmatched_items() {
    let settings = Arc::new(StoredSettingsRepo::default());
    settings
        .set_value(SETTINGS_SCOPE_MEDIA, "movies.path", "/library")
        .await;
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) = bootstrap_with_scan_unmatched_tracking(
        settings,
        library_scanner.clone(),
        unmatched_items.clone(),
    );

    library_scanner
        .set_library_files(vec![build_test_library_file(
            "/library/Unknown.One.2020.1080p.WEB-DL.mkv",
        )])
        .await;
    let first_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("first movie scan");
    assert_eq!(first_summary.scanned, 1);
    assert_eq!(first_summary.unmatched, 1);

    let first_items = unmatched_items.items().await;
    assert_eq!(first_items.len(), 1);
    assert_eq!(first_items[0].facet, MediaFacet::Movie);
    assert_eq!(
        first_items[0].item_path,
        "/library/Unknown.One.2020.1080p.WEB-DL.mkv"
    );
    let first_session_id = first_items[0].scan_session_id.clone();

    library_scanner
        .set_library_files(vec![build_test_library_file(
            "/library/Unknown.One.2020.1080p.WEB-DL.mkv",
        )])
        .await;
    let second_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("second movie scan");
    assert_eq!(second_summary.unmatched, 1);

    let second_items = unmatched_items.items().await;
    assert_eq!(second_items.len(), 1);
    assert_ne!(second_items[0].scan_session_id, first_session_id);

    library_scanner
        .set_library_files(vec![build_test_library_file(
            "/library/Unknown.Two.2021.2160p.BluRay.mkv",
        )])
        .await;
    let third_summary = app
        .scan_library(&user, MediaFacet::Movie)
        .await
        .expect("third movie scan");
    assert_eq!(third_summary.scanned, 1);
    assert_eq!(third_summary.unmatched, 1);

    let third_items = unmatched_items.items().await;
    assert_eq!(third_items.len(), 1);
    assert_eq!(
        third_items[0].item_path,
        "/library/Unknown.Two.2021.2160p.BluRay.mkv"
    );
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
        Arc::new(MockMetadataGateway {
            movies: HashMap::from([(
                123_456,
                MovieMetadata {
                    tvdb_id: 123_456,
                    name: "Existing Movie".into(),
                    slug: "existing-movie".into(),
                    year: Some(2020),
                    content_status: "Released".into(),
                    overview: "Existing overview".into(),
                    poster_url: "https://example.com/poster.jpg".into(),
                    banner_url: None,
                    background_url: None,
                    language: "eng".into(),
                    runtime_minutes: 98,
                    sort_title: "Existing Movie".into(),
                    imdb_id: "tt0654321".into(),
                    anidb_id: None,
                    genres: vec!["Drama".into()],
                    studio: "Existing Studio".into(),
                    tmdb_release_date: Some("2020-01-01".into()),
                },
            )]),
        }),
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
async fn pending_import_counts_and_items_are_facet_scoped() {
    let settings = Arc::new(StoredSettingsRepo::default());
    let library_scanner = Arc::new(MutableLibraryScanner::default());
    let unmatched_items = Arc::new(TrackingLibraryScanUnmatchedItemRepo::default());
    let (app, user) =
        bootstrap_with_scan_unmatched_tracking(settings, library_scanner, unmatched_items.clone());

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
    unmatched_items
        .upsert_library_scan_unmatched_item(&build_test_unmatched_item(
            "series-1",
            MediaFacet::Series,
            "/series",
            "/series/Unknown Show (2020)",
            "Unknown Show (2020)",
            "Unknown Show",
            Some(2020),
        ))
        .await
        .expect("seed series item");

    let counts = app
        .pending_import_counts(&user)
        .await
        .expect("pending import counts");
    assert_eq!(counts.movie, 1);
    assert_eq!(counts.series, 1);
    assert_eq!(counts.anime, 0);

    let movie_items = app
        .pending_imports(&user, MediaFacet::Movie, 50, 0)
        .await
        .expect("movie pending imports");
    assert_eq!(movie_items.total, 1);
    assert_eq!(movie_items.items.len(), 1);
    assert_eq!(movie_items.items[0].display_name, "Unknown Movie");
    assert_eq!(movie_items.items[0].path, "/movies/Unknown.Movie.2020.mkv");
    assert_eq!(movie_items.items[0].folder_path, None);

    let series_items = app
        .pending_imports(&user, MediaFacet::Series, 50, 0)
        .await
        .expect("series pending imports");
    assert_eq!(series_items.total, 1);
    assert_eq!(series_items.items.len(), 1);
    assert_eq!(
        series_items.items[0].folder_path.as_deref(),
        Some("/series/Unknown Show (2020)")
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
    .expect("create tv");

    let tvs = app
        .list_titles(&user, Some(MediaFacet::Series), None)
        .await
        .expect("list titles");

    assert!(tvs.iter().all(|item| item.facet == MediaFacet::Series));
}

#[tokio::test]
async fn search_indexer_requires_query() {
    let (app, user) = bootstrap();

    let result = app
        .search_indexers(
            &user,
            IndexerSearchRequest {
                query: "   ".into(),
                imdb_id: None,
                tvdb_id: None,
                anidb_id: None,
                category: None,
            },
        )
        .await;
    assert!(result.is_err());
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
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "queue-fallback".to_string(),
            source_title: Some(created.name.clone()),
            collection_id: None,
        })
        .await
        .expect("record submission");

    *download_client.queue_items.lock().await = vec![
        DownloadQueueItem {
            id: "queue-direct".to_string(),
            title_id: Some(created.id.clone()),
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
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: true,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-fallback".to_string(),
            title_id: None,
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
            import_error_message: None,
            imported_at: None,
            is_scryer_origin: false,
            tracked_state: None,
            tracked_status: None,
            tracked_status_messages: Vec::new(),
            tracked_match_type: None,
        },
        DownloadQueueItem {
            id: "queue-unrelated".to_string(),
            title_id: None,
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
            import_error_message: None,
            imported_at: None,
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
            download_client_type: "sabnzbd".to_string(),
            download_client_item_id: "foreign-stub".to_string(),
            source_title: Some("Foreign Download".to_string()),
            collection_id: None,
        })
        .await
        .expect("record stub submission");

    *download_client.queue_items.lock().await = vec![DownloadQueueItem {
        id: "foreign-stub".to_string(),
        title_id: None,
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
        import_error_message: None,
        imported_at: None,
        is_scryer_origin: false,
        tracked_state: None,
        tracked_status: None,
        tracked_status_messages: Vec::new(),
        tracked_match_type: None,
    }];

    let items = app
        .list_download_queue(&user, true, false)
        .await
        .expect("list queue");

    assert_eq!(items.len(), 1);
    assert!(!items[0].is_scryer_origin);
    assert!(items[0].title_id.is_none());
    assert!(items[0].facet.is_none());
}

fn failed_history_item(download_client_item_id: &str, title_name: &str) -> DownloadQueueItem {
    DownloadQueueItem {
        id: download_client_item_id.to_string(),
        title_id: None,
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
        import_error_message: None,
        imported_at: None,
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
        episode_id: None,
        collection_id: None,
        season_number: None,
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
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            collection_id: None,
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
        episode_id: None,
        collection_id: None,
        season_number: None,
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
            download_client_type: "nzbget".to_string(),
            download_client_item_id: "failed-job".to_string(),
            source_title: Some("Failed.Release.1080p.WEB-DL".to_string()),
            collection_id: None,
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
        episode_id: None,
        collection_id: None,
        season_number: None,
        media_type: "movie".to_string(),
        search_phase: "initial".to_string(),
        next_search_at: None,
        last_search_at: None,
        search_count: 0,
        baseline_date: None,
        status: WantedStatus::Wanted,
        grabbed_release: None,
        current_score: None,
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
                episode_id: Some(episode_id.clone()),
                collection_id: None,
                season_number: Some("1".to_string()),
                media_type: "episode".to_string(),
                search_phase: "primary".to_string(),
                next_search_at: Some(Utc::now().to_rfc3339()),
                last_search_at: None,
                search_count: 0,
                baseline_date: None,
                status: WantedStatus::Wanted,
                grabbed_release: None,
                current_score: None,
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
        .list_wanted_items(None, None, Some(&title.id), 50, 0)
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
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        thetvdb_id: Some(348545),
        themoviedb_id: Some(438759),
        alt_tvdb_id: Some(131_963),
        thetvdb_season: Some(0),
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
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        thetvdb_id: Some(361218),
        themoviedb_id: None,
        alt_tvdb_id: None,
        thetvdb_season: Some(0),
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
        anilist_id: None,
        anidb_id: None,
        kitsu_id: None,
        thetvdb_id: Some(305074),
        themoviedb_id: Some(505262),
        alt_tvdb_id: Some(149921),
        thetvdb_season: Some(0),
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
